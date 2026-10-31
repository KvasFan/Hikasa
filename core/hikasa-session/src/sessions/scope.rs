//! Effective settings of a session and the scope selector (D-82, ARCH-040/041, SES-021).
//!
//! `rules = defaults(type)` → the parent's `[[override]]` with `scope = "type"` for the child's type → the
//! parent's `[[override]]` with `scope = "pair"` matching the child's relative paths → the child's own changes
//! (what its rules change against the defaults).

use toml::{Table, Value};

use super::file::{Override, OverrideScope, Session, SessionRules};
use crate::api::ApiError;
use crate::settings::tree;

/// Where a change of rules made in the Session Settings dialog is stored (SES-021).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// This view only (nothing but the view's own session).
    View,
    /// The type's defaults (`_defaults/<type>.toml`, rules only).
    Defaults,
    /// These files within the parent session (a `pair` override).
    Pair,
    /// All files of this type within the parent session (a `type` override).
    Type,
}

/// Which files [`apply_scope`] changed and must be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScopeWrites {
    /// The view's own session.
    pub view: bool,
    /// `_defaults/<type>.toml`.
    pub defaults: bool,
    /// The parent session.
    pub parent: bool,
    /// The parent is locked: the override is kept in memory only (SES-004).
    pub parent_in_memory_only: bool,
}

/// The effective rules of `child` (D-82); `parent` is its parent session, if any.
pub fn effective_rules(
    defaults: &Session,
    parent: Option<&Session>,
    child: &Session,
) -> SessionRules {
    let base = defaults.rules.to_value();
    let mut value = base.clone();
    if let (Some(parent), Some(link)) = (parent.filter(|p| p.kind.is_folder()), &child.parent) {
        for o in parent
            .overrides
            .iter()
            .filter(|o| o.scope == OverrideScope::Type && o.kind == child.kind)
        {
            tree::overlay(&mut value, &Value::Table(o.rules.clone()));
        }
        for o in parent.overrides.iter().filter(|o| {
            o.scope == OverrideScope::Pair
                && o.kind == child.kind
                && o.left == link.left
                && o.right == link.right
        }) {
            tree::overlay(&mut value, &Value::Table(o.rules.clone()));
        }
    }
    let own = tree::sparse(&child.rules.to_value(), &base);
    tree::overlay(&mut value, &own);
    SessionRules::from_value(&defaults.rules, value).unwrap_or_else(|_| child.rules.clone())
}

/// Applies a change of rules with a scope (D-82): the view always takes `change`; `Defaults` also writes the
/// defaults; `Pair`/`Type` store an override (sparse against `defaults`) in the parent, replacing one for the
/// same `(left, right, type)` or the same `type`. `Pair`/`Type` without a parent or with a parent that is not a
/// folder session is `invalid_input`; a locked parent keeps the override in memory only.
pub fn apply_scope(
    scope: Scope,
    change: &SessionRules,
    view: &mut Session,
    parent: Option<&mut Session>,
    defaults: &mut Session,
) -> Result<ScopeWrites, ApiError> {
    let mut writes = ScopeWrites {
        view: true,
        ..ScopeWrites::default()
    };
    match scope {
        Scope::View => {}
        Scope::Defaults => {
            defaults.rules = change.clone();
            writes.defaults = true;
        }
        Scope::Pair | Scope::Type => {
            let parent = parent.ok_or_else(|| {
                ApiError::InvalidArgument("this scope needs a parent session".into())
            })?;
            if !parent.kind.is_folder() {
                return Err(ApiError::InvalidArgument(format!(
                    "the parent is a {} session, not a folder session",
                    parent.kind
                )));
            }
            let (left, right) = match (scope, &view.parent) {
                (Scope::Pair, Some(link)) => (link.left.clone(), link.right.clone()),
                (Scope::Pair, None) => {
                    return Err(ApiError::InvalidArgument(
                        "the view has no parent link (left and right paths)".into(),
                    ));
                }
                _ => (String::new(), String::new()),
            };
            let rules = match tree::sparse(&change.to_value(), &defaults.rules.to_value()) {
                Value::Table(t) => t,
                _ => Table::new(),
            };
            let o = Override {
                scope: if scope == Scope::Pair {
                    OverrideScope::Pair
                } else {
                    OverrideScope::Type
                },
                left,
                right,
                kind: view.kind,
                rules,
            };
            let same = |x: &Override| {
                x.scope == o.scope
                    && x.kind == o.kind
                    && (o.scope == OverrideScope::Type || (x.left == o.left && x.right == o.right))
            };
            match parent.overrides.iter_mut().find(|x| same(x)) {
                Some(existing) => *existing = o,
                None => parent.overrides.push(o),
            }
            if parent.locked {
                writes.parent_in_memory_only = true;
            } else {
                writes.parent = true;
            }
        }
    }
    view.rules = change.clone();
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use hikasa_text::TextRules;

    use super::*;
    use crate::sessions::{ParentLink, SessionType};

    fn text(ignore_eol: bool, skew: u32) -> SessionRules {
        let mut r = TextRules::default();
        r.importance.compare_line_endings = ignore_eol;
        r.alignment.skew_tolerance = skew;
        SessionRules::Text(r)
    }

    fn child() -> Session {
        let mut c = Session::builtin(SessionType::TextCompare);
        c.parent = Some(ParentLink {
            session: "p".into(),
            left: "a.c".into(),
            right: "a.c".into(),
        });
        c
    }

    /// Every scope × (named parent, auto-saved parent, no parent) × (locked parent or not).
    #[test]
    fn the_scope_table() {
        let change = text(true, 7);
        for scope in [Scope::View, Scope::Defaults, Scope::Pair, Scope::Type] {
            for parent_kind in ["named", "autosaved", "none"] {
                for locked in [false, true] {
                    let mut view = child();
                    let mut defaults = Session::builtin(SessionType::TextCompare);
                    let mut parent = Session::builtin(SessionType::FolderCompare);
                    parent.locked = locked;
                    if parent_kind == "autosaved" {
                        parent.saved_at = Some(jiff::Timestamp::UNIX_EPOCH);
                    }
                    let p = (parent_kind != "none").then_some(&mut parent);
                    let result = apply_scope(scope, &change, &mut view, p, &mut defaults);
                    let label = format!("{scope:?} {parent_kind} locked={locked}");
                    if matches!(scope, Scope::Pair | Scope::Type) && parent_kind == "none" {
                        assert!(result.is_err(), "{label}");
                        continue;
                    }
                    let writes = result.unwrap();
                    assert!(writes.view, "{label}");
                    assert_eq!(view.rules, change, "{label}");
                    assert_eq!(writes.defaults, scope == Scope::Defaults, "{label}");
                    assert_eq!(
                        defaults.rules == change,
                        scope == Scope::Defaults,
                        "{label}"
                    );
                    let stored = matches!(scope, Scope::Pair | Scope::Type);
                    assert_eq!(writes.parent, stored && !locked, "{label}");
                    assert_eq!(writes.parent_in_memory_only, stored && locked, "{label}");
                    assert_eq!(parent.overrides.len(), usize::from(stored), "{label}");
                    if stored {
                        let o = &parent.overrides[0];
                        assert_eq!(o.kind, SessionType::TextCompare);
                        assert!(o.rules.contains_key("alignment"), "{label}: sparse");
                        assert!(!o.rules.contains_key("schema_version"), "{label}: sparse");
                        // The effective rules of another child see the override.
                        let mut other = child();
                        if scope == Scope::Type {
                            other.parent.as_mut().unwrap().left = "b.c".into();
                        }
                        let d = Session::builtin(SessionType::TextCompare);
                        assert_eq!(
                            effective_rules(&d, Some(&parent), &other),
                            change,
                            "{label}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn overlay_order_and_replacement() {
        let mut parent = Session::builtin(SessionType::FolderCompare);
        let mut defaults = Session::builtin(SessionType::TextCompare);
        let mut view = child();
        apply_scope(
            Scope::Type,
            &text(true, 1),
            &mut view,
            Some(&mut parent),
            &mut defaults,
        )
        .unwrap();
        apply_scope(
            Scope::Type,
            &text(true, 2),
            &mut view,
            Some(&mut parent),
            &mut defaults,
        )
        .unwrap();
        assert_eq!(
            parent.overrides.len(),
            1,
            "a type override replaces the previous one"
        );
        apply_scope(
            Scope::Pair,
            &text(true, 9),
            &mut view,
            Some(&mut parent),
            &mut defaults,
        )
        .unwrap();
        assert_eq!(parent.overrides.len(), 2);
        // Pair wins over type for the matching child; the child's own change wins over both. Overrides are
        // sparse: a value equal to the default does not override the type override.
        let mut plain = child();
        plain.rules = defaults.rules.clone();
        assert_eq!(
            effective_rules(&defaults, Some(&parent), &plain),
            text(true, 9)
        );
        let mut other = child();
        other.parent.as_mut().unwrap().left = "z".into();
        assert_eq!(
            effective_rules(&defaults, Some(&parent), &other),
            text(true, 2)
        );
        let mut own = child();
        own.rules = text(false, 4);
        let SessionRules::Text(r) = effective_rules(&defaults, Some(&parent), &own) else {
            panic!("text rules");
        };
        assert_eq!(r.alignment.skew_tolerance, 4);
        // No parent: the child's own rules.
        assert_eq!(effective_rules(&defaults, None, &own), text(false, 4));
        // A text parent cannot hold overrides.
        let mut text_parent = Session::builtin(SessionType::TextCompare);
        assert!(
            apply_scope(
                Scope::Pair,
                &text(true, 1),
                &mut view,
                Some(&mut text_parent),
                &mut defaults
            )
            .is_err()
        );
    }
}

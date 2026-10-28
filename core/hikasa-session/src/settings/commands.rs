//! `commands.toml` (OPT-011, UI-003): per-view customization of shortcuts, menus and toolbars, the
//! [`Shortcut`] spelling of the command reference, and shortcut conflicts.
//!
//! The effective shortcuts of a view are the catalogue defaults of its sections ([`crate::commands::view_sections`];
//! a command id listed in two of them takes the first), each replaced by the override's `shortcuts` when it has
//! one (`[]` removes them).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use toml::Table;

use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KeySegment, KindRule, ValueKind};
use crate::commands::{VIEWS, catalogue, view_sections};

settings_enum! {
    /// How a view shows its display filters (`display_filter_mode`).
    DisplayFilterMode { Dropdown = "dropdown", Favorites = "favorites", Toggles = "toggles" } default Dropdown
}

/// `commands.toml`: one table per view type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandOverrides {
    /// `[view.<view>]`, by view name (`home`, `folder-compare`, …).
    pub view: BTreeMap<String, ViewCustomization>,
}

settings_section! {
    /// `[view.<view>]`.
    ViewCustomization {
        /// Display filters as a drop-down, favourites or toggles.
        display_filter_mode: DisplayFilterMode = DisplayFilterMode::Dropdown,
        /// Large toolbar buttons.
        large_buttons: bool = false,
        /// Show every command in the menus.
        show_all_in_menu: bool = false,
        /// Show every command on the toolbar.
        show_all_on_toolbar: bool = false,
        /// `[view.<view>.command.<Id>]`, by command id.
        command: BTreeMap<String, CommandOverride> = BTreeMap::new(),
    }
}

/// `[view.<view>.command.<Id>]`: what the user changed about one command (absent fields keep the defaults).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandOverride {
    /// The shortcuts in the reference spelling; replaces the catalogue default (`[]` removes it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Vec<String>>,
    /// Shown in the menu.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menu: Option<bool>,
    /// Shown on the toolbar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolbar: Option<bool>,
}

/// Two or more commands of one view bound to the same shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShortcutConflict {
    /// The view.
    pub view: String,
    /// The shortcut (reference spelling).
    pub shortcut: String,
    /// The command ids, in catalogue order.
    pub commands: Vec<String>,
}

impl CommandOverrides {
    /// The effective shortcuts of `view`: every command of its sections with the catalogue default or the
    /// override, in catalogue order. Spellings that do not parse are left out.
    pub fn effective_shortcuts(&self, view: &str) -> Vec<(String, Vec<Shortcut>)> {
        let overrides = self.view.get(view);
        let mut out: Vec<(String, Vec<Shortcut>)> = Vec::new();
        for key in view_sections(view) {
            let Some(section) = catalogue().section(key) else {
                continue;
            };
            for entry in &section.commands {
                if out.iter().any(|(id, _)| *id == entry.id) {
                    continue;
                }
                let custom = overrides
                    .and_then(|v| v.command.get(&entry.id))
                    .and_then(|c| c.shortcuts.clone());
                let texts = custom.unwrap_or_else(|| {
                    if entry.shortcut.is_empty() {
                        Vec::new()
                    } else {
                        vec![entry.shortcut.clone()]
                    }
                });
                let shortcuts = texts
                    .iter()
                    .filter_map(|t| Shortcut::parse(t).ok())
                    .collect();
                out.push((entry.id.clone(), shortcuts));
            }
        }
        out
    }

    /// Commands of `view` sharing a shortcut without a context (UI-003), sorted by shortcut. Not a load problem:
    /// the catalogue itself binds some keys twice in one view (the GUI resolves them by focus).
    pub fn conflicts(&self, view: &str) -> Vec<ShortcutConflict> {
        let mut by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, shortcuts) in self.effective_shortcuts(view) {
            for shortcut in shortcuts.iter().filter(|s| s.context.is_none()) {
                let ids = by_key.entry(shortcut.to_string()).or_default();
                if !ids.contains(&id) {
                    ids.push(id.clone());
                }
            }
        }
        by_key
            .into_iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(shortcut, commands)| ShortcutConflict {
                view: view.to_owned(),
                shortcut,
                commands,
            })
            .collect()
    }
}

impl CommandOverrides {
    /// The [`CommandOverrides::conflicts`] of `view` that involve a command whose shortcuts this file changes:
    /// what `settings list` reports. The catalogue binds some keys twice in one view by design (Copy and Take
    /// commands of the file views; the GUI resolves them by focus), so those alone are not reported.
    pub fn user_conflicts(&self, view: &str) -> Vec<ShortcutConflict> {
        let Some(custom) = self.view.get(view) else {
            return Vec::new();
        };
        self.conflicts(view)
            .into_iter()
            .filter(|c| {
                c.commands.iter().any(|id| {
                    custom
                        .command
                        .get(id)
                        .is_some_and(|o| o.shortcuts.is_some())
                })
            })
            .collect()
    }
}

impl SettingsSchema for CommandOverrides {
    const FILE: FileId = FileId::Commands;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    /// Unknown views and command ids, and shortcuts that do not parse.
    fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let key = |segments: &[&str]| {
            super::tree::display_path(
                &segments
                    .iter()
                    .map(|s| KeySegment::Name((*s).to_owned()))
                    .collect::<Vec<_>>(),
            )
        };
        for (view, custom) in &self.view {
            if !VIEWS.contains(&view.as_str()) {
                problems.push(Problem::new(
                    key(&["view", view]),
                    format!("unknown view (one of {})", VIEWS.join(", ")),
                ));
                continue;
            }
            let sections = view_sections(view);
            for (id, command) in &custom.command {
                if catalogue().find(sections, id).is_none() {
                    problems.push(Problem::new(
                        key(&["view", view, "command", id]),
                        format!("no command `{id}` in the {view} view"),
                    ));
                    continue;
                }
                for text in command.shortcuts.iter().flatten() {
                    if let Err(e) = Shortcut::parse(text) {
                        problems.push(Problem::new(
                            key(&["view", view, "command", id, "shortcuts"]),
                            e,
                        ));
                        break;
                    }
                }
            }
        }
        problems
    }
}

/// Kinds of the keys of `commands.toml`.
pub(crate) const RULES: &[KindRule] = &[
    KindRule {
        pattern: "view.*.display_filter_mode",
        kind: ValueKind::Enum(DisplayFilterMode::ALL),
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*.command.*.shortcuts",
        kind: ValueKind::List,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*.command.*.menu",
        kind: ValueKind::Bool,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*.command.*.toolbar",
        kind: ValueKind::Bool,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*.command.*",
        kind: ValueKind::Table,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*.command",
        kind: ValueKind::Table,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*.*",
        kind: ValueKind::Bool,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "view.*",
        kind: ValueKind::Table,
        opt: Some("OPT-011"),
        priority: Some("P0"),
    },
];

/// The defaults of the entries of the maps of `commands.toml` (pattern → default table).
pub(crate) fn templates() -> Vec<(&'static str, toml::Value)> {
    vec![
        (
            "view.*",
            super::file::full_table(&ViewCustomization::default()).into(),
        ),
        ("view.*.command.*", toml::Value::Table(Table::new())),
    ]
}

/// The named keys of the reference spelling, in canonical case.
const NAMED_KEYS: &[&str] = &[
    "Enter", "Tab", "Esc", "Space", "BkSp", "Del", "Ins", "Home", "End", "PgUp", "PgDn", "Up",
    "Down", "Left", "Right",
];

/// A keyboard shortcut in the reference spelling (UI-003): `Shift+Ctrl+Alt+Cmd+Key`, optionally followed by a
/// context in parentheses (`Enter (in a path field)`), which is kept as text and ignored for conflicts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shortcut {
    /// Shift.
    pub shift: bool,
    /// Ctrl.
    pub ctrl: bool,
    /// Alt.
    pub alt: bool,
    /// Cmd (macOS command key).
    pub cmd: bool,
    /// The key in canonical spelling (`A`, `7`, `F5`, `Num +`, `PgDn`, `/`).
    pub key: String,
    /// The context text without parentheses.
    pub context: Option<String>,
}

impl Shortcut {
    /// Parses the reference spelling: modifiers `Shift+`, `Ctrl+`, `Alt+`, `Cmd+` in any order and any case,
    /// then one key (a letter, a digit, `F1`…`F24`, `Num 0`…`Num 9`, `Num +`, `Num -`, `Num *`, `Num /`, a
    /// named key or one punctuation character), then an optional ` (context)`.
    pub fn parse(text: &str) -> Result<Shortcut, String> {
        let bad = |why: &str| format!("{text:?} is not a shortcut: {why}");
        let (mut rest, context) = match text.strip_suffix(')').and_then(|t| t.rsplit_once(" (")) {
            Some((head, context)) if !context.is_empty() => (head, Some(context.to_owned())),
            _ => (text, None),
        };
        let mut shortcut = Shortcut {
            shift: false,
            ctrl: false,
            alt: false,
            cmd: false,
            key: String::new(),
            context,
        };
        loop {
            let lower = rest.to_ascii_lowercase();
            let found = ["shift+", "ctrl+", "alt+", "cmd+"]
                .into_iter()
                .find(|m| lower.starts_with(m) && rest.len() > m.len());
            let Some(modifier) = found else {
                break;
            };
            let flag = match modifier {
                "shift+" => &mut shortcut.shift,
                "ctrl+" => &mut shortcut.ctrl,
                "alt+" => &mut shortcut.alt,
                _ => &mut shortcut.cmd,
            };
            if *flag {
                return Err(bad("a modifier is given twice"));
            }
            *flag = true;
            rest = &rest[modifier.len()..];
        }
        shortcut.key = canonical_key(rest).ok_or_else(|| bad("unknown key"))?;
        Ok(shortcut)
    }
}

/// The canonical spelling of a key name, or `None`.
fn canonical_key(key: &str) -> Option<String> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return match c {
            c if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase().to_string()),
            c if c.is_ascii_digit() || c.is_ascii_punctuation() => Some(c.to_string()),
            _ => None,
        };
    }
    if let Some(n) = key
        .strip_prefix('F')
        .or_else(|| key.strip_prefix('f'))
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=24).contains(n) && !key[1..].starts_with('0'))
    {
        return Some(format!("F{n}"));
    }
    // `get` (not slicing): a key of five bytes may end inside a multi-byte character (`Nu€`).
    if key.len() == 5
        && let (Some(head), Some(tail)) = (key.get(..4), key.get(4..))
        && head.eq_ignore_ascii_case("num ")
    {
        let c = tail.chars().next()?;
        return (c.is_ascii_digit() || "+-*/".contains(c)).then(|| format!("Num {c}"));
    }
    NAMED_KEYS
        .iter()
        .find(|named| named.eq_ignore_ascii_case(key))
        .map(|named| (*named).to_owned())
}

impl fmt::Display for Shortcut {
    /// The reference spelling: `Shift+Ctrl+Alt+Cmd+Key`, then ` (context)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.shift, "Shift+"),
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.cmd, "Cmd+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(&self.key)?;
        if let Some(context) = &self.context {
            write!(f, " ({context})")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_and_print() {
        for (text, printed) in [
            ("Ctrl+N", "Ctrl+N"),
            ("ctrl+shift+n", "Shift+Ctrl+N"),
            ("Alt+Cmd+F12", "Alt+Cmd+F12"),
            ("Ctrl+Num +", "Ctrl+Num +"),
            ("num 0", "Num 0"),
            ("Shift+Ctrl+BkSp", "Shift+Ctrl+BkSp"),
            ("pgdn", "PgDn"),
            ("Ctrl++", "Ctrl++"),
            ("Ctrl+/", "Ctrl+/"),
            ("7", "7"),
            ("Enter (in a path field)", "Enter (in a path field)"),
        ] {
            let parsed = Shortcut::parse(text).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(parsed.to_string(), printed, "{text}");
        }
        for bad in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+A",
            "F25",
            "F0",
            "F01",
            "Hyper+A",
            "Num 10",
            "AB",
            "Ctrl+é",
            "Nu€",
            "Ctrl+Nu€",
            "N€ab",
            "€",
        ] {
            assert!(Shortcut::parse(bad).is_err(), "{bad:?}");
        }
        assert!(Shortcut::parse("Enter (x)").unwrap().context.is_some());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2048))]

        /// Any text parses or is refused; nothing panics (a shortcut comes from any settings file).
        #[test]
        fn shortcut_parse_never_panics(text in "\\PC{0,12}") {
            let _ = Shortcut::parse(&text);
        }

        /// The same with the prefixes the parser slices on, followed by any characters.
        #[test]
        fn shortcut_parse_never_panics_after_prefixes(
            prefix in proptest::sample::select(vec!["", "Ctrl+", "num ", "Nu", "F", "f", "shift+Alt+"]),
            tail in "\\PC{0,4}",
        ) {
            let text = format!("{prefix}{tail}");
            let parsed = Shortcut::parse(&text);
            if let Ok(shortcut) = parsed {
                proptest::prop_assert_eq!(Shortcut::parse(&shortcut.to_string()), Ok(shortcut));
            }
        }
    }

    #[test]
    fn every_catalogue_shortcut_parses_and_round_trips() {
        for (_, entry) in catalogue().entries() {
            if entry.shortcut.is_empty() {
                continue;
            }
            let parsed = Shortcut::parse(&entry.shortcut).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(parsed.to_string(), entry.shortcut);
        }
    }

    #[test]
    fn validation_and_conflicts() {
        let text = "[view.home.command.Exit]\nshortcuts = [\"Ctrl+Q\", \"Bogus+1\"]\n\
                    [view.home.command.NoSuch]\nmenu = false\n[view.nowhere]\nlarge_buttons = true\n";
        let value: CommandOverrides = toml::from_str(text).unwrap();
        let keys: Vec<String> = value.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(
            keys,
            vec![
                "view.home.command.Exit.shortcuts",
                "view.home.command.NoSuch",
                "view.nowhere"
            ]
        );
        let mut value = CommandOverrides::default();
        let before = value.conflicts("home").len();
        let (first, second) = {
            let effective = value.effective_shortcuts("home");
            let with_key: Vec<_> = effective
                .iter()
                .filter(|(_, s)| s.iter().any(|s| s.context.is_none()))
                .collect();
            (with_key[0].clone(), with_key[1].clone())
        };
        let custom = value.view.entry("home".into()).or_default();
        custom.command.insert(
            second.0.clone(),
            CommandOverride {
                shortcuts: Some(vec![first.1[0].to_string()]),
                ..CommandOverride::default()
            },
        );
        let conflicts = value.conflicts("home");
        assert_eq!(conflicts.len(), before + 1, "{conflicts:?}");
        assert_eq!(value.user_conflicts("home").len(), 1);
        // The catalogue's own double bindings (Copy and Take in the file views) are not the user's.
        assert!(!value.conflicts("text-compare").is_empty());
        assert!(value.user_conflicts("text-compare").is_empty());
        assert!(
            conflicts
                .iter()
                .any(|c| c.commands == vec![first.0.clone(), second.0.clone()])
        );
    }
}

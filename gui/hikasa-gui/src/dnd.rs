//! Drag and drop from the OS (M2 W4a, UI-008): the targets and the rule of a drop ([`decide`]); the app core
//! applies it ([`crate::app::AppCore::drop_paths`]). The UI layer reads `raw.dropped_files` (paths only;
//! bytes-only drops are ignored) and highlights the target under `raw.hovered_files`; delivery depends on the
//! platform backend.

use std::path::PathBuf;

use hikasa_session::sessions::SessionType;

/// The message of a drop the window cannot use.
pub const DROP_ONE_OR_TWO: &str = "Drop one or two files or folders.";
/// The hint of a drop of one path on a window.
pub const DROP_ONE_MORE: &str =
    "Drop two files or two folders to compare them, or drop one on a path field.";

/// A path field that takes drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldId {
    /// Home: the left path.
    HomeLeft,
    /// Home: the right path.
    HomeRight,
}

/// Where paths were dropped.
#[allow(dead_code)] // variants for W4b/W4c (contract seams)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropTarget {
    /// A path field.
    PathField(FieldId),
    /// A type button of Home (W4b).
    HomeType(SessionType),
    /// The window (any view other than Home's targets).
    Window,
}

/// What a drop does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropOutcome {
    /// Fills path fields.
    Fill(Vec<(FieldId, String)>),
    /// Opens a comparison of two paths like the command line.
    Open([PathBuf; 2]),
    /// Shows a message in the view's message panel.
    Message(&'static str),
    /// Nothing (no path).
    Ignore,
}

/// The rule of a drop: a path field takes the first path (two paths on the left field: left and right); the
/// window opens two paths, hints at one and refuses more. Home type buttons are W4b's (here: like the window).
pub fn decide(target: DropTarget, paths: &[PathBuf]) -> DropOutcome {
    let text = |p: &PathBuf| p.to_string_lossy().into_owned();
    match (target, paths) {
        (_, []) => DropOutcome::Ignore,
        (DropTarget::PathField(FieldId::HomeLeft), [l, r, ..]) => DropOutcome::Fill(vec![
            (FieldId::HomeLeft, text(l)),
            (FieldId::HomeRight, text(r)),
        ]),
        (DropTarget::PathField(field), [first, ..]) => {
            DropOutcome::Fill(vec![(field, text(first))])
        }
        (DropTarget::Window | DropTarget::HomeType(_), [_]) => DropOutcome::Message(DROP_ONE_MORE),
        (DropTarget::Window | DropTarget::HomeType(_), [l, r]) => {
            DropOutcome::Open([l.clone(), r.clone()])
        }
        (DropTarget::Window | DropTarget::HomeType(_), _) => DropOutcome::Message(DROP_ONE_OR_TWO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_follow_the_target() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(decide(DropTarget::Window, &[]), DropOutcome::Ignore);
        assert_eq!(
            decide(
                DropTarget::PathField(FieldId::HomeRight),
                &[p("/a"), p("/b")]
            ),
            DropOutcome::Fill(vec![(FieldId::HomeRight, "/a".into())])
        );
        assert_eq!(
            decide(
                DropTarget::PathField(FieldId::HomeLeft),
                &[p("/a"), p("/b")]
            ),
            DropOutcome::Fill(vec![
                (FieldId::HomeLeft, "/a".into()),
                (FieldId::HomeRight, "/b".into())
            ])
        );
        assert_eq!(
            decide(DropTarget::Window, &[p("/a")]),
            DropOutcome::Message(DROP_ONE_MORE)
        );
        assert_eq!(
            decide(DropTarget::Window, &[p("/a"), p("/b")]),
            DropOutcome::Open([p("/a"), p("/b")])
        );
        assert_eq!(
            decide(DropTarget::Window, &[p("/a"), p("/b"), p("/c")]),
            DropOutcome::Message(DROP_ONE_OR_TWO)
        );
    }
}

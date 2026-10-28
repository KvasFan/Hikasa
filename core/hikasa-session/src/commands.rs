//! The command catalogue (ARCH-070, UI-003; M2 plan W0, Q7a): every command of every view type, with its stable
//! id, menu, caption, default shortcut (reference spelling), description, help topic and status (`implemented`
//! when the GUI executes it, `planned` otherwise).
//!
//! The data is `catalogue/commands.toml`, embedded at build time. It is the source of
//! `docs/reference/commands.md` (`cargo xtask commands-doc`, [`Catalogue::render_markdown`]). The GUI keeps its
//! presentation (toolbar labels, radio groups, availability predicates) next to its view models and checks in
//! its tests that every command it executes is an `implemented` entry here with the same default shortcut.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The embedded catalogue.
const SOURCE: &str = include_str!("../catalogue/commands.toml");

/// Newest `schema_version` of the catalogue file this build reads.
pub const CATALOGUE_SCHEMA_VERSION: u32 = 1;

/// Whether the GUI executes a command yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// The GUI executes the command.
    Implemented,
    /// Specified, not implemented yet.
    Planned,
}

impl CommandStatus {
    /// `implemented` or `planned` (the reference spelling).
    pub fn as_str(self) -> &'static str {
        match self {
            CommandStatus::Implemented => "implemented",
            CommandStatus::Planned => "planned",
        }
    }
}

/// One command of a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEntry {
    /// Stable id (`DiffNext`, `FavCompareAll`, …).
    pub id: String,
    /// Menu or category (`Session`, `Edit`, `Toggles`, …; empty for toolbar-only commands).
    #[serde(default)]
    pub menu: String,
    /// Caption (empty for toolbar, gutter or dynamic commands).
    #[serde(default)]
    pub caption: String,
    /// Default shortcut in the reference spelling (`Shift+Ctrl+N`; empty: none).
    #[serde(default)]
    pub shortcut: String,
    /// Description (the hint).
    #[serde(default)]
    pub description: String,
    /// Implemented or planned.
    pub status: CommandStatus,
}

/// One section: the commands of a view type or of a group of view types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueSection {
    /// Stable key (`global`, `folder-compare`, `text-compare`, …).
    pub key: String,
    /// Title in the reference.
    pub title: String,
    /// The help topic of the view type ([`crate::help`]); F1 on any command of the section opens it.
    pub help: String,
    /// Text printed after the section's table (empty: none).
    #[serde(default)]
    pub note: String,
    /// The commands, in reference order.
    #[serde(rename = "command", default)]
    pub commands: Vec<CommandEntry>,
}

impl CatalogueSection {
    /// The command `id` of this section.
    pub fn command(&self, id: &str) -> Option<&CommandEntry> {
        self.commands.iter().find(|c| c.id == id)
    }
}

/// The whole catalogue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    /// Version of the file layout.
    pub schema_version: u32,
    /// Title of the reference document.
    pub title: String,
    /// Introduction of the reference document.
    pub intro: String,
    /// The sections, in reference order.
    #[serde(rename = "section", default)]
    pub sections: Vec<CatalogueSection>,
}

/// A catalogue file that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("command catalogue: {0}")]
pub struct CatalogueError(pub String);

/// The view types of `commands.toml` (OPT-011, D-65), in reference order.
pub const VIEWS: [&str; 8] = [
    "home",
    "folder-compare",
    "folder-sync",
    "text-compare",
    "text-edit",
    "table-compare",
    "hex-compare",
    "picture-compare",
];

/// The catalogue sections of a view type, own section first (M2 W1; moved here from the GUI). An unknown view has
/// none.
pub fn view_sections(view: &str) -> &'static [&'static str] {
    match view {
        "home" => &["home", "global"],
        "folder-compare" => &["folder-compare", "folder-shared", "session", "global"],
        "folder-sync" => &["folder-sync", "folder-shared", "session", "global"],
        "text-compare" | "text-edit" => &["text-compare", "file-shared", "session", "global"],
        "table-compare" => &["table-compare", "file-shared", "session", "global"],
        "hex-compare" => &["hex-compare", "file-shared", "session", "global"],
        "picture-compare" => &["picture-compare", "session", "global"],
        _ => &[],
    }
}

/// The embedded catalogue (parsed once). A catalogue that does not parse is a build defect that the tests and
/// `cargo xtask commands-doc` report; this function then returns an empty catalogue instead of panicking (T9).
pub fn catalogue() -> &'static Catalogue {
    static PARSED: OnceLock<Catalogue> = OnceLock::new();
    PARSED.get_or_init(|| {
        try_catalogue().unwrap_or_else(|e| {
            tracing::error!(target: "hikasa::commands", "{e}");
            Catalogue::default()
        })
    })
}

/// The embedded catalogue, parsed and validated (every problem of [`Catalogue::problems`] is an error).
pub fn try_catalogue() -> Result<Catalogue, CatalogueError> {
    let catalogue = parse_catalogue(SOURCE)?;
    match catalogue.problems().first() {
        Some(problem) => Err(CatalogueError(problem.clone())),
        None => Ok(catalogue),
    }
}

/// Parses a catalogue file (syntax, types and the schema version; not the cross-checks of
/// [`Catalogue::problems`]).
pub fn parse_catalogue(text: &str) -> Result<Catalogue, CatalogueError> {
    let catalogue: Catalogue = toml::from_str(text).map_err(|e| CatalogueError(e.to_string()))?;
    if catalogue.schema_version == 0 || catalogue.schema_version > CATALOGUE_SCHEMA_VERSION {
        return Err(CatalogueError(format!(
            "schema_version {} is not supported (this build reads 1 to {CATALOGUE_SCHEMA_VERSION})",
            catalogue.schema_version
        )));
    }
    Ok(catalogue)
}

/// True for a section-key character: ASCII letters, digits and `-`.
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

impl Catalogue {
    /// The section `key`.
    pub fn section(&self, key: &str) -> Option<&CatalogueSection> {
        self.sections.iter().find(|s| s.key == key)
    }

    /// The first entry `id` in the sections `keys`, in that order (a view type looks at its own section, then
    /// at the shared ones), with its section.
    pub fn find(&self, keys: &[&str], id: &str) -> Option<(&CatalogueSection, &CommandEntry)> {
        keys.iter()
            .filter_map(|k| self.section(k))
            .find_map(|s| s.command(id).map(|c| (s, c)))
    }

    /// Every entry with its section, in reference order.
    pub fn entries(&self) -> impl Iterator<Item = (&CatalogueSection, &CommandEntry)> {
        self.sections
            .iter()
            .flat_map(|s| s.commands.iter().map(move |c| (s, c)))
    }

    /// Every problem of the catalogue: malformed or duplicate section keys, duplicate ids inside a section,
    /// malformed ids, help topics that do not exist, text that is not one table cell (a `|`, a line break or
    /// surrounding spaces), and shortcuts that do not parse (UI-003, M2 W1).
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (k, section) in self.sections.iter().enumerate() {
            let key = &section.key;
            if key.is_empty() || !key.chars().all(is_key_char) {
                problems.push(format!("section key {key:?} is not a name"));
            }
            if self.sections[..k].iter().any(|s| s.key == *key) {
                problems.push(format!("section key {key:?} is used twice"));
            }
            if crate::help::topic(&section.help).is_none() {
                problems.push(format!(
                    "section {key}: help topic {:?} does not exist",
                    section.help
                ));
            }
            for (i, command) in section.commands.iter().enumerate() {
                let id = &command.id;
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
                    problems.push(format!("section {key}: id {id:?} is not a name"));
                }
                if section.commands[..i].iter().any(|c| c.id == *id) {
                    problems.push(format!("section {key}: id {id} is listed twice"));
                }
                if !command.shortcut.is_empty()
                    && let Err(e) = crate::settings::Shortcut::parse(&command.shortcut)
                {
                    problems.push(format!("section {key}: {id}: {e}"));
                }
                for (field, text) in [
                    ("menu", &command.menu),
                    ("caption", &command.caption),
                    ("shortcut", &command.shortcut),
                    ("description", &command.description),
                ] {
                    if text.contains('|') || text.contains('\n') || text.trim() != text {
                        problems.push(format!(
                            "section {key}: {id}: {field} {text:?} is not one table cell"
                        ));
                    }
                }
            }
        }
        problems
    }

    /// `docs/reference/commands.md`: the title, the introduction, then per section its table (with a Status
    /// column when `status_column`) and its note.
    pub fn render_markdown(&self, status_column: bool) -> String {
        let mut out = format!("# {}\n\n{}", self.title, self.intro);
        if !self.intro.ends_with('\n') {
            out.push('\n');
        }
        for section in &self.sections {
            out.push_str(&format!("\n## {}\n\n", section.title));
            if status_column {
                out.push_str("| Command | Menu | Caption | Shortcut | Description | Status |\n");
                out.push_str("|---|---|---|---|---|---|\n");
            } else {
                out.push_str("| Command | Menu | Caption | Shortcut | Description |\n");
                out.push_str("|---|---|---|---|---|\n");
            }
            for c in &section.commands {
                let mut cells = vec![
                    format!("`{}`", c.id),
                    c.menu.clone(),
                    c.caption.clone(),
                    c.shortcut.clone(),
                    c.description.clone(),
                ];
                if status_column {
                    cells.push(c.status.as_str().to_owned());
                }
                // Empty cells are written as `|  |`, the spelling of the hand-maintained M1 file.
                out.push_str(&format!("| {} |\n", cells.join(" | ")));
            }
            if !section.note.is_empty() {
                out.push('\n');
                out.push_str(&section.note);
                if !section.note.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_catalogue_is_valid() {
        let parsed = try_catalogue().expect("valid catalogue");
        assert_eq!(parsed.schema_version, CATALOGUE_SCHEMA_VERSION);
        assert_eq!(parsed.sections.len(), 14);
        assert!(parsed.entries().count() >= 450);
        assert_eq!(catalogue(), &parsed);
    }

    #[test]
    fn lookups_follow_the_section_order() {
        let catalogue = catalogue();
        let (section, entry) = catalogue
            .find(&["text-compare", "file-shared"], "Reload")
            .expect("Reload");
        assert_eq!(section.key, "file-shared");
        assert_eq!(entry.shortcut, "F5");
        assert_eq!(entry.status, CommandStatus::Implemented);
        let (section, _) = catalogue
            .find(&["text-compare", "file-shared"], "FavCompareAll")
            .expect("FavCompareAll");
        assert_eq!(section.key, "text-compare");
        assert!(catalogue.find(&["home"], "Reload").is_none());
        let (_, help) = catalogue.find(&["global"], "ContextHelp").expect("F1");
        assert_eq!(help.shortcut, "F1");
        assert_eq!(help.status, CommandStatus::Implemented);
    }

    #[test]
    fn problems_are_found() {
        let text = r#"
schema_version = 1
title = "T"
intro = "I"

[[section]]
key = "a b"
title = "A"
help = "no-such-topic"

[[section.command]]
id = "X"
caption = "x | y"
status = "planned"

[[section.command]]
id = "X"
shortcut = "Ctrl+Nope"
status = "implemented"
"#;
        let parsed = parse_catalogue(text).expect("parses");
        let problems = parsed.problems().join("\n");
        assert!(problems.contains("is not a name"), "{problems}");
        assert!(problems.contains("does not exist"), "{problems}");
        assert!(problems.contains("listed twice"), "{problems}");
        assert!(problems.contains("not one table cell"), "{problems}");
        assert!(problems.contains("is not a shortcut"), "{problems}");
        assert!(parse_catalogue("schema_version = 2\ntitle = \"T\"\nintro = \"I\"\n").is_err());
        assert!(parse_catalogue("schema_version = 1\ntitle = 3\nintro = \"I\"\n").is_err());
        assert!(
            parse_catalogue("schema_version = 1\ntitle = \"T\"\nintro = \"I\"\nx = 1\n").is_err()
        );
    }

    #[test]
    fn every_view_has_sections_of_the_catalogue() {
        for view in VIEWS {
            let sections = view_sections(view);
            assert!(!sections.is_empty(), "{view}");
            for key in sections {
                assert!(catalogue().section(key).is_some(), "{view}: {key}");
            }
        }
        assert!(view_sections("nowhere").is_empty());
    }

    #[test]
    fn markdown_has_one_row_per_command() {
        let catalogue = catalogue();
        let rows = |md: &str| md.lines().filter(|l| l.starts_with("| `")).count();
        for status in [true, false] {
            let md = catalogue.render_markdown(status);
            assert_eq!(rows(&md), catalogue.entries().count());
            assert!(md.starts_with(&format!("# {}\n\n", catalogue.title)));
            assert!(md.ends_with('\n') && !md.ends_with("\n\n"));
        }
        let with = catalogue.render_markdown(true);
        assert!(with.contains("| `ContextHelp` | Help | Context Sensitive Help | F1 | "));
        assert!(with.contains(" | implemented |\n") && with.contains(" | planned |\n"));
    }
}

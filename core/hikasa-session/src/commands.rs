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

use crate::settings::{CommandOverrides, Shortcut, ShortcutConflict, ViewCustomization};

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
    /// On the view's default toolbar (M2 W4a, D-109; absent: `false`). Not a column of the reference.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub toolbar: bool,
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

/// One command of a view's effective table (M2 W4a, D-109): the catalogue entry with the overrides of
/// `commands.toml` applied.
///
/// *W4 note:* the text and caption fields are merged over the view's sections: an entry that only refines
/// another section's entry of the same id (the `text-compare` display filters over `file-shared`) has no menu
/// and no caption of its own, so `menu` and `caption` are the first non-empty ones in `view_sections` order
/// (`description` likewise). `entry` stays the first entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveCommand<'a> {
    /// The section of `entry`.
    pub section: &'a str,
    /// The first catalogue entry of the id in the view's sections.
    pub entry: &'a CommandEntry,
    /// The menu (first non-empty over the sections).
    pub menu: &'a str,
    /// The caption (first non-empty over the sections).
    pub caption: &'a str,
    /// The description, the hint (first non-empty over the sections).
    pub description: &'a str,
    /// The effective shortcuts: the override's, else the parsed default (empty when none).
    pub shortcuts: Vec<Shortcut>,
    /// The catalogue default, parsed (first non-empty over the sections, like `menu`).
    pub default_shortcuts: Vec<Shortcut>,
    /// Shown in its menu (default: a menu and a caption; an override replaces it).
    pub in_menu: bool,
    /// Shown on the toolbar (default: the catalogue's `toolbar` of `entry`; an override replaces it).
    pub on_toolbar: bool,
    /// `commands.toml` changes something of this command.
    pub customized: bool,
}

impl EffectiveCommand<'_> {
    /// The command id.
    pub fn id(&self) -> &str {
        &self.entry.id
    }

    /// True for an `implemented` command (the only ones a GUI shows).
    pub fn implemented(&self) -> bool {
        self.entry.status == CommandStatus::Implemented
    }
}

/// The effective command table of one view type (M2 W4a, D-109).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewCommands<'a> {
    /// The view type (`home`, `folder-compare`, …).
    pub view: &'a str,
    /// The view's customization (defaults when `commands.toml` has none).
    pub custom: ViewCustomization,
    /// Every command of the view's sections, one per id, in `view_sections` order then catalogue order.
    pub entries: Vec<EffectiveCommand<'a>>,
}

/// The effective table of `view`: the catalogue entries of its sections ([`view_sections`]; an id listed in two
/// sections takes the first) with the shortcut, menu and toolbar overrides of `overrides` (`[]` removes the
/// shortcuts). `show_all_in_menu` / `show_all_on_toolbar` are not applied here: they force entries on for
/// display only. `planned` entries are present (for `settings`); a GUI filters them out.
pub fn effective<'a>(
    catalogue: &'a Catalogue,
    view: &'a str,
    overrides: &CommandOverrides,
) -> ViewCommands<'a> {
    let custom = overrides.view.get(view).cloned().unwrap_or_default();
    let sections: Vec<&CatalogueSection> = view_sections(view)
        .iter()
        .filter_map(|k| catalogue.section(k))
        .collect();
    let mut entries: Vec<EffectiveCommand<'a>> = Vec::new();
    for section in &sections {
        for entry in &section.commands {
            if entries.iter().any(|e| e.entry.id == entry.id) {
                continue;
            }
            let same: Vec<&'a CommandEntry> = sections
                .iter()
                .filter_map(|s| s.command(&entry.id))
                .collect();
            let first = |field: fn(&'a CommandEntry) -> &'a str| {
                same.iter()
                    .map(|e| field(e))
                    .find(|t| !t.is_empty())
                    .unwrap_or("")
            };
            let menu = first(|e| e.menu.as_str());
            let caption = first(|e| e.caption.as_str());
            let description = first(|e| e.description.as_str());
            // The shortcut is inherited like the texts: a refining entry without one takes the shared one.
            let shortcut = first(|e| e.shortcut.as_str());
            let default_shortcuts: Vec<Shortcut> = if shortcut.is_empty() {
                Vec::new()
            } else {
                Shortcut::parse(shortcut).into_iter().collect()
            };
            let over = custom.command.get(&entry.id);
            let shortcuts = match over.and_then(|o| o.shortcuts.as_ref()) {
                Some(texts) => texts
                    .iter()
                    .filter_map(|t| Shortcut::parse(t).ok())
                    .collect(),
                None => default_shortcuts.clone(),
            };
            let in_menu = over
                .and_then(|o| o.menu)
                .unwrap_or(!menu.is_empty() && !caption.is_empty());
            let on_toolbar = over.and_then(|o| o.toolbar).unwrap_or(entry.toolbar);
            entries.push(EffectiveCommand {
                section: &section.key,
                entry,
                menu,
                caption,
                description,
                shortcuts,
                default_shortcuts,
                in_menu,
                on_toolbar,
                customized: over.is_some_and(|o| *o != crate::settings::CommandOverride::default()),
            });
        }
    }
    ViewCommands {
        view,
        custom,
        entries,
    }
}

impl<'a> ViewCommands<'a> {
    /// The command `id` (the first in `view_sections` order).
    pub fn get(&self, id: &str) -> Option<&EffectiveCommand<'a>> {
        self.entries.iter().find(|e| e.entry.id == id)
    }

    /// The commands whose effective shortcuts contain `shortcut`, compared without contexts, in table order.
    pub fn with_shortcut(&self, shortcut: &Shortcut) -> Vec<&EffectiveCommand<'a>> {
        let bare = |s: &Shortcut| Shortcut {
            context: None,
            ..s.clone()
        };
        let wanted = bare(shortcut);
        self.entries
            .iter()
            .filter(|e| e.shortcuts.iter().any(|s| bare(s) == wanted))
            .collect()
    }

    /// Commands sharing a shortcut without a context, sorted by shortcut (the same as
    /// [`CommandOverrides::conflicts`] for this view's overrides).
    pub fn conflicts(&self) -> Vec<ShortcutConflict> {
        let mut by_key: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for entry in &self.entries {
            for shortcut in entry.shortcuts.iter().filter(|s| s.context.is_none()) {
                let ids = by_key.entry(shortcut.to_string()).or_default();
                if !ids.contains(&entry.entry.id) {
                    ids.push(entry.entry.id.clone());
                }
            }
        }
        by_key
            .into_iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(shortcut, commands)| ShortcutConflict {
                view: self.view.to_owned(),
                shortcut,
                commands,
            })
            .collect()
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

    /// M2 W4a (A4.3): the effective table of a view: defaults, overrides replacing, removing and adding
    /// shortcuts, menu and toolbar overrides, the merged captions, `planned` entries present.
    #[test]
    fn effective_tables_apply_the_overrides() {
        use crate::settings::{CommandOverride, CommandOverrides};
        let cat = catalogue();
        let none = CommandOverrides::default();
        let text = effective(cat, "text-compare", &none);
        assert_eq!(text.view, "text-compare");
        let reload = text.get("Reload").expect("Reload");
        assert_eq!(reload.section, "file-shared");
        assert_eq!(reload.shortcuts, vec![Shortcut::parse("F5").unwrap()]);
        assert_eq!(reload.shortcuts, reload.default_shortcuts);
        assert!(reload.in_menu && reload.on_toolbar && !reload.customized);
        // The text view's display filters refine the shared entries: own description, shared menu and caption.
        let all = text.get("FavCompareAll").expect("FavCompareAll");
        assert_eq!(all.section, "text-compare");
        assert_eq!(all.caption, "Show All");
        assert_eq!(all.menu, "Favorites");
        assert_eq!(all.description, all.entry.description);
        // … and the shared shortcut when they have none; the status is the view's own entry's.
        let exists = text.get("ExistsNext").expect("ExistsNext");
        assert_eq!(exists.section, "text-compare");
        assert!(exists.entry.shortcut.is_empty());
        assert_eq!(
            exists.default_shortcuts,
            vec![Shortcut::parse("Alt+Down").unwrap()]
        );
        assert_eq!(exists.shortcuts, exists.default_shortcuts);
        assert!(
            all.implemented(),
            "the text view's own entry is implemented"
        );
        // Planned entries are in the table, marked.
        let save = text.get("SessionClear").expect("SessionClear");
        assert!(!save.implemented());
        // Ids are unique; the order is view_sections then catalogue order.
        let ids: Vec<&str> = text.entries.iter().map(|e| e.id()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        assert_eq!(text.entries[0].section, "text-compare");
        assert_eq!(text.entries.last().map(|e| e.section), Some("global"));
        // Overrides: replace, remove, add; menu and toolbar flags.
        let mut overrides = CommandOverrides::default();
        let custom = overrides.view.entry("text-compare".into()).or_default();
        custom.command.insert(
            "Reload".into(),
            CommandOverride {
                shortcuts: Some(vec!["Ctrl+R".into(), "Shift+F5".into()]),
                menu: Some(false),
                toolbar: Some(false),
            },
        );
        custom.command.insert(
            "DiffSectionNext".into(),
            CommandOverride {
                shortcuts: Some(Vec::new()),
                ..CommandOverride::default()
            },
        );
        custom.command.insert(
            "About".into(),
            CommandOverride {
                shortcuts: Some(vec!["Ctrl+F1".into()]),
                toolbar: Some(true),
                ..CommandOverride::default()
            },
        );
        custom.show_all_in_menu = true;
        let text = effective(cat, "text-compare", &overrides);
        assert!(text.custom.show_all_in_menu);
        let reload = text.get("Reload").unwrap();
        assert_eq!(
            reload.shortcuts,
            vec![
                Shortcut::parse("Ctrl+R").unwrap(),
                Shortcut::parse("Shift+F5").unwrap()
            ]
        );
        assert_eq!(
            reload.default_shortcuts,
            vec![Shortcut::parse("F5").unwrap()]
        );
        assert!(!reload.in_menu && !reload.on_toolbar && reload.customized);
        assert!(text.get("DiffSectionNext").unwrap().shortcuts.is_empty());
        let about = text.get("About").unwrap();
        assert!(about.on_toolbar && about.in_menu);
        assert_eq!(
            text.with_shortcut(&Shortcut::parse("Ctrl+F1").unwrap())
                .iter()
                .map(|e| e.id())
                .collect::<Vec<_>>(),
            vec!["About"]
        );
        assert!(
            text.with_shortcut(&Shortcut::parse("F5").unwrap())
                .is_empty()
        );
        // Another view is untouched by the text view's overrides.
        let folder = effective(cat, "folder-compare", &overrides);
        assert_eq!(
            folder.get("About").unwrap().shortcuts,
            Vec::<Shortcut>::new()
        );
        assert!(!folder.custom.show_all_in_menu);
        // The conflicts are those of `CommandOverrides::conflicts`.
        assert_eq!(text.conflicts(), overrides.conflicts("text-compare"));
        assert_eq!(folder.conflicts(), overrides.conflicts("folder-compare"));
        // A context is kept but not part of the match.
        let home = effective(cat, "home", &none);
        let enter = home.with_shortcut(&Shortcut::parse("Enter").unwrap());
        assert_eq!(
            enter.iter().map(|e| e.id()).collect::<Vec<_>>(),
            vec!["HomeCompare", "TreeOpen"]
        );
        assert!(effective(cat, "nowhere", &none).entries.is_empty());
    }

    /// M2 W4a: the `toolbar` field parses, absent is `false`, and the reference layout does not change.
    #[test]
    fn the_toolbar_field_is_optional() {
        let text = "schema_version = 1\ntitle = \"T\"\nintro = \"I\"\n[[section]]\nkey = \"global\"\n\
                    title = \"G\"\nhelp = \"main-window\"\n[[section.command]]\nid = \"A\"\nstatus = \"planned\"\n\
                    [[section.command]]\nid = \"B\"\nstatus = \"planned\"\ntoolbar = true\n";
        let parsed = parse_catalogue(text).expect("parses");
        let section = parsed.section("global").unwrap();
        assert!(!section.command("A").unwrap().toolbar);
        assert!(section.command("B").unwrap().toolbar);
        let md = parsed.render_markdown(true);
        assert!(md.contains("| `B` |  |  |  |  | planned |"), "{md}");
        assert!(catalogue().entries().any(|(_, e)| e.toolbar));
    }
}

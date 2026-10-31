//! `openwith.toml` (OPT-012) and the pure expansion of its command lines (D-76).
//!
//! ```text
//! expand_command(template, ctx):
//!   1. split_words: whitespace separates words; "…" groups (whitespace kept); inside a group \" is a quote;
//!      every other backslash is literal (Windows paths); an unclosed group is an error
//!   2. in each word: %% → %; %<v>[1|2] → the variable of the first (no suffix, 1) or second (2) item;
//!      an unknown variable, a %…2 without a second item and a dangling % are errors
//!      %f path+name   %n name   %p folder   %x extension (no dot; "" without one)   %b name without extension
//!      %F path+name relative to the working folder   %P path relative to the item's base folder (lexical)
//!      %l the current line (1 without one)
//!   3. a path delimiter replaces both / and \ in substituted paths (never in the template's own text)
//! A substituted path is always inside one word, so it is exactly one argument whatever it contains.
//! ```

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml::Table;

use super::commands::Shortcut;
use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KindRule, ValueKind};

/// `openwith.toml`: `[[app]]` entries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenWith {
    /// The applications, in list order.
    pub app: Vec<OpenWithApp>,
}

/// The folder an application starts in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum WorkingFolder {
    /// The folder of the (first) item: `parent`.
    #[default]
    Parent,
    /// The base folder of the comparison: `base`.
    Base,
    /// A fixed folder.
    Path(String),
}

impl WorkingFolder {
    /// `parent`, `base` or the path.
    pub fn as_str(&self) -> &str {
        match self {
            WorkingFolder::Parent => "parent",
            WorkingFolder::Base => "base",
            WorkingFolder::Path(path) => path,
        }
    }
}

impl Serialize for WorkingFolder {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkingFolder {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<WorkingFolder, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(match text.as_str() {
            "parent" | "" => WorkingFolder::Parent,
            "base" => WorkingFolder::Base,
            _ => WorkingFolder::Path(text),
        })
    }
}

/// One external application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenWithApp {
    /// Description (required).
    pub description: String,
    /// Shortcut in the reference spelling (empty: none).
    pub shortcut: String,
    /// Command line with variables (required; D-76).
    pub command: String,
    /// The folder the application starts in.
    pub working_folder: WorkingFolder,
    /// `""` (native), `"/"` or `"\\"`: replaces the separators of substituted paths.
    pub path_delimiter: String,
    /// Offered for files.
    pub accepts_files: bool,
    /// Offered for folders.
    pub accepts_folders: bool,
    /// Refresh the view when the application finishes.
    pub refresh_when_finished: bool,
    /// Wait for the previous instance to finish.
    pub wait_for_previous: bool,
    /// Keys of the entry this version does not know (kept, Q11).
    #[serde(flatten)]
    pub extra: Table,
}

impl Default for OpenWithApp {
    fn default() -> OpenWithApp {
        OpenWithApp {
            description: String::new(),
            shortcut: String::new(),
            command: String::new(),
            working_folder: WorkingFolder::Parent,
            path_delimiter: String::new(),
            accepts_files: true,
            accepts_folders: false,
            refresh_when_finished: false,
            wait_for_previous: false,
            extra: Table::new(),
        }
    }
}

impl OpenWithApp {
    /// The delimiter as a character (`None`: native).
    pub fn delimiter(&self) -> Option<char> {
        self.path_delimiter.chars().next()
    }

    /// One instance per pair or per item.
    pub fn instance_mode(&self) -> InstanceMode {
        InstanceMode::of(&self.command)
    }
}

impl SettingsSchema for OpenWith {
    const FILE: FileId = FileId::OpenWith;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    /// Description and command non-empty, the command splits and names known variables, the shortcut parses, the
    /// delimiter is empty, `/` or `\`. A bad entry is a problem of the whole entry (`app.<n>`; lenient loading
    /// keeps it as it is, with a warning).
    fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (k, app) in self.app.iter().enumerate() {
            let key = format!("app.{k}");
            let problem = if app.description.trim().is_empty() {
                Some("the description is empty".to_owned())
            } else if app.command.trim().is_empty() {
                Some("the command is empty".to_owned())
            } else if let Err(e) = check_template(&app.command) {
                Some(format!("command: {e}"))
            } else if !app.shortcut.is_empty()
                && let Err(e) = Shortcut::parse(&app.shortcut)
            {
                Some(e)
            } else if !matches!(app.path_delimiter.as_str(), "" | "/" | "\\") {
                Some(format!(
                    "path_delimiter {:?} must be \"\", \"/\" or \"\\\\\"",
                    app.path_delimiter
                ))
            } else {
                None
            };
            if let Some(message) = problem {
                problems.push(Problem::new(key, message));
            }
        }
        problems
    }
}

/// One instance per item or per pair (OPT-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstanceMode {
    /// One instance per selected item.
    PerItem,
    /// One instance per pair (the command uses a `%…2` variable).
    PerPair,
}

impl InstanceMode {
    /// `PerPair` when any `%<v>2` occurs in `template`, else `PerItem`.
    pub fn of(template: &str) -> InstanceMode {
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                continue;
            }
            match chars.next() {
                Some('%') | None => {}
                Some(_) => {
                    if chars.peek() == Some(&'2') {
                        return InstanceMode::PerPair;
                    }
                }
            }
        }
        InstanceMode::PerItem
    }
}

/// One item of an expansion: an absolute path and the base folder of its side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandItem {
    /// The file or folder (absolute).
    pub path: PathBuf,
    /// The base folder of the comparison side (absolute).
    pub base: PathBuf,
}

/// What a command line is expanded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandContext {
    /// The first (or only) item.
    pub first: ExpandItem,
    /// The second item of a pair.
    pub second: Option<ExpandItem>,
    /// The current line (`%l`; 1 when none).
    pub line: Option<u64>,
    /// The working folder (`%F` is relative to it).
    pub working_folder: PathBuf,
    /// Replaces `/` and `\` in substituted paths.
    pub path_delimiter: Option<char>,
}

/// Why a command line cannot be expanded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpandError {
    /// A `"` group is not closed.
    #[error("a quoted group is not closed")]
    UnclosedQuote,
    /// `%` followed by an unknown variable.
    #[error("unknown variable %{0}")]
    UnknownVariable(String),
    /// A `%…2` variable without a second item.
    #[error("%{0}2 needs a second item")]
    MissingSecondItem(char),
    /// A `%` at the end of a word.
    #[error("a % at the end of a word (write %% for a percent sign)")]
    DanglingPercent,
}

/// Splits a command line into words (D-76 step 1).
pub fn split_words(line: &str) -> Result<Vec<String>, ExpandError> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if chars.peek() == Some(&'"') => {
                            chars.next();
                            word.push('"');
                        }
                        Some(c) => word.push(c),
                        None => return Err(ExpandError::UnclosedQuote),
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// The variables of D-76.
const VARIABLES: &str = "flnpxbFP";

/// Checks a command line without expanding it (the words split; every variable is known; no dangling `%`).
pub fn check_template(template: &str) -> Result<(), ExpandError> {
    for word in split_words(template)? {
        let mut chars = word.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                continue;
            }
            match chars.next() {
                None => return Err(ExpandError::DanglingPercent),
                Some('%') => {}
                Some(v) if VARIABLES.contains(v) => {
                    if matches!(chars.peek(), Some('1' | '2')) {
                        chars.next();
                    }
                }
                Some(v) => return Err(ExpandError::UnknownVariable(v.to_string())),
            }
        }
    }
    Ok(())
}

/// Expands a command line into the program and its arguments (D-76).
pub fn expand_command(template: &str, ctx: &ExpandContext) -> Result<Vec<String>, ExpandError> {
    split_words(template)?
        .iter()
        .map(|word| expand_word(word, ctx))
        .collect()
}

/// Expands the variables of one word.
fn expand_word(word: &str, ctx: &ExpandContext) -> Result<String, ExpandError> {
    let mut out = String::new();
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let v = match chars.next() {
            None => return Err(ExpandError::DanglingPercent),
            Some('%') => {
                out.push('%');
                continue;
            }
            Some(v) if VARIABLES.contains(v) => v,
            Some(v) => return Err(ExpandError::UnknownVariable(v.to_string())),
        };
        let item = match chars.peek() {
            Some('2') => {
                chars.next();
                ctx.second
                    .as_ref()
                    .ok_or(ExpandError::MissingSecondItem(v))?
            }
            Some('1') => {
                chars.next();
                &ctx.first
            }
            _ => &ctx.first,
        };
        let value = match v {
            'l' => {
                out.push_str(&ctx.line.unwrap_or(1).to_string());
                continue;
            }
            'f' => text(&item.path),
            'n' => name(&item.path),
            'p' => item.path.parent().map(text).unwrap_or_default(),
            'x' => split_ext(&name(&item.path)).1.to_owned(),
            'b' => split_ext(&name(&item.path)).0.to_owned(),
            'F' => text(&relative(&item.path, &ctx.working_folder)),
            _ => text(&relative(&item.path, &item.base)),
        };
        match ctx.path_delimiter {
            Some(d) => out.extend(
                value
                    .chars()
                    .map(|c| if c == '/' || c == '\\' { d } else { c }),
            ),
            None => out.push_str(&value),
        }
    }
    Ok(out)
}

/// A path as text (lossy for names that are not Unicode).
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The last component of a path.
fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A name split at its last `.` (a leading `.` starts no extension: `.bashrc` has none).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 => (&name[..at], &name[at + 1..]),
        _ => (name, ""),
    }
}

/// `path` relative to `base`, computed lexically (`.` when equal; `..` for the parts of `base` it is not in).
fn relative(path: &Path, base: &Path) -> PathBuf {
    let path: Vec<Component<'_>> = path.components().collect();
    let base: Vec<Component<'_>> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for component in &path[common..] {
        out.push(component.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

impl fmt::Display for InstanceMode {
    /// `per_item` or `per_pair`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InstanceMode::PerItem => "per_item",
            InstanceMode::PerPair => "per_pair",
        })
    }
}

/// Kinds of the keys of `openwith.toml`.
pub(crate) const RULES: &[KindRule] = &[
    rule("app.*.description", ValueKind::String),
    rule("app.*.shortcut", ValueKind::Shortcut),
    rule("app.*.command", ValueKind::String),
    rule("app.*.working_folder", ValueKind::String),
    rule("app.*.path_delimiter", ValueKind::Enum(&["", "/", "\\"])),
    rule("app.*.accepts_files", ValueKind::Bool),
    rule("app.*.accepts_folders", ValueKind::Bool),
    rule("app.*.refresh_when_finished", ValueKind::Bool),
    rule("app.*.wait_for_previous", ValueKind::Bool),
    rule("app.*", ValueKind::Table),
];

/// An OPT-012 rule.
const fn rule(pattern: &'static str, kind: ValueKind) -> KindRule {
    KindRule {
        pattern,
        kind,
        opt: Some("OPT-012"),
        priority: Some("P0"),
    }
}

/// The defaults of a new application entry.
pub(crate) fn templates() -> Vec<(&'static str, toml::Value)> {
    vec![(
        "app.*",
        super::file::full_table(&OpenWithApp::default()).into(),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, base: &str) -> ExpandItem {
        ExpandItem {
            path: PathBuf::from(path),
            base: PathBuf::from(base),
        }
    }

    fn ctx() -> ExpandContext {
        ExpandContext {
            first: item("/w/left/src/main.rs", "/w/left"),
            second: Some(item("/w/right/src/main.old.rs", "/w/right")),
            line: Some(42),
            working_folder: PathBuf::from("/w/left/src"),
            path_delimiter: None,
        }
    }

    /// The D-76 table: template, context change, expected words or error. It runs on every platform; the rows
    /// whose relative paths join components with the platform separator differ between Unix and Windows.
    #[test]
    fn expansion_table() {
        type Case = (
            &'static str,
            fn(&mut ExpandContext),
            Result<Vec<&'static str>, ExpandError>,
        );
        let none: fn(&mut ExpandContext) = |_| {};
        let cases: Vec<Case> = vec![
            ("edit %f", none, Ok(vec!["edit", "/w/left/src/main.rs"])),
            ("edit %f1", none, Ok(vec!["edit", "/w/left/src/main.rs"])),
            (
                "edit %f2",
                none,
                Ok(vec!["edit", "/w/right/src/main.old.rs"]),
            ),
            ("x %n", none, Ok(vec!["x", "main.rs"])),
            ("x %n2", none, Ok(vec!["x", "main.old.rs"])),
            ("x %p", none, Ok(vec!["x", "/w/left/src"])),
            ("x %p2", none, Ok(vec!["x", "/w/right/src"])),
            ("x %x", none, Ok(vec!["x", "rs"])),
            ("x %b2", none, Ok(vec!["x", "main.old"])),
            (
                "x %b",
                |c| c.first = item("/h/.bashrc", "/h"),
                Ok(vec!["x", ".bashrc"]),
            ),
            (
                "x %x",
                |c| c.first = item("/h/.bashrc", "/h"),
                Ok(vec!["x", ""]),
            ),
            (
                "x %x",
                |c| c.first = item("/h/Makefile", "/h"),
                Ok(vec!["x", ""]),
            ),
            ("x %F", none, Ok(vec!["x", "main.rs"])),
            (
                "x %P",
                |c| c.first = item("/w/left", "/w/left"),
                Ok(vec!["x", "."]),
            ),
            ("x +%l", none, Ok(vec!["x", "+42"])),
            ("x +%l", |c| c.line = None, Ok(vec!["x", "+1"])),
            ("x 100%%", none, Ok(vec!["x", "100%"])),
            ("x %%f", none, Ok(vec!["x", "%f"])),
            (
                "\"my editor\" --diff \"%f\" \"%f2\"",
                none,
                Ok(vec![
                    "my editor",
                    "--diff",
                    "/w/left/src/main.rs",
                    "/w/right/src/main.old.rs",
                ]),
            ),
            ("x \"a \\\"b\\\" c\"", none, Ok(vec!["x", "a \"b\" c"])),
            (
                "x --file=\"%n\"",
                |c| c.first = item("/w/a b.txt", "/w"),
                Ok(vec!["x", "--file=a b.txt"]),
            ),
            (
                "x %f",
                |c| c.first = item("/w/a b \"c\".txt", "/w"),
                Ok(vec!["x", "/w/a b \"c\".txt"]),
            ),
            ("x \"\" y", none, Ok(vec!["x", "", "y"])),
            (
                "C:\\tools\\x.exe %f",
                none,
                Ok(vec!["C:\\tools\\x.exe", "/w/left/src/main.rs"]),
            ),
            (
                "x %f",
                |c| c.path_delimiter = Some('\\'),
                Ok(vec!["x", "\\w\\left\\src\\main.rs"]),
            ),
            (
                "x/y %P2",
                |c| c.path_delimiter = Some('\\'),
                Ok(vec!["x/y", "src\\main.old.rs"]),
            ),
            (
                "x %f2",
                |c| c.second = None,
                Err(ExpandError::MissingSecondItem('f')),
            ),
            ("x %q", none, Err(ExpandError::UnknownVariable("q".into()))),
            ("x 100%", none, Err(ExpandError::DanglingPercent)),
            ("x \"open", none, Err(ExpandError::UnclosedQuote)),
        ];
        let mut cases = cases;
        if cfg!(windows) {
            cases.extend::<Vec<Case>>(vec![
                (
                    "x %F2",
                    none,
                    Ok(vec!["x", "..\\..\\right\\src\\main.old.rs"]),
                ),
                ("x %P", none, Ok(vec!["x", "src\\main.rs"])),
                ("x %P2", none, Ok(vec!["x", "src\\main.old.rs"])),
            ]);
        } else {
            cases.extend::<Vec<Case>>(vec![
                ("x %F2", none, Ok(vec!["x", "../../right/src/main.old.rs"])),
                ("x %P", none, Ok(vec!["x", "src/main.rs"])),
                ("x %P2", none, Ok(vec!["x", "src/main.old.rs"])),
            ]);
        }
        assert!(cases.len() >= 25);
        for (template, change, expected) in cases {
            let mut c = ctx();
            change(&mut c);
            let got = expand_command(template, &c);
            let expected = expected.map(|w| w.into_iter().map(String::from).collect::<Vec<_>>());
            assert_eq!(got, expected, "{template}");
        }
    }

    #[test]
    fn instance_modes_and_checks() {
        assert_eq!(InstanceMode::of("x %f %f2"), InstanceMode::PerPair);
        assert_eq!(InstanceMode::of("x %P2"), InstanceMode::PerPair);
        assert_eq!(InstanceMode::of("x %f %%2"), InstanceMode::PerItem);
        assert_eq!(InstanceMode::of("x %f1 %n"), InstanceMode::PerItem);
        assert!(check_template("x %f2 %%").is_ok());
        assert_eq!(
            check_template("x %z"),
            Err(ExpandError::UnknownVariable("z".into()))
        );
        let mut apps = OpenWith::default();
        apps.app.push(OpenWithApp {
            description: "ok".into(),
            command: "x %f".into(),
            ..OpenWithApp::default()
        });
        apps.app.push(OpenWithApp {
            description: "bad".into(),
            command: "x %z".into(),
            ..OpenWithApp::default()
        });
        apps.app.push(OpenWithApp {
            description: "".into(),
            command: "x".into(),
            ..OpenWithApp::default()
        });
        let keys: Vec<String> = apps.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(keys, vec!["app.1", "app.2"]);
    }
}

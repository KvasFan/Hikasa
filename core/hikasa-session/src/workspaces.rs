//! Workspaces (M2 W2; contract §1.2.5, D-85, SES-040…042): `workspaces/<name>.toml`, loaded and saved by the
//! D-70 engine. A workspace holds windows (geometry, active tab) and their tabs in order; each tab is a
//! reference to a named session or an inline session table, plus its opaque view layout.
//!
//! Loading closes nothing in core: [`restore`] turns a workspace into the sessions of its tabs (a reference to
//! a session that no longer exists restores a Home tab and adds warning `session_missing`); the GUI closes its
//! views and opens these (tabs load lazily).

use std::path::{Path, PathBuf};

use hikasa_core::{Warning, WarningCode};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::api::ApiError;
use crate::error::SessionError;
use crate::paths::ConfigLocation;
use crate::sessions::store::{entries, in_file};
use crate::sessions::{Session, SessionDoc, SessionName, SessionRef, SessionStore, SessionType};
use crate::settings::file::{full_table, migrate_from_zero};
use crate::settings::{FileId, LoadMode, Loaded, Overwrite, Problem, SettingsFile, SettingsSchema};

/// `schema_version` of workspace files.
pub const WORKSPACE_SCHEMA_VERSION: u32 = 1;

/// A workspace file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    /// The keyboard shortcut that loads it (SES-040 Manage Workspaces; `""` = none).
    pub shortcut: String,
    /// The windows (`[[window]]`).
    #[serde(rename = "window")]
    pub windows: Vec<WorkspaceWindow>,
}

/// One window of a workspace (geometry as `state.toml` `[window]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceWindow {
    /// Width (200…32767).
    pub width: i64,
    /// Height (200…32767).
    pub height: i64,
    /// Left edge (−1 = let the platform place it).
    pub x: i64,
    /// Top edge (−1 = let the platform place it).
    pub y: i64,
    /// Maximized.
    pub maximized: bool,
    /// The active tab (index into `tabs`).
    pub active_tab: u32,
    /// The tabs in order (`[[window.tab]]`).
    #[serde(rename = "tab")]
    pub tabs: Vec<WorkspaceTab>,
}

impl Default for WorkspaceWindow {
    fn default() -> WorkspaceWindow {
        WorkspaceWindow {
            width: 1200,
            height: 800,
            x: -1,
            y: -1,
            maximized: false,
            active_tab: 0,
            tabs: Vec::new(),
        }
    }
}

/// One tab: exactly one of `session` and `inline`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceTab {
    /// A named session reference (`work/api-v2`).
    pub session: String,
    /// An inline session table (§1.2.1 without `schema_version`): an unsaved or auto-saved view.
    pub inline: Table,
    /// The view layout, opaque.
    pub view: Table,
}

impl SettingsSchema for Workspace {
    const FILE: FileId = FileId::Workspace;
    const VERSION: u32 = WORKSPACE_SCHEMA_VERSION;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (w, window) in self.windows.iter().enumerate() {
            for (key, value, min) in [
                ("width", window.width, 200),
                ("height", window.height, 200),
                ("x", window.x, -1),
                ("y", window.y, -1),
            ] {
                if !(min..=32767).contains(&value) {
                    problems.push(Problem::new(
                        format!("window.{w}.{key}"),
                        format!("must be from {min} to 32767, found {value}"),
                    ));
                }
            }
            let tabs = window.tabs.len();
            if (tabs == 0 && window.active_tab != 0)
                || (tabs > 0 && usize::try_from(window.active_tab).unwrap_or(usize::MAX) >= tabs)
            {
                problems.push(Problem::new(
                    format!("window.{w}.active_tab"),
                    format!("must name one of the {tabs} tabs"),
                ));
            }
            for (t, tab) in window.tabs.iter().enumerate() {
                if tab.session.is_empty() == tab.inline.is_empty() {
                    problems.push(Problem::new(
                        format!("window.{w}.tab.{t}"),
                        "a tab has exactly one of `session` and `inline`",
                    ));
                } else if !tab.session.is_empty() && SessionRef::parse(&tab.session).is_err() {
                    problems.push(Problem::new(
                        format!("window.{w}.tab.{t}.session"),
                        "not a session reference",
                    ));
                }
            }
        }
        problems
    }
}

/// The defaults of the entries of `[[window]]` and `[[window.tab]]` (sparse writes, D-70).
pub(crate) fn templates() -> Vec<(&'static str, Value)> {
    vec![
        ("window.*", full_table(&WorkspaceWindow::default()).into()),
        (
            "window.*.tab.*",
            full_table(&WorkspaceTab::default()).into(),
        ),
    ]
}

/// One restored tab ([`restore`]).
#[derive(Debug, Clone, PartialEq)]
pub enum RestoredTab {
    /// The Home view (a missing or unreadable session).
    Home,
    /// A named session.
    Named {
        /// The reference.
        reference: SessionRef,
        /// The session.
        session: Box<Session>,
    },
    /// An inline session.
    Inline(Box<Session>),
}

/// One restored window.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoredWindow {
    /// The window as stored.
    pub window: WorkspaceWindow,
    /// Its tabs, in order.
    pub tabs: Vec<RestoredTab>,
}

/// The sessions of a workspace's tabs (D-85): a missing named session is a Home tab with warning
/// `session_missing`; an unreadable one or an inline table that does not read is a Home tab with warning
/// `invalid_config_value`.
pub fn restore(
    workspace: &Workspace,
    sessions: &SessionStore,
) -> (Vec<RestoredWindow>, Vec<Warning>) {
    let mut warnings = Vec::new();
    let mut windows = Vec::new();
    for window in &workspace.windows {
        let mut tabs = Vec::new();
        for tab in &window.tabs {
            let restored = if tab.session.is_empty() {
                match inline_session(&tab.inline, sessions) {
                    Ok(session) => RestoredTab::Inline(Box::new(session)),
                    Err(e) => {
                        warnings.push(Warning {
                            code: WarningCode::InvalidConfigValue,
                            message: format!("an inline session does not read: {e}"),
                            spec: None,
                        });
                        RestoredTab::Home
                    }
                }
            } else {
                let found = SessionRef::parse(&tab.session)
                    .ok()
                    .and_then(|r| sessions.resolve(&r));
                match found {
                    None => {
                        warnings.push(Warning {
                            code: WarningCode::SessionMissing,
                            message: format!(
                                "the session `{}` no longer exists; a Home tab takes its place",
                                tab.session
                            ),
                            spec: None,
                        });
                        RestoredTab::Home
                    }
                    Some(reference) => match sessions.load(&reference, LoadMode::Lenient) {
                        Ok(loaded) => RestoredTab::Named {
                            reference,
                            session: Box::new(loaded.value),
                        },
                        Err(e) => {
                            warnings.push(Warning {
                                code: WarningCode::InvalidConfigValue,
                                message: format!("the session `{reference}` does not read: {e}"),
                                spec: None,
                            });
                            RestoredTab::Home
                        }
                    },
                }
            };
            tabs.push(restored);
        }
        windows.push(RestoredWindow {
            window: WorkspaceWindow {
                tabs: Vec::new(),
                ..window.clone()
            },
            tabs,
        });
    }
    (windows, warnings)
}

/// The session of an inline table (rules against the current defaults of its type).
pub fn inline_session(table: &Table, sessions: &SessionStore) -> Result<Session, ApiError> {
    let doc: SessionDoc = Value::Table(table.clone())
        .try_into()
        .map_err(|e: toml::de::Error| ApiError::InvalidData(e.message().to_owned()))?;
    let kind = SessionType::parse(&doc.kind)?;
    let defaults = sessions.defaults(kind)?;
    Session::from_doc(&doc, Some(&defaults))
}

/// An inline table of a session (for "Save Workspace As", W4): the on-disk shape without `schema_version`.
pub fn inline_table(session: &Session, sessions: &SessionStore) -> Result<Table, ApiError> {
    let defaults = sessions.defaults(session.kind)?;
    let mut doc = SettingsFile::<SessionDoc>::new_default(Path::new("inline.toml"));
    *doc.value_mut() = session.to_doc(&defaults);
    let mut table: Table = toml::from_str(&doc.to_toml())
        .map_err(|e| ApiError::Internal(format!("inline session: {e}")))?;
    table.remove("schema_version");
    Ok(table)
}

/// A workspace in a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    /// The name.
    pub name: String,
    /// The shortcut.
    pub shortcut: String,
    /// Number of windows.
    pub windows: usize,
    /// Number of tabs.
    pub tabs: usize,
}

/// The workspaces of one configuration folder.
#[derive(Debug, Clone)]
pub struct WorkspaceStore {
    dir: PathBuf,
}

impl WorkspaceStore {
    /// The store of a configuration.
    pub fn new(config: &ConfigLocation) -> WorkspaceStore {
        WorkspaceStore {
            dir: config.dir.join("workspaces"),
        }
    }

    /// `<config>/workspaces`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file of a name.
    pub fn path(&self, name: &SessionName) -> PathBuf {
        self.dir.join(format!("{name}.toml"))
    }

    /// The existing workspace `name` names case-insensitively (its spelling on disk).
    pub fn resolve(&self, name: &SessionName) -> Option<SessionName> {
        let wanted = format!("{name}.toml").to_lowercase();
        entries(&self.dir)
            .into_iter()
            .filter(|(_, is_dir)| !is_dir)
            .find(|(n, _)| n.to_lowercase() == wanted)
            .and_then(|(n, _)| SessionName::parse(n.strip_suffix(".toml")?).ok())
    }

    /// D-88 lookup: exactly, then case-insensitively.
    pub fn find(&self, text: &str) -> Option<SessionName> {
        let name = SessionName::parse(text).ok()?;
        if self.path(&name).is_file() {
            return Some(name);
        }
        self.resolve(&name)
    }

    /// Every workspace, sorted by name (unreadable files skipped with a warning).
    pub fn list(&self) -> (Vec<WorkspaceEntry>, Vec<Warning>) {
        let mut out = Vec::new();
        let mut warnings = Vec::new();
        for (file, is_dir) in entries(&self.dir) {
            let Some(stem) = file.strip_suffix(".toml").filter(|_| !is_dir) else {
                continue;
            };
            let Ok(name) = SessionName::parse(stem) else {
                continue;
            };
            match SettingsFile::<Workspace>::load(&self.path(&name), LoadMode::Lenient) {
                Ok(f) => {
                    let w = f.value();
                    out.push(WorkspaceEntry {
                        name: name.to_string(),
                        shortcut: w.shortcut.clone(),
                        windows: w.windows.len(),
                        tabs: w.windows.iter().map(|x| x.tabs.len()).sum(),
                    });
                }
                Err(e) => warnings.push(Warning {
                    code: WarningCode::InvalidConfigValue,
                    message: format!("{}: skipped: {e}", self.path(&name).display()),
                    spec: None,
                }),
            }
        }
        (out, warnings)
    }

    /// Loads a workspace (`not_found` when it does not exist).
    pub fn load(&self, name: &SessionName, mode: LoadMode) -> Result<Loaded<Workspace>, ApiError> {
        let actual = self
            .resolve(name)
            .ok_or_else(|| ApiError::NotFound(format!("no workspace `{name}`")))?;
        let path = self.path(&actual);
        let file =
            SettingsFile::<Workspace>::load(&path, mode).map_err(|e| in_file(&path, e.into()))?;
        Ok(file.loaded().clone())
    }

    /// Saves a workspace (unknown keys of an existing file kept; a name in another case is `already_exists`).
    pub fn save(&self, name: &SessionName, workspace: &Workspace) -> Result<PathBuf, ApiError> {
        if let Some(existing) = self.resolve(name)
            && existing != *name
        {
            return Err(ApiError::AlreadyExists(format!(
                "a workspace `{existing}` exists"
            )));
        }
        let path = self.path(name);
        let mut file = SettingsFile::<Workspace>::load(&path, LoadMode::Lenient)?;
        *file.value_mut() = workspace.clone();
        file.save(Overwrite::Force)?;
        Ok(path)
    }

    /// Deletes a workspace.
    pub fn delete(&self, name: &SessionName) -> Result<PathBuf, ApiError> {
        let actual = self
            .resolve(name)
            .ok_or_else(|| ApiError::NotFound(format!("no workspace `{name}`")))?;
        let path = self.path(&actual);
        std::fs::remove_file(&path).map_err(|e| SessionError::io(&path, e))?;
        Ok(path)
    }

    /// Renames a workspace (onto an existing one: `already_exists`; a change of case only is allowed).
    pub fn rename(
        &self,
        from: &SessionName,
        to: &SessionName,
    ) -> Result<(PathBuf, PathBuf), ApiError> {
        let source = self
            .resolve(from)
            .ok_or_else(|| ApiError::NotFound(format!("no workspace `{from}`")))?;
        if let Some(existing) = self.resolve(to)
            && existing != source
        {
            return Err(ApiError::AlreadyExists(format!(
                "a workspace `{existing}` exists"
            )));
        }
        let (a, b) = (self.path(&source), self.path(to));
        let temp = a.with_extension("hikasa-rename");
        std::fs::rename(&a, &temp).map_err(|e| SessionError::io(&a, e))?;
        std::fs::rename(&temp, &b).map_err(|e| SessionError::io(&b, e))?;
        Ok((a, b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_need_exactly_one_content() {
        let mut w = Workspace::default();
        w.windows.push(WorkspaceWindow {
            active_tab: 3,
            tabs: vec![WorkspaceTab::default()],
            ..WorkspaceWindow::default()
        });
        let keys: Vec<String> = w.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(keys, ["window.0.active_tab", "window.0.tab.0"]);
    }
}

//! `hikasa session …` and `hikasa workspace …` (M2 W2; contract §1.2.7, D-80…D-85, D-88): the [`Service`]
//! calls and their DTOs, the effective request of a saved session (`session run`, `LoadSession`), and the
//! D-88 resolution of a single command-line argument.
//!
//! Every call reads the configuration fresh. A session's specs are stored as canonical absolute specs; `-` and
//! `clipboard://` are never stored (the side stays empty, warning `spec_not_saved`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use hikasa_core::{Pair, Spec, Warning, WarningCode};
use hikasa_text::ViewOptions;
use serde::Serialize;
use toml::{Table, Value};

use super::gui::ViewRequest;
use super::requests::{FolderCompareRequest, TextCompareRequest};
use super::settings::parse_value;
use super::{ApiError, Service};
use crate::help::edit_distance;
use crate::ipc::{OpenOptions, ViewKind};
use crate::jobs::run_guarded;
use crate::sessions::store::label;
use crate::sessions::{
    AutoId, KeyEdits, Session, SessionDoc, SessionName, SessionRef, SessionRules, SessionStore,
    SessionType, effective_rules, parse_folder,
};
use crate::settings::keys::{KeySegment, ValueKind, parse_path};
use crate::settings::{LoadMode, Options, Overwrite, SettingsFile, SettingsStore, tree};
use crate::workspaces::{RestoredTab, Workspace, WorkspaceStore, inline_session, restore};

/// One named session of `hikasa.session_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionItemDto {
    /// The reference (`work/api-v2`).
    #[serde(rename = "ref")]
    pub reference: String,
    /// The type.
    #[serde(rename = "type")]
    pub kind: String,
    /// The description.
    pub description: String,
    /// Locked.
    pub locked: bool,
    /// Favorite.
    pub favorite: bool,
}

/// One auto-saved session of `hikasa.session_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoSavedDto {
    /// The id.
    pub id: String,
    /// The type.
    #[serde(rename = "type")]
    pub kind: String,
    /// RFC 3339 (`null`: unknown).
    pub saved_at: Option<String>,
    /// `today`, `yesterday`, `days_2_to_6`, `older`.
    pub group: &'static str,
    /// The description, or the names of the inputs.
    pub label: String,
}

/// `hikasa.session_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionListDto {
    /// The folders (references).
    pub folders: Vec<String>,
    /// The named sessions.
    pub sessions: Vec<SessionItemDto>,
    /// The types with a defaults file (`--defaults`).
    pub defaults: Vec<String>,
    /// The auto-saved sessions, newest first (`--autosaved`).
    pub autosaved: Vec<AutoSavedDto>,
    /// Files that could not be read.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// `hikasa.session_show.v1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionShowDto {
    /// The reference (`_defaults/<type>` for the defaults).
    #[serde(rename = "ref")]
    pub reference: String,
    /// The file.
    pub path: String,
    /// The file as JSON of its TOML.
    pub session: serde_json::Value,
    /// The TOML (human output).
    #[serde(skip)]
    pub toml: String,
}

/// `hikasa.session_write.v1` and `hikasa.workspace_write.v1` (`ref` is the workspace name there).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionWriteDto {
    /// `new`, `edit`, `delete`, `rename`, `lock`, `unlock`, `mkdir`, `rmdir`.
    pub action: &'static str,
    /// The reference (or folder, or workspace name).
    #[serde(rename = "ref")]
    pub reference: String,
    /// The files written.
    pub written: Vec<String>,
    /// The files removed.
    pub removed: Vec<String>,
    /// Warnings of the call (`spec_not_saved`).
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// `hikasa.workspace_write.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceWriteDto {
    /// `delete`, `rename`, `save`.
    pub action: &'static str,
    /// The workspace.
    pub name: String,
    /// The files written.
    pub written: Vec<String>,
    /// The files removed.
    pub removed: Vec<String>,
}

/// One workspace of `hikasa.workspace_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceItemDto {
    /// The name.
    pub name: String,
    /// The shortcut (`""`: none).
    pub shortcut: String,
    /// Windows.
    pub windows: usize,
    /// Tabs.
    pub tabs: usize,
}

/// `hikasa.workspace_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceListDto {
    /// The workspaces, sorted by name.
    pub workspaces: Vec<WorkspaceItemDto>,
    /// Files that could not be read.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// `hikasa.workspace_show.v1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceShowDto {
    /// The name.
    pub name: String,
    /// The file.
    pub path: String,
    /// The file as JSON of its TOML.
    pub workspace: serde_json::Value,
    /// Tabs whose session no longer exists (`session_missing`).
    #[serde(skip)]
    pub warnings: Vec<Warning>,
    /// The TOML (human output).
    #[serde(skip)]
    pub toml: String,
}

/// Which session `session edit` / `show` addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTarget {
    /// A named session.
    Named(SessionRef),
    /// The defaults of a type.
    Defaults(SessionType),
}

/// The changes of `session edit`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionEdit {
    /// `--set KEY=VALUE` (paths inside the session file, D-73 syntax).
    pub set: Vec<(String, String)>,
    /// `--unset KEY`: removes a key of `rules`, `text_rules` (back to following the defaults, D-90), `display`
    /// or `view`.
    pub unset: Vec<String>,
    /// `--left`.
    pub left: Option<String>,
    /// `--right`.
    pub right: Option<String>,
    /// `--description`.
    pub description: Option<String>,
}

/// The comparison of a saved session (`session run`, T11).
// Built once per command and moved into the comparison: the size difference does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum SessionRunRequest {
    /// A Text Compare session.
    Text(TextCompareRequest),
    /// A Folder Compare session.
    Folder(FolderCompareRequest),
}

/// What the GUI opens for a session (`LoadSession`, `LoadWorkspace`).
#[derive(Debug, Clone)]
pub struct SessionView {
    /// The view kind.
    pub kind: ViewKind,
    /// The canonical absolute specs.
    pub specs: Vec<String>,
    /// The options (format, encodings).
    pub options: OpenOptions,
    /// The service request of the view, with the session's effective rules.
    pub request: ViewRequest,
    /// The title: the reference, the description or the input names.
    pub title: String,
}

/// What one command-line argument of `gui open` / `hikasa-gui` names (D-88).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SingleArg {
    /// An existing path, or nothing known (the M1 handling and its error).
    Spec,
    /// A named session.
    Session(SessionRef),
    /// A workspace.
    Workspace(SessionName),
    /// An existing `.hikasa-pkg` file.
    Package(PathBuf),
}

/// The keys of a session file `session edit --set` may change (`type`, `locked`, `saved_at` and `override`
/// have their own commands or are program-owned); the defaults allow fewer.
const EDITABLE: &[&str] = &[
    "description",
    "favorite",
    "parent",
    "specs",
    "rules",
    "text_rules",
    "display",
    "view",
];
/// The keys `session edit --defaults --set` may change.
const EDITABLE_DEFAULTS: &[&str] = &["description", "rules", "text_rules", "display", "view"];

impl Service {
    /// The program options read now (leniently: these commands never fail on a bad option).
    fn fresh_options(&self) -> Options {
        SettingsStore::new(self.config())
            .options(LoadMode::Lenient)
            .map(|l| l.value)
            .unwrap_or_default()
    }

    /// The session store of this configuration.
    pub fn session_store(&self) -> SessionStore {
        SessionStore::new(self.config(), &self.fresh_options())
    }

    /// The workspace store of this configuration.
    pub fn workspace_store(&self) -> WorkspaceStore {
        WorkspaceStore::new(self.config())
    }

    /// A spec as a session stores it (D-80, §1.2.1): canonical absolute; `-` and `clipboard://` are not stored
    /// (`""` and warning `spec_not_saved`). Standard input is never read.
    pub fn stored_spec(&self, text: &str) -> Result<(String, Option<Warning>), ApiError> {
        let spec = Spec::parse_os(OsStr::new(text), &self.spec_context())?;
        match spec {
            Spec::Stdin | Spec::Clipboard => Ok((
                String::new(),
                Some(Warning {
                    code: WarningCode::SpecNotSaved,
                    message: format!(
                        "`{text}` cannot be stored in a session; that side stays empty"
                    ),
                    spec: Some(spec.canonical()),
                }),
            )),
            spec => Ok((spec.canonical(), None)),
        }
    }

    /// `session list`: folders and named sessions (under `folder`), plus the defaults files and the auto-saved
    /// sessions when asked; groups against the injected clock (D-83).
    pub fn session_list(
        &self,
        folder: Option<&str>,
        autosaved: bool,
        defaults: bool,
    ) -> Result<SessionListDto, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            let prefix = match folder {
                Some(text) => {
                    let wanted = parse_folder(text)?;
                    let found = store
                        .resolve_folder(&wanted)
                        .filter(|f| !f.is_empty())
                        .ok_or_else(|| ApiError::NotFound(format!("no session folder `{text}`")))?;
                    Some(format!(
                        "{}/",
                        found
                            .iter()
                            .map(SessionName::as_str)
                            .collect::<Vec<_>>()
                            .join("/")
                    ))
                }
                None => None,
            };
            let under = |r: &str| prefix.as_ref().is_none_or(|p| r.starts_with(p.as_str()));
            let now = self.clock().now();
            let tree = store.list(&now)?;
            let zone = now.time_zone().clone();
            Ok(SessionListDto {
                folders: tree
                    .folders
                    .into_iter()
                    .filter(|f| under(f) && prefix.as_deref() != Some(format!("{f}/").as_str()))
                    .collect(),
                sessions: tree
                    .named
                    .into_iter()
                    .filter(|e| under(&e.reference.to_string()))
                    .map(|e| SessionItemDto {
                        reference: e.reference.to_string(),
                        kind: e.kind.as_str().to_owned(),
                        description: e.description,
                        locked: e.locked,
                        favorite: e.favorite,
                    })
                    .collect(),
                defaults: if defaults {
                    tree.defaults
                        .iter()
                        .map(|k| k.as_str().to_owned())
                        .collect()
                } else {
                    Vec::new()
                },
                autosaved: if autosaved {
                    tree.autosaved
                        .into_iter()
                        .map(|e| AutoSavedDto {
                            id: e.id.to_string(),
                            kind: e.kind.as_str().to_owned(),
                            saved_at: e.saved_at.map(|t| {
                                t.display_with_offset(t.to_zoned(zone.clone()).offset())
                                    .to_string()
                            }),
                            group: e.group.as_str(),
                            label: e.label,
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                warnings: tree.warnings,
            })
        })
    }

    /// `session show <REF>` / `session show --defaults <TYPE>`: the file as it is (the defaults without a file:
    /// the built-in defaults' file).
    pub fn session_show(&self, target: &SessionTarget) -> Result<SessionShowDto, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            let (reference, path) = match target {
                SessionTarget::Named(r) => {
                    let actual = store
                        .resolve(r)
                        .ok_or_else(|| ApiError::NotFound(format!("no session `{r}`")))?;
                    (actual.to_string(), store.path(&actual))
                }
                SessionTarget::Defaults(kind) => {
                    (format!("_defaults/{kind}"), store.defaults_path(*kind))
                }
            };
            // The file is loaded first so that a broken file is an error, not a dump.
            let file = SettingsFile::<SessionDoc>::load(&path, LoadMode::Strict)?;
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => {
                    let mut doc = file;
                    if let SessionTarget::Defaults(kind) = target {
                        *doc.value_mut() = Session::builtin(*kind).to_doc(&Session::builtin(*kind));
                    }
                    doc.to_toml()
                }
            };
            let table: Table = toml::from_str(&text).map_err(|e| {
                ApiError::InvalidData(format!("{}: {}", path.display(), e.message()))
            })?;
            Ok(SessionShowDto {
                reference,
                path: path.display().to_string(),
                session: tree::to_json(&Value::Table(table)),
                toml: text,
            })
        })
    }

    /// `session new`: a session of `kind` from the type's defaults (SES-001) with the given specs and
    /// description; a name taken (in any case) is `already_exists`.
    pub fn session_new(
        &self,
        reference: &SessionRef,
        kind: SessionType,
        left: Option<&str>,
        right: Option<&str>,
        description: Option<&str>,
    ) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            if let Some(existing) = store.resolve(reference) {
                return Err(ApiError::AlreadyExists(format!(
                    "the session `{existing}` exists"
                )));
            }
            let mut session = Session::new(kind, &store.defaults(kind)?);
            let mut warnings = Vec::new();
            self.apply_specs(&mut session, left, right, &mut warnings)?;
            if let Some(d) = description {
                d.clone_into(&mut session.description);
            }
            let path = store.save(reference, &session, Overwrite::IfUnchanged)?;
            Ok(write_dto(
                "new",
                reference.to_string(),
                vec![path],
                Vec::new(),
                warnings,
            ))
        })
    }

    /// Sets the specs of a session (`-`/`clipboard://` not stored).
    fn apply_specs(
        &self,
        session: &mut Session,
        left: Option<&str>,
        right: Option<&str>,
        warnings: &mut Vec<Warning>,
    ) -> Result<(), ApiError> {
        if right.is_some() && session.kind == SessionType::TextEdit {
            return Err(ApiError::InvalidArgument(
                "a text-edit session has only a left file".into(),
            ));
        }
        for (text, slot) in [
            (left, &mut session.specs.left),
            (right, &mut session.specs.right),
        ] {
            if let Some(text) = text {
                let (spec, warning) = self.stored_spec(text)?;
                *slot = spec;
                warnings.extend(warning);
            }
        }
        Ok(())
    }

    /// `session edit`: `--set` keys (paths inside the session file, parsed and checked like `settings set`),
    /// specs and description. A locked session is refused unless `force`.
    pub fn session_edit(
        &self,
        target: &SessionTarget,
        edit: &SessionEdit,
        force: bool,
    ) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            let (mut session, reference) = match target {
                SessionTarget::Named(r) => {
                    let actual = store
                        .resolve(r)
                        .ok_or_else(|| ApiError::NotFound(format!("no session `{r}`")))?;
                    let loaded = store.load(&actual, LoadMode::Lenient)?;
                    if loaded.value.locked && !force {
                        return Err(ApiError::InvalidArgument(format!(
                            "the session `{actual}` is locked (unlock it, or add --force)"
                        )));
                    }
                    (loaded.value, Some(actual))
                }
                SessionTarget::Defaults(kind) => {
                    if edit.left.is_some() || edit.right.is_some() {
                        return Err(ApiError::InvalidArgument(
                            "the defaults hold no specs (D-82)".into(),
                        ));
                    }
                    (store.defaults(*kind)?, None)
                }
            };
            let editable = if reference.is_some() {
                EDITABLE
            } else {
                EDITABLE_DEFAULTS
            };
            let file_path = match (&reference, target) {
                (Some(r), _) => store.path(r),
                (None, SessionTarget::Defaults(kind)) => store.defaults_path(*kind),
                (None, SessionTarget::Named(r)) => store.path(r),
            };
            let file = SettingsFile::<SessionDoc>::load(&file_path, LoadMode::Lenient)?;
            let mut edits = KeyEdits::default();
            if !edit.unset.is_empty() {
                let defaults = if reference.is_some() {
                    store.defaults(session.kind)?
                } else {
                    Session::builtin(session.kind)
                };
                let unset = unset_keys(&session, &defaults, file.value(), &edit.unset, editable)?;
                session = unset.0;
                edits.unset = unset.1;
            }
            if !edit.set.is_empty() {
                session = set_keys(&session, &edit.set, editable, self)?;
                for (key, _) in &edit.set {
                    edits
                        .set
                        .push(parse_path(key).map_err(ApiError::InvalidArgument)?);
                }
            }
            let mut warnings = Vec::new();
            // D-80, §1.2.1: specs set with --set are stored like --left/--right (canonical absolute; `-` and
            // `clipboard://` not stored).
            for (key, _) in &edit.set {
                let slot = match key.trim() {
                    "specs.left" => &mut session.specs.left,
                    "specs.right" => &mut session.specs.right,
                    _ => continue,
                };
                if !slot.is_empty() {
                    let (spec, warning) = self.stored_spec(slot)?;
                    *slot = spec;
                    warnings.extend(warning);
                }
            }
            self.apply_specs(
                &mut session,
                edit.left.as_deref(),
                edit.right.as_deref(),
                &mut warnings,
            )?;
            if let Some(d) = &edit.description {
                d.clone_into(&mut session.description);
            }
            let (name, path) = match reference {
                Some(r) => {
                    let path = store.save_edited(&r, &session, Overwrite::Force, &edits)?;
                    (r.to_string(), path)
                }
                None => (
                    format!("_defaults/{}", session.kind),
                    store.save_defaults_edited(&session, &edits)?,
                ),
            };
            Ok(write_dto("edit", name, vec![path], Vec::new(), warnings))
        })
    }

    /// `session delete`; a locked session needs `force` (SES-004).
    pub fn session_delete(
        &self,
        reference: &SessionRef,
        force: bool,
    ) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            refuse_locked(&store, reference, force)?;
            let path = store.delete(reference)?;
            Ok(write_dto(
                "delete",
                reference.to_string(),
                Vec::new(),
                vec![path],
                Vec::new(),
            ))
        })
    }

    /// `session rename` (also moves between folders); a locked session needs `force` (SES-004).
    pub fn session_rename(
        &self,
        from: &SessionRef,
        to: &SessionRef,
        force: bool,
    ) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            refuse_locked(&store, from, force)?;
            let (old, new) = store.rename(from, to)?;
            Ok(write_dto(
                "rename",
                to.to_string(),
                vec![new],
                vec![old],
                Vec::new(),
            ))
        })
    }

    /// `session lock` / `unlock` (SES-004).
    pub fn session_lock(
        &self,
        reference: &SessionRef,
        locked: bool,
    ) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let path = self.session_store().set_locked(reference, locked)?;
            let action = if locked { "lock" } else { "unlock" };
            Ok(write_dto(
                action,
                reference.to_string(),
                vec![path],
                Vec::new(),
                Vec::new(),
            ))
        })
    }

    /// `session mkdir`.
    pub fn session_mkdir(&self, folder: &str) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let path = self.session_store().create_folder(&parse_folder(folder)?)?;
            Ok(write_dto(
                "mkdir",
                folder.to_owned(),
                vec![path],
                Vec::new(),
                Vec::new(),
            ))
        })
    }

    /// `session rmdir` (a folder that is not empty needs `recursive`).
    pub fn session_rmdir(
        &self,
        folder: &str,
        recursive: bool,
    ) -> Result<SessionWriteDto, ApiError> {
        run_guarded(|| {
            let path = self
                .session_store()
                .delete_folder(&parse_folder(folder)?, recursive)?;
            Ok(write_dto(
                "rmdir",
                folder.to_owned(),
                Vec::new(),
                vec![path],
                Vec::new(),
            ))
        })
    }

    /// The effective session of a reference (D-82): its rules overlaid on the defaults, the parent's overrides
    /// (a missing parent is warning-free: its overrides simply do not apply) and its own changes.
    pub fn session_effective(&self, reference: &SessionRef) -> Result<Session, ApiError> {
        let store = self.session_store();
        let mut session = store.load(reference, LoadMode::Strict)?.value;
        self.resolve_parent(&store, &mut session)?;
        Ok(session)
    }

    /// Replaces the rules of a child session by its effective rules.
    fn resolve_parent(&self, store: &SessionStore, session: &mut Session) -> Result<(), ApiError> {
        let Some(link) = &session.parent else {
            return Ok(());
        };
        let parent = match link.session.strip_prefix("_autosaved/") {
            Some(id) => AutoId::parse(id)
                .ok()
                .and_then(|id| store.load_autosaved(&id, LoadMode::Lenient).ok()),
            None => SessionRef::parse(&link.session)
                .ok()
                .and_then(|r| store.load(&r, LoadMode::Lenient).ok()),
        };
        let defaults = store.defaults(session.kind)?;
        session.rules = effective_rules(&defaults, parent.as_ref().map(|p| &p.value), session);
        Ok(())
    }

    /// `session run`: the request of the same Service call as `folder compare` / `text compare` (T11) with the
    /// session's specs and effective rules. Other types are `unsupported` (Folder Sync after W6).
    pub fn session_run_request(
        &self,
        reference: &SessionRef,
    ) -> Result<SessionRunRequest, ApiError> {
        run_guarded(|| {
            let session = self.session_effective(reference)?;
            run_request(&session)
        })
    }

    /// What the GUI opens for a named session (`LoadSession`, D-89): Text and Folder Compare sessions; other
    /// types are `unsupported`.
    pub fn session_view(&self, reference: &SessionRef) -> Result<SessionView, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            let actual = store
                .resolve(reference)
                .ok_or_else(|| ApiError::NotFound(format!("no session `{reference}`")))?;
            let session = self.session_effective(&actual)?.for_script();
            self.view_of(&session, actual.to_string())
        })
    }

    /// The view of a session (absolute specs required).
    fn view_of(&self, session: &Session, title: String) -> Result<SessionView, ApiError> {
        let kind = session.kind.view_kind();
        if !kind.is_served_in_m1() {
            return Err(ApiError::Unsupported(format!(
                "{} sessions cannot be opened in this version",
                session.kind
            )));
        }
        let specs = vec![session.specs.left.clone(), session.specs.right.clone()];
        if specs.iter().any(String::is_empty) {
            return Err(ApiError::InvalidArgument(format!(
                "the session `{title}` has no {} spec",
                if specs[0].is_empty() { "left" } else { "right" }
            )));
        }
        let options = OpenOptions {
            format: non_empty(&session.specs.format),
            encodings: [
                non_empty(&session.specs.left_encoding),
                non_empty(&session.specs.right_encoding),
            ],
            ..OpenOptions::default()
        };
        let request = match (
            self.open_view_request(kind, &specs, &options)?,
            &session.rules,
        ) {
            (ViewRequest::Text { mut request, view }, SessionRules::Text(rules)) => {
                request.base_rules = Some(rules.clone());
                ViewRequest::Text {
                    request,
                    view: ViewOptions { ..view },
                }
            }
            (
                ViewRequest::Folder {
                    mut request,
                    display,
                    expand_all,
                },
                SessionRules::Folder(rules),
            ) => {
                request.base_rules = Some(rules.clone());
                request.base_text_rules.clone_from(&session.text_rules);
                ViewRequest::Folder {
                    request,
                    display,
                    expand_all,
                }
            }
            (other, _) => other,
        };
        let title = if session.description.is_empty() {
            title
        } else {
            session.description.clone()
        };
        Ok(SessionView {
            kind,
            specs,
            options,
            request,
            title,
        })
    }

    /// What the GUI opens for a workspace (`LoadWorkspace`, D-85): the views of its tabs in order (Home tabs
    /// and tabs of types the GUI cannot show yet open nothing; each adds a warning), and the warnings.
    pub fn workspace_views(
        &self,
        name: &SessionName,
    ) -> Result<(Vec<SessionView>, Vec<Warning>), ApiError> {
        run_guarded(|| {
            let workspace = self.workspace_store().load(name, LoadMode::Lenient)?.value;
            let store = self.session_store();
            let (windows, mut warnings) = restore(&workspace, &store);
            let mut views = Vec::new();
            for window in windows {
                for tab in window.tabs {
                    let made = match tab {
                        RestoredTab::Home => continue,
                        RestoredTab::Named { reference, session } => {
                            let mut session = *session;
                            self.resolve_parent(&store, &mut session)?;
                            self.view_of(&session.for_script(), reference.to_string())
                        }
                        RestoredTab::Inline(session) => {
                            let title = label(&session);
                            self.view_of(&session.for_script(), title)
                        }
                    };
                    match made {
                        Ok(view) => views.push(view),
                        Err(e) => warnings.push(Warning {
                            code: WarningCode::SessionMissing,
                            message: format!("a tab cannot be opened: {e}"),
                            spec: None,
                        }),
                    }
                }
            }
            Ok((views, warnings))
        })
    }

    /// D-88: one argument that is not an existing path is a named session (exact, then case-insensitive),
    /// else a workspace; an existing file named `*.hikasa-pkg` (any case) is a settings package. `cwd` resolves
    /// relative paths (the captured one).
    pub fn classify_single_arg(&self, arg: &OsStr) -> SingleArg {
        let path = Path::new(arg);
        let absolute = match &self.env().cwd {
            Some(cwd) if path.is_relative() => cwd.join(path),
            _ => path.to_path_buf(),
        };
        if absolute.exists() {
            let is_package = absolute.is_file()
                && absolute
                    .extension()
                    .and_then(OsStr::to_str)
                    .is_some_and(|e| e.eq_ignore_ascii_case(crate::package::PACKAGE_EXTENSION));
            return if is_package {
                SingleArg::Package(absolute)
            } else {
                SingleArg::Spec
            };
        }
        let Some(text) = arg.to_str() else {
            return SingleArg::Spec;
        };
        if let Some(r) = self.session_store().find(text) {
            return SingleArg::Session(r);
        }
        if let Some(w) = self.workspace_store().find(text) {
            return SingleArg::Workspace(w);
        }
        SingleArg::Spec
    }

    /// `workspace list`.
    pub fn workspace_list(&self) -> WorkspaceListDto {
        let (entries, warnings) = self.workspace_store().list();
        WorkspaceListDto {
            workspaces: entries
                .into_iter()
                .map(|e| WorkspaceItemDto {
                    name: e.name,
                    shortcut: e.shortcut,
                    windows: e.windows,
                    tabs: e.tabs,
                })
                .collect(),
            warnings,
        }
    }

    /// `workspace show`: the file, with `session_missing` warnings for tabs whose session no longer exists.
    pub fn workspace_show(&self, name: &SessionName) -> Result<WorkspaceShowDto, ApiError> {
        run_guarded(|| {
            let store = self.workspace_store();
            let actual = store
                .resolve(name)
                .ok_or_else(|| ApiError::NotFound(format!("no workspace `{name}`")))?;
            let loaded = store.load(&actual, LoadMode::Lenient)?;
            let (_, mut warnings) = restore(&loaded.value, &self.session_store());
            warnings.splice(0..0, loaded.warnings);
            let path = store.path(&actual);
            let text = std::fs::read_to_string(&path)
                .map_err(|e| crate::error::SessionError::io(&path, e))?;
            let table: Table = toml::from_str(&text).map_err(|e| {
                ApiError::InvalidData(format!("{}: {}", path.display(), e.message()))
            })?;
            Ok(WorkspaceShowDto {
                name: actual.to_string(),
                path: path.display().to_string(),
                workspace: tree::to_json(&Value::Table(table)),
                warnings,
                toml: text,
            })
        })
    }

    /// `workspace delete`.
    pub fn workspace_delete(&self, name: &SessionName) -> Result<WorkspaceWriteDto, ApiError> {
        run_guarded(|| {
            let path = self.workspace_store().delete(name)?;
            Ok(ws_dto("delete", name, Vec::new(), vec![path]))
        })
    }

    /// `workspace rename`.
    pub fn workspace_rename(
        &self,
        from: &SessionName,
        to: &SessionName,
    ) -> Result<WorkspaceWriteDto, ApiError> {
        run_guarded(|| {
            let (old, new) = self.workspace_store().rename(from, to)?;
            Ok(ws_dto("rename", to, vec![new], vec![old]))
        })
    }

    /// Saves a workspace ("Save Workspace As", "On exit, save workspace as"; the GUI calls it in W4).
    pub fn workspace_save(
        &self,
        name: &SessionName,
        workspace: &Workspace,
    ) -> Result<WorkspaceWriteDto, ApiError> {
        run_guarded(|| {
            let path = self.workspace_store().save(name, workspace)?;
            Ok(ws_dto("save", name, vec![path], Vec::new()))
        })
    }

    /// The session of an inline workspace tab (rules against the current defaults).
    pub fn inline_session(&self, table: &Table) -> Result<Session, ApiError> {
        inline_session(table, &self.session_store())
    }
}

/// The request of `session run`.
fn run_request(session: &Session) -> Result<SessionRunRequest, ApiError> {
    let need = |spec: &str, side: &str| {
        if spec.is_empty() {
            Err(ApiError::InvalidArgument(format!(
                "the session has no {side} spec"
            )))
        } else {
            Ok(spec.into())
        }
    };
    match (&session.kind, &session.rules) {
        (SessionType::TextCompare, SessionRules::Text(rules)) => {
            Ok(SessionRunRequest::Text(TextCompareRequest {
                left: need(&session.specs.left, "left")?,
                right: need(&session.specs.right, "right")?,
                format: non_empty(&session.specs.format),
                encodings: Pair::new(
                    non_empty(&session.specs.left_encoding),
                    non_empty(&session.specs.right_encoding),
                ),
                base_rules: Some(rules.clone()),
                ..TextCompareRequest::default()
            }))
        }
        (SessionType::FolderCompare, SessionRules::Folder(rules)) => {
            Ok(SessionRunRequest::Folder(FolderCompareRequest {
                left: need(&session.specs.left, "left")?,
                right: need(&session.specs.right, "right")?,
                base_rules: Some(rules.clone()),
                base_text_rules: session.text_rules.clone(),
                ..FolderCompareRequest::default()
            }))
        }
        (kind, _) => Err(ApiError::Unsupported(format!(
            "`session run` runs folder-compare and text-compare sessions; {kind} sessions are not supported in \
             this version"
        ))),
    }
}

/// `invalid_input` when `reference` names a locked session and `force` is off (SES-004).
fn refuse_locked(
    store: &SessionStore,
    reference: &SessionRef,
    force: bool,
) -> Result<(), ApiError> {
    if !force && store.is_locked(reference) {
        return Err(ApiError::InvalidArgument(format!(
            "the session `{reference}` is locked (unlock it, or add --force)"
        )));
    }
    Ok(())
}

/// `Some(text)` unless empty.
fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

/// A [`SessionWriteDto`].
fn write_dto(
    action: &'static str,
    reference: String,
    written: Vec<PathBuf>,
    removed: Vec<PathBuf>,
    warnings: Vec<Warning>,
) -> SessionWriteDto {
    let text = |paths: Vec<PathBuf>| paths.iter().map(|p| p.display().to_string()).collect();
    SessionWriteDto {
        action,
        reference,
        written: text(written),
        removed: text(removed),
        warnings,
    }
}

/// A [`WorkspaceWriteDto`].
fn ws_dto(
    action: &'static str,
    name: &SessionName,
    written: Vec<PathBuf>,
    removed: Vec<PathBuf>,
) -> WorkspaceWriteDto {
    let text = |paths: Vec<PathBuf>| paths.iter().map(|p| p.display().to_string()).collect();
    WorkspaceWriteDto {
        action,
        name: name.to_string(),
        written: text(written),
        removed: text(removed),
    }
}

/// Applies `session edit --set` keys to the whole session (every rule spelled out): a key must exist (an
/// unknown key is `not_found` with suggestions), except below the opaque `display` and `view` tables; values
/// parse like `settings set`; the result must read back (types, text rules that compile), else `invalid_input`.
fn set_keys(
    session: &Session,
    sets: &[(String, String)],
    editable: &[&str],
    service: &Service,
) -> Result<Session, ApiError> {
    let doc = session.full_doc();
    let mut root = Value::try_from(&doc).map_err(|e| ApiError::Internal(e.to_string()))?;
    for (key, text) in sets {
        let path = parse_path(key).map_err(ApiError::InvalidArgument)?;
        let Some(KeySegment::Name(first)) = path.first() else {
            return Err(ApiError::InvalidArgument(format!(
                "{key:?}: not a session key"
            )));
        };
        if !editable.contains(&first.as_str()) {
            return Err(ApiError::InvalidArgument(format!(
                "`{first}` cannot be set with --set (keys: {})",
                editable.join(", ")
            )));
        }
        let opaque = matches!(first.as_str(), "display" | "view") && path.len() > 1;
        let existing = tree::get(&root, &path).cloned();
        let kind = match (&existing, opaque) {
            (Some(Value::Table(_)), _) => {
                return Err(ApiError::InvalidArgument(format!(
                    "`{key}` is a table: set one of its keys"
                )));
            }
            (Some(v), _) => ValueKind::of_value(v),
            (None, true) => ValueKind::String,
            (None, false) => {
                let mut near: Vec<String> = match &root {
                    Value::Table(t) => tree::leaves(t, false)
                        .into_iter()
                        .map(|(p, _)| tree::display_path(&p))
                        .filter(|p| edit_distance(p, key) <= 2)
                        .collect(),
                    _ => Vec::new(),
                };
                near.truncate(5);
                let hint = if near.is_empty() {
                    String::new()
                } else {
                    format!(" (did you mean {}?)", near.join(", "))
                };
                return Err(ApiError::NotFound(format!("no session key `{key}`{hint}")));
            }
        };
        let value = if opaque && existing.is_none() {
            toml::from_str::<Table>(&format!("v = {text}"))
                .ok()
                .and_then(|mut t| t.remove("v"))
                .unwrap_or_else(|| Value::String(text.clone()))
        } else {
            parse_value(text, kind)?
        };
        tree::set(&mut root, &path, value).map_err(ApiError::InvalidArgument)?;
    }
    let doc: SessionDoc = root
        .try_into()
        .map_err(|e: toml::de::Error| ApiError::InvalidArgument(e.message().to_owned()))?;
    let edited =
        Session::from_doc(&doc, None).map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
    let registry = service.registry();
    let limits = registry.limits();
    let check = |rules: &hikasa_text::TextRules| {
        rules
            .validate(limits)
            .map_err(|e| ApiError::InvalidArgument(e.to_string()))
    };
    if let SessionRules::Text(rules) = &edited.rules {
        check(rules)?;
    }
    if let Some(rules) = &edited.text_rules {
        check(rules)?;
    }
    Ok(edited)
}

/// Applies `session edit --unset` keys (D-90): a key of `rules` or `text_rules` takes the value of `defaults`
/// (an optional rule the defaults lack is off) and is removed from the file; a key of `display` or `view` is
/// removed. The key must exist in the session, its defaults or its file (else `not_found`). Returns the
/// session and the key paths.
fn unset_keys(
    session: &Session,
    defaults: &Session,
    file: &SessionDoc,
    keys: &[String],
    editable: &[&str],
) -> Result<(Session, Vec<Vec<KeySegment>>), ApiError> {
    let to_value =
        |doc: &SessionDoc| Value::try_from(doc).map_err(|e| ApiError::Internal(e.to_string()));
    let mut root = to_value(&session.full_doc())?;
    let base = to_value(&defaults.full_doc())?;
    let on_disk = to_value(file)?;
    let mut paths = Vec::new();
    for key in keys {
        let path = parse_path(key).map_err(ApiError::InvalidArgument)?;
        let first = match path.first() {
            Some(KeySegment::Name(first)) => first.as_str(),
            _ => "",
        };
        if !matches!(first, "rules" | "text_rules" | "display" | "view")
            || !editable.contains(&first)
        {
            return Err(ApiError::InvalidArgument(format!(
                "`{key}` cannot be unset (keys below: rules, text_rules, display, view)"
            )));
        }
        let found = [&root, &base, &on_disk]
            .iter()
            .any(|v| tree::get(v, &path).is_some());
        if !found {
            return Err(ApiError::NotFound(format!("no session key `{key}`")));
        }
        match tree::get(&base, &path).cloned() {
            Some(value) if matches!(first, "rules" | "text_rules") => {
                tree::set(&mut root, &path, value).map_err(ApiError::InvalidArgument)?;
            }
            _ => {
                tree::remove(&mut root, &path);
            }
        }
        paths.push(path);
    }
    let doc: SessionDoc = root
        .try_into()
        .map_err(|e: toml::de::Error| ApiError::InvalidArgument(e.message().to_owned()))?;
    let edited =
        Session::from_doc(&doc, None).map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
    Ok((edited, paths))
}

/// `KEY=VALUE` of `--set`.
pub fn parse_assignment(text: &str) -> Result<(String, String), ApiError> {
    text.split_once('=')
        .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
        .filter(|(k, _)| !k.is_empty())
        .ok_or_else(|| ApiError::InvalidArgument(format!("{text:?}: expected KEY=VALUE")))
}

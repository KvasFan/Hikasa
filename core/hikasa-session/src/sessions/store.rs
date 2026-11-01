//! [`SessionStore`] (contract §1.2.2, D-80, D-83): named sessions in folders, the defaults of each type and the
//! auto-saved sessions, all under `<config>/sessions`, loaded and saved by the D-70 engine.
//!
//! Collisions: names are unique per folder case-insensitively, so a save or rename onto a name that exists with
//! another case is `already_exists` (never two files differing only by case); a folder that exists in another
//! case is reused with its spelling. Every write is atomic; a rename is `std::fs::rename` inside the
//! configuration folder (folders created on demand). The methods return [`ApiError`] (W2 note: the store reports
//! `not_found`, `already_exists` and `invalid_input`, which `SessionError` has no variants for).

use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

use hikasa_core::{Warning, WarningCode};
use toml::{Table, Value};

use super::autosave::{AutoGroup, auto_group};
use super::file::{Session, SessionDoc, SessionType};
use super::name::{AutoId, SessionName, SessionRef, check_name};
use crate::api::ApiError;
use crate::error::SessionError;
use crate::paths::ConfigLocation;
use crate::settings::keys::KeySegment;
use crate::settings::stamp::modified_ns;
use crate::settings::{LoadMode, Loaded, Options, Overwrite, SettingsFile, tree};

/// The folder of the defaults files.
pub const DEFAULTS_FOLDER: &str = "_defaults";
/// The folder of the auto-saved sessions.
pub const AUTOSAVED_FOLDER: &str = "_autosaved";

/// A named session in a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedEntry {
    /// The reference.
    pub reference: SessionRef,
    /// The type.
    pub kind: SessionType,
    /// The description.
    pub description: String,
    /// Locked.
    pub locked: bool,
    /// Favorite.
    pub favorite: bool,
}

/// An auto-saved session in a listing.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoEntry {
    /// The id.
    pub id: AutoId,
    /// The type.
    pub kind: SessionType,
    /// When it was saved.
    pub saved_at: Option<jiff::Timestamp>,
    /// Its group (D-83).
    pub group: AutoGroup,
    /// The description, or the names of its inputs.
    pub label: String,
}

/// Everything under `sessions/` (contract §1.2.2 `list`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionTree {
    /// Every folder (references, sorted).
    pub folders: Vec<String>,
    /// The named sessions, sorted by reference.
    pub named: Vec<NamedEntry>,
    /// The types that have a defaults file.
    pub defaults: Vec<SessionType>,
    /// The auto-saved sessions, newest first.
    pub autosaved: Vec<AutoEntry>,
    /// Files that could not be read (skipped).
    pub warnings: Vec<Warning>,
}

/// The keys `session edit` names (D-90), as paths inside the session file (`rules.…`, `text_rules.…`): the
/// keys of `set` stay written by the save, those of `unset` are removed from the file (back to following the
/// defaults).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyEdits {
    /// `--set` keys.
    pub set: Vec<Vec<KeySegment>>,
    /// `--unset` keys.
    pub unset: Vec<Vec<KeySegment>>,
}

impl KeyEdits {
    /// The file `previous` as the save sees it: the `unset` keys removed, the `set` keys present.
    fn applied(&self, previous: &SessionDoc) -> SessionDoc {
        let mut doc = previous.clone();
        for (path, set) in self
            .unset
            .iter()
            .map(|p| (p, false))
            .chain(self.set.iter().map(|p| (p, true)))
        {
            let Some((KeySegment::Name(first), rest)) = path.split_first() else {
                continue;
            };
            let table = match first.as_str() {
                "rules" => &mut doc.rules,
                "text_rules" => &mut doc.text_rules,
                _ => continue,
            };
            let mut root = Value::Table(std::mem::take(table));
            if set {
                let _ = tree::set(&mut root, rest, Value::Boolean(true));
            } else if rest.is_empty() {
                root = Value::Table(Table::new());
            } else {
                tree::remove(&mut root, rest);
            }
            if let Value::Table(t) = root {
                *table = t;
            }
        }
        doc
    }
}

/// A stamp of everything below `<config>/sessions`: paths, lengths and modification times, no content. It changes when
/// a session or folder is added, removed or rewritten, by this process or another one; the GUI's watch polls it to keep
/// the Home tree current (D-124).
pub(crate) fn store_stamp(config: &ConfigLocation) -> u64 {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, Option<i128>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            if meta.is_dir() {
                walk(&path, out);
                out.push((path, 0, None));
            } else {
                out.push((path, meta.len(), modified_ns(&meta)));
            }
        }
    }
    let mut files = Vec::new();
    walk(&config.dir.join("sessions"), &mut files);
    files.sort();
    let mut hasher = DefaultHasher::new();
    files.hash(&mut hasher);
    hasher.finish()
}

/// The sessions of one configuration folder.
#[derive(Debug, Clone)]
pub struct SessionStore {
    dir: PathBuf,
    max_auto_saved: u32,
}

impl SessionStore {
    /// The store of a configuration; `options` gives `max_auto_saved_sessions` (OPT-019).
    pub fn new(config: &ConfigLocation, options: &Options) -> SessionStore {
        SessionStore {
            dir: config.dir.join("sessions"),
            max_auto_saved: options.tweaks.metrics.max_auto_saved_sessions,
        }
    }

    /// `max_auto_saved_sessions` of this store.
    pub fn max_auto_saved(&self) -> u32 {
        self.max_auto_saved
    }

    /// `<config>/sessions`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file of a reference (as spelled).
    pub fn path(&self, reference: &SessionRef) -> PathBuf {
        self.dir.join(reference.relative_path())
    }

    /// `_defaults/<type>.toml`.
    pub fn defaults_path(&self, kind: SessionType) -> PathBuf {
        self.dir
            .join(DEFAULTS_FOLDER)
            .join(format!("{}.toml", kind.as_str()))
    }

    /// `_autosaved/<id>.toml`.
    pub fn autosaved_path(&self, id: &AutoId) -> PathBuf {
        self.dir.join(AUTOSAVED_FOLDER).join(format!("{id}.toml"))
    }

    /// The existing session `reference` names, matched case-insensitively segment by segment (its spelling on
    /// disk), or `None`.
    pub fn resolve(&self, reference: &SessionRef) -> Option<SessionRef> {
        let folder = self.resolve_folder(&reference.folder)?;
        let dir = self.folder_dir(&folder);
        let wanted = format!("{}.toml", reference.name).to_lowercase();
        let found = entries(&dir)
            .into_iter()
            .filter(|(_, is_dir)| !is_dir)
            .find(|(name, _)| name.to_lowercase() == wanted)?;
        let stem = found.0.strip_suffix(".toml")?;
        Some(SessionRef {
            folder,
            name: SessionName::parse(stem).ok()?,
        })
    }

    /// The existing folder `folder` names case-insensitively (its spelling on disk), or `None`.
    pub fn resolve_folder(&self, folder: &[SessionName]) -> Option<Vec<SessionName>> {
        let mut out = Vec::with_capacity(folder.len());
        for segment in folder {
            let dir = self.folder_dir(&out);
            let wanted = segment.folded();
            let (name, _) = entries(&dir)
                .into_iter()
                .filter(|(_, is_dir)| *is_dir)
                .find(|(name, _)| name.to_lowercase() == wanted)?;
            out.push(SessionName::parse(&name).ok()?);
        }
        Some(out)
    }

    /// D-88 lookup: the session named by `text`, exactly, then case-insensitively.
    pub fn find(&self, text: &str) -> Option<SessionRef> {
        let reference = SessionRef::parse(text).ok()?;
        if self.path(&reference).is_file() && self.spelled_on_disk(&reference) {
            return Some(reference);
        }
        self.resolve(&reference)
    }

    /// True when `reference` is the exact spelling on disk: on a case-insensitive file system `is_file` is true
    /// for any case of the name, and the lookup must still answer with the name on disk.
    fn spelled_on_disk(&self, reference: &SessionRef) -> bool {
        let mut dir = self.dir.clone();
        for segment in &reference.folder {
            let folder = segment.as_str();
            if !entries(&dir).iter().any(|(n, d)| *d && n == folder) {
                return false;
            }
            dir.push(folder);
        }
        let file = format!("{}.toml", reference.name);
        entries(&dir).iter().any(|(n, d)| !*d && *n == file)
    }

    /// Loads a named session (`not_found` when it does not exist; matched case-insensitively).
    pub fn load(
        &self,
        reference: &SessionRef,
        mode: LoadMode,
    ) -> Result<Loaded<Session>, ApiError> {
        let actual = self
            .resolve(reference)
            .ok_or_else(|| ApiError::NotFound(format!("no session `{reference}`")))?;
        self.load_path(&self.path(&actual), mode)
    }

    /// Loads an auto-saved session.
    pub fn load_autosaved(&self, id: &AutoId, mode: LoadMode) -> Result<Loaded<Session>, ApiError> {
        let path = self.autosaved_path(id);
        if !path.is_file() {
            return Err(ApiError::NotFound(format!("no auto-saved session `{id}`")));
        }
        self.load_path(&path, mode)
    }

    /// Loads the session file at `path` against the current defaults of its type.
    pub fn load_path(&self, path: &Path, mode: LoadMode) -> Result<Loaded<Session>, ApiError> {
        let file = SettingsFile::<SessionDoc>::load(path, mode)?;
        let kind = SessionType::parse(&file.value().kind).map_err(|e| in_file(path, e))?;
        let defaults = self.defaults(kind)?;
        let session =
            Session::from_doc(file.value(), Some(&defaults)).map_err(|e| in_file(path, e))?;
        let loaded = file.loaded();
        Ok(Loaded {
            value: session,
            stamp: loaded.stamp,
            migrated_from: loaded.migrated_from,
            warnings: loaded.warnings.clone(),
            ignored_keys: loaded.ignored_keys.clone(),
        })
    }

    /// The defaults of a type (a missing file: the built-in defaults).
    pub fn defaults(&self, kind: SessionType) -> Result<Session, ApiError> {
        let path = self.defaults_path(kind);
        let file = SettingsFile::<SessionDoc>::load(&path, LoadMode::Strict)?;
        if file.stamp() == &crate::settings::ChangeStamp::Absent {
            return Ok(Session::builtin(kind));
        }
        let session = Session::from_doc(file.value(), None).map_err(|e| in_file(&path, e))?;
        if session.kind != kind {
            return Err(in_file(
                &path,
                ApiError::InvalidData(format!("the defaults of {kind} have type {}", session.kind)),
            ));
        }
        Ok(session)
    }

    /// Saves a named session (rules sparse against the current defaults of its type, D-82; the keys the file
    /// held stay written, D-90). An existing file keeps its unknown keys; a locked one is refused unless
    /// `overwrite` is `Force`; a name that exists in another case is `already_exists`.
    pub fn save(
        &self,
        reference: &SessionRef,
        session: &Session,
        overwrite: Overwrite,
    ) -> Result<PathBuf, ApiError> {
        self.save_edited(reference, session, overwrite, &KeyEdits::default())
    }

    /// [`SessionStore::save`] after `session edit`: the keys of `edits.set` stay written, those of
    /// `edits.unset` are removed from the file (D-90).
    pub fn save_edited(
        &self,
        reference: &SessionRef,
        session: &Session,
        overwrite: Overwrite,
        edits: &KeyEdits,
    ) -> Result<PathBuf, ApiError> {
        let target = self.target(reference)?;
        let path = self.path(&target);
        self.write_session(&path, session, overwrite, edits)?;
        Ok(path)
    }

    /// True when the named session exists and is locked (SES-004).
    pub fn is_locked(&self, reference: &SessionRef) -> bool {
        self.resolve(reference).is_some_and(|actual| {
            SettingsFile::<SessionDoc>::load(&self.path(&actual), LoadMode::Lenient)
                .is_ok_and(|f| f.value().locked)
        })
    }

    /// The spelling a save of `reference` uses: existing folders keep their case; a name that exists in
    /// another case is `already_exists`.
    fn target(&self, reference: &SessionRef) -> Result<SessionRef, ApiError> {
        let mut folder = Vec::new();
        for (k, segment) in reference.folder.iter().enumerate() {
            match self.resolve_folder(&reference.folder[..=k]) {
                Some(found) => folder.push(found[k].clone()),
                None => folder.push(segment.clone()),
            }
        }
        let target = SessionRef {
            folder,
            name: reference.name.clone(),
        };
        if let Some(existing) = self.resolve(&target)
            && existing.name != target.name
        {
            return Err(ApiError::AlreadyExists(format!(
                "a session `{existing}` exists (names are unique per folder whatever their case)"
            )));
        }
        Ok(target)
    }

    /// Writes `session` to `path` (unknown keys of an existing file kept; a locked file needs `Force`).
    fn write_session(
        &self,
        path: &Path,
        session: &Session,
        overwrite: Overwrite,
        edits: &KeyEdits,
    ) -> Result<bool, ApiError> {
        let mut file = SettingsFile::<SessionDoc>::load(path, LoadMode::Lenient)?;
        if file.value().locked && overwrite != Overwrite::Force {
            return Err(ApiError::InvalidArgument(format!(
                "{}: the session is locked (unlock it, or use --force)",
                path.display()
            )));
        }
        let defaults = self.defaults(session.kind)?;
        let previous = edits.applied(file.value());
        *file.value_mut() = session.to_doc_over(&defaults, Some(&previous));
        Ok(file.save(Overwrite::Force)?)
    }

    /// Writes `_defaults/<type>.toml` (sparse against the built-in defaults; never locked, no parent).
    pub fn save_defaults(&self, session: &Session) -> Result<PathBuf, ApiError> {
        self.save_defaults_edited(session, &KeyEdits::default())
    }

    /// [`SessionStore::save_defaults`] after `session edit --defaults` (D-90, as [`SessionStore::save_edited`]).
    pub fn save_defaults_edited(
        &self,
        session: &Session,
        edits: &KeyEdits,
    ) -> Result<PathBuf, ApiError> {
        let path = self.defaults_path(session.kind);
        let mut file = SettingsFile::<SessionDoc>::load(&path, LoadMode::Lenient)?;
        let clean = Session {
            locked: false,
            favorite: false,
            saved_at: None,
            parent: None,
            ..session.clone()
        };
        let previous = edits.applied(file.value());
        *file.value_mut() = clean.to_doc_over(&Session::builtin(session.kind), Some(&previous));
        file.save(Overwrite::Force)?;
        Ok(path)
    }

    /// Deletes a named session (`not_found` when it does not exist).
    pub fn delete(&self, reference: &SessionRef) -> Result<PathBuf, ApiError> {
        let actual = self
            .resolve(reference)
            .ok_or_else(|| ApiError::NotFound(format!("no session `{reference}`")))?;
        let path = self.path(&actual);
        std::fs::remove_file(&path).map_err(|e| SessionError::io(&path, e))?;
        Ok(path)
    }

    /// Renames (and moves between folders) a named session; onto an existing session is `already_exists` (a
    /// change of case only is allowed).
    pub fn rename(
        &self,
        from: &SessionRef,
        to: &SessionRef,
    ) -> Result<(PathBuf, PathBuf), ApiError> {
        let source = self
            .resolve(from)
            .ok_or_else(|| ApiError::NotFound(format!("no session `{from}`")))?;
        let source_path = self.path(&source);
        let mut folder = Vec::new();
        for k in 0..to.folder.len() {
            match self.resolve_folder(&to.folder[..=k]) {
                Some(found) => folder.push(found[k].clone()),
                None => folder.push(to.folder[k].clone()),
            }
        }
        let target = SessionRef {
            folder,
            name: to.name.clone(),
        };
        if let Some(existing) = self.resolve(&target)
            && self.path(&existing) != source_path
        {
            return Err(ApiError::AlreadyExists(format!(
                "a session `{existing}` exists"
            )));
        }
        let target_path = self.path(&target);
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SessionError::io(parent, e))?;
        }
        rename_via_temp(&source_path, &target_path)?;
        Ok((source_path, target_path))
    }

    /// The folder of a folder path (as spelled).
    fn folder_dir(&self, folder: &[SessionName]) -> PathBuf {
        folder
            .iter()
            .fold(self.dir.clone(), |dir, s| dir.join(s.as_str()))
    }

    /// Creates a session folder (`already_exists` when it exists in any case).
    pub fn create_folder(&self, folder: &[SessionName]) -> Result<PathBuf, ApiError> {
        if folder.is_empty() {
            return Err(ApiError::InvalidArgument("name a folder".into()));
        }
        if let Some(found) = self.resolve_folder(folder) {
            return Err(ApiError::AlreadyExists(format!(
                "the folder `{}` exists",
                join(&found)
            )));
        }
        let dir = self.folder_dir(folder);
        std::fs::create_dir_all(&dir).map_err(|e| SessionError::io(&dir, e))?;
        Ok(dir)
    }

    /// Deletes a session folder; a folder that is not empty needs `recursive` (else `invalid_input`).
    pub fn delete_folder(
        &self,
        folder: &[SessionName],
        recursive: bool,
    ) -> Result<PathBuf, ApiError> {
        let found = self
            .resolve_folder(folder)
            .filter(|f| !f.is_empty())
            .ok_or_else(|| ApiError::NotFound(format!("no session folder `{}`", join(folder))))?;
        let dir = self.folder_dir(&found);
        let empty = entries(&dir).is_empty();
        if !empty && !recursive {
            return Err(ApiError::InvalidArgument(format!(
                "the folder `{}` is not empty (add --recursive to delete it with its sessions)",
                join(&found)
            )));
        }
        let result = if recursive {
            std::fs::remove_dir_all(&dir)
        } else {
            std::fs::remove_dir(&dir)
        };
        result.map_err(|e| SessionError::io(&dir, e))?;
        Ok(dir)
    }

    /// The named sessions below a folder, at any depth (`not_found` when it does not exist; unreadable files are
    /// skipped as in [`SessionStore::list`]).
    pub fn sessions_in_folder(&self, folder: &[SessionName]) -> Result<Vec<NamedEntry>, ApiError> {
        let mut found = self
            .resolve_folder(folder)
            .filter(|f| !f.is_empty())
            .ok_or_else(|| ApiError::NotFound(format!("no session folder `{}`", join(folder))))?;
        let mut tree = SessionTree::default();
        self.walk(&mut found, &mut tree);
        tree.named.sort_by_key(|e| e.reference.to_string());
        Ok(tree.named)
    }

    /// Renames (or moves) a session folder.
    pub fn rename_folder(
        &self,
        from: &[SessionName],
        to: &[SessionName],
    ) -> Result<(PathBuf, PathBuf), ApiError> {
        let found = self
            .resolve_folder(from)
            .filter(|f| !f.is_empty())
            .ok_or_else(|| ApiError::NotFound(format!("no session folder `{}`", join(from))))?;
        if to.is_empty() {
            return Err(ApiError::InvalidArgument("name the new folder".into()));
        }
        let source = self.folder_dir(&found);
        if let Some(existing) = self.resolve_folder(to)
            && self.folder_dir(&existing) != source
        {
            return Err(ApiError::AlreadyExists(format!(
                "the folder `{}` exists",
                join(&existing)
            )));
        }
        let target = self.folder_dir(to);
        if target.starts_with(&source) && target != source {
            return Err(ApiError::InvalidArgument(
                "a folder cannot move into itself".into(),
            ));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SessionError::io(parent, e))?;
        }
        rename_via_temp(&source, &target)?;
        Ok((source, target))
    }

    /// Every folder, named session, defaults file and auto-saved session; groups against `now`'s date (W2 note:
    /// `list` takes the injected clock's time, T1). Unreadable files are skipped with a warning.
    pub fn list(&self, now: &jiff::Zoned) -> Result<SessionTree, ApiError> {
        let mut tree = SessionTree::default();
        self.walk(&mut Vec::new(), &mut tree);
        tree.folders.sort();
        tree.named.sort_by_key(|e| e.reference.to_string());
        tree.defaults = SessionType::ALL
            .into_iter()
            .filter(|k| self.defaults_path(*k).is_file())
            .collect();
        let today = now.date();
        for (id, path) in self.autosaved_files() {
            match self.load_path(&path, LoadMode::Lenient) {
                Ok(loaded) => {
                    let s = loaded.value;
                    let group = match s.saved_at {
                        Some(t) => auto_group(t.to_zoned(now.time_zone().clone()).date(), today),
                        None => AutoGroup::Older,
                    };
                    tree.autosaved.push(AutoEntry {
                        id,
                        kind: s.kind,
                        saved_at: s.saved_at,
                        group,
                        label: label(&s),
                    });
                }
                Err(e) => tree.warnings.push(skipped(&path, &e)),
            }
        }
        tree.autosaved
            .sort_by(|a, b| (b.saved_at, &b.id).cmp(&(a.saved_at, &a.id)));
        Ok(tree)
    }

    /// Walks named sessions and folders below `folder`.
    fn walk(&self, folder: &mut Vec<SessionName>, tree: &mut SessionTree) {
        let dir = self.folder_dir(folder);
        for (name, is_dir) in entries(&dir) {
            if is_dir {
                let Ok(segment) = SessionName::parse(&name) else {
                    continue;
                };
                folder.push(segment);
                tree.folders.push(join(folder));
                self.walk(folder, tree);
                folder.pop();
            } else if let Some(stem) = name.strip_suffix(".toml")
                && check_name(stem).is_ok()
                && let Ok(name) = SessionName::parse(stem)
            {
                let reference = SessionRef {
                    folder: folder.clone(),
                    name,
                };
                let path = self.path(&reference);
                match self.load_path(&path, LoadMode::Lenient) {
                    Ok(loaded) => tree.named.push(NamedEntry {
                        reference,
                        kind: loaded.value.kind,
                        description: loaded.value.description,
                        locked: loaded.value.locked,
                        favorite: loaded.value.favorite,
                    }),
                    Err(e) => tree.warnings.push(skipped(&path, &e)),
                }
            }
        }
    }

    /// The auto-saved files whose names are ids, sorted by id.
    fn autosaved_files(&self) -> Vec<(AutoId, PathBuf)> {
        let dir = self.dir.join(AUTOSAVED_FOLDER);
        let mut files: Vec<(AutoId, PathBuf)> = entries(&dir)
            .into_iter()
            .filter(|(_, is_dir)| !is_dir)
            .filter_map(|(name, _)| {
                let id = AutoId::parse(name.strip_suffix(".toml")?).ok()?;
                Some((id, dir.join(&name)))
            })
            .collect();
        files.sort();
        files
    }

    /// Auto-saves a view (D-83): the first save (`slot = None`) creates `_autosaved/<YYYYMMDD-HHMMSS>-<n>.toml`
    /// (local time of `now`, `n` the first free counter), later saves of the same view overwrite `slot`. The
    /// file carries `saved_at` (RFC 3339 with `now`'s offset). Then the store keeps the newest
    /// `max_auto_saved_sessions`; with 0 nothing is saved (`None`) and every auto-saved file is removed.
    pub fn autosave(
        &self,
        slot: Option<&AutoId>,
        session: &Session,
        now: &jiff::Zoned,
    ) -> Result<Option<AutoId>, ApiError> {
        if self.max_auto_saved == 0 {
            self.prune_autosaved(0)?;
            return Ok(None);
        }
        let id = match slot {
            Some(id) => id.clone(),
            None => {
                let mut n = 1;
                loop {
                    let id = AutoId::at(now, n);
                    if !self.autosaved_path(&id).exists() {
                        break id;
                    }
                    n += 1;
                }
            }
        };
        let path = self.autosaved_path(&id);
        let mut file = SettingsFile::<SessionDoc>::load(&path, LoadMode::Lenient)?;
        if file.value().locked {
            // D-84: auto-save never writes a locked file; the slot stays as it is.
            return Ok(Some(id));
        }
        let defaults = self.defaults(session.kind)?;
        let saved = Session {
            saved_at: Some(now.timestamp()),
            ..session.clone()
        };
        let previous = file.value().clone();
        let mut doc = saved.to_doc_over(&defaults, Some(&previous));
        doc.saved_at = now
            .timestamp()
            .display_with_offset(now.offset())
            .to_string();
        *file.value_mut() = doc;
        file.save(Overwrite::Force)?;
        self.prune_autosaved(self.max_auto_saved)?;
        Ok(Some(id))
    }

    /// Deletes an auto-saved file (a missing one is no error); returns its path.
    pub fn delete_autosaved(&self, id: &AutoId) -> Result<PathBuf, ApiError> {
        let path = self.autosaved_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path),
            Err(e) => Err(SessionError::io(&path, e).into()),
        }
    }

    /// Keeps the newest `max` auto-saved sessions by `saved_at`, then by name (a file without a readable time
    /// counts as the oldest); returns the removed ids. Named sessions are never touched.
    pub fn prune_autosaved(&self, max: u32) -> Result<Vec<AutoId>, ApiError> {
        let mut files: Vec<(Option<jiff::Timestamp>, AutoId, PathBuf)> = self
            .autosaved_files()
            .into_iter()
            .map(|(id, path)| {
                let saved = SettingsFile::<SessionDoc>::load(&path, LoadMode::Lenient)
                    .ok()
                    .and_then(|f| f.value().saved_at.parse::<jiff::Timestamp>().ok());
                (saved, id, path)
            })
            .collect();
        files.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
        let keep = usize::try_from(max).unwrap_or(usize::MAX);
        let mut removed = Vec::new();
        for (_, id, path) in files.into_iter().skip(keep) {
            match std::fs::remove_file(&path) {
                Ok(()) => removed.push(id),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(SessionError::io(&path, e).into()),
            }
        }
        Ok(removed)
    }

    /// Sets or clears `locked` of a named session (SES-004).
    pub fn set_locked(&self, reference: &SessionRef, locked: bool) -> Result<PathBuf, ApiError> {
        let actual = self
            .resolve(reference)
            .ok_or_else(|| ApiError::NotFound(format!("no session `{reference}`")))?;
        let path = self.path(&actual);
        let mut file = SettingsFile::<SessionDoc>::load(&path, LoadMode::Lenient)?;
        file.value_mut().locked = locked;
        file.save(Overwrite::Force)?;
        Ok(path)
    }
}

/// The label of an auto-saved session: its description, or the names of its inputs.
pub fn label(session: &Session) -> String {
    if !session.description.is_empty() {
        return session.description.clone();
    }
    let name = |spec: &str| {
        spec.trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_owned()
    };
    match (session.specs.left.as_str(), session.specs.right.as_str()) {
        ("", "") => format!("({})", session.kind),
        (l, "") => name(l),
        (l, r) => format!("{} ⇄ {}", name(l), name(r)),
    }
}

/// The entries of a folder: `(name, is_dir)`, sorted by name; nothing for a folder that cannot be read.
pub(crate) fn entries(dir: &Path) -> Vec<(String, bool)> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let is_dir = e.file_type().ok()?.is_dir();
            Some((name, is_dir))
        })
        .collect();
    out.sort();
    out
}

/// A folder path joined by `/`.
fn join(folder: &[SessionName]) -> String {
    folder
        .iter()
        .map(SessionName::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

/// Renames `from` to `to`; a change of case only goes through a temporary name (case-insensitive file systems).
fn rename_via_temp(from: &Path, to: &Path) -> Result<(), ApiError> {
    let same_ignoring_case =
        from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
    if same_ignoring_case && from != to {
        let temp = from.with_extension("hikasa-rename");
        std::fs::rename(from, &temp).map_err(|e| SessionError::io(from, e))?;
        std::fs::rename(&temp, to).map_err(|e| SessionError::io(to, e))?;
    } else {
        std::fs::rename(from, to).map_err(|e| SessionError::io(from, e))?;
    }
    Ok(())
}

/// An error about the file at `path`: the path is added to the message (kinds unchanged; a bad value of a
/// file is `invalid_data`).
pub(crate) fn in_file(path: &Path, error: ApiError) -> ApiError {
    let at = path.display();
    match error {
        ApiError::Unsupported(m) => ApiError::Unsupported(format!("{at}: {m}")),
        ApiError::InvalidData(m) | ApiError::InvalidArgument(m) => {
            ApiError::InvalidData(format!("{at}: {m}"))
        }
        other => other,
    }
}

/// The warning of a session file that a listing skipped.
fn skipped(path: &Path, error: &ApiError) -> Warning {
    Warning {
        code: WarningCode::InvalidConfigValue,
        message: format!("{}: skipped: {error}", path.display()),
        spec: None,
    }
}

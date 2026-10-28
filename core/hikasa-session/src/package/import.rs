//! Import and restore (SES-061, SES-062; D-86, D-87, T12): [`plan_import`] reads and validates **every** entry of
//! a package before anything is written; [`execute_import`] then deletes, and writes the items one by one with
//! per-item results. [`plan_restore`] is the same plan against the built-ins: deletes only.
//!
//! Validation (each failure is `invalid_data` naming the entry; a newer schema is `unsupported`): a zip;
//! ≤ [`MAX_ENTRIES`] entries; each entry path `/`-separated, relative, without `.`/`..`/empty segments or
//! backslashes; no encryption, stored or deflate only; ≤ [`MAX_ENTRY_BYTES`] per entry and ≤
//! [`MAX_TOTAL_BYTES`] in total, uncompressed (also when the header lies); `manifest.toml` present and in
//! agreement with the entries (every entry listed once, every item's path allowed for its category and its
//! name matching the path); every file UTF-8 and parsing with its schema, none newer than this build.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use hikasa_core::{Warning, WarningCode};
use hikasa_formats::{FormatOrder, RegexLimits, parse_format};
use hikasa_text::TextRules;
use serde::Serialize;
use toml::Value;
use zip::CompressionMethod;

use super::manifest::{MANIFEST_FILE, Manifest, check_entry_path, item_of};
use super::{Category, category_files};
use crate::api::ApiError;
use crate::atomic::write_atomic;
use crate::error::SessionError;
use crate::paths::ConfigLocation;
use crate::sessions::store::entries;
use crate::sessions::{Session, SessionDoc, SessionRules, SessionType};
use crate::settings::{
    ArchiveTypes, CommandOverrides, FilterPresets, LoadMode, OpenWith, Options, SchemeFile,
    SettingsFile, SettingsSchema, State, ThemeSelection,
};
use crate::workspaces::Workspace;

/// Most entries a package may hold.
pub const MAX_ENTRIES: usize = 10_000;
/// Largest uncompressed entry (16 MiB).
pub const MAX_ENTRY_BYTES: u64 = 16 << 20;
/// Largest uncompressed total (64 MiB).
pub const MAX_TOTAL_BYTES: u64 = 64 << 20;

/// What to import (SES-061).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportChoices {
    /// The categories (empty: every category the package holds, except `system_info`).
    pub categories: Vec<Category>,
    /// Per category, the item names to import (absent or empty: all).
    pub items: BTreeMap<Category, Vec<String>>,
    /// "Delete all existing …" (sessions, formats and workspaces only).
    pub delete_existing: BTreeSet<Category>,
    /// Also replace (or delete) locked named sessions (SES-004; otherwise they are skipped with a warning).
    pub force: bool,
}

/// Whether an item adds a file or replaces one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAction {
    /// The target does not exist (or is deleted first).
    Add,
    /// The target exists and is replaced (a collision; matched case-insensitively, its spelling kept).
    Replace,
    /// The target is a locked named session: kept as it is (warning; `--force` replaces it).
    SkipLocked,
}

/// One item of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanItem {
    /// The category.
    pub category: Category,
    /// The item name.
    pub name: String,
    /// The file written.
    pub target: PathBuf,
    /// Add or replace.
    pub action: PlanAction,
    /// The validated bytes.
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

/// Counts of one category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct CategoryCounts {
    /// Items added.
    pub added: usize,
    /// Items replacing an existing file.
    pub replaced: usize,
    /// Files deleted.
    pub deleted: usize,
}

/// An import (or restore) plan (T12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportPlan {
    /// The items, in manifest order.
    pub items: Vec<PlanItem>,
    /// The files deleted first, sorted.
    pub deletes: Vec<PathBuf>,
    /// Per category: added, replaced, deleted.
    pub summary: BTreeMap<Category, CategoryCounts>,
    /// Warnings of the plan (`session_locked`: a locked session kept).
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// The result of one item (or of a failed delete).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemResult {
    /// The category.
    pub category: String,
    /// The item name (a failed delete: its path).
    pub name: String,
    /// Written (or deleted).
    pub ok: bool,
    /// Why not.
    pub error: Option<String>,
}

/// What [`execute_import`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ImportReport {
    /// One result per item, plus one per failed delete.
    pub results: Vec<ItemResult>,
}

impl ImportReport {
    /// True when every item and delete succeeded.
    pub fn ok(&self) -> bool {
        self.results.iter().all(|r| r.ok)
    }
}

/// A read and validated package.
#[derive(Debug, Clone)]
struct Package {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

/// Reads a package and validates every entry (nothing is written).
fn read_package(path: &Path) -> Result<Package, ApiError> {
    let shown = path.display();
    let file = std::fs::File::open(path).map_err(|e| SessionError::io(path, e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| ApiError::InvalidData(format!("{shown}: not a settings package ({e})")))?;
    if archive.len() > MAX_ENTRIES {
        return Err(ApiError::InvalidData(format!(
            "{shown}: {} entries (at most {MAX_ENTRIES})",
            archive.len()
        )));
    }
    let mut files = BTreeMap::new();
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| ApiError::InvalidData(format!("{shown}: entry {i}: {e}")))?;
        let name = entry
            .name()
            .map_err(|e| ApiError::InvalidData(format!("{shown}: entry {i}: {e}")))?
            .into_owned();
        let bad = |m: String| ApiError::InvalidData(format!("{shown}: entry {name:?}: {m}"));
        check_entry_path(&name).map_err(bad)?;
        if entry.encrypted() {
            return Err(bad("encrypted entries are not accepted".into()));
        }
        if !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(bad("only stored and deflate entries are accepted".into()));
        }
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(bad(format!("larger than {MAX_ENTRY_BYTES} bytes")));
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| bad(e.to_string()))?;
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if len > MAX_ENTRY_BYTES {
            return Err(bad(format!("larger than {MAX_ENTRY_BYTES} bytes")));
        }
        total = total.saturating_add(len);
        if total > MAX_TOTAL_BYTES {
            return Err(ApiError::InvalidData(format!(
                "{shown}: more than {MAX_TOTAL_BYTES} bytes uncompressed"
            )));
        }
        if files.insert(name.clone(), bytes).is_some() {
            return Err(bad("listed twice".into()));
        }
    }
    let manifest_bytes = files
        .remove(MANIFEST_FILE)
        .ok_or_else(|| ApiError::InvalidData(format!("{shown}: no {MANIFEST_FILE}")))?;
    let manifest = Manifest::parse(&utf8(MANIFEST_FILE, &manifest_bytes)?)?;
    let mut listed = BTreeSet::new();
    for item in &manifest.items {
        let bad = |m: String| {
            ApiError::InvalidData(format!("{MANIFEST_FILE}: item {:?}: {m}", item.path))
        };
        let category = Category::parse(&item.category).map_err(|e| match e {
            ApiError::Unsupported(m) => ApiError::Unsupported(format!("{MANIFEST_FILE}: {m}")),
            other => bad(other.to_string()),
        })?;
        match item_of(&item.path) {
            Some((c, n)) if c == category && n == item.name => {}
            Some(_) => {
                return Err(bad(format!(
                    "category {:?} and name {:?} do not match the path",
                    item.category, item.name
                )));
            }
            None => return Err(bad("not an allowed path".into())),
        }
        if !files.contains_key(&item.path) {
            return Err(bad("the package has no such entry".into()));
        }
        if !listed.insert(item.path.clone()) {
            return Err(bad("listed twice".into()));
        }
    }
    if let Some(extra) = files.keys().find(|k| !listed.contains(*k)) {
        return Err(ApiError::InvalidData(format!(
            "{shown}: entry {extra:?} is not in {MANIFEST_FILE}"
        )));
    }
    let mut folded: BTreeMap<String, &str> = BTreeMap::new();
    for path in files.keys() {
        if let Some(other) = folded.insert(path.to_lowercase(), path) {
            return Err(ApiError::InvalidData(format!(
                "{shown}: entries {other:?} and {path:?} differ only by case (names are unique whatever \
                 their case, D-80)"
            )));
        }
    }
    let limits = RegexLimits::default();
    for (path, bytes) in &files {
        validate_file(path, bytes, &limits)?;
    }
    Ok(Package { manifest, files })
}

/// The text of an entry (`invalid_data` when it is not UTF-8).
fn utf8(entry: &str, bytes: &[u8]) -> Result<String, ApiError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ApiError::InvalidData(format!("{entry}: not valid UTF-8")))
}

/// An error of an entry: `unsupported` stays (a newer schema), anything else is `invalid_data`.
fn entry_error(entry: &str, error: impl Into<ApiError>) -> ApiError {
    let error: ApiError = error.into();
    let text = error.to_string();
    // The engine's messages already start with the entry (its origin): it is not repeated.
    let message = if text.starts_with(&format!("{entry}: ")) {
        text
    } else {
        format!("{entry}: {text}")
    };
    if error.kind() == hikasa_core::ErrorKind::Unsupported {
        ApiError::Unsupported(message)
    } else {
        ApiError::InvalidData(message)
    }
}

/// Parses one entry with the schema of its path, **strictly** (a type error, a problem or an unknown type
/// refuses the package, as the commands that later read the file would), and compiles the rules of sessions
/// (folder name filters, text patterns) as the compare path does.
fn validate_file(path: &str, bytes: &[u8], limits: &RegexLimits) -> Result<(), ApiError> {
    fn settings<T: SettingsSchema>(path: &str, text: &str) -> Result<SettingsFile<T>, ApiError> {
        SettingsFile::<T>::parse(text, path, LoadMode::Strict).map_err(|e| entry_error(path, e))
    }
    let text = utf8(path, bytes)?;
    let Some((category, name)) = item_of(path) else {
        return Err(ApiError::InvalidData(format!(
            "{path}: not an allowed path"
        )));
    };
    match category {
        Category::Options => match name.as_str() {
            "options" => settings::<Options>(path, &text).map(|_| ()),
            "filters" => settings::<FilterPresets>(path, &text).map(|_| ()),
            "openwith" => settings::<OpenWith>(path, &text).map(|_| ()),
            _ => settings::<ArchiveTypes>(path, &text).map(|_| ()),
        },
        Category::Theme if name == "theme" => settings::<ThemeSelection>(path, &text).map(|_| ()),
        Category::Theme => settings::<SchemeFile>(path, &text).map(|_| ()),
        Category::Commands => settings::<CommandOverrides>(path, &text).map(|_| ()),
        Category::State => settings::<State>(path, &text).map(|_| ()),
        Category::Sessions => {
            let file = settings::<SessionDoc>(path, &text)?;
            let session = check_session(file.value(), limits).map_err(|e| entry_error(path, e))?;
            if let Some(kind) = name.strip_prefix("_defaults/")
                && SessionType::parse(kind).ok() != Some(session.kind)
            {
                return Err(ApiError::InvalidData(format!(
                    "{path}: the defaults of {kind} have type {}",
                    session.kind
                )));
            }
            Ok(())
        }
        Category::Formats if name == "order" => FormatOrder::parse(&text, path)
            .map(|_| ())
            .map_err(|e| entry_error(path, e)),
        Category::Formats => {
            let (format, _) = parse_format(&text, path).map_err(|e| entry_error(path, e))?;
            if format.id.as_str() != name {
                return Err(ApiError::InvalidData(format!(
                    "{path}: the format's id is {:?}",
                    format.id.as_str()
                )));
            }
            Ok(())
        }
        Category::Workspaces => {
            let file = settings::<Workspace>(path, &text)?;
            for (w, window) in file.value().windows.iter().enumerate() {
                for (t, tab) in window.tabs.iter().enumerate() {
                    if tab.inline.is_empty() {
                        continue;
                    }
                    let at = format!("`window.{w}.tab.{t}.inline`");
                    let doc: SessionDoc = Value::Table(tab.inline.clone()).try_into().map_err(
                        |e: toml::de::Error| {
                            entry_error(
                                path,
                                ApiError::InvalidData(format!("{at}: {}", e.message().trim())),
                            )
                        },
                    )?;
                    check_session(&doc, limits).map_err(|e| entry_error(path, within(&at, e)))?;
                }
            }
            Ok(())
        }
        Category::SystemInfo => Ok(()),
    }
}

/// `error` located at `at` (the kind kept: `unsupported` stays, anything else is `invalid_data`).
fn within(at: &str, error: ApiError) -> ApiError {
    match error {
        ApiError::Unsupported(m) => ApiError::Unsupported(format!("{at}: {m}")),
        other => ApiError::InvalidData(format!("{at}: {other}")),
    }
}

/// A session file read strictly: its type known, its rules (and the rules of its overrides) reading with
/// their types and compiling as the compare path compiles them.
fn check_session(doc: &SessionDoc, limits: &RegexLimits) -> Result<Session, ApiError> {
    let session = Session::from_doc(doc, None)?;
    let check =
        |rules: &SessionRules, text: Option<&TextRules>, at: &str| -> Result<(), ApiError> {
            let bad = |e: String| ApiError::InvalidData(format!("{at}: {e}"));
            match rules {
                SessionRules::Folder(r) => r.validate().map_err(|e| bad(e.to_string()))?,
                SessionRules::Text(r) => r.validate(limits).map_err(|e| bad(e.to_string()))?,
                SessionRules::Opaque(_) => {}
            }
            if let Some(r) = text {
                r.validate(limits)
                    .map_err(|e| ApiError::InvalidData(format!("text_rules: {e}")))?;
            }
            Ok(())
        };
    check(&session.rules, session.text_rules.as_ref(), "rules")?;
    for (i, o) in session.overrides.iter().enumerate() {
        let base = Session::builtin(o.kind);
        let mut value = base.rules.to_value();
        crate::settings::tree::overlay(&mut value, &Value::Table(o.rules.clone()));
        let at = format!("override.{i}.rules");
        let rules = SessionRules::from_value(&base.rules, value)
            .map_err(|e| ApiError::InvalidData(format!("{at}: {e}")))?;
        check(&rules, None, &at)?;
    }
    Ok(session)
}

/// Plans an import (SES-061): the package is read and validated completely first (nothing is written); the
/// items of the chosen categories and names become plan items (a target that exists, and is not deleted first,
/// is a collision: `replace`); `delete_existing` adds the existing files of those categories as deletes.
pub fn plan_import(
    config: &ConfigLocation,
    package: &Path,
    choices: &ImportChoices,
) -> Result<ImportPlan, ApiError> {
    for c in &choices.delete_existing {
        if !c.per_item() {
            return Err(ApiError::InvalidArgument(format!(
                "--delete-existing applies to sessions, formats and workspaces, not {c}"
            )));
        }
    }
    if choices.categories.contains(&Category::SystemInfo)
        || choices.items.contains_key(&Category::SystemInfo)
    {
        return Err(ApiError::InvalidArgument(
            "system_info is never imported".into(),
        ));
    }
    let pkg = read_package(package)?;
    let mut categories: BTreeSet<Category> = choices.categories.iter().copied().collect();
    categories.extend(choices.items.keys().copied());
    if categories.is_empty() {
        categories = pkg
            .manifest
            .items
            .iter()
            .filter_map(|i| Category::parse(&i.category).ok())
            .filter(|c| *c != Category::SystemInfo)
            .collect();
    }
    for (category, names) in &choices.items {
        for name in names {
            let found = pkg
                .manifest
                .items
                .iter()
                .any(|i| i.category == category.as_str() && &i.name == name);
            if !found {
                return Err(ApiError::NotFound(format!(
                    "the package has no {category} item `{name}`"
                )));
            }
        }
    }
    let mut deletes = Vec::new();
    let mut warnings = Vec::new();
    let mut summary: BTreeMap<Category, CategoryCounts> = BTreeMap::new();
    for category in &choices.delete_existing {
        for (name, path) in category_files(&config.dir, *category) {
            let full = config.dir.join(&path);
            if *category == Category::Sessions && !choices.force && is_locked(&full) {
                warnings.push(locked_warning(&name, "deleted"));
                continue;
            }
            deletes.push(full);
            summary.entry(*category).or_default().deleted += 1;
        }
    }
    deletes.sort();
    let mut items: Vec<PlanItem> = Vec::new();
    for item in &pkg.manifest.items {
        let Ok(category) = Category::parse(&item.category) else {
            continue;
        };
        if !categories.contains(&category) {
            continue;
        }
        if let Some(names) = choices.items.get(&category).filter(|n| !n.is_empty())
            && !names.contains(&item.name)
        {
            continue;
        }
        let planned: Vec<&Path> = items.iter().map(|i| i.target.as_path()).collect();
        let target = resolve_target(&config.dir, &item.path, &planned);
        let exists = target.is_file() && !deletes.contains(&target);
        let action = if !exists {
            PlanAction::Add
        } else if category == Category::Sessions
            && !item.name.starts_with("_defaults/")
            && !choices.force
            && is_locked(&target)
        {
            warnings.push(locked_warning(&item.name, "replaced"));
            PlanAction::SkipLocked
        } else {
            PlanAction::Replace
        };
        let counts = summary.entry(category).or_default();
        match action {
            PlanAction::Add => counts.added += 1,
            PlanAction::Replace => counts.replaced += 1,
            PlanAction::SkipLocked => {}
        }
        items.push(PlanItem {
            category,
            name: item.name.clone(),
            target,
            action,
            bytes: pkg.files.get(&item.path).cloned().unwrap_or_default(),
        });
    }
    Ok(ImportPlan {
        items,
        deletes,
        summary,
        warnings,
    })
}

/// The file an item at the configuration-relative `path` writes (D-80): each folder and the file name are
/// matched case-insensitively against what exists (and against the targets `planned` before it), and an
/// existing spelling is kept; otherwise the package's spelling is used.
fn resolve_target(config_dir: &Path, path: &str, planned: &[&Path]) -> PathBuf {
    let mut out = config_dir.to_path_buf();
    for segment in path.split('/') {
        let wanted = segment.to_lowercase();
        let on_disk = entries(&out).into_iter().map(|(name, _)| name);
        let earlier = planned.iter().filter_map(|p| {
            p.strip_prefix(&out)
                .ok()
                .and_then(|rest| rest.components().next())
                .and_then(|c| c.as_os_str().to_str())
                .map(str::to_owned)
        });
        let mut candidates: Vec<String> = on_disk.chain(earlier).collect();
        candidates.sort();
        let found = if candidates.iter().any(|c| c == segment) {
            None
        } else {
            candidates.into_iter().find(|c| c.to_lowercase() == wanted)
        };
        match found {
            Some(existing) => out.push(existing),
            None => out.push(segment),
        }
    }
    out
}

/// True when the session file at `path` exists and is locked.
fn is_locked(path: &Path) -> bool {
    SettingsFile::<SessionDoc>::load(path, LoadMode::Lenient).is_ok_and(|f| f.value().locked)
}

/// The warning of a locked session the import keeps (SES-004).
fn locked_warning(name: &str, what: &str) -> Warning {
    Warning {
        code: WarningCode::SessionLocked,
        message: format!(
            "the session `{name}` is locked and is not {what} (unlock it, or import with --force)"
        ),
        spec: None,
    }
}

/// Executes a plan: the deletes, then the items in plan order; a failure of one item is recorded and the
/// others continue.
pub fn execute_import(config: &ConfigLocation, plan: &ImportPlan) -> ImportReport {
    let mut report = ImportReport::default();
    for path in &plan.deletes {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => report.results.push(ItemResult {
                category: category_of(config, path),
                name: path.display().to_string(),
                ok: false,
                error: Some(SessionError::io(path, e).to_string()),
            }),
        }
    }
    for item in plan
        .items
        .iter()
        .filter(|i| i.action != PlanAction::SkipLocked)
    {
        let written = item
            .target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| write_atomic(&item.target, &item.bytes))
            .map_err(|e| SessionError::io(&item.target, e).to_string());
        report.results.push(ItemResult {
            category: item.category.as_str().to_owned(),
            name: item.name.clone(),
            ok: written.is_ok(),
            error: written.err(),
        });
    }
    report
}

/// The category of a configuration file (for the result of a failed delete).
fn category_of(config: &ConfigLocation, path: &Path) -> String {
    path.strip_prefix(&config.dir)
        .ok()
        .and_then(|p| p.to_str())
        .map(|p| p.replace('\\', "/"))
        .and_then(|p| item_of(&p))
        .map_or_else(String::new, |(c, _)| c.as_str().to_owned())
}

/// Restore factory defaults (D-87, SES-062): a plan of deletes only. Per category the user files are deleted
/// (options, theme and scheme, commands, state files; `sessions/_defaults/*.toml`; every format file and
/// `order.toml`); named sessions and workspaces only with `delete_sessions` / `delete_workspaces`.
pub fn plan_restore(
    config: &ConfigLocation,
    categories: &[Category],
    delete_sessions: bool,
    delete_workspaces: bool,
) -> ImportPlan {
    let mut deletes: Vec<(Category, PathBuf)> = Vec::new();
    let mut wanted: BTreeSet<Category> = categories.iter().copied().collect();
    if delete_sessions {
        wanted.insert(Category::Sessions);
    }
    if delete_workspaces {
        wanted.insert(Category::Workspaces);
    }
    for category in wanted {
        for (name, path) in category_files(&config.dir, category) {
            let take = match category {
                Category::Sessions => delete_sessions || name.starts_with("_defaults/"),
                Category::Workspaces => delete_workspaces,
                _ => true,
            };
            if take {
                deletes.push((category, config.dir.join(path)));
            }
        }
    }
    let mut summary: BTreeMap<Category, CategoryCounts> = BTreeMap::new();
    for (category, _) in &deletes {
        summary.entry(*category).or_default().deleted += 1;
    }
    let mut paths: Vec<PathBuf> = deletes.into_iter().map(|(_, p)| p).collect();
    paths.sort();
    ImportPlan {
        items: Vec::new(),
        deletes: paths,
        summary,
        warnings: Vec::new(),
    }
}

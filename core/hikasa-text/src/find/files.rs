//! Find in Files (TC-064; D-284; contract §1.10.19): [`find_in_files`] searches the files of folders through the
//! VFS with one [`Finder`] and streams [`FilesEvent`]s to a [`FilesSink`].
//!
//! ```text
//! traversal   each root: a file as given (an archive opened with `archives = opened`); a folder listed through the
//!             VFS, sorted by name (Listing::sort_by_name); files by the name masks (FC-050), subfolders with
//!             `recursive`, hidden items (a leading `.`, the DOS hidden bit) with `include_hidden`; links followed
//!             for files, not for folders; archives as folders with `archives = opened`
//! each file   larger than max_file_bytes: Skipped TooLarge (before reading); a format with a conversion (External,
//!             Sorted) without `convert`: Skipped Converted; load_document (detected or forced encoding, no CPL:
//!             hits are on physical lines); binary: Skipped Binary; else File, Hit…, Done
//! order       the traversal's pre-order whatever the pool: files are searched on the pool in a window of 64 (and
//!             at most 256 MiB of files) in flight, each worker buffering at most 1,024 hits; the caller's thread
//!             delivers the files' events in order, continuing a file past its buffer itself, so the events are
//!             the same with 1 or 8 threads (T2) and memory never grows with the hits
//! limits      max_matches: the search stops (warning find_limit_reached); max_matches_per_file: that file stops
//!             (its Done says `stopped`); previews cut to preview_chars characters around the hit
//! ```

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::{ControlFlow, Range};
use std::path::Path;
use std::sync::{Arc, Mutex};

use hikasa_core::{
    CancelToken, Ctl, ErrorReport, FolderVerdict, NameFilter, Phase, Progress, RelPath, Spec, Unit,
    Warning, WarningCode,
};
use hikasa_formats::{
    CompiledFormat, Conversion, EnabledOverrides, FormatRegistry, ProcessRunner, TextEncoding,
};
use hikasa_vfs::{DosAttrs, Entry, EntryKind, FollowLinks, ListOptions, Vfs};
use serde::{Deserialize, Serialize};

use super::{FindError, FindQuery, Finder};
use crate::document::{LoadOptions, TextDocument, load_document};
use crate::edit::TextPos;
use crate::error::TextError;

/// What an archive file is in a Find in Files (D-284).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindArchives {
    /// A file: searched as its bytes say (an archive is binary and skipped). The open default.
    #[default]
    Files,
    /// A folder: its members are searched (`recursive` decides about its folders).
    Opened,
}

impl FindArchives {
    /// `files`, `opened`.
    pub const fn as_str(self) -> &'static str {
        match self {
            FindArchives::Files => "files",
            FindArchives::Opened => "opened",
        }
    }
}

/// The limits of a Find in Files (D-284's open defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesLimits {
    /// Hits after which the search stops. Default 100,000.
    pub max_matches: u64,
    /// Hits after which a file stops. Default 10,000.
    pub max_matches_per_file: u32,
    /// Files larger than this are skipped before they are read. Default 128 MiB (`DEFAULT_RULES_MAX_BYTES`).
    pub max_file_bytes: u64,
    /// Characters of a preview. Default 400.
    pub preview_chars: u32,
}

impl Default for FilesLimits {
    fn default() -> FilesLimits {
        FilesLimits {
            max_matches: 100_000,
            max_matches_per_file: 10_000,
            max_file_bytes: crate::quick::DEFAULT_RULES_MAX_BYTES,
            preview_chars: 400,
        }
    }
}

/// A Find in Files request (§1.10.19).
#[derive(Debug, Clone)]
pub struct FilesQuery {
    /// What to find.
    pub find: FindQuery,
    /// Files and folders searched, in this order.
    pub roots: Vec<Spec>,
    /// The name masks (FC-050 syntax): files by their names, folders by theirs. Roots are searched as given.
    pub masks: NameFilter,
    /// Folders below the roots are searched.
    pub recursive: bool,
    /// Hidden files and folders (a name starting with `.`, the DOS hidden attribute) are searched.
    pub include_hidden: bool,
    /// Archives as files or opened.
    pub archives: FindArchives,
    /// The limits.
    pub limits: FilesLimits,
    /// The encoding every file is read in (else each file's detection, D-42).
    pub encoding: Option<TextEncoding>,
    /// Files whose format converts them (External, Sorted) are searched in their converted text; else skipped.
    pub convert: bool,
    /// One format for every file (additive; else the registry's detection by name).
    pub format: Option<Arc<CompiledFormat>>,
    /// The detection's enabled overrides (additive; FF-002).
    pub overrides: EnabledOverrides,
    /// The locale's encoding, the detection's fallback (additive; core reads no locale).
    pub locale: TextEncoding,
}

impl FilesQuery {
    /// A query of `find` over `roots` with every other field at its default (all names, recursive, no hidden files,
    /// archives as files, the default limits, detected encodings, no conversion).
    pub fn new(find: FindQuery, roots: Vec<Spec>) -> FilesQuery {
        FilesQuery {
            find,
            roots,
            masks: NameFilter::match_all(hikasa_core::MaskOptions {
                case: hikasa_core::CaseSensitivity::HOST_DEFAULT,
            }),
            recursive: true,
            include_hidden: false,
            archives: FindArchives::Files,
            limits: FilesLimits::default(),
            encoding: None,
            convert: false,
            format: None,
            overrides: EnabledOverrides::default(),
            locale: TextEncoding::UTF_8,
        }
    }
}

/// Why a file was not searched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "reason", content = "error")]
pub enum SkipWhy {
    /// Its content is binary (an archive searched as a file included).
    Binary,
    /// Larger than `max_file_bytes`.
    TooLarge,
    /// It (or the folder) could not be read, listed or converted.
    Unreadable(ErrorReport),
    /// Its format converts it and `convert` is off.
    Converted,
    /// A link to a folder (links are followed for files only, so a search never loops).
    Filtered,
}

impl SkipWhy {
    /// `binary`, `too_large`, `unreadable`, `converted`, `filtered` (JSON `reason`).
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipWhy::Binary => "binary",
            SkipWhy::TooLarge => "too_large",
            SkipWhy::Unreadable(_) => "unreadable",
            SkipWhy::Converted => "converted",
            SkipWhy::Filtered => "filtered",
        }
    }
}

/// One event of a Find in Files, in the traversal's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesEvent<'a> {
    /// A file is searched: its index among the searched files (from 0) and the encoding it was read in.
    File {
        /// Index among the searched files.
        index: u64,
        /// The file.
        spec: &'a Spec,
        /// Its encoding.
        encoding: TextEncoding,
    },
    /// A hit of the current file.
    Hit {
        /// The file's index.
        file: u64,
        /// 1-based physical line (as the gutter numbers lines, TC-003).
        line: u32,
        /// 1-based column in characters.
        column: u32,
        /// Length in characters.
        len_chars: u32,
        /// The line, cut to `preview_chars` characters around the hit.
        preview: &'a str,
        /// The hit in the preview, in characters.
        preview_range: Range<u32>,
        /// The preview is cut.
        truncated: bool,
    },
    /// A file or folder not searched.
    Skipped {
        /// The file or folder.
        spec: &'a Spec,
        /// Why.
        reason: SkipWhy,
    },
    /// The end of a searched file.
    Done {
        /// The file's index.
        file: u64,
        /// Its hits delivered.
        hits: u64,
        /// The file stopped at `max_matches_per_file` (or the search at `max_matches`) before its end.
        stopped: bool,
    },
}

/// Receives the events of [`find_in_files`] on the caller's thread.
pub trait FilesSink {
    /// One event.
    fn event(&mut self, e: FilesEvent<'_>);
}

/// What a Find in Files found (the counts of `hikasa.text_find.v1`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FilesSummary {
    /// Files searched.
    pub files_searched: u64,
    /// Files with at least one hit.
    pub files_matched: u64,
    /// Hits delivered.
    pub hits: u64,
    /// Files and folders skipped.
    pub skipped: u64,
    /// `max_matches` was reached: the rest was not searched.
    pub limit_reached: bool,
    /// `find_limit_reached`, `regex_runtime_error`.
    pub warnings: Vec<Warning>,
}

/// Hits a worker buffers for a file before the caller's thread continues it.
const BUFFER_HITS: usize = 1024;
/// Files in flight at most.
const WINDOW_FILES: usize = 64;
/// Bytes of files in flight at most (one file always goes).
const WINDOW_BYTES: u64 = 256 << 20;

/// One hit, owned (a worker's buffer).
#[derive(Debug, Clone)]
struct OwnedHit {
    line: u32,
    column: u32,
    len_chars: u32,
    preview: String,
    preview_range: Range<u32>,
    truncated: bool,
}

/// The result of one file.
enum Outcome {
    Skipped(SkipWhy),
    Searched {
        encoding: TextEncoding,
        hits: Vec<OwnedHit>,
        /// Where the caller's thread continues (the buffer filled): the document, the finder and the position.
        resume: Option<(Box<TextDocument>, Arc<Finder>, TextPos)>,
    },
    /// A worker's load or search was cancelled.
    Cancelled,
}

/// Searches `q` (§1.10.19; module documentation) and streams the events to `sink`. `Err` for an invalid query, a
/// root that cannot be read (`Load`) and a cancel; a file or folder that fails is `Skipped Unreadable`.
pub fn find_in_files(
    vfs: &dyn Vfs,
    registry: &FormatRegistry,
    q: &FilesQuery,
    sink: &mut dyn FilesSink,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
) -> Result<FilesSummary, TextError> {
    find_in_files_with(vfs, registry, q, None, sink, pool, ctl)
}

/// [`find_in_files`] with the temporary folder and the process runner of External conversions (`convert`).
pub fn find_in_files_with(
    vfs: &dyn Vfs,
    registry: &FormatRegistry,
    q: &FilesQuery,
    external: Option<(&Path, &dyn ProcessRunner)>,
    sink: &mut dyn FilesSink,
    pool: Option<&rayon::ThreadPool>,
    ctl: &Ctl<'_>,
) -> Result<FilesSummary, TextError> {
    let finders = Finders::new(q, registry)?;
    // The roots are checked first: a missing root refuses the search (exit 100 at the CLI).
    let mut walk = Walk::new(vfs, q, ctl)?;
    let search = Search {
        vfs,
        registry,
        q,
        external,
        finders: &finders,
    };
    let stop = ctl.cancel.child();
    let mut out = Deliver {
        sink,
        q,
        summary: FilesSummary::default(),
        index: 0,
        bytes_done: 0,
    };
    match pool {
        None => {
            while let Some(item) = walk.next(ctl)? {
                ctl.check()?;
                let outcome = match &item {
                    Item::File(spec, size) => search.file(spec, *size, &stop),
                    Item::Skip(_, _) => Outcome::Cancelled,
                };
                if out.item(item, outcome, ctl)?.is_break() {
                    break;
                }
            }
        }
        Some(pool) => {
            let result: Result<(), TextError> = pool.in_place_scope(|scope| {
                let (tx, rx) = std::sync::mpsc::channel::<(u64, Outcome)>();
                let mut pending: VecDeque<(u64, Item, u64)> = VecDeque::new();
                let mut ready: BTreeMap<u64, Outcome> = BTreeMap::new();
                let mut seq = 0u64;
                let mut in_flight_bytes = 0u64;
                let mut walked_all = false;
                loop {
                    // Fill the window.
                    while !walked_all
                        && pending.len() < WINDOW_FILES
                        && (pending.is_empty() || in_flight_bytes < WINDOW_BYTES)
                    {
                        match walk.next(ctl)? {
                            None => walked_all = true,
                            Some(item) => {
                                let size = match &item {
                                    Item::File(spec, size) => {
                                        let size = *size;
                                        let tx = tx.clone();
                                        let (spec, stop) = (spec.clone(), stop.clone());
                                        let search = &search;
                                        let n = seq;
                                        scope.spawn(move |_| {
                                            let outcome = search.file(&spec, size, &stop);
                                            let _ = tx.send((n, outcome));
                                        });
                                        size
                                    }
                                    Item::Skip(_, _) => {
                                        ready.insert(seq, Outcome::Cancelled);
                                        0
                                    }
                                };
                                in_flight_bytes += size;
                                pending.push_back((seq, item, size));
                                seq += 1;
                            }
                        }
                    }
                    let Some((head, _, _)) = pending.front() else {
                        return Ok(());
                    };
                    let head = *head;
                    while !ready.contains_key(&head) {
                        ctl.check()?;
                        match rx.recv_timeout(std::time::Duration::from_millis(20)) {
                            Ok((n, outcome)) => {
                                ready.insert(n, outcome);
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                return Err(TextError::Cancelled);
                            }
                        }
                    }
                    let (_, item, size) = pending.pop_front().ok_or(TextError::Cancelled)?;
                    in_flight_bytes -= size;
                    let outcome = ready.remove(&head).ok_or(TextError::Cancelled)?;
                    if out.item(item, outcome, ctl)?.is_break() {
                        // The rest is not searched: the workers in flight stop at their next check.
                        stop.cancel();
                        return Ok(());
                    }
                }
            });
            if result.is_err() {
                stop.cancel();
            }
            result?;
        }
    }
    ctl.check()?;
    let mut summary = out.summary;
    if let Some(w) = finders.take_warning() {
        summary.warnings.push(w);
    }
    Ok(summary)
}

/// The finders by word characters (whole word depends on each file's format, D-58).
struct Finders {
    query: FindQuery,
    limits: hikasa_formats::RegexLimits,
    plain: Arc<Finder>,
    by_words: Mutex<HashMap<String, Arc<Finder>>>,
}

impl Finders {
    fn new(q: &FilesQuery, registry: &FormatRegistry) -> Result<Finders, TextError> {
        let limits = *registry.limits();
        let plain = Arc::new(Finder::new(&q.find, "", &limits)?);
        Ok(Finders {
            query: q.find.clone(),
            limits,
            plain,
            by_words: Mutex::new(HashMap::new()),
        })
    }

    /// The finder for a format's word characters.
    fn get(&self, word_chars: &str) -> Arc<Finder> {
        if !self.query.whole_word || word_chars.is_empty() {
            return Arc::clone(&self.plain);
        }
        let Ok(mut map) = self.by_words.lock() else {
            return Arc::clone(&self.plain);
        };
        if let Some(f) = map.get(word_chars) {
            return Arc::clone(f);
        }
        match Finder::new(&self.query, word_chars, &self.limits) {
            Ok(f) => {
                let f = Arc::new(f);
                map.insert(word_chars.to_owned(), Arc::clone(&f));
                f
            }
            Err(_) => Arc::clone(&self.plain),
        }
    }

    /// One `regex_runtime_error` for the whole search.
    fn take_warning(&self) -> Option<Warning> {
        let mut found = self.plain.take_warning();
        if let Ok(map) = self.by_words.lock() {
            for f in map.values() {
                if let Some(w) = f.take_warning()
                    && found.is_none()
                {
                    found = Some(w);
                }
            }
        }
        found
    }
}

/// An item of the traversal.
enum Item {
    /// A file to search, with its size.
    File(Spec, u64),
    /// A file or folder skipped by the traversal.
    Skip(Spec, SkipWhy),
}

/// A root: a file as given, or a folder (an opened archive) walked in its place.
enum Root {
    File(Spec, u64),
    Dir(Spec),
}

/// One folder being walked.
struct Frame {
    dir: Spec,
    path: RelPath,
    verdict: FolderVerdict,
    entries: std::vec::IntoIter<Entry>,
    errors: VecDeque<(Spec, ErrorReport)>,
}

/// The pre-order traversal of the roots.
struct Walk<'a> {
    vfs: &'a dyn Vfs,
    q: &'a FilesQuery,
    roots: VecDeque<Root>,
    stack: Vec<Frame>,
    /// Folders still to list for the roots (in order, after the files before them).
    pending_dirs: VecDeque<Spec>,
}

/// A hidden name: a leading `.` or the DOS hidden attribute.
fn is_hidden(entry: &Entry) -> bool {
    entry.name.as_str().starts_with('.')
        || entry
            .attrs
            .dos()
            .is_some_and(|a| a.contains(DosAttrs::HIDDEN))
}

impl<'a> Walk<'a> {
    fn new(vfs: &'a dyn Vfs, q: &'a FilesQuery, ctl: &Ctl<'_>) -> Result<Walk<'a>, TextError> {
        let mut roots = VecDeque::new();
        for root in &q.roots {
            ctl.check()?;
            let entry = vfs
                .stat(root, FollowLinks::Yes)
                .map_err(|source| TextError::Load {
                    side: hikasa_core::Side::Left,
                    source,
                })?;
            match entry.kind {
                EntryKind::Dir => roots.push_back(Root::Dir(root.clone())),
                _ => {
                    let archive = q.archives == FindArchives::Opened
                        && root
                            .file_name()
                            .is_some_and(|n| vfs.archive_format(&n).is_some());
                    if archive {
                        roots.push_back(Root::Dir(Spec::archive_member(root.clone(), "")));
                    } else {
                        roots.push_back(Root::File(root.clone(), entry.size));
                    }
                }
            }
        }
        Ok(Walk {
            vfs,
            q,
            roots,
            stack: Vec::new(),
            pending_dirs: VecDeque::new(),
        })
    }

    /// Lists a folder into a new frame (sorted by name); a failed listing is a skipped folder.
    fn open(
        &mut self,
        dir: Spec,
        path: RelPath,
        verdict: FolderVerdict,
        ctl: &Ctl<'_>,
    ) -> Result<Option<Item>, TextError> {
        let opts = ListOptions {
            follow: FollowLinks::Yes,
            prefilter: None,
        };
        match self.vfs.list(&dir, &opts, ctl.cancel) {
            Ok(mut listing) => {
                listing.sort_by_name();
                let mut errors = VecDeque::new();
                for e in listing.errors {
                    let described = listing.entries.iter().any(|x| x.name == e.name);
                    if !described && let Ok(spec) = self.vfs.child(&dir, &e.name) {
                        errors.push_back((spec, e.error));
                    }
                }
                self.stack.push(Frame {
                    dir,
                    path,
                    verdict,
                    entries: listing.entries.into_iter(),
                    errors,
                });
                Ok(None)
            }
            Err(e) if e.is_cancelled() => Err(TextError::Cancelled),
            Err(e) => {
                let report = ErrorReport::capture(&e, Some(&dir));
                Ok(Some(Item::Skip(dir, SkipWhy::Unreadable(report))))
            }
        }
    }

    /// The next file or skipped item in pre-order.
    fn next(&mut self, ctl: &Ctl<'_>) -> Result<Option<Item>, TextError> {
        loop {
            ctl.check()?;
            if let Some(frame) = self.stack.last_mut() {
                if let Some((spec, error)) = frame.errors.pop_front() {
                    return Ok(Some(Item::Skip(spec, SkipWhy::Unreadable(error))));
                }
                let Some(entry) = frame.entries.next() else {
                    self.stack.pop();
                    continue;
                };
                let name = entry.name.as_str().to_owned();
                if !self.q.include_hidden && is_hidden(&entry) {
                    continue;
                }
                let Ok(spec) = self.vfs.child(&frame.dir, &entry.name) else {
                    continue;
                };
                let (path, verdict) = (frame.path.clone(), frame.verdict);
                match entry.kind {
                    EntryKind::Dir => {
                        if !self.q.recursive {
                            continue;
                        }
                        if entry.followed {
                            return Ok(Some(Item::Skip(spec, SkipWhy::Filtered)));
                        }
                        let v = self.q.masks.folder(&path, &name, verdict);
                        if v == FolderVerdict::Excluded {
                            continue;
                        }
                        let Ok(child) = path.join(&name) else {
                            continue;
                        };
                        if let Some(item) = self.open(spec, child, v, ctl)? {
                            return Ok(Some(item));
                        }
                    }
                    EntryKind::File => {
                        let archive = self.q.archives == FindArchives::Opened
                            && self.vfs.archive_format(&name).is_some();
                        if archive {
                            if !self.q.recursive {
                                continue;
                            }
                            let v = self.q.masks.folder(&path, &name, verdict);
                            if v == FolderVerdict::Excluded {
                                continue;
                            }
                            let Ok(child) = path.join(&name) else {
                                continue;
                            };
                            if let Some(item) =
                                self.open(Spec::archive_member(spec, ""), child, v, ctl)?
                            {
                                return Ok(Some(item));
                            }
                            continue;
                        }
                        if self.q.masks.file(&path, &name, verdict) {
                            return Ok(Some(Item::File(spec, entry.size)));
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if let Some(dir) = self.pending_dirs.pop_front() {
                if let Some(item) = self.open(dir, RelPath::root(), FolderVerdict::Base, ctl)? {
                    return Ok(Some(item));
                }
                continue;
            }
            match self.roots.pop_front() {
                None => return Ok(None),
                // A root folder (or an opened root archive) is walked here, in its place.
                Some(Root::Dir(dir)) => self.pending_dirs.push_back(dir),
                Some(Root::File(spec, size)) => return Ok(Some(Item::File(spec, size))),
            }
        }
    }
}

/// The per-file work (Sync: the workers share it).
struct Search<'a> {
    vfs: &'a dyn Vfs,
    registry: &'a FormatRegistry,
    q: &'a FilesQuery,
    external: Option<(&'a Path, &'a dyn ProcessRunner)>,
    finders: &'a Finders,
}

impl Search<'_> {
    /// Loads and searches one file, buffering at most [`BUFFER_HITS`] hits.
    fn file(&self, spec: &Spec, size: u64, stop: &CancelToken) -> Outcome {
        if stop.is_cancelled() {
            return Outcome::Cancelled;
        }
        if size > self.q.limits.max_file_bytes {
            return Outcome::Skipped(SkipWhy::TooLarge);
        }
        let format = match &self.q.format {
            Some(f) => Arc::clone(f),
            None => {
                let name = spec.file_name().unwrap_or_default();
                match self.registry.detect(&name, &self.q.overrides) {
                    Ok(d) => d.format,
                    Err(e) => {
                        return Outcome::Skipped(SkipWhy::Unreadable(ErrorReport::capture(
                            &e,
                            Some(spec),
                        )));
                    }
                }
            }
        };
        let converts = !matches!(format.conversion(), Conversion::None);
        if converts && !self.q.convert {
            return Outcome::Skipped(SkipWhy::Converted);
        }
        let mut opts = LoadOptions::default();
        opts.decode.locale_fallback = self.q.locale;
        opts.decode.override_encoding = self.q.encoding;
        opts.chars_per_line_limit = Some(0);
        opts.max_bytes = self.q.limits.max_file_bytes;
        let progress = hikasa_core::NullProgress;
        let ctl = Ctl {
            cancel: stop,
            progress: &progress,
        };
        let doc = match load_document(self.vfs, spec, &format, &opts, self.external, &ctl) {
            Ok((doc, _raw)) => doc,
            Err(e) if e.is_cancelled() => return Outcome::Cancelled,
            Err(TextError::TooLarge { .. }) => return Outcome::Skipped(SkipWhy::TooLarge),
            Err(e) => {
                return Outcome::Skipped(SkipWhy::Unreadable(ErrorReport::capture(&e, Some(spec))));
            }
        };
        if doc.binary().is_binary {
            return Outcome::Skipped(SkipWhy::Binary);
        }
        let finder = self.finders.get(&format.text().word_chars);
        let encoding = doc.encoding().encoding;
        let cap = (self.q.limits.max_matches_per_file as usize).min(BUFFER_HITS);
        let mut hits = Vec::new();
        let mut next = None;
        let mut columns = Columns::default();
        let preview = self.q.limits.preview_chars;
        let res = finder.scan(&doc, TextPos::ZERO, &ctl, &mut |hit, line, range| {
            if hits.len() >= cap {
                next = Some(hit.range.start);
                return ControlFlow::Break(());
            }
            hits.push(owned_hit(&doc, &mut columns, &hit, line, range, preview));
            ControlFlow::Continue(())
        });
        match res {
            Err(FindError::Cancelled) => Outcome::Cancelled,
            Err(_) => Outcome::Cancelled,
            Ok(()) => Outcome::Searched {
                encoding,
                hits,
                resume: next.map(|pos| (Box::new(doc), finder, pos)),
            },
        }
    }
}

/// The running character column of the hits of one physical line (hits come in order, so a long line is counted
/// once).
#[derive(Default)]
struct Columns {
    line: Option<TextPos>,
    byte: usize,
    chars: u32,
}

impl Columns {
    fn column(&mut self, unit: TextPos, text: &str, at: usize) -> u32 {
        if self.line != Some(unit) || at < self.byte {
            self.line = Some(unit);
            self.byte = 0;
            self.chars = 0;
        }
        self.chars += text
            .get(self.byte..at)
            .map_or(0, |s| s.chars().count() as u32);
        self.byte = at;
        self.chars + 1
    }
}

/// A hit's line, column, length and preview.
fn owned_hit(
    doc: &TextDocument,
    columns: &mut Columns,
    hit: &super::Hit,
    text: &str,
    range: Range<usize>,
    preview_chars: u32,
) -> OwnedHit {
    let line = doc.physical_line(hit.range.start.line);
    let unit = TextPos::new(line, 0);
    let column = columns.column(unit, text, range.start);
    let matched = text.get(range.clone()).unwrap_or("");
    let len_chars = matched.chars().count() as u32;
    let (preview, preview_range, truncated) = preview_of(text, range, len_chars, preview_chars);
    OwnedHit {
        line,
        column,
        len_chars,
        preview,
        preview_range,
        truncated,
    }
}

/// The line cut to `p` characters around the hit `range` (`len` characters): the hit centred, the room a line edge
/// leaves given to the other side; a hit longer than `p` shows its first `p` characters.
fn preview_of(text: &str, range: Range<usize>, len: u32, p: u32) -> (String, Range<u32>, bool) {
    let p = p.max(1);
    if text.len() <= p as usize {
        // At most `p` bytes: at most `p` characters.
        let start = text[..range.start].chars().count() as u32;
        return (text.to_owned(), start..start + len, false);
    }
    if len >= p {
        let end = text[range.start..]
            .char_indices()
            .nth(p as usize)
            .map_or(text.len(), |(k, _)| range.start + k);
        let truncated = range.start > 0 || end < text.len();
        return (text[range.start..end].to_owned(), 0..p, truncated);
    }
    let room = p - len;
    let before_avail = text[..range.start]
        .chars()
        .rev()
        .take(room as usize)
        .count() as u32;
    let after_avail = text[range.end..].chars().take(room as usize).count() as u32;
    let mut before = room / 2;
    let mut after = room - before;
    if before_avail < before {
        after += before - before_avail;
        before = before_avail;
    }
    if after_avail < after {
        before = (before + (after - after_avail)).min(before_avail);
        after = after_avail;
    }
    let start = if before == 0 {
        range.start
    } else {
        text[..range.start]
            .char_indices()
            .rev()
            .nth(before as usize - 1)
            .map_or(0, |(k, _)| k)
    };
    let end = text[range.end..]
        .char_indices()
        .nth(after as usize)
        .map_or(text.len(), |(k, _)| range.end + k);
    let truncated = start > 0 || end < text.len();
    (text[start..end].to_owned(), before..before + len, truncated)
}

/// Delivers items in order on the caller's thread.
struct Deliver<'s, 'q> {
    sink: &'s mut dyn FilesSink,
    q: &'q FilesQuery,
    summary: FilesSummary,
    index: u64,
    bytes_done: u64,
}

impl Deliver<'_, '_> {
    /// One item's events; `Break` when the search stops (`max_matches`).
    fn item(
        &mut self,
        item: Item,
        outcome: Outcome,
        ctl: &Ctl<'_>,
    ) -> Result<ControlFlow<()>, TextError> {
        let (spec, size) = match item {
            Item::Skip(spec, reason) => {
                self.summary.skipped += 1;
                self.sink.event(FilesEvent::Skipped {
                    spec: &spec,
                    reason,
                });
                return Ok(ControlFlow::Continue(()));
            }
            Item::File(spec, size) => (spec, size),
        };
        self.bytes_done += size;
        ctl.progress(Phase::Searching, || Progress {
            phase: Phase::Searching,
            done: self.bytes_done,
            total: None,
            unit: Unit::Bytes,
            current: Some(spec.canonical()),
        });
        let (encoding, hits, resume) = match outcome {
            Outcome::Cancelled => return Err(TextError::Cancelled),
            Outcome::Skipped(reason) => {
                self.summary.skipped += 1;
                self.sink.event(FilesEvent::Skipped {
                    spec: &spec,
                    reason,
                });
                return Ok(ControlFlow::Continue(()));
            }
            Outcome::Searched {
                encoding,
                hits,
                resume,
            } => (encoding, hits, resume),
        };
        let file = self.index;
        self.index += 1;
        self.summary.files_searched += 1;
        self.sink.event(FilesEvent::File {
            index: file,
            spec: &spec,
            encoding,
        });
        let per_file = u64::from(self.q.limits.max_matches_per_file);
        let mut count = 0u64;
        let mut stopped = false;
        let mut stop_all = false;
        let mut emit = |h: &OwnedHit, summary: &mut FilesSummary, sink: &mut dyn FilesSink| {
            if summary.hits >= self.q.limits.max_matches {
                stop_all = true;
                return ControlFlow::Break(());
            }
            if count >= per_file {
                stopped = true;
                return ControlFlow::Break(());
            }
            sink.event(FilesEvent::Hit {
                file,
                line: h.line,
                column: h.column,
                len_chars: h.len_chars,
                preview: &h.preview,
                preview_range: h.preview_range.clone(),
                truncated: h.truncated,
            });
            count += 1;
            summary.hits += 1;
            ControlFlow::Continue(())
        };
        let mut broke = false;
        for h in &hits {
            if emit(h, &mut self.summary, &mut *self.sink).is_break() {
                broke = true;
                break;
            }
        }
        if !broke && let Some((doc, finder, pos)) = resume {
            // The worker's buffer filled: the rest of the file is searched here, streamed.
            let mut columns = Columns::default();
            let preview = self.q.limits.preview_chars;
            let sink = &mut *self.sink;
            let summary = &mut self.summary;
            finder
                .scan(&doc, pos, ctl, &mut |hit, line, range| {
                    let h = owned_hit(&doc, &mut columns, &hit, line, range, preview);
                    emit(&h, summary, sink)
                })
                .map_err(|e| match e {
                    // One shape for a cancel, wherever it lands (the walk, a worker, this scan).
                    FindError::Cancelled => TextError::Cancelled,
                    e => TextError::Find(e),
                })?;
        }
        if count > 0 {
            self.summary.files_matched += 1;
        }
        // A file that reached the global limit with more to find also says so.
        let at_limit = self.summary.hits >= self.q.limits.max_matches;
        self.sink.event(FilesEvent::Done {
            file,
            hits: count,
            stopped: stopped || stop_all,
        });
        if stop_all || at_limit {
            if stop_all || !self.summary.limit_reached {
                self.summary.limit_reached = true;
                self.summary.warnings.push(Warning {
                    code: WarningCode::FindLimitReached,
                    message: format!(
                        "the search stopped at {} hits (--max-matches); the rest was not searched",
                        self.q.limits.max_matches
                    ),
                    spec: None,
                });
            }
            return Ok(ControlFlow::Break(()));
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews() {
        let (p, r, t) = preview_of("abcdefghij", 4..6, 2, 6);
        assert_eq!((p.as_str(), r, t), ("cdefgh", 2..4, true));
        let (p, r, t) = preview_of("abcdefghij", 0..2, 2, 6);
        assert_eq!((p.as_str(), r, t), ("abcdef", 0..2, true));
        let (p, r, t) = preview_of("abcdefghij", 8..10, 2, 6);
        assert_eq!((p.as_str(), r, t), ("efghij", 4..6, true));
        let (p, r, t) = preview_of("abc", 1..2, 1, 6);
        assert_eq!((p.as_str(), r, t), ("abc", 1..2, false));
        let (p, r, t) = preview_of("abcdefghij", 2..9, 7, 4);
        assert_eq!((p.as_str(), r, t), ("cdef", 0..4, true));
        let (p, r, _) = preview_of("ééééxéééé", 8..9, 1, 3);
        assert_eq!((p.as_str(), r), ("éxé", 1..2));
    }
}

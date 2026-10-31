//! Shared helpers of the hikasa-report integration tests (§6.1): the golden-file helper, the proptest
//! configuration, text inputs loaded through `MemoryVfs` and the real text engine, the folder fixtures of
//! hikasa-folder (`tree.toml`, read by a small reader of the TOML subset they use, since this crate has no `toml`
//! dependency) materialised into `MemoryVfs` and scanned by the real folder engine with a scripted rules
//! comparator, the HTML checker of P-RPT-3, and position-exact patch appliers ([`apply`]).
#![allow(dead_code)]

pub mod apply;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hikasa_core::{
    CancelToken, CaseSensitivity, ContentComparator, ContentMethod, ContentOutcome, ContentResult,
    ContentSource, Ctl, ErrorKind, ErrorReport, FileTime, LocalPath, NullProgress, Pair, Spec,
    Warning, WarningCode,
};
use hikasa_folder::{
    ContentEngines, FolderComparison, FolderEnv, FolderError, FolderModel, FolderRules,
    NullObserver, ScanEvent, ScanObserver,
};
use hikasa_text::{
    CompareInput, CompiledFormat, LoadOptions, RegexLimits, TextComparison, TextRules, compare,
    load_document,
};
use hikasa_vfs::{
    Attrs, Capabilities, DosAttrs, Fault, MemMeta, MemoryVfs, StreamOptions, UnixMeta, VfsOp,
};
use proptest::test_runner::{Config, FileFailurePersistence};
use serde::Deserialize;
use serde_json::{Map, Value};

// ----------------------------------------------------------------------------------------------------------
// Golden files, fixtures, proptest
// ----------------------------------------------------------------------------------------------------------

/// Compares `actual` with the golden file `expected`; with `HIKASA_BLESS=1` the golden file is (re)written
/// instead. Text files are shown as a `pretty_assertions` diff.
pub fn assert_golden(expected: &Path, actual: &[u8]) {
    let bless = std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1");
    if bless {
        if let Some(dir) = expected.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let golden = std::fs::read(expected).unwrap_or_else(|e| {
        panic!(
            "missing golden file {} ({e}); run with HIKASA_BLESS=1",
            expected.display()
        )
    });
    if golden != actual {
        match (std::str::from_utf8(&golden), std::str::from_utf8(actual)) {
            (Ok(want), Ok(got)) => {
                pretty_assertions::assert_eq!(want, got, "golden {}", expected.display())
            }
            _ => panic!("golden {} differs (binary)", expected.display()),
        }
    }
}

/// `tests/fixtures/<suite>` of this crate.
pub fn fixtures(suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
}

/// The fixture folder of a sibling crate: `core/<krate>/tests/fixtures/<suite>` (the report goldens reuse the
/// text and folder cases, §6.2.7).
pub fn sibling_fixtures(krate: &str, suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(krate)
        .join("tests")
        .join("fixtures")
        .join(suite)
}

/// The case folders below `dir`, sorted.
pub fn cases(dir: &Path) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    cases
}

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are persisted
/// in `regressions` (relative to the package) and committed.
pub fn config(regressions: &'static str) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// A host-style absolute path for `MemoryVfs` from a `/`-separated one: `/a/b` on Unix, `C:\a\b` on Windows.
pub fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

/// `Spec::Local` of [`host`]`(path)`.
pub fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(host(path))).unwrap())
}

// ----------------------------------------------------------------------------------------------------------
// Text inputs
// ----------------------------------------------------------------------------------------------------------

/// The modification times of the two text inputs.
pub const LEFT_MTIME: &str = "2026-10-19T13:12:00Z";
/// See [`LEFT_MTIME`].
pub const RIGHT_MTIME: &str = "2026-10-19T14:00:00Z";

/// Compares two byte strings stored in a `MemoryVfs` as `/l/<left_name>` and `/r/<right_name>`, loaded with
/// `load_document` and compared with the Everything Else format (the report crate cannot compile grammars: it has
/// no dependency on hikasa-formats) under `rules`.
pub fn compare_bytes(
    left: (&str, &[u8]),
    right: (&str, &[u8]),
    rules: &TextRules,
) -> TextComparison {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let format = Arc::new(CompiledFormat::everything_else());
    let mut loaded = Vec::new();
    for (base, (name, bytes), mtime) in [("/l", left, LEFT_MTIME), ("/r", right, RIGHT_MTIME)] {
        let path = format!("{base}/{name}");
        vfs.insert_file(
            &host(&path),
            bytes.to_vec(),
            MemMeta {
                mtime: Some(FileTime::parse(mtime).unwrap()),
                ..MemMeta::default()
            },
        )
        .unwrap();
        let (doc, raw) = load_document(
            &vfs,
            &mspec(&path),
            &format,
            &LoadOptions::default(),
            None,
            &Ctl::detached(),
        )
        .unwrap();
        loaded.push((Arc::new(doc), raw));
    }
    rules.validate(&RegexLimits::default()).unwrap();
    let (right_doc, right_raw) = loaded.pop().unwrap();
    let (left_doc, left_raw) = loaded.pop().unwrap();
    let cmp = compare(
        CompareInput {
            left: left_doc,
            right: right_doc,
            format,
            rules: rules.clone(),
            bytes_identical: left_raw == right_raw,
            force_tokenize: false,
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    cmp.check_invariants().unwrap();
    cmp
}

/// Compares two texts with the default rules.
pub fn compare_texts(left: &str, right: &str) -> TextComparison {
    compare_bytes(
        ("left.txt", left.as_bytes()),
        ("right.txt", right.as_bytes()),
        &TextRules::default(),
    )
}

/// The file of `dir` whose name starts with `stem.`.
pub fn side_file(dir: &Path, stem: &str) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{stem}.")))
        })
        .unwrap_or_else(|| panic!("{}: no {stem}.* file", dir.display()))
}

/// The rules of a case folder (`rules.toml`, or the defaults).
pub fn text_rules(dir: &Path) -> TextRules {
    match std::fs::read_to_string(dir.join("rules.toml")) {
        Ok(text) => {
            let (rules, warnings) = TextRules::from_toml(&text).unwrap();
            assert!(warnings.is_empty(), "{}: {warnings:?}", dir.display());
            rules
        }
        Err(_) => TextRules::default(),
    }
}

/// Compares the `left.*` and `right.*` files of a case folder under its rules.
pub fn compare_case(dir: &Path) -> TextComparison {
    let (l, r) = (side_file(dir, "left"), side_file(dir, "right"));
    let name = |p: &Path| p.file_name().unwrap().to_str().unwrap().to_owned();
    compare_bytes(
        (&name(&l), &std::fs::read(&l).unwrap()),
        (&name(&r), &std::fs::read(&r).unwrap()),
        &text_rules(dir),
    )
}

// ----------------------------------------------------------------------------------------------------------
// A reader of the TOML subset of the fixtures
// ----------------------------------------------------------------------------------------------------------

/// Reads the TOML subset the `tree.toml` fixtures use into JSON: `key = value` lines (basic strings with
/// escapes, integers, booleans and one-line arrays of them), `[table]` and `[[array of tables]]` headers (one
/// level), `#` comments and blank lines.
pub fn toml_subset(text: &str) -> Value {
    let mut root = Map::new();
    // None = root; Some((name, true)) = the last element of array `name`; Some((name, false)) = table `name`.
    let mut target: Option<(String, bool)> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim().to_owned();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("[[").and_then(|l| l.strip_suffix("]]")) {
            let name = name.trim().to_owned();
            let entry = root
                .entry(name.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            entry
                .as_array_mut()
                .unwrap_or_else(|| panic!("line {}: {name} is not an array", n + 1))
                .push(Value::Object(Map::new()));
            target = Some((name, true));
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_owned();
            root.entry(name.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            target = Some((name, false));
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("line {}: not `key = value`: {raw}", n + 1));
        let mut chars = value.trim().chars().peekable();
        let value = parse_value(&mut chars);
        let table = match &target {
            None => &mut root,
            Some((name, true)) => root
                .get_mut(name)
                .and_then(Value::as_array_mut)
                .and_then(|a| a.last_mut())
                .and_then(Value::as_object_mut)
                .unwrap(),
            Some((name, false)) => root.get_mut(name).and_then(Value::as_object_mut).unwrap(),
        };
        table.insert(key.trim().to_owned(), value);
    }
    Value::Object(root)
}

/// `line` without a `#` comment (a `#` inside a string is kept).
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            '\\' if in_string && !escaped => {
                escaped = true;
                continue;
            }
            '"' if !escaped => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
        escaped = false;
    }
    line
}

/// One value: a basic string, an integer, a boolean or an array.
fn parse_value(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Value {
    while chars.peek().is_some_and(|c| c.is_whitespace()) {
        chars.next();
    }
    match chars.peek().copied() {
        Some('"') => {
            chars.next();
            let mut s = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return Value::String(s),
                    '\\' => match chars.next() {
                        Some('n') => s.push('\n'),
                        Some('r') => s.push('\r'),
                        Some('t') => s.push('\t'),
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some('u') => {
                            let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                            let code = u32::from_str_radix(&hex, 16).unwrap();
                            s.push(char::from_u32(code).unwrap());
                        }
                        other => panic!("unsupported escape {other:?}"),
                    },
                    c => s.push(c),
                }
            }
            panic!("unterminated string");
        }
        Some('[') => {
            chars.next();
            let mut items = Vec::new();
            loop {
                while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
                    chars.next();
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    return Value::Array(items);
                }
                items.push(parse_value(chars));
            }
        }
        _ => {
            let word: String = std::iter::from_fn(|| {
                chars.next_if(|c| !c.is_whitespace() && *c != ',' && *c != ']')
            })
            .collect();
            match word.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                w => Value::from(
                    w.replace('_', "")
                        .parse::<i64>()
                        .unwrap_or_else(|e| panic!("value {w:?}: {e}")),
                ),
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------------------
// Folder fixtures (the `tree.toml` format of hikasa-folder)
// ----------------------------------------------------------------------------------------------------------

/// The default modification time of fixture entries.
pub const DEFAULT_MTIME: &str = "2026-01-01T00:00:00Z";

/// A `tree.toml` fixture: both sides of a comparison.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeFile {
    /// What the case shows.
    pub description: String,
    /// Options.
    pub options: TreeOptions,
    /// Entries below the left base (`/l`).
    pub left: Vec<EntrySpec>,
    /// Entries below the right base (`/r`).
    pub right: Vec<EntrySpec>,
    /// Injected faults.
    pub faults: Vec<FaultSpec>,
}

/// Options of a fixture.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeOptions {
    /// `MemoryVfs` name case: `sensitive` (default) or `insensitive`.
    pub fs_case: String,
    /// Replaces the back-end capabilities (snake_case names).
    pub capabilities: Option<Vec<String>>,
    /// Cancel after this many `FolderListed` events (the scan then runs on the current thread).
    pub cancel_after_listed: Option<usize>,
    /// "Now" (RFC 3339) for date filters.
    pub now: Option<String>,
    /// Size limit of the fake rules comparator (bytes).
    pub rules_limit: u64,
    /// Content budget in bytes.
    pub content_budget: u64,
}

impl Default for TreeOptions {
    fn default() -> TreeOptions {
        TreeOptions {
            fs_case: "sensitive".into(),
            capabilities: None,
            cancel_after_listed: None,
            now: None,
            rules_limit: 64,
            content_budget: 1 << 30,
        }
    }
}

/// One entry of a fixture side.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EntrySpec {
    /// Path relative to the base (`/`-separated).
    pub path: String,
    /// `file` (default), `dir` or `symlink`.
    pub kind: Option<String>,
    /// Inline content.
    pub content: Option<String>,
    /// Generated content of this length (`'x'` repeated) when `content` is absent.
    pub size: Option<u64>,
    /// Modification time (RFC 3339); default [`DEFAULT_MTIME`]; `"none"` for no time.
    pub mtime: Option<String>,
    /// DOS attribute letters.
    pub dos: Option<String>,
    /// Unix mode (sets Unix attributes; uid/gid default 1000).
    pub mode: Option<u32>,
    /// Unix uid.
    pub uid: Option<u32>,
    /// Unix gid.
    pub gid: Option<u32>,
    /// Symlink target (verbatim; an absolute `/…` target is made host-style).
    pub target: Option<String>,
    /// Back-end CRC.
    pub crc32: Option<u32>,
    /// The directory record states no kind (`MemoryVfs::set_kind_unknown`).
    pub kind_unknown: bool,
}

/// An injected fault.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultSpec {
    /// `left` or `right`.
    pub side: String,
    /// Path relative to that side's base.
    pub path: String,
    /// `stat`, `list`, `open`, `read`.
    pub op: VfsOp,
    /// Error kind.
    pub kind: ErrorKind,
}

fn base(side: &str) -> &'static str {
    match side {
        "left" => "/l",
        "right" => "/r",
        other => panic!("unknown side {other}"),
    }
}

fn join(base: &str, path: &str) -> String {
    if path.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{path}")
    }
}

/// Reads a case folder of hikasa-folder: `tree.toml` and `rules.toml` (or the default rules).
pub fn load_tree(case: &Path) -> (TreeFile, FolderRules) {
    let text = std::fs::read_to_string(case.join("tree.toml")).unwrap();
    let tree: TreeFile = serde_json::from_value(toml_subset(&text))
        .unwrap_or_else(|e| panic!("{}: {e}", case.display()));
    let rules = match std::fs::read_to_string(case.join("rules.toml")) {
        Ok(text) => {
            let (rules, warnings) = FolderRules::from_toml(&text).unwrap();
            assert!(warnings.is_empty(), "{}: {warnings:?}", case.display());
            rules
        }
        Err(_) => FolderRules::default(),
    };
    (tree, rules)
}

/// Builds the `MemoryVfs` of a fixture (bases `/l` and `/r`).
pub fn materialize(tree: &TreeFile) -> MemoryVfs {
    let case = match tree.options.fs_case.as_str() {
        "insensitive" => CaseSensitivity::Insensitive,
        _ => CaseSensitivity::Sensitive,
    };
    let vfs = MemoryVfs::new(case);
    let meta = MemMeta {
        mtime: Some(FileTime::parse(DEFAULT_MTIME).unwrap()),
        ..MemMeta::default()
    };
    vfs.insert_dir(&host("/l"), meta.clone()).unwrap();
    vfs.insert_dir(&host("/r"), meta).unwrap();
    for (side, entries) in [("left", &tree.left), ("right", &tree.right)] {
        for e in entries {
            insert_entry(&vfs, base(side), e);
        }
    }
    for f in &tree.faults {
        vfs.inject(Fault {
            path: host(&join(base(&f.side), &f.path)),
            op: f.op,
            kind: f.kind,
        });
    }
    if let Some(names) = &tree.options.capabilities {
        let caps = names.iter().fold(Capabilities::empty(), |acc, n| {
            acc | Capabilities::parse_name(n).unwrap_or_else(|| panic!("capability {n}"))
        });
        vfs.set_capabilities(caps);
    }
    vfs
}

fn insert_entry(vfs: &MemoryVfs, base: &str, e: &EntrySpec) {
    let path = host(&join(base, &e.path));
    let mtime = match e.mtime.as_deref() {
        Some("none") => None,
        Some(t) => Some(FileTime::parse(t).unwrap()),
        None => Some(FileTime::parse(DEFAULT_MTIME).unwrap()),
    };
    let attrs = if let Some(letters) = &e.dos {
        Some(Attrs::Dos {
            attrs: DosAttrs::parse_letters(letters).unwrap(),
        })
    } else {
        e.mode.map(|mode| Attrs::Unix {
            meta: UnixMeta {
                mode,
                uid: e.uid.unwrap_or(1000),
                gid: e.gid.unwrap_or(1000),
            },
        })
    };
    let meta = MemMeta {
        mtime,
        ctime: None,
        attrs,
        crc32: e.crc32,
    };
    match e.kind.as_deref().unwrap_or("file") {
        "file" => {
            let content: Vec<u8> = match (&e.content, e.size) {
                (Some(c), _) => c.as_bytes().to_vec(),
                (None, Some(n)) => vec![b'x'; n as usize],
                (None, None) => Vec::new(),
            };
            vfs.insert_file(&path, content, meta).unwrap();
        }
        "dir" => vfs.insert_dir(&path, meta).unwrap(),
        "symlink" => {
            let target = e.target.clone().unwrap_or_default();
            let target = if target.starts_with('/') {
                host(&target)
            } else if cfg!(windows) {
                target.replace('/', "\\")
            } else {
                target
            };
            vfs.insert_symlink(&path, &target, meta).unwrap();
        }
        other => panic!("unknown kind {other}"),
    }
    if e.kind_unknown {
        vfs.set_kind_unknown(&path).unwrap();
    }
}

/// A scripted rules comparator (as in hikasa-folder's tests; the real one needs a format registry, which this
/// crate cannot build): content starting with `!error` fails; content above `limit` is compared by bytes with
/// warning `rules_size_limit`; equal bytes → `BinarySame`; equal after CRLF → LF → `RulesSame`; equal after
/// removing ASCII whitespace → `UnimportantOnly`; else `ImportantDifferences`.
#[derive(Debug, Clone, Copy)]
pub struct FakeRules {
    /// Largest file loaded completely.
    pub limit: u64,
}

impl ContentComparator for FakeRules {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn size_limit(&self) -> Option<u64> {
        Some(self.limit)
    }

    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, hikasa_core::Cancelled> {
        ctl.check()?;
        let read = |s: &dyn ContentSource| s.read_all(u64::MAX, ctl.cancel);
        let (l, r) = match (read(left), read(right)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(hikasa_core::ReadError::Cancelled), _)
            | (_, Err(hikasa_core::ReadError::Cancelled)) => {
                return Err(hikasa_core::Cancelled);
            }
            (Err(e), _) | (_, Err(e)) => {
                return Ok(ContentOutcome {
                    method: ContentMethod::Rules,
                    result: ContentResult::Error {
                        error: ErrorReport::capture(&e, None),
                    },
                    warnings: Vec::new(),
                });
            }
        };
        let mut warnings = Vec::new();
        let result = if l.starts_with(b"!error") || r.starts_with(b"!error") {
            ContentResult::Error {
                error: ErrorReport {
                    kind: ErrorKind::InvalidData,
                    message: "scripted rules failure".into(),
                    spec: None,
                    causes: Vec::new(),
                },
            }
        } else if l.len() as u64 > self.limit || r.len() as u64 > self.limit {
            warnings.push(Warning {
                code: WarningCode::RulesSizeLimit,
                message: "compared by bytes".into(),
                spec: None,
            });
            if l == r {
                ContentResult::BinarySame
            } else {
                ContentResult::BinaryDifferent
            }
        } else if l == r {
            ContentResult::BinarySame
        } else {
            let crlf = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
            let squeeze = |b: &[u8]| {
                b.iter()
                    .copied()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect::<Vec<u8>>()
            };
            if crlf(&l) == crlf(&r) {
                ContentResult::RulesSame
            } else if squeeze(&l) == squeeze(&r) {
                ContentResult::UnimportantOnly
            } else {
                ContentResult::ImportantDifferences
            }
        };
        Ok(ContentOutcome {
            method: ContentMethod::Rules,
            result,
            warnings,
        })
    }
}

/// Cancels a token after `after` `FolderListed` events.
pub struct CancelAfter<'a> {
    pub after: usize,
    pub seen: AtomicUsize,
    pub cancel: &'a CancelToken,
}

impl ScanObserver for CancelAfter<'_> {
    fn on_event(&self, event: ScanEvent) {
        if let ScanEvent::FolderListed { .. } = event
            && self.seen.fetch_add(1, Ordering::SeqCst) + 1 >= self.after
        {
            self.cancel.cancel();
        }
    }
}

/// The default "now" of the tests: 2026-10-20T12:00:00Z in UTC.
pub fn default_now() -> jiff::Zoned {
    "2026-10-20T12:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC)
}

/// Scans a fixture with the real folder engine on the current thread (`/l` against `/r`).
pub fn run_tree(
    tree: &TreeFile,
    vfs: &MemoryVfs,
    rules: &FolderRules,
) -> Result<FolderComparison, FolderError> {
    let engines = ContentEngines::with_stream_options(
        StreamOptions::default(),
        Some(Arc::new(FakeRules {
            limit: tree.options.rules_limit,
        })),
    );
    let now = match &tree.options.now {
        Some(t) => t
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC),
        None => default_now(),
    };
    let cancel = CancelToken::new();
    let progress = NullProgress;
    let observer = CancelAfter {
        after: tree.options.cancel_after_listed.unwrap_or(usize::MAX),
        seen: AtomicUsize::new(0),
        cancel: &cancel,
    };
    let observer: &dyn ScanObserver = if tree.options.cancel_after_listed.is_some() {
        &observer
    } else {
        &NullObserver
    };
    let env = FolderEnv {
        vfs,
        content: &engines,
        pool: None,
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer,
        now: &now,
        content_budget: tree.options.content_budget,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone())?;
    hikasa_folder::compare_into(&model, &env)
}

/// Loads and scans a case folder of hikasa-folder.
pub fn folder_case(case: &Path) -> (TreeFile, FolderComparison) {
    let (tree, rules) = load_tree(case);
    let vfs = materialize(&tree);
    let cmp = run_tree(&tree, &vfs, &rules).unwrap_or_else(|e| panic!("{}: {e}", case.display()));
    cmp.check_invariants()
        .unwrap_or_else(|e| panic!("{}: {e}", case.display()));
    (tree, cmp)
}

/// A pair of specs for tests that need one.
pub fn spec_pair() -> Pair<Spec> {
    Pair::new(mspec("/l"), mspec("/r"))
}

// ----------------------------------------------------------------------------------------------------------
// P-RPT-3: a small HTML checker
// ----------------------------------------------------------------------------------------------------------

/// Elements without an end tag.
const VOID: [&str; 6] = ["meta", "col", "br", "hr", "img", "link"];

/// Checks that `html` is well-formed: a doctype, then tags that balance (void elements excepted), lower-case
/// names, attributes `name="value"` with escaped values; text and attribute values contain no raw `<`, `>` or `"`
/// and no `&` that does not start an entity (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;`, `&#…;`); no control
/// characters except LF and TAB; the content of `<style>` cannot close the element; no `<script>`.
pub fn check_html(html: &str) -> Result<(), String> {
    let rest = html
        .strip_prefix("<!DOCTYPE html>\n")
        .ok_or("missing doctype")?;
    if let Some(c) = html
        .chars()
        .find(|c| c.is_control() && *c != '\n' && *c != '\t')
    {
        return Err(format!("control character {c:?}"));
    }
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0usize;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = rest[i..]
                .find('>')
                .map(|e| i + e)
                .ok_or_else(|| format!("unterminated tag at {i}"))?;
            let tag = &rest[i + 1..end];
            if let Some(name) = tag.strip_prefix('/') {
                let open = stack.pop().ok_or_else(|| format!("stray </{name}>"))?;
                if open != name {
                    return Err(format!("</{name}> closes <{open}>"));
                }
            } else {
                let (name, attrs) = tag.split_once(' ').unwrap_or((tag, ""));
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                {
                    return Err(format!("bad tag name {name:?}"));
                }
                if name == "script" {
                    return Err("script element".into());
                }
                check_attrs(attrs)?;
                if !VOID.contains(&name) {
                    stack.push(name.to_owned());
                }
                if name == "style" {
                    let close = rest[end..].find("</style>").ok_or("unterminated style")?;
                    let css = &rest[end + 1..end + close];
                    if css.contains("</") {
                        return Err("style content closes an element".into());
                    }
                    i = end + close;
                    continue;
                }
            }
            i = end + 1;
        } else {
            let next = rest[i..].find('<').map_or(rest.len(), |e| i + e);
            check_text(&rest[i..next])?;
            i = next;
        }
    }
    if !stack.is_empty() {
        return Err(format!("unclosed elements {stack:?}"));
    }
    Ok(())
}

fn check_attrs(attrs: &str) -> Result<(), String> {
    let mut rest = attrs.trim();
    while !rest.is_empty() {
        let (name, after) = rest
            .split_once("=\"")
            .ok_or_else(|| format!("attribute without quoted value: {rest:?}"))?;
        if !name.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            return Err(format!("bad attribute name {name:?}"));
        }
        let (value, after) = after
            .split_once('"')
            .ok_or_else(|| format!("unterminated attribute value: {after:?}"))?;
        check_text(value)?;
        rest = after.trim_start();
    }
    Ok(())
}

fn check_text(text: &str) -> Result<(), String> {
    if text.contains(['<', '>', '"']) {
        return Err(format!("unescaped character in {text:?}"));
    }
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        let tail = &rest[at..];
        let ok = ["&amp;", "&lt;", "&gt;", "&quot;", "&#39;"]
            .iter()
            .any(|e| tail.starts_with(e))
            || (tail.starts_with("&#")
                && tail[2..].find(';').is_some_and(|e| {
                    e > 0
                        && tail[2..2 + e]
                            .chars()
                            .all(|c| c.is_ascii_hexdigit() || c == 'x')
                }));
        if !ok {
            return Err(format!("raw & in {text:?}"));
        }
        rest = &tail[1..];
    }
    Ok(())
}

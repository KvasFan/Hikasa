//! A10.23 (D-284), the Service's share, and A10.24's Service rows (D-285): `Service::text_find` on disk trees (masks
//! with excludes, recursion on and off, hidden items, binary, too large and unreadable files, UTF-16 and windows-1252
//! files, a zip searched as a file and opened, an External format with and without `convert`), the stream identical
//! with 1, 2 and 8 threads, the limits, a cancel inside a 256 MiB file (≤ 100 ms, G4), the heap with 100,000 hits (a
//! counting allocator: it does not grow with the hits), the request's refusals, and the replace in files: one save plan,
//! the operation question, unchanged lines byte-exact, a file changed between the plan and the write failing its item.
//! A10.32: a file whose name is not UTF-8 (Linux). The engine's share is `hikasa-text`'s `find_files.rs`; the CLI's is
//! `cli/hikasa-cli/tests/cli_text_find.rs`.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); as in the other memory
// tests of the workspace, this test binary opts out explicitly and every occurrence carries a SAFETY note.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use common::{facts, fixed_clock, job, os, service, write};
use hikasa_core::confirm::{Ask, Reply};
use hikasa_core::{CancelToken, ExitCode, JobKind, Spec, WarningCode};
use hikasa_session::api::{
    NoEvents, SaveAnswers, TextFindOutcome, TextFindReplace, TextFindRequest,
};
use hikasa_session::{ApiError, Service, ServiceInit};
use hikasa_text::find::files::{FilesEvent, FilesSink, FindArchives};
use hikasa_vfs::ops::{ItemStatus, SkipReason};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Entry, FileName, FollowLinks, ListOptions, Listing, MemMeta,
    MemoryVfs, ReadSeek, Vfs, VfsError, WriteOptions,
};

// ---- a counting allocator (the memory row) ------------------------------------------------------------------------

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

/// The counters see the whole process, and `cargo test` runs the tests of a binary on threads whose frees land in the
/// middle of a measure (a cancelled search drops its 256 MiB later): a test that reads them runs again alone, in a child
/// process of this binary (nextest gives each test a process anyway). True in the parent, whose child did the test.
fn in_a_process_of_its_own(name: &str) -> bool {
    if std::env::var_os("HIKASA_ALONE_CHILD").is_some() {
        return false;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--test-threads", "1", "--nocapture"])
        .env("HIKASA_ALONE_CHILD", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("1 passed"),
        "{name}: {text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; the counters are static atomics,
// so counting never allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now =
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed) + layout.size() as isize;
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }
    // SAFETY: `ptr` and `layout` come from `alloc`/`realloc` above, as `System::dealloc` requires.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }
    // SAFETY: the caller's guarantees for `ptr`, `layout` and `new_size` are passed on unchanged.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let delta = new_size as isize - layout.size() as isize;
        let now = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// ---- helpers ------------------------------------------------------------------------------------------------------

/// Every event as one line, the tree's folder replaced by `<T>`.
struct Record {
    root: String,
    lines: Vec<String>,
}

impl Record {
    /// The events of a search under `root`, named as the Service names `root` (macOS `/private/var`, Windows
    /// separators and drive letters: whatever the platform's canonical spelling).
    fn new(service: &Service, root: &Path) -> Record {
        Record {
            root: service.parse_spec(root.as_os_str()).unwrap().canonical(),
            lines: Vec::new(),
        }
    }

    fn rel(&self, spec: &Spec) -> String {
        spec.canonical()
            .replace(&self.root, "<T>")
            .replace('\\', "/")
    }
}

impl FilesSink for Record {
    fn event(&mut self, e: FilesEvent<'_>) {
        let line = match e {
            FilesEvent::File {
                index,
                spec,
                encoding,
            } => format!("file {index} {} {}", self.rel(spec), encoding.name()),
            FilesEvent::Hit {
                line,
                column,
                len_chars,
                preview,
                ..
            } => format!("hit {line}:{column}+{len_chars} {preview}"),
            FilesEvent::Skipped { spec, reason } => {
                format!("skip {} {}", self.rel(spec), reason.as_str())
            }
            FilesEvent::Done { hits, stopped, .. } => format!("done {hits} {stopped}"),
        };
        self.lines.push(line);
    }
}

fn request(pattern: &str, roots: &[&Path]) -> TextFindRequest {
    TextFindRequest {
        pattern: pattern.into(),
        roots: roots.iter().map(|p| os(p)).collect(),
        recursive: true,
        ..TextFindRequest::default()
    }
}

fn find(service: &Service, req: &TextFindRequest, root: &Path) -> (TextFindOutcome, Vec<String>) {
    let mut record = Record::new(service, root);
    let outcome = service
        .text_find(req, &mut record, &job(service, JobKind::TextFind))
        .unwrap();
    (outcome, record.lines)
}

/// The files of a stream (`file …` lines without the index and the encoding).
fn files(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("file "))
        .map(|l| l.split(' ').nth(1).unwrap().to_owned())
        .collect()
}

fn skips(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("skip "))
        .map(str::to_owned)
        .collect()
}

/// A service with `threads` workers.
fn service_threads(root: &Path, threads: usize) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(threads),
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The tree of the disk rows.
fn tree(t: &Path) {
    write(t, "a.txt", b"foo\nbar foo\n");
    write(t, "b.c", b"int foo;\r\n");
    write(t, "bin.dat", b"foo\0\0\x01\x02binary");
    write(t, "big.txt", "foo\n".repeat(600).as_bytes());
    write(t, ".hidden.txt", b"foo hidden file\n");
    write(t, ".hid/x.txt", b"foo in a hidden folder\n");
    write(t, "sub/c.txt", b"Foo\n");
    write(t, "sub/deep/d.txt", b"a foo deep\n");
    write(t, "sub/e.log", b"foo log\n");
}

// ---- A10.23 on disk -----------------------------------------------------------------------------------------------

#[test]
fn a10_23_a_disk_tree_in_pre_order_with_masks_recursion_and_hidden_items() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    tree(&t);
    let mut req = request("foo", &[&t]);
    req.max_file_size = Some(1024);
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(outcome.exit_code, ExitCode::Success);
    // Sorted by name, files before their folder's subfolders as listed, hidden items out, case ignored.
    assert_eq!(
        files(&lines),
        [
            "<T>/a.txt",
            "<T>/b.c",
            "<T>/sub/c.txt",
            "<T>/sub/deep/d.txt",
            "<T>/sub/e.log"
        ]
    );
    assert_eq!(
        skips(&lines),
        ["<T>/big.txt too_large", "<T>/bin.dat binary"]
    );
    let dto = &outcome.dto;
    assert_eq!(dto.counts.files_searched, 5);
    assert_eq!(dto.counts.files_matched, 5);
    assert_eq!(dto.counts.hits, 6);
    assert_eq!(dto.counts.skipped, 2);
    assert!(!dto.limit_reached);
    assert_eq!(dto.files.len(), 5);
    // Without keep_hits the document has the files and their counts, no hits.
    assert!(dto.files.iter().all(|f| f.hits.is_empty()));
    assert_eq!(dto.files[0].hit_count, 2);
    assert_eq!(dto.skipped[0].reason, "too_large");
    assert_eq!(dto.archives, "files");
    // Not recursive: the root's files only.
    req.recursive = false;
    let (_, lines) = find(&service, &req, &t);
    assert_eq!(files(&lines), ["<T>/a.txt", "<T>/b.c"]);
    // Hidden items, case, and masks with a folder excluded.
    let mut req = request("foo", &[&t]);
    req.include_hidden = true;
    req.match_case = true;
    req.mask = Some("*.txt;-deep/".into());
    req.max_file_size = Some(1024);
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(
        files(&lines),
        [
            "<T>/.hid/x.txt",
            "<T>/.hidden.txt",
            "<T>/a.txt",
            "<T>/sub/c.txt"
        ]
    );
    // `Foo` is not `foo` with case: searched, no hit, not in the document.
    assert_eq!(outcome.dto.counts.files_matched, 3);
    assert_eq!(outcome.dto.mask.as_deref(), Some("*.txt;-deep/"));
    // A file root is searched whatever the masks; the hits are kept with keep_hits.
    let mut req = request("foo", &[&t.join("b.c")]);
    req.mask = Some("*.txt".into());
    req.keep_hits = true;
    let (outcome, _) = find(&service, &req, &t);
    let hit = &outcome.dto.files[0].hits[0];
    assert_eq!((hit.line, hit.column, hit.len), (1, 5, 3));
    assert_eq!(hit.preview, "int foo;");
    assert_eq!(hit.preview_range, [4, 7]);
    assert!(!hit.truncated);
}

#[cfg(unix)]
#[test]
fn a10_23_unreadable_files_and_folders_are_skipped_in_their_place() {
    use std::os::unix::fs::PermissionsExt;
    if common::my_uid() == Some(0) {
        return; // root reads everything
    }
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.txt", b"foo\n");
    let locked = write(&t, "b.txt", b"foo\n");
    write(&t, "c/x.txt", b"foo\n");
    write(&t, "d.txt", b"foo\n");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    std::fs::set_permissions(t.join("c"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let (outcome, lines) = find(&service, &request("foo", &[&t]), &t);
    std::fs::set_permissions(t.join("c"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(files(&lines), ["<T>/a.txt", "<T>/d.txt"]);
    assert_eq!(skips(&lines), ["<T>/b.txt unreadable", "<T>/c unreadable"]);
    let skipped = &outcome.dto.skipped;
    assert!(skipped.iter().all(|s| s.error.is_some()), "{skipped:?}");
}

#[test]
fn a10_23_a_zip_searched_as_a_file_and_opened() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.txt", b"foo\n");
    let zip = write(
        &t,
        "z.zip",
        &zip_of(&[("m.txt", b"foo member\n"), ("in/n.txt", b"nested foo\n")]),
    );
    let (_, lines) = find(&service, &request("foo", &[&t]), &t);
    assert_eq!(files(&lines), ["<T>/a.txt"]);
    assert_eq!(skips(&lines), ["<T>/z.zip binary"]);
    let mut req = request("foo", &[&t]);
    req.archives = FindArchives::Opened;
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(
        files(&lines),
        ["<T>/a.txt", "<T>/z.zip/in/n.txt", "<T>/z.zip/m.txt"]
    );
    assert_eq!(outcome.dto.archives, "opened");
    // Not recursive: an archive in a folder is a folder, so it is not entered; as a root it is walked.
    req.recursive = false;
    let (_, lines) = find(&service, &req, &t);
    assert_eq!(files(&lines), ["<T>/a.txt"]);
    let mut req = request("foo", &[&zip]);
    req.archives = FindArchives::Opened;
    req.recursive = false;
    let (_, lines) = find(&service, &req, &t);
    assert_eq!(files(&lines), ["<T>/z.zip/m.txt"]);
}

#[test]
fn a10_23_utf16_and_windows_1252_files() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(
            "x café y\r\nfoo\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    write(&t, "u16.txt", &utf16);
    write(&t, "w1252.txt", b"un caf\xe9 noir\n");
    let mut req = request("café", &[&t]);
    req.keep_hits = true;
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(files(&lines), ["<T>/u16.txt", "<T>/w1252.txt"], "{lines:?}");
    let encodings: Vec<&str> = outcome.dto.files.iter().map(|f| f.encoding).collect();
    assert_eq!(encodings, ["UTF-16LE", "windows-1252"]);
    let hits: Vec<(u32, u32)> = outcome
        .dto
        .files
        .iter()
        .map(|f| (f.hits[0].line, f.hits[0].column))
        .collect();
    assert_eq!(hits, [(1, 3), (1, 4)]);
    // A forced encoding reads every file in it: the UTF-16 file then has no `café`.
    req.encoding = Some("windows-1252".into());
    let (outcome, _) = find(&service, &req, &t);
    let matched: Vec<&str> = outcome.dto.files.iter().map(|f| f.spec.as_str()).collect();
    assert_eq!(matched.len(), 1);
    assert!(matched[0].ends_with("w1252.txt"));
    // An unknown label refuses the request.
    req.encoding = Some("no-such-encoding".into());
    let err = service
        .text_find(&req, &mut NoEvents, &job(&service, JobKind::TextFind))
        .unwrap_err();
    assert!(matches!(err, ApiError::Format(_)), "{err:?}");
}

#[test]
fn a10_23_an_external_format_with_and_without_convert() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    let service = service(tmp.path());
    assert!(service.startup_warnings().is_empty());
    let t = tmp.path().join("work/t");
    write(&t, "a.up", b"some foo\n");
    write(&t, "b.txt", b"FOO\n");
    let mut req = request("FOO", &[&t]);
    req.match_case = true;
    let (_, lines) = find(&service, &req, &t);
    assert_eq!(files(&lines), ["<T>/b.txt"]);
    assert_eq!(skips(&lines), ["<T>/a.up converted"]);
    // With convert: the converted text (upper case) is searched.
    req.convert = true;
    req.keep_hits = true;
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(files(&lines), ["<T>/a.up", "<T>/b.txt"]);
    assert_eq!(outcome.dto.files[0].hits[0].preview, "SOME FOO");
}

#[test]
fn a10_23_the_stream_is_the_same_with_1_2_and_8_threads() {
    let tmp = tempfile::tempdir().unwrap();
    let t = tmp.path().join("work/t");
    let mut seed = 0x5EED_F1D5u64;
    for d in 0..12 {
        for f in 0..20 {
            let mut text = String::new();
            for k in 0..(5 + (d * 7 + f * 3) % 40) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let word = ["foo", "bar", "baz", "Foo", "qux"][(seed % 5) as usize];
                text.push_str(&format!("{k} {word} {}\n", seed % 1000));
            }
            write(&t, &format!("d{d:02}/f{f:02}.txt"), text.as_bytes());
        }
    }
    write(&t, "d03/bin.dat", b"foo\0\0\0");
    let mut req = request("foo", &[&t]);
    req.keep_hits = true;
    let mut streams = Vec::new();
    let mut documents = Vec::new();
    for threads in [1, 2, 8] {
        let service = service_threads(tmp.path(), threads);
        let (outcome, lines) = find(&service, &req, &t);
        assert!(outcome.dto.counts.hits > 500);
        streams.push(lines);
        documents.push(outcome.dto);
    }
    assert_eq!(streams[0], streams[1]);
    assert_eq!(streams[0], streams[2]);
    assert_eq!(documents[0], documents[1]);
    assert_eq!(documents[0], documents[2]);
}

#[test]
fn a10_23_limits_stop_a_file_and_the_search() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.txt", "foo\n".repeat(5).as_bytes());
    write(&t, "b.txt", "foo\n".repeat(5).as_bytes());
    write(&t, "c.txt", b"foo\n");
    let mut req = request("foo", &[&t]);
    req.max_matches_per_file = Some(3);
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(outcome.dto.counts.hits, 7);
    assert!(outcome.dto.files[0].stopped && outcome.dto.files[1].stopped);
    assert!(!outcome.dto.files[2].stopped);
    assert!(lines.contains(&"done 3 true".to_owned()));
    assert!(!outcome.dto.limit_reached);
    req.max_matches = Some(4);
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert!(outcome.dto.limit_reached);
    assert_eq!(outcome.dto.counts.hits, 4);
    assert_eq!(files(&lines), ["<T>/a.txt", "<T>/b.txt"]);
    let warning = outcome
        .warnings
        .iter()
        .find(|w| w.code == WarningCode::FindLimitReached)
        .unwrap();
    assert!(warning.message.contains("4 hits"), "{}", warning.message);
}

#[test]
fn a10_23_requests_that_cannot_run_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.txt", b"foo\n");
    let job = job(&service, JobKind::TextFind);
    let refused = |req: &TextFindRequest| service.text_find(req, &mut NoEvents, &job).unwrap_err();
    let exit = |e: &ApiError| e.exit_code(hikasa_session::api::ApiOp::TextFind);
    // A bad pattern, a line-break pattern, an empty one, an unknown group in the replacement.
    for (pattern, regex) in [("(", true), (r"a|\n", true), ("", false)] {
        let mut req = request(pattern, &[&t]);
        req.regex = regex;
        let e = refused(&req);
        assert_eq!(exit(&e), ExitCode::UnknownError, "{pattern}");
        assert_eq!(
            e.report().kind,
            hikasa_core::ErrorKind::InvalidInput,
            "{pattern}: {e}"
        );
    }
    let mut req = request("f(o)o", &[&t]);
    req.regex = true;
    req.replace = Some(TextFindReplace {
        text: "${2}".into(),
        ..TextFindReplace::default()
    });
    let e = refused(&req);
    assert!(e.to_string().contains("replacement"), "{e}");
    // A missing root (before anything is read), a bad mask, limits of 0, an unknown format.
    let e = refused(&request("foo", &[&t, &t.join("missing")]));
    assert_eq!(e.report().kind, hikasa_core::ErrorKind::NotFound, "{e}");
    let mut req = request("foo", &[&t]);
    req.mask = Some("[".into());
    assert!(matches!(refused(&req), ApiError::InvalidArgument(_)));
    for set in [
        |r: &mut TextFindRequest| r.max_matches = Some(0),
        |r: &mut TextFindRequest| r.max_matches_per_file = Some(0),
        |r: &mut TextFindRequest| r.max_file_size = Some(0),
    ] {
        let mut req = request("foo", &[&t]);
        set(&mut req);
        assert!(refused(&req).to_string().contains("at least 1"));
    }
    let mut req = request("foo", &[&t]);
    req.format = Some("no such format".into());
    assert!(matches!(refused(&req), ApiError::Format(_)));
    assert!(matches!(
        refused(&TextFindRequest::default()),
        ApiError::InvalidArgument(_)
    ));
}

// ---- the cancel row ------------------------------------------------------------------------------------------------

/// A back-end that cancels `token` once `after` bytes of one file were read (the cancel lands inside that file).
struct CancelInside {
    inner: Arc<hikasa_vfs::VfsRouter>,
    name: &'static str,
    after: u64,
    token: CancelToken,
    at: Arc<std::sync::Mutex<Option<Instant>>>,
}

struct CancelReader {
    inner: Box<dyn ReadSeek>,
    read: u64,
    after: u64,
    token: CancelToken,
    at: Arc<std::sync::Mutex<Option<Instant>>>,
}

impl Read for CancelReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if self.read >= self.after && !self.token.is_cancelled() {
            *self.at.lock().unwrap() = Some(Instant::now());
            self.token.cancel();
        }
        Ok(n)
    }
}

impl Seek for CancelReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

impl Vfs for CancelInside {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        let inner = self.inner.open_read(spec)?;
        if spec.file_name().as_deref() != Some(self.name) {
            return Ok(inner);
        }
        Ok(Box::new(CancelReader {
            inner,
            read: 0,
            after: self.after,
            token: self.token.clone(),
            at: Arc::clone(&self.at),
        }))
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.inner.rename(from, to, replace)
    }
    fn set_attrs(&self, spec: &Spec, change: &hikasa_vfs::AttrChange) -> Result<(), VfsError> {
        self.inner.set_attrs(spec, change)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.inner.create_dir(spec, parents)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.inner.remove(spec)
    }
}

/// G4 in A10.23: a cancel that lands inside a 256 MiB file (half of it read) returns within 100 ms, `Cancelled`.
#[test]
fn a10_23_a_cancel_inside_a_256_mib_file_returns_within_100_ms() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.txt", b"foo\n");
    {
        use std::io::Write;
        let line = "0123456789 the quick brown fox jumps over the lazy dog foo bar baz qux quux\n";
        let mut chunk = String::new();
        while chunk.len() < 1 << 20 {
            chunk.push_str(line);
        }
        let mut file = std::io::BufWriter::new(std::fs::File::create(t.join("big.txt")).unwrap());
        let mut written = 0u64;
        while written < 256 << 20 {
            file.write_all(chunk.as_bytes()).unwrap();
            written += chunk.len() as u64;
        }
        file.flush().unwrap();
    }
    let job = job(&service, JobKind::TextFind);
    let at = Arc::new(std::sync::Mutex::new(None));
    let vfs = CancelInside {
        inner: service.vfs(),
        name: "big.txt",
        after: 128 << 20,
        token: job.cancel.clone(),
        at: Arc::clone(&at),
    };
    let mut req = request("foo", &[&t]);
    req.max_file_size = Some(512 << 20);
    let result = service.text_find_on(&vfs, &req, &mut NoEvents, &job);
    let returned = Instant::now();
    assert!(matches!(result, Err(ApiError::Cancelled)), "{result:?}");
    let cancelled = at
        .lock()
        .unwrap()
        .expect("the cancel landed inside the file");
    let latency = returned - cancelled;
    eprintln!("cancel inside a 256 MiB file: returned after {latency:?}");
    assert!(latency <= Duration::from_millis(100), "{latency:?}");
}

// ---- the memory row ------------------------------------------------------------------------------------------------

/// The heap peak of a search of `text` on `MemoryVfs` through the Service, above what was live before it.
fn peak_of(service: &Service, text: &[u8], max: u64) -> (isize, u64) {
    let vfs = MemoryVfs::new(hikasa_core::CaseSensitivity::Sensitive);
    let path = if cfg!(windows) {
        "C:\\m\\a.txt"
    } else {
        "/m/a.txt"
    };
    vfs.insert_file(path, text.to_vec(), MemMeta::default())
        .unwrap();
    let mut req = request("foo", &[Path::new(path)]);
    req.max_matches = Some(max);
    req.max_matches_per_file = Some(u32::MAX);
    let job = job(service, JobKind::TextFind);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let outcome = service
        .text_find_on(&vfs, &req, &mut NoEvents, &job)
        .unwrap();
    (PEAK.load(Ordering::Relaxed) - base, outcome.dto.counts.hits)
}

/// A10.23's memory row: with a sink that drops the events (and no hits kept), the heap of a search with 100,000 hits
/// peaks no higher than one of the same file size with 1,000 (the workers buffer at most 1,024 hits of a file).
#[test]
fn a10_23_the_heap_does_not_grow_with_100000_hits() {
    if in_a_process_of_its_own("a10_23_the_heap_does_not_grow_with_100000_hits") {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let service = service_threads(tmp.path(), 1);
    let many: String = (0..100_000).map(|k| format!("{k:06} foo\n")).collect();
    let few: String = (0..100_000)
        .map(|k| {
            if k % 100 == 0 {
                format!("{k:06} foo\n")
            } else {
                format!("{k:06} bar\n")
            }
        })
        .collect();
    assert_eq!(many.len(), few.len());
    // A first run warms what a search allocates once (the registry, the pool's threads).
    peak_of(&service, few.as_bytes(), 1_000_000);
    let (few_peak, few_hits) = peak_of(&service, few.as_bytes(), 1_000_000);
    let (many_peak, many_hits) = peak_of(&service, many.as_bytes(), 1_000_000);
    assert_eq!((few_hits, many_hits), (1_000, 100_000));
    eprintln!("heap peak: 1,000 hits {few_peak} bytes, 100,000 hits {many_peak} bytes");
    assert!(
        many_peak - few_peak < 1 << 20,
        "100,000 hits peak {many_peak}, 1,000 hits peak {few_peak}"
    );
}

// ---- A10.24's Service rows: the replace in files --------------------------------------------------------------------

fn replace(text: &str, answers: SaveAnswers) -> Option<TextFindReplace> {
    Some(TextFindReplace {
        text: text.into(),
        answers,
        ..TextFindReplace::default()
    })
}

const YES: SaveAnswers = SaveAnswers {
    yes: true,
    force: false,
    unmappable_replace: false,
    no_to_all: false,
};

#[test]
fn a10_24_replace_in_files_is_one_plan_with_the_operation_question() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    let a = write(
        &t,
        "a.c",
        b"int old_a = 1;\r\nint b = old_a;\r\nint old_ab;\r\n",
    );
    let b = write(&t, "sub/b.txt", b"keep\nold_x\n");
    let c = write(&t, "c.txt", b"nothing here\n");
    let mut req = request(r"old_(\w)\b", &[&t]);
    req.regex = true;
    req.match_case = true;
    req.replace = replace("new_${1}", SaveAnswers::default());
    let job = job(&service, JobKind::TextFind);
    let plan = service
        .text_find_replace_plan(&req, &mut NoEvents, &job)
        .unwrap();
    assert_eq!((plan.occurrences(), plan.files_to_write()), (3, 2));
    assert_eq!(plan.question(), "Replace 3 occurrences in 2 files?");
    assert_eq!(plan.save_plan().unwrap().items.len(), 2);
    // The operation question first, then nothing: a person says yes.
    let mut driver = service.text_find_replace_confirmation(&plan).unwrap();
    assert_eq!(driver.question(), Some("Replace 3 occurrences in 2 files?"));
    assert_eq!(driver.remaining(), 1);
    let refusal = driver.refusal().unwrap();
    assert!(refusal.to_string().contains("replace 1"), "{refusal}");
    driver.answer(Reply::Yes);
    assert!(driver.question().is_none() && driver.refusal().is_none());
    let outcome = service
        .text_find_replace_execute(plan.answered(driver).unwrap(), &job)
        .unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    // Unchanged lines and terminators byte for byte; backups by back_up_before_save (on by default).
    assert_eq!(
        std::fs::read(&a).unwrap(),
        b"int new_a = 1;\r\nint b = new_a;\r\nint old_ab;\r\n"
    );
    assert_eq!(std::fs::read(&b).unwrap(), b"keep\nnew_x\n");
    assert_eq!(std::fs::read(&c).unwrap(), b"nothing here\n");
    assert_eq!(
        std::fs::read(t.join("a.c.bak")).unwrap(),
        b"int old_a = 1;\r\nint b = old_a;\r\nint old_ab;\r\n"
    );
    let r = outcome.dto.replace.as_ref().unwrap();
    assert!(!r.dry_run);
    assert_eq!((r.summary.done, r.summary.replaced), (2, 3));
    assert_eq!(r.results[0].status, ItemStatus::Done);
    assert_eq!(r.plan.len(), 2);
    assert_eq!(
        outcome.replace_lines()[0],
        format!(
            "replaced 2 occurrences: {}",
            service.parse_spec(a.as_os_str()).unwrap().canonical()
        )
    );
    // No to the operation: nothing runs.
    let mut req = request("new", &[&t]);
    req.replace = replace("x", SaveAnswers::default());
    let plan = service
        .text_find_replace_plan(&req, &mut NoEvents, &job)
        .unwrap();
    let mut driver = service.text_find_replace_confirmation(&plan).unwrap();
    driver.answer(Reply::No);
    assert!(matches!(plan.answered(driver), Err(ApiError::Cancelled)));
    // The one call: an open operation question refuses before anything is written; --confirm no-to-all answers it.
    let err = service.text_find(&req, &mut NoEvents, &job).unwrap_err();
    assert!(err.to_string().contains("pass --yes"), "{err}");
    assert_eq!(std::fs::read(&b).unwrap(), b"keep\nnew_x\n");
    req.replace = replace(
        "x",
        SaveAnswers {
            no_to_all: true,
            ..SaveAnswers::default()
        },
    );
    req.replace.as_mut().unwrap().backup = Some(false);
    let outcome = service.text_find(&req, &mut NoEvents, &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(std::fs::read(&b).unwrap(), b"keep\nx_x\n");
}

#[test]
fn a10_24_a_dry_run_writes_nothing_and_a_file_changed_since_the_plan_fails_its_item() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    let a = write(&t, "a.txt", b"one foo\ntwo\n");
    let b = write(&t, "b.txt", b"keep\nfoo foo\n");
    let mut req = request("foo", &[&t]);
    req.replace = replace("bar", YES);
    req.replace.as_mut().unwrap().dry_run = true;
    let job = job(&service, JobKind::TextFind);
    let outcome = service.text_find(&req, &mut NoEvents, &job).unwrap();
    let r = outcome.dto.replace.as_ref().unwrap();
    assert!(r.dry_run);
    assert_eq!(r.results[1].replaced, 2);
    assert!(r.results.iter().all(|x| x.status == ItemStatus::Planned));
    assert_eq!(r.summary.done, 0);
    assert!(outcome.replace_lines()[0].starts_with("would replace 1 occurrences: "));
    assert!(
        outcome.plan_lines[0].starts_with("write "),
        "{:?}",
        outcome.plan_lines
    );
    assert_eq!(std::fs::read(&a).unwrap(), b"one foo\ntwo\n");
    // The plan, then b.txt changes (same size, the time kept): the CRC of its kept line's source finds it at the
    // write, its item fails, a.txt is written. (A file changed with a new time fails by its stamp.)
    req.replace.as_mut().unwrap().dry_run = false;
    let plan = service
        .text_find_replace_plan(&req, &mut NoEvents, &job)
        .unwrap();
    let time = std::fs::metadata(&b).unwrap().modified().unwrap();
    std::fs::write(&b, b"KEEP\nfoo foo\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&b)
        .unwrap()
        .set_modified(time)
        .unwrap();
    let driver = service.text_find_replace_confirmation(&plan).unwrap();
    assert!(driver.question().is_none(), "--yes answered the operation");
    let outcome = service
        .text_find_replace_execute(plan.answered(driver).unwrap(), &job)
        .unwrap();
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    let r = outcome.dto.replace.as_ref().unwrap();
    assert_eq!(r.results[0].status, ItemStatus::Done);
    assert_eq!(r.results[1].status, ItemStatus::Failed);
    let message = &r.results[1].error.as_ref().unwrap().message;
    assert!(
        message.contains("changed since the plan was made"),
        "{message}"
    );
    assert_eq!(std::fs::read(&a).unwrap(), b"one bar\ntwo\n");
    assert_eq!(std::fs::read(&b).unwrap(), b"KEEP\nfoo foo\n");
    assert_eq!((r.summary.done, r.summary.failed), (1, 1));
}

#[test]
fn a10_24_files_that_cannot_be_rewritten_fail_their_item() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.up", b"foo\n");
    write(&t, "b.txt", b"foo\n");
    let mut req = request("FOO", &[&t]);
    req.convert = true;
    req.replace = replace("x", YES);
    req.replace.as_mut().unwrap().backup = Some(false);
    let job = job(&service, JobKind::TextFind);
    let outcome = service.text_find(&req, &mut NoEvents, &job).unwrap();
    let r = outcome.dto.replace.as_ref().unwrap();
    // The External format has no save program: its file is refused, the other written.
    assert_eq!(r.results[0].status, ItemStatus::Failed);
    assert_eq!(
        r.results[0].error.as_ref().unwrap().kind,
        hikasa_core::ErrorKind::PermissionDenied
    );
    assert_eq!(r.results[1].status, ItemStatus::Done);
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert_eq!(std::fs::read(t.join("a.up")).unwrap(), b"foo\n");
    assert_eq!(std::fs::read(t.join("b.txt")).unwrap(), b"x\n");
    assert_eq!(r.plan[0].action, "write");
    assert_eq!(r.plan[0].status, ItemStatus::Failed);
    // A character windows-1252 cannot write: refused for that file (no terminal), or `?` with replace.
    write(&t, "w.txt", b"caf\xe9 foo\n");
    let mut req = request("foo", &[&t.join("w.txt")]);
    req.replace = replace("\u{3a9}", YES);
    req.replace.as_mut().unwrap().unmappable = hikasa_text::save::UnmappablePolicy::Refuse;
    req.replace.as_mut().unwrap().backup = Some(false);
    let outcome = service.text_find(&req, &mut NoEvents, &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert_eq!(std::fs::read(t.join("w.txt")).unwrap(), b"caf\xe9 foo\n");
    req.replace.as_mut().unwrap().unmappable = hikasa_text::save::UnmappablePolicy::Replace;
    let outcome = service.text_find(&req, &mut NoEvents, &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(std::fs::read(t.join("w.txt")).unwrap(), b"caf\xe9 ?\n");
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::UnmappableReplaced)
    );
}

#[test]
fn a10_24_a_file_without_an_occurrence_when_loaded_again_is_not_rewritten() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    let a = write(&t, "a.txt", b"foo\n");
    // A sink that changes the file once the search delivered it: the plan loads it again and finds nothing.
    struct Change(std::path::PathBuf);
    impl FilesSink for Change {
        fn event(&mut self, e: FilesEvent<'_>) {
            if let FilesEvent::Done { .. } = e {
                std::fs::write(&self.0, b"bar\n").unwrap();
            }
        }
    }
    let mut req = request("foo", &[&t]);
    req.replace = replace("x", YES);
    let job = job(&service, JobKind::TextFind);
    let outcome = service
        .text_find(&req, &mut Change(a.clone()), &job)
        .unwrap();
    let r = outcome.dto.replace.as_ref().unwrap();
    assert_eq!(r.results[0].status, ItemStatus::Skipped);
    assert_eq!(r.results[0].skip, Some(SkipReason::NoChange));
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(
        outcome.replace_lines()[0].split(':').next(),
        Some("no change")
    );
    assert_eq!(std::fs::read(&a).unwrap(), b"bar\n");
    assert!(!t.join("a.txt.bak").exists());
}

/// A10.32: a file whose name is not UTF-8 is found and rewritten (Linux).
#[cfg(target_os = "linux")]
#[test]
fn a10_32_a_non_utf8_file_name_is_found_and_rewritten() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    std::fs::create_dir_all(&t).unwrap();
    let odd = t.join(std::ffi::OsStr::from_bytes(b"n\xffme.txt"));
    std::fs::write(&odd, b"a foo\n").unwrap();
    let mut req = request("foo", &[&t]);
    let (outcome, lines) = find(&service, &req, &t);
    assert_eq!(outcome.dto.counts.files_matched, 1, "{lines:?}");
    req.replace = replace("bar", YES);
    req.replace.as_mut().unwrap().backup = Some(false);
    let outcome = service
        .text_find(&req, &mut NoEvents, &job(&service, JobKind::TextFind))
        .unwrap();
    assert_eq!(
        outcome.exit_code,
        ExitCode::Success,
        "{:?}",
        outcome.dto.replace
    );
    assert_eq!(std::fs::read(&odd).unwrap(), b"a bar\n");
}

/// A sanity check of the counter the memory row reads.
#[test]
fn the_counting_allocator_counts() {
    if in_a_process_of_its_own("the_counting_allocator_counts") {
        return;
    }
    let base = LIVE.load(Ordering::Relaxed);
    let v = std::hint::black_box(vec![1u8; 1 << 20]);
    assert!(LIVE.load(Ordering::Relaxed) - base >= 1 << 20);
    drop(v);
}

/// Whole words follow each file's format (D-58): COBOL's `-` is a word character, so `WS-FOO` holds no word `FOO`,
/// in the search and in the replace (which loads the file again with its format's finder).
#[test]
fn a10_24_whole_words_follow_the_file_s_format_in_the_replace_too() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    let cbl = write(&t, "a.cbl", b"MOVE WS-FOO TO FOO.\n");
    let txt = write(&t, "b.txt", b"WS-FOO FOO\n");
    let mut req = request("FOO", &[&t]);
    req.whole_word = true;
    req.match_case = true;
    req.keep_hits = true;
    let (outcome, _) = find(&service, &req, &t);
    let counts: Vec<u64> = outcome.dto.files.iter().map(|f| f.hit_count).collect();
    assert_eq!(counts, [1, 2]);
    req.replace = replace("BAR", YES);
    req.replace.as_mut().unwrap().backup = Some(false);
    let outcome = service
        .text_find(&req, &mut NoEvents, &job(&service, JobKind::TextFind))
        .unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(std::fs::read(&cbl).unwrap(), b"MOVE WS-FOO TO BAR.\n");
    assert_eq!(std::fs::read(&txt).unwrap(), b"WS-BAR BAR\n");
}

/// Masks match names by the host's case rule (D-10): `*.TXT` takes `a.txt` only where names ignore case.
#[test]
fn a10_23_masks_follow_the_host_s_case_rule() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let t = tmp.path().join("work/t");
    write(&t, "a.txt", b"foo\n");
    write(&t, "b.TXT", b"foo\n");
    let mut req = request("foo", &[&t]);
    req.mask = Some("*.TXT".into());
    let (_, lines) = find(&service, &req, &t);
    let insensitive =
        hikasa_core::CaseSensitivity::HOST_DEFAULT == hikasa_core::CaseSensitivity::Insensitive;
    if insensitive {
        assert_eq!(files(&lines), ["<T>/a.txt", "<T>/b.TXT"]);
    } else {
        assert_eq!(files(&lines), ["<T>/b.TXT"]);
    }
}

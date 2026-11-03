//! M2 W7b acceptance through the Service: A7.11 (`pair_text_request`), A7.12 (linked reports on `MemoryVfs` and a
//! tempdir: links resolve, the files folder's states, faults at every step, cancels, P-WRITE), A7.14 (targets and the
//! `--output` semantics), A7.15 (the clipboard payload), A7.16 (printer and browser), A7.19 (remembered choices).
//!
//! The comparisons read their sources from a tempdir; the report is written to the back-end the test gives
//! `report_plan_on` / `report_execute_on`: a `MemoryVfs` (faults, journal, durabilities, `dump()`) or the Service's
//! own router (the disk).

#![allow(clippy::type_complexity)]

mod common;

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{facts, fixed_clock, job, os, write};
use hikasa_core::{CancelToken, CaseSensitivity, ErrorKind, JobKind, Spec, WarningCode};
use hikasa_folder::NullObserver;
use hikasa_report::{ReportKind, ReportRequest, ReportWords};
use hikasa_session::api::{
    FilesFolder, FolderCompareRequest, FolderCompareResponse, PairStatus, PreparedFolderCompare,
    ReportChoices, ReportFlags, ReportOutcome, ReportSource, ReportTarget, TextCompareRequest,
    TextCompareResponse,
};
use hikasa_session::clipboard::{CLIPBOARD_REPORT_MAX, FixedClipboard};
use hikasa_session::jobs::JobControl;
use hikasa_session::launcher::{Recorded, RecordingLauncher};
use hikasa_session::{ApiError, Service, ServiceInit};
use hikasa_vfs::memory::{Fault, MemDump, MemMeta, MemoryVfs};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Durability, Entry, FileName, FollowLinks, ListOptions, Listing,
    ReadSeek, Vfs, VfsError, VfsOp, WriteOptions,
};

// ---- fixtures --------------------------------------------------------------------------------------------------

/// A service on `root` with a recording launcher and a fixed clipboard.
fn service_on(
    root: &Path,
    launcher: Arc<RecordingLauncher>,
    clipboard: Arc<FixedClipboard>,
) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(3),
        launcher,
        clipboard,
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

fn service(root: &Path) -> Service {
    service_on(
        root,
        Arc::new(RecordingLauncher::default()),
        Arc::new(FixedClipboard::empty()),
    )
}

/// A host path for `MemoryVfs`: `/a/b` on Unix, `C:\a\b` on Windows.
fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

/// A `MemoryVfs` with the folder `/o`.
fn mem(case: CaseSensitivity) -> MemoryVfs {
    let vfs = MemoryVfs::new(case);
    vfs.insert_dir(&host("/o"), MemMeta::default()).unwrap();
    vfs
}

/// The two trees of a folder comparison below `root`: `l/<rel>` and `r/<rel>` for each `(rel, left, right)`; `None`
/// leaves that side out.
fn trees(root: &Path, files: &[(&str, Option<&[u8]>, Option<&[u8]>)]) {
    std::fs::create_dir_all(root.join("l")).unwrap();
    std::fs::create_dir_all(root.join("r")).unwrap();
    for (rel, left, right) in files {
        if let Some(bytes) = left {
            write(&root.join("l"), rel, bytes);
        }
        if let Some(bytes) = right {
            write(&root.join("r"), rel, bytes);
        }
    }
}

/// A finished folder comparison of `root/l` and `root/r`.
struct Folder {
    request: FolderCompareRequest,
    prepared: PreparedFolderCompare,
    response: FolderCompareResponse,
}

impl Folder {
    fn source(&self) -> ReportSource<'_> {
        ReportSource::Folder {
            request: &self.request,
            prepared: &self.prepared,
            response: &self.response,
        }
    }
}

fn compare(service: &Service, root: &Path) -> Folder {
    let request = FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        ..FolderCompareRequest::default()
    };
    let control = job(service, JobKind::FolderCompare);
    let prepared = service.folder_prepare_in(&request, &control).unwrap();
    let response = service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    Folder {
        request,
        prepared,
        response,
    }
}

/// Folder side-by-side HTML with links.
fn linked(service: &Service, target: &ReportTarget) -> ReportRequest {
    let flags = ReportFlags {
        words: ReportWords {
            options: vec!["include-file-links".into()],
            ..ReportWords::default()
        },
        html: Some(false),
        ..ReportFlags::default()
    };
    service
        .report_request(ReportKind::Folder, "side-by-side", &flags, target)
        .unwrap()
}

fn file_target(path: &str) -> ReportTarget {
    ReportTarget::File(OsString::from(path))
}

/// Plans and runs a report of `folder` on `vfs`.
fn run(
    service: &Service,
    vfs: &dyn Vfs,
    folder: &Folder,
    req: &mut ReportRequest,
    target: &ReportTarget,
    control: &JobControl,
) -> Result<ReportOutcome, ApiError> {
    let plan = service.report_plan_on(vfs, folder.source(), req, target, None, control)?;
    service.report_execute_on(vfs, folder.source(), req, plan, None, control)
}

/// The `href`s of an HTML document.
fn hrefs(html: &str) -> Vec<String> {
    html.split("href=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap_or("").to_owned())
        .collect()
}

/// The path an href names, joined to `base` with `sep`, the segments percent-decoded.
fn join_href(base: &str, link: &str, sep: char) -> String {
    let segments = hikasa_report::links::href_segments(link).expect("a well-formed href");
    let mut path = base.to_owned();
    for segment in segments {
        let segment = String::from_utf8(segment).expect("UTF-8 in these tests");
        if segment == ".." {
            let cut = path.rfind(sep).unwrap();
            path.truncate(cut);
        } else {
            path.push(sep);
            path.push_str(&segment);
        }
    }
    path
}

/// The text of a file on `vfs`.
fn text_of(vfs: &MemoryVfs, path: &str) -> String {
    String::from_utf8(vfs.read_back(path).expect(path).to_vec()).unwrap()
}

/// The paths of a dump.
fn paths(dump: &[MemDump]) -> BTreeSet<String> {
    dump.iter().map(|d| d.path.clone()).collect()
}

/// The three pairs every linked test compares: two differ, one is the same, one is an orphan.
const PAIRS: &[(&str, Option<&[u8]>, Option<&[u8]>)] = &[
    ("a.txt", Some(b"one\ntwo\n"), Some(b"one\nTWO!\n")),
    ("sub/b.c", Some(b"int x;\n"), Some(b"int yy;\n")),
    ("same.txt", Some(b"same\n"), Some(b"same\n")),
    ("only-left.txt", Some(b"alone\n"), None),
];

// ---- A7.11 -----------------------------------------------------------------------------------------------------

/// A7.11: the one builder of a pair's Text Compare request; `None` for a folder, an orphan and a link.
#[test]
fn a7_11_pair_text_request_table() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a.txt", root.join("l/link")).unwrap();
        std::os::unix::fs::symlink("a.txt", root.join("r/link")).unwrap();
    }
    let rules = write(root, "text.toml", b"");
    let service = service(root);
    let text_patch = hikasa_text::TextRulesPatch {
        compare_line_endings: Some(true),
        ..Default::default()
    };
    let patch = hikasa_folder::FolderRulesPatch {
        format_overrides: Some([("text".to_owned(), false)].into_iter().collect()),
        ..Default::default()
    };
    let request = FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        text_rules_file: Some(os(&rules)),
        text_patch: text_patch.clone(),
        patch,
        ignore_unimportant: true,
        ..FolderCompareRequest::default()
    };
    let control = job(&service, JobKind::FolderCompare);
    let prepared = service.folder_prepare_in(&request, &control).unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    let read = prepared.model().read();
    let row = |path: &str| {
        read.node_at(&hikasa_core::RelPath::parse(path).unwrap())
            .unwrap_or_else(|| panic!("{path}"))
    };
    let got = service
        .pair_text_request(&request, &prepared, row("a.txt"))
        .unwrap();
    assert_eq!(got.left, os(&root.join("l").join("a.txt")));
    assert_eq!(got.right, os(&root.join("r").join("a.txt")));
    assert_eq!(got.rules_file, Some(os(&rules)));
    assert_eq!(got.rules_patch, text_patch);
    assert!(got.ignore_unimportant, "the effective ignore-unimportant");
    assert_eq!(&got.format_overrides, prepared.format_overrides());
    assert_eq!(got.format_overrides.get("text"), Some(&false));
    let base = TextCompareRequest::for_specs(
        &read.spec(row("a.txt"), hikasa_core::Side::Left).unwrap(),
        &read.spec(row("a.txt"), hikasa_core::Side::Right).unwrap(),
    );
    assert_eq!(
        (got.left.clone(), got.right.clone()),
        (base.left, base.right)
    );
    assert!(
        service
            .pair_text_request(&request, &prepared, row("sub"))
            .is_none(),
        "a folder"
    );
    assert!(
        service
            .pair_text_request(&request, &prepared, row("only-left.txt"))
            .is_none(),
        "an orphan"
    );
    #[cfg(unix)]
    assert!(
        service
            .pair_text_request(&request, &prepared, row("link"))
            .is_none(),
        "a link shown as a link"
    );
}

/// A7.11 on a Linux disk: names that are not UTF-8 reach the request exactly.
#[cfg(target_os = "linux")]
#[test]
fn a7_11_pair_specs_that_are_not_utf8_are_exact() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, &[]);
    let name = OsStr::from_bytes(b"caf\xe9.txt");
    std::fs::write(root.join("l").join(name), b"x\n").unwrap();
    std::fs::write(root.join("r").join(name), b"yy\n").unwrap();
    let service = service(root);
    let folder = compare(&service, root);
    let read = folder.prepared.model().read();
    let node = read
        .children(hikasa_folder::NodeId::ROOT)
        .find(|&id| {
            read.node(id)
                .is_some_and(|n| n.left.as_ref().is_some_and(|e| e.name_lossy))
        })
        .unwrap();
    let got = service
        .pair_text_request(&folder.request, &folder.prepared, node)
        .unwrap();
    assert_eq!(got.left, root.join("l").join(name).into_os_string());
    assert_eq!(got.right, root.join("r").join(name).into_os_string());
}

// ---- A7.12 -----------------------------------------------------------------------------------------------------

/// A7.12 on disk: a first run makes the files folder (`New`); every href of the main report names a written pair
/// report, every pair report's back link names the main file, and nothing else is written. A second run replaces
/// it (`ReplaceOurs`): a pair that no longer differs loses its file.
#[test]
fn a7_12_links_resolve_on_disk_then_a_rerun_replaces_ours() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    std::fs::create_dir_all(root.join("out")).unwrap();
    let service = service(root);
    let folder = compare(&service, root);
    let main = root.join("out").join("report.html");
    let target = ReportTarget::File(main.clone().into_os_string());
    let mut req = linked(&service, &target);
    let control = job(&service, JobKind::Report);
    let router = service.vfs();
    let outcome = run(&service, &*router, &folder, &mut req, &target, &control).unwrap();
    let links = outcome.plan.links.as_ref().unwrap();
    assert_eq!(links.state, FilesFolder::New);
    assert_eq!(outcome.pairs.len(), 2, "{:?}", outcome.pairs);
    assert!(
        outcome
            .pairs
            .iter()
            .all(|p| p.status == PairStatus::Written)
    );
    let html = std::fs::read_to_string(&main).unwrap();
    let sep = std::path::MAIN_SEPARATOR;
    let base = root.join("out").display().to_string();
    let mut named = BTreeSet::new();
    for link in hrefs(&html) {
        let path = join_href(&base, &link, sep);
        assert!(Path::new(&path).is_file(), "{link} → {path}");
        named.insert(path.clone());
        let pair = std::fs::read_to_string(&path).unwrap();
        let back: Vec<String> = hrefs(&pair);
        assert_eq!(back.len(), 1, "{back:?}");
        let pair_dir = Path::new(&path).parent().unwrap().display().to_string();
        assert_eq!(
            join_href(&pair_dir, &back[0], sep),
            main.display().to_string()
        );
        assert!(pair.contains("<th>Folder report</th>"));
    }
    assert_eq!(
        named.len(),
        2,
        "each written pair is linked twice (both names), {named:?}"
    );
    let listed: BTreeSet<String> = std::fs::read_dir(root.join("out/report_files"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    // Report order: the folder `sub` comes before the files of the base.
    assert_eq!(
        listed,
        ["1-b.c.html", "2-a.txt.html", ".hikasa-report.toml"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    let out: BTreeSet<String> = std::fs::read_dir(root.join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        out,
        ["report.html", "report_files"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "no hidden folder stays"
    );
    // A second run: `sub/b.c` no longer differs, so its report goes.
    write(&root.join("r"), "sub/b.c", b"int x;\n");
    let folder = compare(&service, root);
    let mut req = linked(&service, &target);
    let outcome = run(&service, &*router, &folder, &mut req, &target, &control).unwrap();
    let links = outcome.plan.links.as_ref().unwrap();
    assert_eq!(links.state, FilesFolder::ReplaceOurs);
    assert!(outcome.plan.replaces);
    let listed: BTreeSet<String> = std::fs::read_dir(root.join("out/report_files"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        listed,
        ["1-a.txt.html", ".hikasa-report.toml"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    assert_eq!(
        std::fs::read_dir(root.join("out")).unwrap().count(),
        2,
        "the old folder is gone"
    );
}

/// The fixture of the `MemoryVfs` tests: the comparison on disk, the report on `vfs`.
struct OnMem {
    _dir: tempfile::TempDir,
    service: Service,
    folder: Folder,
}

fn on_mem() -> OnMem {
    let dir = tempfile::tempdir().unwrap();
    trees(dir.path(), PAIRS);
    let service = service(dir.path());
    let folder = compare(&service, dir.path());
    OnMem {
        _dir: dir,
        service,
        folder,
    }
}

impl OnMem {
    fn run(&self, vfs: &dyn Vfs, path: &str) -> Result<ReportOutcome, ApiError> {
        let target = file_target(&host(path));
        let mut req = linked(&self.service, &target);
        let control = job(&self.service, JobKind::Report);
        run(
            &self.service,
            vfs,
            &self.folder,
            &mut req,
            &target,
            &control,
        )
    }
}

/// A7.12: a files folder that is not this report's is left as it is, byte for byte, and the report goes to
/// `_files-2` with the warning; 99 taken is `already_exists`.
#[test]
fn a7_12_folders_that_are_not_ours_are_left_alone() {
    let fx = on_mem();
    // An earlier report of ours, to copy a marker from.
    let ours = mem(CaseSensitivity::Sensitive);
    fx.run(&ours, "/o/report.html").unwrap();
    let marker = text_of(&ours, &host("/o/report_files/.hikasa-report.toml"));
    let pair = ours
        .read_back(&host("/o/report_files/2-a.txt.html"))
        .unwrap();
    let cases: Vec<(&str, Box<dyn Fn(&MemoryVfs)>)> = vec![
        (
            "a foreign folder",
            Box::new(|v: &MemoryVfs| {
                v.insert_file(
                    &host("/o/report_files/mine.txt"),
                    &b"keep"[..],
                    MemMeta::default(),
                )
                .unwrap();
            }),
        ),
        (
            "another report's folder",
            Box::new(|v: &MemoryVfs| {
                let other = marker.replace("report = \"report.html\"", "report = \"other.html\"");
                v.insert_file(
                    &host("/o/report_files/.hikasa-report.toml"),
                    other.into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/1-b.c.html"),
                    pair.clone(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/2-a.txt.html"),
                    pair.clone(),
                    MemMeta::default(),
                )
                .unwrap();
            }),
        ),
        (
            "one extra file",
            Box::new(|v: &MemoryVfs| {
                v.insert_file(
                    &host("/o/report_files/.hikasa-report.toml"),
                    marker.clone().into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/1-b.c.html"),
                    pair.clone(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/2-a.txt.html"),
                    pair.clone(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/extra"),
                    &b"x"[..],
                    MemMeta::default(),
                )
                .unwrap();
            }),
        ),
        (
            "one changed file",
            Box::new(|v: &MemoryVfs| {
                v.insert_file(
                    &host("/o/report_files/.hikasa-report.toml"),
                    marker.clone().into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/1-b.c.html"),
                    &b"changed"[..],
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/2-a.txt.html"),
                    pair.clone(),
                    MemMeta::default(),
                )
                .unwrap();
            }),
        ),
        (
            "a file at the folder's name",
            Box::new(|v: &MemoryVfs| {
                v.insert_file(&host("/o/report_files"), &b"a file"[..], MemMeta::default())
                    .unwrap();
            }),
        ),
    ];
    for (what, setup) in cases {
        let vfs = mem(CaseSensitivity::Sensitive);
        setup(&vfs);
        let before: Vec<MemDump> = vfs
            .dump()
            .into_iter()
            .filter(|d| d.path.contains("report_files") && !d.path.contains("report_files-2"))
            .collect();
        let outcome = fx
            .run(&vfs, "/o/report.html")
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        let links = outcome.plan.links.as_ref().unwrap();
        assert_eq!(
            links.folder.canonical(),
            host("/o/report_files-2"),
            "{what}"
        );
        assert_eq!(links.wanted.canonical(), host("/o/report_files"), "{what}");
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.code == WarningCode::ReportFolderRenamed
                    && w.message.contains("report_files-2")),
            "{what}: {:?}",
            outcome.warnings
        );
        let after: Vec<MemDump> = vfs
            .dump()
            .into_iter()
            .filter(|d| d.path.contains("report_files") && !d.path.contains("report_files-2"))
            .collect();
        assert_eq!(before, after, "{what}: the foreign content is as it was");
        assert!(
            text_of(&vfs, &host("/o/report.html")).contains("report_files-2/2-a.txt.html"),
            "{what}"
        );
    }
    // Every name taken.
    let vfs = mem(CaseSensitivity::Sensitive);
    for n in 1..=99 {
        let name = if n == 1 {
            "report_files".to_owned()
        } else {
            format!("report_files-{n}")
        };
        vfs.insert_file(&host(&format!("/o/{name}")), &b"x"[..], MemMeta::default())
            .unwrap();
    }
    let before = vfs.dump();
    let err = fx.run(&vfs, "/o/report.html").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists, "{err}");
    assert_eq!(vfs.dump(), before);
}

/// A back-end that delegates to a `MemoryVfs` and cancels a token at the `n`-th mutating call (`create_dir`,
/// `open_write`, `rename`, `remove`): a cancel "before step k" of the execution table.
struct CancelAt<'a> {
    inner: &'a MemoryVfs,
    token: CancelToken,
    at: usize,
    calls: AtomicUsize,
}

impl CancelAt<'_> {
    fn tick(&self) {
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            self.token.cancel();
        }
    }
}

impl Vfs for CancelAt<'_> {
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
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        let w = self.inner.open_write(spec, opts);
        self.tick();
        w
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        let r = self.inner.create_dir(spec, parents);
        self.tick();
        r
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        let r = self.inner.remove(spec);
        self.tick();
        r
    }
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        let r = self.inner.rename(from, to, replace);
        self.tick();
        r
    }
}

/// A `MemoryVfs` holding an earlier report of ours at `/o/report.html` (so the run replaces its files folder).
fn with_earlier(fx: &OnMem) -> MemoryVfs {
    let vfs = mem(CaseSensitivity::Sensitive);
    fx.run(&vfs, "/o/report.html").unwrap();
    vfs.insert_file(
        &host("/o/sibling.txt"),
        &b"a neighbour"[..],
        MemMeta::default(),
    )
    .unwrap();
    vfs
}

/// No hidden folder of a report stays in `/o`.
fn no_hidden(vfs: &MemoryVfs) {
    for d in vfs.dump() {
        assert!(
            !d.path.contains(".hikasa-tmp") && !d.path.contains(".hikasa-old"),
            "{} stayed",
            d.path
        );
    }
}

/// A7.12: a fault at each step of the execution table leaves the main file, the files folder and its siblings
/// exactly as before, with no hidden folder; a fault at step 7 leaves the new report and the warning.
#[test]
fn a7_12_a_fault_at_each_step_leaves_everything_as_before() {
    let fx = on_mem();
    let staging = host("/o/.report_files.hikasa-tmp");
    let steps: Vec<(&str, Fault)> = vec![
        (
            "1 staging folder",
            Fault {
                path: staging.clone(),
                op: VfsOp::Mkdir,
                kind: ErrorKind::PermissionDenied,
            },
        ),
        (
            "2 the marker",
            Fault {
                path: host("/o/.report_files.hikasa-tmp/.hikasa-report.toml"),
                op: VfsOp::Write,
                kind: ErrorKind::Io,
            },
        ),
        (
            "3 the main writer",
            Fault {
                path: host("/o/report.html"),
                op: VfsOp::Write,
                kind: ErrorKind::Io,
            },
        ),
        (
            "4 the old folder's rename",
            Fault {
                path: host("/o/report_files"),
                op: VfsOp::Rename,
                kind: ErrorKind::PermissionDenied,
            },
        ),
        (
            "5 staging's rename",
            Fault {
                path: staging.clone(),
                op: VfsOp::Rename,
                kind: ErrorKind::PermissionDenied,
            },
        ),
        (
            "6 the main commit",
            Fault {
                path: host("/o/report.html"),
                op: VfsOp::Commit,
                kind: ErrorKind::Io,
            },
        ),
    ];
    for earlier in [false, true] {
        for (step, fault) in &steps {
            if !earlier && step.starts_with('4') {
                continue; // nothing to rename when the folder is new
            }
            let vfs = if earlier {
                with_earlier(&fx)
            } else {
                mem(CaseSensitivity::Sensitive)
            };
            let before = vfs.dump();
            vfs.inject(fault.clone());
            let err = fx.run(&vfs, "/o/report.html");
            assert!(err.is_err(), "step {step} (earlier {earlier}) succeeded");
            vfs.clear_faults();
            assert_eq!(vfs.dump(), before, "step {step}, earlier {earlier}");
        }
    }
    // Step 7: the report stands, the old folder is kept with the warning.
    let vfs = with_earlier(&fx);
    vfs.inject(Fault {
        path: host("/o/.report_files.hikasa-old/2-a.txt.html"),
        op: VfsOp::Remove,
        kind: ErrorKind::PermissionDenied,
    });
    let outcome = fx.run(&vfs, "/o/report.html").unwrap();
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::ReportOldFolderKept
                && w.message.contains(".report_files.hikasa-old")),
        "{:?}",
        outcome.warnings
    );
    assert!(
        vfs.read_back(&host("/o/report_files/2-a.txt.html"))
            .is_some()
    );
}

/// A7.12: a cancel before each of steps 1–4 and between pairs leaves everything as before, no hidden folder left.
#[test]
fn a7_12_a_cancel_before_each_step_leaves_everything_as_before() {
    let fx = on_mem();
    for earlier in [false, true] {
        // The mutating calls of a run: mkdir staging (1), each pair's write (2, 3), the marker (4), the main writer
        // (5), then renames. A cancel right after the k-th is seen before the next step.
        for at in 1..=5 {
            let vfs = if earlier {
                with_earlier(&fx)
            } else {
                mem(CaseSensitivity::Sensitive)
            };
            let before = vfs.dump();
            let token = CancelToken::new();
            let wrapper = CancelAt {
                inner: &vfs,
                token: token.clone(),
                at,
                calls: AtomicUsize::new(0),
            };
            let target = file_target(&host("/o/report.html"));
            let mut req = linked(&fx.service, &target);
            let control = JobControl {
                cancel: token,
                ..job(&fx.service, JobKind::Report)
            };
            let result = run(
                &fx.service,
                &wrapper,
                &fx.folder,
                &mut req,
                &target,
                &control,
            );
            assert!(
                matches!(result, Err(ApiError::Cancelled)),
                "cancel at {at}, earlier {earlier}: {result:?}"
            );
            assert_eq!(vfs.dump(), before, "cancel at {at}, earlier {earlier}");
            no_hidden(&vfs);
        }
    }
}

/// A7.12: the old folder changed between the plan and step 4: the report fails "changed since the plan was made",
/// and nothing changes.
#[test]
fn a7_12_a_folder_changed_since_the_plan_fails() {
    let fx = on_mem();
    let vfs = with_earlier(&fx);
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&fx.service, &target);
    let control = job(&fx.service, JobKind::Report);
    let plan = fx
        .service
        .report_plan_on(&vfs, fx.folder.source(), &mut req, &target, None, &control)
        .unwrap();
    assert_eq!(plan.links.as_ref().unwrap().state, FilesFolder::ReplaceOurs);
    vfs.insert_file(
        &host("/o/report_files/new.txt"),
        &b"someone's"[..],
        MemMeta::default(),
    )
    .unwrap();
    let before = vfs.dump();
    let err = fx
        .service
        .report_execute_on(&vfs, fx.folder.source(), &req, plan, None, &control)
        .unwrap_err();
    assert!(
        err.to_string().contains("changed since the plan was made"),
        "{err}"
    );
    assert_eq!(vfs.dump(), before);
}

/// A7.12: durabilities: `None` for the pair files and a new main file, `FileOnly` for a replaced main file.
#[test]
fn a7_12_durabilities() {
    let fx = on_mem();
    let vfs = mem(CaseSensitivity::Sensitive);
    fx.run(&vfs, "/o/report.html").unwrap();
    for (spec, durability) in vfs.durabilities() {
        assert_eq!(durability, Durability::None, "{spec}");
    }
    let again = mem(CaseSensitivity::Sensitive);
    fx.run(&again, "/o/report.html").unwrap();
    let before = again.durabilities().len();
    fx.run(&again, "/o/report.html").unwrap();
    let replaced: Vec<(String, Durability)> =
        again.durabilities().into_iter().skip(before).collect();
    for (spec, durability) in &replaced {
        let want = if spec.ends_with("report.html") {
            Durability::FileOnly
        } else {
            Durability::None
        };
        assert_eq!(*durability, want, "{spec}");
    }
    assert!(replaced.iter().any(|(s, _)| s.ends_with("report.html")));
}

/// A7.12 on a case-insensitive back-end (`MemoryVfs` folding, the same on every host): an existing `REPORT_FILES`
/// that is not ours makes `_files-2`; a marker that names `REPORT.html` is ours for `report.html`.
#[test]
fn a7_12_case_insensitive_names() {
    let fx = on_mem();
    let vfs = mem(CaseSensitivity::Insensitive);
    vfs.insert_file(
        &host("/o/REPORT_FILES/theirs.txt"),
        &b"x"[..],
        MemMeta::default(),
    )
    .unwrap();
    let outcome = fx.run(&vfs, "/o/report.html").unwrap();
    assert_eq!(
        outcome.plan.links.as_ref().unwrap().folder.canonical(),
        host("/o/report_files-2")
    );
    assert!(vfs.read_back(&host("/o/REPORT_FILES/theirs.txt")).is_some());
    // Ours, by a name that differs in case.
    let vfs = mem(CaseSensitivity::Insensitive);
    fx.run(&vfs, "/o/REPORT.html").unwrap();
    let outcome = fx.run(&vfs, "/o/report.html").unwrap();
    let links = outcome.plan.links.as_ref().unwrap();
    assert_eq!(
        links.state,
        FilesFolder::ReplaceOurs,
        "{:?}",
        outcome.warnings
    );
}

/// A7.12: two pairs that differ in case only get names whose case folds differ.
#[cfg(target_os = "linux")]
#[test]
fn a7_12_two_pairs_differing_in_case_get_distinct_files() {
    let dir = tempfile::tempdir().unwrap();
    trees(
        dir.path(),
        &[
            ("A.txt", Some(b"1\n"), Some(b"22\n")),
            ("a.txt", Some(b"3\n"), Some(b"44\n")),
        ],
    );
    let service = service(dir.path());
    let folder = compare(&service, dir.path());
    let vfs = mem(CaseSensitivity::Insensitive);
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&service, &target);
    let outcome = run(
        &service,
        &vfs,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let files: Vec<String> = outcome
        .pairs
        .iter()
        .filter_map(|p| p.file.clone())
        .collect();
    assert_eq!(files.len(), 2);
    assert_ne!(files[0].to_lowercase(), files[1].to_lowercase());
}

/// A7.12: a binary pair is skipped, a pair over the rules-based limit is skipped (`too_large`) from the plan, and a
/// pair whose comparison fails is `failed` with the warning; the folder report is written.
#[test]
fn a7_12_skipped_and_failed_pairs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(
        root,
        &[
            (
                "bin.dat",
                Some(b"\x00\x01\x02binary"),
                Some(b"\x00\x01\x03binary!"),
            ),
            ("gone.txt", Some(b"a\n"), Some(b"bb\n")),
            ("ok.txt", Some(b"a\n"), Some(b"bb\n")),
        ],
    );
    // A sparse file over 128 MiB: the plan skips it without reading.
    let big = std::fs::File::create(root.join("l/big.txt")).unwrap();
    big.set_len(129 << 20).unwrap();
    write(&root.join("r"), "big.txt", b"small\n");
    let service = service(root);
    let folder = compare(&service, root);
    std::fs::remove_file(root.join("r/gone.txt")).unwrap();
    let vfs = mem(CaseSensitivity::Sensitive);
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&service, &target);
    let outcome = run(
        &service,
        &vfs,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let status = |path: &str| {
        outcome
            .pairs
            .iter()
            .find(|p| p.path == path)
            .unwrap_or_else(|| panic!("{path}"))
    };
    assert_eq!(status("bin.dat").status, PairStatus::Skipped);
    assert_eq!(status("bin.dat").reason.as_deref(), Some("binary"));
    assert_eq!(status("big.txt").status, PairStatus::Skipped);
    assert_eq!(status("big.txt").reason.as_deref(), Some("too_large"));
    let skip = outcome
        .plan
        .links
        .as_ref()
        .unwrap()
        .pairs
        .iter()
        .find(|p| p.path == "big.txt")
        .unwrap();
    assert_eq!(skip.skip.as_deref(), Some("too_large"), "known in the plan");
    assert_eq!(status("gone.txt").status, PairStatus::Failed);
    assert!(status("gone.txt").error.is_some());
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::FileReportFailed && w.message.contains("gone.txt"))
    );
    assert_eq!(status("ok.txt").status, PairStatus::Written);
    let html = text_of(&vfs, &host("/o/report.html"));
    assert!(
        html.contains("title=\"no file report: binary\""),
        "the skipped row's names say why"
    );
}

// ---- P-WRITE ---------------------------------------------------------------------------------------------------

/// What stands at the files folder's name before a P-WRITE case.
#[derive(Debug, Clone, Copy)]
enum Before {
    Nothing,
    Ours,
    Foreign,
    AFile,
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(48),
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    /// P-WRITE: over random trees, random files folders before and random faults, a dry run's plan is the plan
    /// execution takes and writes nothing; a run either writes the report whole (every link resolves, the files folder
    /// holds exactly the marker and the written pairs, nothing foreign changed) or leaves the back-end as it was.
    #[test]
    fn p_write_plans_are_executed_whole_or_not_at_all(
        files in proptest::collection::vec((0u8..6, 0u8..3, 0u8..3), 1..7),
        before in proptest::prop_oneof![
            proptest::strategy::Just(Before::Nothing), proptest::strategy::Just(Before::Ours),
            proptest::strategy::Just(Before::Foreign), proptest::strategy::Just(Before::AFile)
        ],
        fault in proptest::option::weighted(0.5, 0usize..7),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut spec: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>)> = Vec::new();
        let mut seen = BTreeSet::new();
        for (name, l, r) in &files {
            if !seen.insert(*name) {
                continue;
            }
            let content = |k: u8, side: &str| match k {
                0 => None,
                1 => Some(b"same\n".to_vec()),
                _ => Some(format!("{side} {name}\n").into_bytes()),
            };
            spec.push((format!("f{name}.txt"), content(*l, "left"), content(*r, "right!")));
        }
        let borrowed: Vec<(&str, Option<&[u8]>, Option<&[u8]>)> =
            spec.iter().map(|(n, l, r)| (n.as_str(), l.as_deref(), r.as_deref())).collect();
        trees(root, &borrowed);
        let service = service(root);
        let folder = compare(&service, root);
        let vfs = mem(CaseSensitivity::Sensitive);
        match before {
            Before::Nothing => {}
            Before::Ours => {
                let target = file_target(&host("/o/report.html"));
                let mut req = linked(&service, &target);
                run(&service, &vfs, &folder, &mut req, &target, &job(&service, JobKind::Report)).unwrap();
            }
            Before::Foreign => vfs.insert_file(&host("/o/report_files/theirs"), &b"t"[..], MemMeta::default()).unwrap(),
            Before::AFile => vfs.insert_file(&host("/o/report_files"), &b"f"[..], MemMeta::default()).unwrap(),
        }
        let target = file_target(&host("/o/report.html"));
        let mut req = linked(&service, &target);
        let control = job(&service, JobKind::Report);
        let plan = service.report_plan_on(&vfs, folder.source(), &mut req, &target, None, &control).unwrap();
        let links = plan.links.clone().unwrap();
        match before {
            Before::Nothing => proptest::prop_assert!(links.state == FilesFolder::New && links.folder == links.wanted),
            Before::Ours => proptest::prop_assert!(links.state == FilesFolder::ReplaceOurs && links.folder == links.wanted, "an earlier report is ours"),
            Before::Foreign | Before::AFile => proptest::prop_assert!(links.state == FilesFolder::New && links.folder != links.wanted),
        }
        let state = vfs.dump();
        let dry = service.report_dry_run(plan.clone());
        proptest::prop_assert_eq!(vfs.dump(), state.clone(), "a dry run wrote");
        let planned = serde_json::to_string(&dry.plan).unwrap();
        let faults = [
            (".report_files.hikasa-tmp", VfsOp::Mkdir),
            (".report_files.hikasa-tmp/.hikasa-report.toml", VfsOp::Write),
            ("report.html", VfsOp::Write),
            ("report_files", VfsOp::Rename),
            (".report_files.hikasa-tmp", VfsOp::Rename),
            ("report.html", VfsOp::Commit),
            (".report_files.hikasa-old/.hikasa-report.toml", VfsOp::Remove),
        ];
        if let Some(k) = fault {
            let (path, op) = faults[k];
            vfs.inject(Fault { path: host(&format!("/o/{path}")), op, kind: ErrorKind::Io });
        }
        let executed = service.report_execute_on(&vfs, folder.source(), &req, plan, None, &control);
        vfs.clear_faults();
        match executed {
            Ok(outcome) => {
                proptest::prop_assert_eq!(serde_json::to_string(&outcome.plan).unwrap(), planned, "the executed plan is the dry run's");
                let links = outcome.plan.links.as_ref().unwrap();
                let folder_path = links.folder.canonical();
                let html = text_of(&vfs, &host("/o/report.html"));
                let mut linked_files = BTreeSet::new();
                for link in hrefs(&html) {
                    let path = join_href(&host("/o"), &link, std::path::MAIN_SEPARATOR);
                    proptest::prop_assert!(vfs.read_back(&path).is_some(), "{} does not resolve", link);
                    linked_files.insert(path);
                }
                let written: BTreeSet<String> = outcome.pairs.iter().filter(|p| p.status == PairStatus::Written)
                    .map(|p| format!("{folder_path}{}{}", std::path::MAIN_SEPARATOR, p.file.clone().unwrap())).collect();
                proptest::prop_assert_eq!(&linked_files, &written);
                let mut held: BTreeSet<String> = vfs.dump().into_iter().map(|d| d.path)
                    .filter(|p| p.starts_with(&format!("{folder_path}{}", std::path::MAIN_SEPARATOR))).collect();
                let marker = format!("{folder_path}{}.hikasa-report.toml", std::path::MAIN_SEPARATOR);
                proptest::prop_assert!(held.remove(&marker));
                proptest::prop_assert_eq!(held, written);
                for d in &state {
                    if !d.path.starts_with(&host("/o/report")) || d.path.starts_with(&host("/o/report_files-")) {
                        continue;
                    }
                    if matches!(before, Before::Foreign | Before::AFile) && d.path.starts_with(&host("/o/report_files")) {
                        proptest::prop_assert!(vfs.dump().contains(d), "foreign {} changed", d.path);
                    }
                }
                for d in vfs.dump() {
                    proptest::prop_assert!(!d.path.contains(".hikasa-tmp"), "{} stayed", d.path);
                }
                // The old folder stays only with its warning (step 7).
                let old_kept = vfs.dump().iter().any(|d| d.path.contains(".hikasa-old"));
                let warned = outcome.warnings.iter().any(|w| w.code == WarningCode::ReportOldFolderKept);
                proptest::prop_assert_eq!(old_kept, warned);
            }
            Err(_) => {
                proptest::prop_assert_eq!(paths(&vfs.dump()), paths(&state));
                proptest::prop_assert_eq!(vfs.dump(), state, "a failed run changed the back-end");
            }
        }
    }
}

// ---- A7.14 -----------------------------------------------------------------------------------------------------

/// A text comparison of two files below `root`.
fn text_pair(service: &Service, root: &Path) -> TextCompareResponse {
    let left = write(root, "t/left.txt", b"one\ntwo\nthree\n");
    let right = write(root, "t/right.txt", b"one\nTWO\nthree\nfour\n");
    let request = TextCompareRequest {
        left: os(&left),
        right: os(&right),
        ..TextCompareRequest::default()
    };
    service
        .text_compare(&request, &job(service, JobKind::TextCompare), None)
        .unwrap()
}

fn text_request(
    service: &Service,
    layout: &str,
    output: &[&str],
    target: &ReportTarget,
) -> Result<ReportRequest, ApiError> {
    let flags = ReportFlags {
        words: ReportWords {
            layout: Some(layout.to_owned()),
            output_options: output.iter().map(|s| (*s).to_owned()).collect(),
            ..ReportWords::default()
        },
        ..ReportFlags::default()
    };
    service.report_request(ReportKind::Text, "side-by-side", &flags, target)
}

fn text_run(
    service: &Service,
    vfs: &dyn Vfs,
    resp: &TextCompareResponse,
    req: &mut ReportRequest,
    target: &ReportTarget,
    control: &JobControl,
) -> Result<ReportOutcome, ApiError> {
    let plan = service.report_plan_on(vfs, ReportSource::Text(resp), req, target, None, control)?;
    service.report_execute_on(vfs, ReportSource::Text(resp), req, plan, None, control)
}

/// A7.14: for every text layout, a write fault, a commit fault and a cancel leave an existing target byte for byte,
/// with no temporary file; a refused combination is refused before anything is compared.
#[test]
fn a7_14_a_failed_file_report_leaves_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    let target = file_target(&host("/o/r.out"));
    for layout in [
        "unified",
        "side-by-side",
        "summary",
        "interleaved",
        "patch",
        "statistics",
        "xml",
    ] {
        for fault in [Some(VfsOp::Write), Some(VfsOp::Commit), None] {
            let vfs = mem(CaseSensitivity::Sensitive);
            vfs.insert_file(&host("/o/r.out"), &b"before"[..], MemMeta::default())
                .unwrap();
            let before = vfs.dump();
            let token = CancelToken::new();
            if let Some(op) = fault {
                vfs.inject(Fault {
                    path: host("/o/r.out"),
                    op,
                    kind: ErrorKind::Io,
                });
            } else {
                token.cancel();
            }
            let mut req = text_request(&service, layout, &[], &target).unwrap();
            let control = JobControl {
                cancel: token,
                ..job(&service, JobKind::Report)
            };
            let err = text_run(&service, &vfs, &resp, &mut req, &target, &control).unwrap_err();
            if fault.is_none() {
                assert!(matches!(err, ApiError::Cancelled), "{layout}: {err}");
            }
            vfs.clear_faults();
            assert_eq!(vfs.dump(), before, "{layout} {fault:?}");
        }
        // Without a fault: the report replaces the file.
        let vfs = mem(CaseSensitivity::Sensitive);
        vfs.insert_file(&host("/o/r.out"), &b"before"[..], MemMeta::default())
            .unwrap();
        let before_ok = vfs.dump();
        let mut req = text_request(&service, layout, &[], &target).unwrap();
        let outcome = text_run(
            &service,
            &vfs,
            &resp,
            &mut req,
            &target,
            &job(&service, JobKind::Report),
        )
        .unwrap();
        assert!(outcome.plan.replaces);
        assert_eq!(
            vfs.read_back(&host("/o/r.out")).unwrap().len() as u64,
            outcome.bytes,
            "{layout}"
        );
        assert_eq!(
            paths(&vfs.dump()),
            paths(&before_ok),
            "{layout}: no temporary file"
        );
    }
    // Refused before the comparison: an HTML patch; links to stdout and to the clipboard.
    let err = text_request(&service, "patch", &["html-color"], &target).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");
    for target in [ReportTarget::Stdout, ReportTarget::Clipboard] {
        let flags = ReportFlags {
            words: ReportWords {
                options: vec!["include-file-links".into()],
                ..ReportWords::default()
            },
            html: Some(false),
            ..ReportFlags::default()
        };
        let err = service
            .report_request(ReportKind::Folder, "side-by-side", &flags, &target)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported, "{target:?}: {err}");
        assert!(
            err.to_string()
                .contains("file reports need a folder on disk"),
            "{err}"
        );
    }
}

/// A7.14: `html-custom:<css>` reads a stylesheet through the VFS; over 1 MiB is `too_large`; `-` is refused.
#[test]
fn a7_14_custom_stylesheets() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    let css = write(dir.path(), "s.css", b"td { color: teal; }\n");
    let big = write(dir.path(), "big.css", &vec![b' '; (1 << 20) + 1]);
    let vfs = mem(CaseSensitivity::Sensitive);
    let target = file_target(&host("/o/r.html"));
    let word = format!("html-custom:{}", css.display());
    let mut req = text_request(&service, "side-by-side", &[&word], &target).unwrap();
    text_run(
        &service,
        &vfs,
        &resp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    assert!(text_of(&vfs, &host("/o/r.html")).contains("td { color: teal; }"));
    let word = format!("html-custom:{}", big.display());
    let mut req = text_request(&service, "side-by-side", &[&word], &target).unwrap();
    let err = text_run(
        &service,
        &vfs,
        &resp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge, "{err}");
    let mut req = text_request(&service, "side-by-side", &["html-custom:-"], &target).unwrap();
    let err = text_run(
        &service,
        &vfs,
        &resp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
}

/// A7.14 (`cfg(unix)`): a link target is replaced through it (the file it points to gets the report), a FIFO is
/// written directly.
#[cfg(unix)]
#[test]
fn a7_14_a_link_target_and_a_fifo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let service = service(root);
    let resp = text_pair(&service, root);
    let real = write(root, "real.txt", b"old");
    std::os::unix::fs::symlink(&real, root.join("link.txt")).unwrap();
    let target = ReportTarget::File(root.join("link.txt").into_os_string());
    let mut req = text_request(&service, "unified", &[], &target).unwrap();
    text_run(
        &service,
        &*service.vfs(),
        &resp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    assert!(
        std::fs::symlink_metadata(root.join("link.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::read_to_string(&real)
            .unwrap()
            .contains("Text Compare")
    );
    // A FIFO: a reader thread takes what is written.
    let fifo = root.join("fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let reader = {
        let fifo = fifo.clone();
        std::thread::spawn(move || std::fs::read(fifo).unwrap())
    };
    let target = ReportTarget::File(fifo.clone().into_os_string());
    let mut req = text_request(&service, "unified", &[], &target).unwrap();
    let outcome = text_run(
        &service,
        &*service.vfs(),
        &resp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let got = reader.join().unwrap();
    assert_eq!(got.len() as u64, outcome.bytes);
    assert!(std::fs::metadata(&fifo).unwrap().file_type().is_fifo_like());
}

#[cfg(unix)]
trait FifoLike {
    fn is_fifo_like(&self) -> bool;
}

#[cfg(unix)]
impl FifoLike for std::fs::FileType {
    fn is_fifo_like(&self) -> bool {
        use std::os::unix::fs::FileTypeExt;
        self.is_fifo()
    }
}

// ---- A7.15 -----------------------------------------------------------------------------------------------------

/// A7.15: the payload is `{text}` for a layout without HTML and `{html, text}` for an HTML report, the alternative
/// equal to the plain-text rendering of the same request; bytes as the file target's.
#[test]
fn a7_15_clipboard_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    let vfs = mem(CaseSensitivity::Sensitive);
    for (layout, html) in [
        ("unified", false),
        ("patch", false),
        ("statistics", false),
        ("xml", false),
        ("side-by-side", true),
        ("summary", true),
        ("interleaved", true),
        ("side-by-side", false),
    ] {
        let palette: &[&str] = if html { &["html-color"] } else { &[] };
        let mut req = text_request(&service, layout, palette, &ReportTarget::Clipboard).unwrap();
        let outcome = text_run(
            &service,
            &vfs,
            &resp,
            &mut req,
            &ReportTarget::Clipboard,
            &job(&service, JobKind::Report),
        )
        .unwrap();
        let payload = outcome.clipboard.unwrap();
        // The same request to a file.
        let file = file_target(&host(&format!("/o/{layout}-{html}")));
        let mut req = text_request(&service, layout, palette, &file).unwrap();
        text_run(
            &service,
            &vfs,
            &resp,
            &mut req,
            &file,
            &job(&service, JobKind::Report),
        )
        .unwrap();
        let bytes = text_of(&vfs, &host(&format!("/o/{layout}-{html}")));
        if html {
            assert_eq!(payload.html.as_deref(), Some(bytes.as_str()), "{layout}");
            let plain = file_target(&host(&format!("/o/{layout}-plain")));
            let mut req = text_request(&service, layout, &[], &plain).unwrap();
            text_run(
                &service,
                &vfs,
                &resp,
                &mut req,
                &plain,
                &job(&service, JobKind::Report),
            )
            .unwrap();
            assert_eq!(
                payload.text,
                text_of(&vfs, &host(&format!("/o/{layout}-plain"))),
                "{layout}"
            );
        } else {
            assert_eq!(payload.html, None, "{layout}");
            assert_eq!(payload.text, bytes, "{layout}");
        }
    }
    assert_eq!(CLIPBOARD_REPORT_MAX, 128 << 20);
}

// ---- A7.16 -----------------------------------------------------------------------------------------------------

/// A7.16: the printer and the browser write below `<cache>/print/` and hand the main file to the associated
/// application once; two reports in one second get two folders; `NoLauncher` fails naming the file, which stays.
#[test]
fn a7_16_printer_and_browser() {
    let dir = tempfile::tempdir().unwrap();
    let launcher = Arc::new(RecordingLauncher::default());
    let service = service_on(
        dir.path(),
        Arc::clone(&launcher),
        Arc::new(FixedClipboard::empty()),
    );
    let resp = text_pair(&service, dir.path());
    let router = service.vfs();
    let cache = service.config().cache_dir.join("print");
    let mut opened = Vec::new();
    for target in [
        ReportTarget::Printer,
        ReportTarget::Browser,
        ReportTarget::Printer,
    ] {
        let mut req = text_request(&service, "side-by-side", &[], &target).unwrap();
        let outcome = text_run(
            &service,
            &*router,
            &resp,
            &mut req,
            &target,
            &job(&service, JobKind::Report),
        )
        .unwrap();
        let path = outcome.opened.clone().unwrap();
        assert!(path.starts_with(&cache), "{}", path.display());
        assert_eq!(path.file_name().unwrap(), "report.html");
        let html = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            html.contains("@page"),
            target == ReportTarget::Printer,
            "{target:?}"
        );
        opened.push(path);
    }
    let calls = launcher.calls();
    assert_eq!(calls.len(), 3);
    for (call, path) in calls.iter().zip(&opened) {
        assert_eq!(call, &Recorded::Associated(path.clone()));
    }
    let folders: BTreeSet<_> = opened
        .iter()
        .map(|p| p.parent().unwrap().to_path_buf())
        .collect();
    assert_eq!(
        folders.len(),
        3,
        "one folder per report, also in one second: {folders:?}"
    );
    // NoLauncher: the file stays, the message names it.
    let plain = Service::new(ServiceInit {
        config_dir: Some(dir.path().join("config")),
        ..ServiceInit::new(
            facts(dir.path()),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    })
    .unwrap();
    let mut req = text_request(&plain, "side-by-side", &[], &ReportTarget::Browser).unwrap();
    let err = text_run(
        &plain,
        &*plain.vfs(),
        &resp,
        &mut req,
        &ReportTarget::Browser,
        &job(&plain, JobKind::Report),
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(
        message.starts_with("no application could open "),
        "{message}"
    );
    let path = message.trim_start_matches("no application could open ");
    assert!(Path::new(path).is_file(), "{path}");
    // A layout without HTML is refused before anything is compared.
    let err = text_request(&service, "unified", &[], &ReportTarget::Printer).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

/// A7.16: before writing, print folders older than seven days by the clock are removed; newer ones and other names
/// stay.
#[test]
fn a7_16_old_print_folders_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    let cache = service.config().cache_dir.join("print");
    // The clock is 2026-10-21T12:00:00Z.
    for name in [
        "20261013-115959-1",
        "20261014-120001-1",
        "20261001-000000-3",
        "keep-me",
        "20261001-000000",
    ] {
        std::fs::create_dir_all(cache.join(name)).unwrap();
        std::fs::write(cache.join(name).join("report.html"), b"x").unwrap();
    }
    let mut req = text_request(&service, "side-by-side", &[], &ReportTarget::Browser).unwrap();
    text_run(
        &service,
        &*service.vfs(),
        &resp,
        &mut req,
        &ReportTarget::Browser,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let left: BTreeSet<String> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !left.contains("20261013-115959-1") && !left.contains("20261001-000000-3"),
        "{left:?}"
    );
    for kept in [
        "20261014-120001-1",
        "keep-me",
        "20261001-000000",
        "20261021-120000-1",
    ] {
        assert!(left.contains(kept), "{kept} in {left:?}");
    }
}

// ---- A7.19 -----------------------------------------------------------------------------------------------------

/// A7.19: the remembered choices round trip through `state.toml`, sparse, other `[dialogs]` tables kept; a word
/// `ReportRequest::parse` refuses is dropped with one warning.
#[test]
fn a7_19_remembered_choices() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    assert_eq!(
        service.report_choices(ReportKind::Text),
        ReportChoices::default()
    );
    service
        .state_update(&mut |state| state.set_dialog_text("options", "page", "tabs"))
        .unwrap();
    let choices = ReportChoices {
        layout: Some("interleaved".into()),
        options: vec![
            "display-context".into(),
            "line-numbers".into(),
            "strikeout-left-diffs".into(),
        ],
        output_options: vec!["html-color".into(), "wrap-word".into()],
        output_to: Some("file".into()),
        file: Some("/home/u/reports/a.html".into()),
    };
    service
        .remember_report_choices(ReportKind::Text, &choices)
        .unwrap();
    assert_eq!(service.report_choices(ReportKind::Text), choices);
    assert_eq!(
        service.report_choices(ReportKind::Folder),
        ReportChoices::default()
    );
    let sparse = ReportChoices {
        layout: Some("xml".into()),
        ..ReportChoices::default()
    };
    service
        .remember_report_choices(ReportKind::Folder, &sparse)
        .unwrap();
    let state = std::fs::read_to_string(dir.path().join("config/state.toml")).unwrap();
    assert!(
        state.contains("[dialogs.report-folder]") || state.contains("[dialogs.\"report-folder\"]"),
        "{state}"
    );
    assert!(!state.contains("output_to = \"\""), "sparse: {state}");
    assert_eq!(service.report_choices(ReportKind::Folder), sparse);
    assert_eq!(service.report_choices(ReportKind::Text), choices);
    let mut kept = None;
    service
        .state_update(&mut |state| kept = state.dialog_text("options", "page").map(str::to_owned))
        .unwrap();
    assert_eq!(kept.as_deref(), Some("tabs"), "other tables stay");
    // Refused words go, with one warning.
    let _ = service.take_warnings();
    let broken = ReportChoices {
        layout: Some("no-such-layout".into()),
        options: vec![
            "display-all".into(),
            "column-crc".into(),
            "display-none".into(),
            "frobnicate".into(),
        ],
        output_options: vec!["html-mono".into()],
        output_to: Some("fax".into()),
        file: None,
    };
    service
        .remember_report_choices(ReportKind::Text, &broken)
        .unwrap();
    let read = service.report_choices(ReportKind::Text);
    assert_eq!(read.layout, None);
    assert_eq!(read.options, ["display-all"]);
    assert_eq!(read.output_options, ["html-mono"]);
    assert_eq!(read.output_to, None);
    let warnings = service.take_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].code, WarningCode::InvalidConfigValue);
    for word in [
        "no-such-layout",
        "column-crc",
        "display-none",
        "frobnicate",
        "fax",
    ] {
        assert!(
            warnings[0].message.contains(word),
            "{word}: {}",
            warnings[0].message
        );
    }
}

// ---- A7.17 (Service half) ----------------------------------------------------------------------------------------

/// A7.17: one instant for a folder report and its pair reports; `SOURCE_DATE_EPOCH` wins over the clock, and a value
/// that does not parse gives the warning and the clock.
#[test]
fn a7_17_report_time() {
    let dir = tempfile::tempdir().unwrap();
    let with = |epoch: Option<&str>| {
        let mut env = facts(dir.path());
        env.source_date_epoch = epoch.map(str::to_owned);
        Service::new(ServiceInit {
            config_dir: Some(dir.path().join("config")),
            ..ServiceInit::new(env, fixed_clock(), Arc::new(common::UpperRunner::default()))
        })
        .unwrap()
    };
    let clock = fixed_clock();
    let cases: [(&str, Option<&str>); 7] = [
        ("0", Some("1970-01-01T00:00:00+00:00")),
        ("2208988800", Some("2040-01-01T00:00:00+00:00")),
        ("", None),
        ("-1", None),
        ("abc", None),
        ("99999999999999999", None),
        (" 1", None),
    ];
    for (raw, want) in cases {
        let service = with(Some(raw));
        let _ = service.take_warnings();
        let at = service.report_time().unwrap();
        let warnings = service.take_warnings();
        match want {
            Some(want) => {
                assert_eq!(
                    at.timestamp().display_with_offset(at.offset()).to_string(),
                    want,
                    "{raw:?}"
                );
                assert!(warnings.is_empty(), "{raw:?}: {warnings:?}");
            }
            None => {
                assert_eq!(
                    at,
                    hikasa_session::Clock::now(&*clock),
                    "{raw:?}: the clock"
                );
                assert_eq!(warnings.len(), 1, "{raw:?}");
                assert_eq!(warnings[0].code, WarningCode::SourceDateEpochInvalid);
            }
        }
    }
    // One instant for the folder report and its pairs.
    let service = with(Some("1700000000"));
    trees(dir.path(), PAIRS);
    let folder = compare(&service, dir.path());
    let vfs = mem(CaseSensitivity::Sensitive);
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&service, &target);
    run(
        &service,
        &vfs,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let stamp = "2023-11-14T22:13:20+00:00";
    for path in [
        "/o/report.html",
        "/o/report_files/1-b.c.html",
        "/o/report_files/2-a.txt.html",
    ] {
        assert!(text_of(&vfs, &host(path)).contains(stamp), "{path}");
    }
    let _ = OsStr::new("");
}

/// A7.14: an archive member target is one rebuild of the archive; links into an archive are refused.
#[test]
fn a7_14_an_archive_member_target_is_one_rebuild() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let service = service(root);
    let resp = text_pair(&service, root);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("keep.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"kept").unwrap();
    write(root, "a.zip", &zip.finish().unwrap().into_inner());
    let target = ReportTarget::File(root.join("a.zip").join("r.html").into_os_string());
    let mut req = text_request(&service, "side-by-side", &["html-color"], &target).unwrap();
    text_run(
        &service,
        &*service.vfs(),
        &resp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    assert_eq!(service.archive_counters().unwrap().rebuilds, 1);
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(root.join("a.zip")).unwrap()).unwrap();
    assert!(archive.by_name("r.html").is_ok() && archive.by_name("keep.txt").is_ok());
    // Links into an archive: the files folder needs a folder on disk.
    trees(root, PAIRS);
    let folder = compare(&service, root);
    let mut req = linked(&service, &target);
    let err = run(
        &service,
        &*service.vfs(),
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");
    assert!(
        err.to_string()
            .contains("file reports need a folder on disk"),
        "{err}"
    );
    assert_eq!(
        service.archive_counters().unwrap().rebuilds,
        1,
        "nothing written"
    );
}

/// A writer that cancels a token once `at` bytes went through it, counting them.
struct CancellingWriter {
    inner: Box<dyn AtomicWrite>,
    token: CancelToken,
    at: u64,
    seen: Arc<std::sync::atomic::AtomicU64>,
}

impl std::io::Write for CancellingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        let total = self.seen.fetch_add(n as u64, Ordering::SeqCst) + n as u64;
        if total >= self.at {
            self.token.cancel();
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl std::io::Seek for CancellingWriter {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl AtomicWrite for CancellingWriter {
    fn commit(self: Box<Self>) -> Result<(), VfsError> {
        self.inner.commit()
    }
}

/// `MemoryVfs` whose writers cancel the job at the `at`-th byte.
struct CancelAtByte<'a> {
    inner: &'a MemoryVfs,
    token: CancelToken,
    at: u64,
    seen: Arc<std::sync::atomic::AtomicU64>,
}

impl Vfs for CancelAtByte<'_> {
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
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        Ok(Box::new(CancellingWriter {
            inner: self.inner.open_write(spec, opts)?,
            token: self.token.clone(),
            at: self.at,
            seen: Arc::clone(&self.seen),
        }))
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
}

/// §1.7.17's cancel row: a cancel at the n-th MiB of a file output stops with at most one MiB (and the 64 KiB buffer)
/// more written; the target is as before, no temporary file stays.
#[test]
fn a7_14_a_cancel_at_the_nth_mib() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let service = service(root);
    let line = "x".repeat(90);
    let text: String = (0..60_000).map(|i| format!("{i} {line}\n")).collect();
    let left = write(root, "big/l.txt", text.as_bytes());
    let right = write(root, "big/r.txt", text.as_bytes());
    let request = TextCompareRequest {
        left: os(&left),
        right: os(&right),
        ..TextCompareRequest::default()
    };
    let resp = service
        .text_compare(&request, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    let target = file_target(&host("/o/big.html"));
    for n in [1u64, 2, 4] {
        let vfs = mem(CaseSensitivity::Sensitive);
        vfs.insert_file(&host("/o/big.html"), &b"before"[..], MemMeta::default())
            .unwrap();
        let before = vfs.dump();
        let token = CancelToken::new();
        let seen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let wrapper = CancelAtByte {
            inner: &vfs,
            token: token.clone(),
            at: n << 20,
            seen: Arc::clone(&seen),
        };
        let mut req = text_request(&service, "side-by-side", &["html-color"], &target).unwrap();
        let control = JobControl {
            cancel: token,
            ..job(&service, JobKind::Report)
        };
        let err = text_run(&service, &wrapper, &resp, &mut req, &target, &control).unwrap_err();
        assert!(matches!(err, ApiError::Cancelled), "{err}");
        let written = seen.load(Ordering::SeqCst);
        assert!(written >= n << 20, "{written} at {n} MiB");
        assert!(
            written <= (n + 1) * (1 << 20) + (64 << 10),
            "{written} B written after a cancel at {n} MiB"
        );
        assert_eq!(vfs.dump(), before, "{n} MiB");
    }
}

// ---- A7.13 -----------------------------------------------------------------------------------------------------

/// A7.13 on a Linux disk: a file and a folder whose names are not UTF-8 show their lossy names in the side-by-side HTML
/// with links, the XML has `raw-name`, the pair report exists and its link resolves to it by the OS bytes; a main file
/// named with bytes that are not UTF-8 keeps a working back link.
#[cfg(target_os = "linux")]
#[test]
fn a7_13_names_that_are_not_utf8_on_a_linux_disk() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, &[]);
    let folder = OsStr::from_bytes(b"d\xff");
    let file = OsStr::from_bytes(b"f\xfe.txt");
    for (side, body) in [("l", &b"left\n"[..]), ("r", &b"right!\n"[..])] {
        std::fs::create_dir_all(root.join(side).join(folder)).unwrap();
        std::fs::write(root.join(side).join(folder).join(file), body).unwrap();
    }
    let service = service(root);
    let cmp = compare(&service, root);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let main_name = OsStr::from_bytes(b"r\xe9port.html");
    let main = out.join(main_name);
    let target = ReportTarget::File(main.clone().into_os_string());
    let mut req = linked(&service, &target);
    let router = service.vfs();
    let outcome = run(
        &service,
        &*router,
        &cmp,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    assert_eq!(outcome.pairs.len(), 1, "{:?}", outcome.pairs);
    let html = String::from_utf8_lossy(&std::fs::read(&main).unwrap()).into_owned();
    assert!(html.contains("f\u{fffd}.txt"), "the lossy name is shown");
    let links = hrefs(&html);
    assert!(!links.is_empty());
    for link in links {
        let mut path = out.clone().into_os_string().into_vec();
        for segment in hikasa_report::links::href_segments(&link).unwrap() {
            path.push(b'/');
            path.extend(segment);
        }
        let pair = std::path::PathBuf::from(OsString::from_vec(path));
        assert!(pair.is_file(), "{link} → {}", pair.display());
        let back = hrefs(&String::from_utf8_lossy(&std::fs::read(&pair).unwrap()));
        assert_eq!(back.len(), 1);
        let segments = hikasa_report::links::href_segments(&back[0]).unwrap();
        assert_eq!(
            segments,
            [b"..".to_vec(), main_name.as_bytes().to_vec()],
            "the back link keeps the OS bytes"
        );
    }
    // XML: `raw-name` holds the OS bytes of both names.
    let xml_target = ReportTarget::File(out.join("r.xml").into_os_string());
    let flags = ReportFlags {
        words: ReportWords {
            layout: Some("xml".into()),
            options: vec!["display-all".into()],
            ..ReportWords::default()
        },
        ..ReportFlags::default()
    };
    let mut req = service
        .report_request(ReportKind::Folder, "side-by-side", &flags, &xml_target)
        .unwrap();
    run(
        &service,
        &*router,
        &cmp,
        &mut req,
        &xml_target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let xml = std::fs::read_to_string(out.join("r.xml")).unwrap();
    assert!(xml.contains("raw-name=\"64ff\""), "{xml}");
    assert!(xml.contains("raw-name=\"66fe2e747874\""), "{xml}");
}

/// A7.13 on macOS: APFS refuses a name that is not UTF-8 (`EILSEQ`), so no such name reaches a report there.
#[cfg(target_os = "macos")]
#[test]
fn a7_13_apfs_refuses_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let err = std::fs::write(dir.path().join(OsStr::from_bytes(b"f\xfe.txt")), b"x").unwrap_err();
    assert_eq!(err.raw_os_error(), Some(92), "{err}");
}

/// A7.13 on a Windows disk: `C:\…\a b\r#1%.html`, `\\?\C:\…\r.html` and `C:/…/r.html` write the same files, and every
/// href is relative with `/`, no drive and no `\\?\`.
#[cfg(windows)]
#[test]
fn a7_13_windows_spellings_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    let service = service(root);
    let cmp = compare(&service, root);
    std::fs::create_dir_all(root.join("a b")).unwrap();
    let base = root.display().to_string();
    let spellings = [
        (
            format!("{base}\\a b\\r#1%.html"),
            root.join("a b").join("r_1__files"),
        ),
        (format!("\\\\?\\{base}\\r.html"), root.join("r_files")),
        (
            format!("{}/q.html", base.replace('\\', "/")),
            root.join("q_files"),
        ),
    ];
    let router = service.vfs();
    for (spelling, folder) in spellings {
        let target = ReportTarget::File(OsString::from(&spelling));
        let mut req = linked(&service, &target);
        run(
            &service,
            &*router,
            &cmp,
            &mut req,
            &target,
            &job(&service, JobKind::Report),
        )
        .unwrap_or_else(|e| panic!("{spelling}: {e}"));
        assert!(folder.join(".hikasa-report.toml").is_file(), "{spelling}");
        let main = folder.parent().unwrap().join(format!(
            "{}.html",
            folder
                .file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches("_files")
                .replace("r_1_", "r#1%")
        ));
        let html =
            std::fs::read_to_string(&main).unwrap_or_else(|e| panic!("{}: {e}", main.display()));
        for link in hrefs(&html) {
            assert!(
                !link.contains('\\') && !link.contains(':') && !link.starts_with('/'),
                "{link}"
            );
            let path = join_href(&folder.parent().unwrap().display().to_string(), &link, '\\');
            assert!(Path::new(&path).is_file(), "{link} → {path}");
        }
    }
}

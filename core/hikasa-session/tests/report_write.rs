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
            Err(e) => {
                proptest::prop_assert!(fault.is_some(), "no fault was injected, yet the run failed: {}", e);
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
    // Links into an archive: the files folder needs a folder on disk (refused by the request, before the comparison).
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

// ---- W7b review --------------------------------------------------------------------------------------------------

/// A7.16 (review): the printer and the browser with `include-file-links`: the plan is `New` in the fresh print folder
/// without looking below it (it is made at execution), a dry run writes nothing, execution writes the report with its
/// files folder and every link resolves; the launcher gets the main file.
#[test]
fn a7_16_printer_and_browser_with_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    let launcher = Arc::new(RecordingLauncher::default());
    let service = service_on(
        root,
        Arc::clone(&launcher),
        Arc::new(FixedClipboard::empty()),
    );
    let folder = compare(&service, root);
    for target in [ReportTarget::Printer, ReportTarget::Browser] {
        let mut req = linked(&service, &target);
        let control = job(&service, JobKind::Report);
        let plan = service
            .report_plan(folder.source(), &mut req, &target, None, &control)
            .unwrap_or_else(|e| panic!("{target:?}: {e}"));
        let links = plan.links.clone().unwrap();
        assert_eq!(links.state, FilesFolder::New, "{target:?}");
        assert_eq!(links.folder, links.wanted, "{target:?}");
        let print = plan.print_folder.clone().unwrap();
        assert_eq!(links.folder.parent().unwrap(), print, "{target:?}");
        let dry = service.report_dry_run(plan.clone());
        assert_eq!(dry.pairs.len(), 2, "{target:?}");
        assert!(dry.pairs.iter().all(|p| p.status == PairStatus::Planned));
        assert!(
            !Path::new(&print.canonical()).exists(),
            "{target:?}: a dry run makes nothing"
        );
        let outcome = service
            .report_execute(folder.source(), &req, plan, None, &control)
            .unwrap_or_else(|e| panic!("{target:?}: {e}"));
        let opened = outcome.opened.clone().unwrap();
        assert!(
            opened.starts_with(print.canonical()),
            "{}",
            opened.display()
        );
        let html = std::fs::read_to_string(&opened).unwrap();
        let base = opened.parent().unwrap().display().to_string();
        let links = hrefs(&html);
        assert_eq!(links.len(), 4, "{target:?}: both names of two pairs");
        for link in links {
            let path = join_href(&base, &link, std::path::MAIN_SEPARATOR);
            assert!(Path::new(&path).is_file(), "{target:?}: {link} → {path}");
        }
        assert_eq!(
            launcher.calls().last(),
            Some(&Recorded::Associated(opened.clone()))
        );
    }
}

/// A7.16 (review): a print folder another report took between the plan and the execution (the same second) is left
/// alone: the report makes the next `n` (its own, made without parents), and a report that fails removes only that.
#[test]
fn a7_16_a_print_folder_taken_since_the_plan_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    for fail in [false, true] {
        let vfs = mem(CaseSensitivity::Sensitive);
        let mut req = text_request(&service, "side-by-side", &[], &ReportTarget::Browser).unwrap();
        let control = job(&service, JobKind::Report);
        let plan = service
            .report_plan_on(
                &vfs,
                ReportSource::Text(&resp),
                &mut req,
                &ReportTarget::Browser,
                None,
                &control,
            )
            .unwrap();
        let planned = plan.print_folder.clone().unwrap().canonical();
        assert!(planned.ends_with("-1"), "{planned}");
        // Another report of the same second.
        let theirs = format!("{planned}{}report.html", std::path::MAIN_SEPARATOR);
        vfs.insert_file(&theirs, &b"theirs"[..], MemMeta::default())
            .unwrap();
        let next = format!("{}-2", planned.trim_end_matches("-1"));
        let ours = format!("{next}{}report.html", std::path::MAIN_SEPARATOR);
        if fail {
            vfs.inject(Fault {
                path: ours.clone(),
                op: VfsOp::Commit,
                kind: ErrorKind::Io,
            });
        }
        let result =
            service.report_execute_on(&vfs, ReportSource::Text(&resp), &req, plan, None, &control);
        vfs.clear_faults();
        assert_eq!(text_of(&vfs, &theirs), "theirs", "fail {fail}");
        if fail {
            assert!(result.is_err());
            assert!(
                !paths(&vfs.dump()).iter().any(|p| p.starts_with(&next)),
                "the failed report's own folder goes: {:?}",
                paths(&vfs.dump())
            );
        } else {
            let outcome = result.unwrap();
            assert_eq!(outcome.plan.print_folder.unwrap().canonical(), next);
            assert_eq!(outcome.opened.unwrap().display().to_string(), ours);
            assert!(text_of(&vfs, &ours).contains("Text Compare"));
        }
    }
}

/// A7.12 (review, `cfg(unix)`): a main file that is a symbolic link to a file in another folder is followed: the files
/// folder goes beside the file the report is written to, so its hrefs resolve; the plan names that file; the link
/// stays a link.
#[cfg(unix)]
#[test]
fn a7_12_a_main_file_that_is_a_link_is_followed() {
    for absolute in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        trees(root, PAIRS);
        std::fs::create_dir_all(root.join("out")).unwrap();
        let real = write(root, "else/x.html", b"old");
        let to = if absolute {
            real.clone()
        } else {
            std::path::PathBuf::from("../else/x.html")
        };
        std::os::unix::fs::symlink(&to, root.join("out/r.html")).unwrap();
        let service = service(root);
        let folder = compare(&service, root);
        let target = ReportTarget::File(root.join("out/r.html").into_os_string());
        let mut req = linked(&service, &target);
        let control = job(&service, JobKind::Report);
        let plan = service
            .report_plan(folder.source(), &mut req, &target, None, &control)
            .unwrap();
        let main = plan.main.clone().unwrap();
        assert_eq!(
            std::fs::canonicalize(main.canonical()).unwrap(),
            std::fs::canonicalize(&real).unwrap(),
            "absolute {absolute}: the plan names the file"
        );
        let dry = service.report_dry_run(plan.clone()).dry_run_lines();
        assert!(
            dry[0].starts_with("replace ") && dry[0].ends_with("x.html"),
            "{dry:?}"
        );
        service
            .report_execute(folder.source(), &req, plan, None, &control)
            .unwrap();
        assert!(
            std::fs::symlink_metadata(root.join("out/r.html"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!root.join("out/r_files").exists());
        let html = std::fs::read_to_string(&real).unwrap();
        let base = root.join("else").display().to_string();
        let links = hrefs(&html);
        assert_eq!(links.len(), 4);
        for link in links {
            let path = join_href(&base, &link, '/');
            assert!(Path::new(&path).is_file(), "{link} → {path}");
            let pair = std::fs::read_to_string(&path).unwrap();
            let back = hrefs(&pair);
            assert_eq!(
                join_href(
                    &root.join("else/x_files").display().to_string(),
                    &back[0],
                    '/'
                ),
                real.display().to_string()
            );
        }
    }
}

/// A7.12, T12 (review): a folder at the target's name is refused by the plan, so the dry run says what execution
/// does (`is_a_directory`, exit 100), with links or without, through a link to a folder too.
#[test]
fn a7_12_a_folder_target_is_refused_by_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    std::fs::create_dir_all(root.join("out/r.html")).unwrap();
    let service = service(root);
    let folder = compare(&service, root);
    let resp = text_pair(&service, root);
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut targets = vec![root.join("out/r.html"), root.join("out")];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("out"), root.join("dirlink")).unwrap();
        targets.push(root.join("dirlink"));
    }
    for path in targets {
        let target = ReportTarget::File(path.clone().into_os_string());
        let control = job(&service, JobKind::Report);
        let mut req = linked(&service, &target);
        let err = service
            .report_plan(folder.source(), &mut req, &target, None, &control)
            .unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::IsADirectory,
            "{}: {err}",
            path.display()
        );
        let mut req = text_request(&service, "unified", &[], &target).unwrap();
        let err = service
            .report_plan(ReportSource::Text(&resp), &mut req, &target, None, &control)
            .unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::IsADirectory,
            "{}: {err}",
            path.display()
        );
        assert!(err.to_string().contains("is a folder"), "{err}");
    }
}

/// A7.14 (review): links into an archive are refused by `report_request`, before anything is compared, as links to
/// stdout and to the clipboard; a report without links into the archive is not.
#[test]
fn a7_14_links_into_an_archive_are_refused_before_the_comparison() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("keep.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, b"keep").unwrap();
    std::fs::write(root.join("b.zip"), zip.finish().unwrap().into_inner()).unwrap();
    let service = service(root);
    let target = ReportTarget::File(root.join("b.zip").join("r.html").into_os_string());
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
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err}");
    assert!(
        err.to_string()
            .contains("file reports need a folder on disk"),
        "{err}"
    );
    let plain = ReportFlags {
        html: Some(false),
        ..ReportFlags::default()
    };
    service
        .report_request(ReportKind::Folder, "side-by-side", &plain, &target)
        .unwrap();
}

/// A7.18 (review): M1's flags stay silent on a layout that does not use them (`--wrap`, `--patch-style`,
/// `--no-line-numbers` on `unified`, `--columns` on `summary`); the same words given as words still warn.
#[test]
fn a7_18_m1_flags_are_not_reported_unused() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let m1 = ReportFlags {
        wrap: Some("word".into()),
        patch_style: Some("context".into()),
        line_numbers: Some(false),
        ..ReportFlags::default()
    };
    let req = service
        .report_request(ReportKind::Text, "unified", &m1, &ReportTarget::Stdout)
        .unwrap();
    assert!(req.unused.is_empty(), "{:?}", req.unused);
    assert!(req.unused_warning().is_none());
    let folder = ReportFlags {
        columns: Some("size,crc".into()),
        words: ReportWords {
            layout: Some("summary".into()),
            ..ReportWords::default()
        },
        ..ReportFlags::default()
    };
    let req = service
        .report_request(ReportKind::Folder, "list", &folder, &ReportTarget::Stdout)
        .unwrap();
    assert!(req.unused.is_empty(), "{:?}", req.unused);
    // The words themselves: reported, and only they.
    let words = ReportFlags {
        words: ReportWords {
            options: vec!["patch-context".into()],
            output_options: vec!["wrap-word".into()],
            ..ReportWords::default()
        },
        line_numbers: Some(false),
        ..ReportFlags::default()
    };
    let req = service
        .report_request(ReportKind::Text, "unified", &words, &ReportTarget::Stdout)
        .unwrap();
    assert_eq!(req.unused, ["patch-context", "wrap-word"]);
    let warning = req.unused_warning().unwrap();
    assert_eq!(warning.code, WarningCode::ReportOptionUnused);
    assert!(
        !warning.message.contains("line-numbers"),
        "{}",
        warning.message
    );
}

// ---- W7b review: the mutations that survived ---------------------------------------------------------------------

/// A back-end over a `MemoryVfs` with three behaviours of the world outside: a folder that appears right after a look
/// found nothing (`appears`), a writer that cancels a token when flushed (the cancel lands after the last byte), and
/// a writer whose commit lands and then fails (a sync after the rename).
struct Hooked<'a> {
    inner: &'a MemoryVfs,
    appears: Option<String>,
    cancel_on_flush: Option<(String, CancelToken)>,
    commit_then_fail: Option<String>,
}

impl<'a> Hooked<'a> {
    fn new(inner: &'a MemoryVfs) -> Hooked<'a> {
        Hooked {
            inner,
            appears: None,
            cancel_on_flush: None,
            commit_then_fail: None,
        }
    }
}

struct HookedWriter {
    inner: Box<dyn AtomicWrite>,
    cancel: Option<CancelToken>,
    fail_after_commit: Option<String>,
}

impl std::io::Write for HookedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(token) = &self.cancel {
            token.cancel();
        }
        self.inner.flush()
    }
}

impl std::io::Seek for HookedWriter {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl AtomicWrite for HookedWriter {
    fn commit(self: Box<Self>) -> Result<(), VfsError> {
        self.inner.commit()?;
        match self.fail_after_commit {
            Some(path) => Err(VfsError::from_io(
                path,
                VfsOp::Commit,
                std::io::Error::other("the sync after the rename failed"),
            )),
            None => Ok(()),
        }
    }
}

impl Vfs for Hooked<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        let got = self.inner.stat(spec, follow);
        if got.is_err()
            && let Some(path) = &self.appears
            && spec.canonical() == *path
        {
            self.inner.insert_dir(path, MemMeta::default()).unwrap();
        }
        got
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
        let inner = self.inner.open_write(spec, opts)?;
        let path = spec.canonical();
        Ok(Box::new(HookedWriter {
            inner,
            cancel: self
                .cancel_on_flush
                .as_ref()
                .filter(|(p, _)| *p == path)
                .map(|(_, t)| t.clone()),
            fail_after_commit: self
                .commit_then_fail
                .as_ref()
                .filter(|p| **p == path)
                .cloned(),
        }))
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.inner.create_dir(spec, parents)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.inner.remove(spec)
    }
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.inner.rename(from, to, replace)
    }
}

/// A linked report of the `OnMem` fixture on `vfs` with the job `control`.
fn run_with(fx: &OnMem, vfs: &dyn Vfs, control: &JobControl) -> Result<ReportOutcome, ApiError> {
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&fx.service, &target);
    run(&fx.service, vfs, &fx.folder, &mut req, &target, control)
}

/// A7.12 (review, M1 and P1): the old folder changed between the plan and step 4 with as many entries as before (a
/// file of another size, a file under another name) fails "changed since the plan was made"; nothing changes.
#[test]
fn a7_12_a_change_that_keeps_the_count_still_fails_step_4() {
    let fx = on_mem();
    for change in ["another size", "another name"] {
        let vfs = with_earlier(&fx);
        let target = file_target(&host("/o/report.html"));
        let mut req = linked(&fx.service, &target);
        let control = job(&fx.service, JobKind::Report);
        let plan = fx
            .service
            .report_plan_on(&vfs, fx.folder.source(), &mut req, &target, None, &control)
            .unwrap();
        assert_eq!(plan.links.as_ref().unwrap().state, FilesFolder::ReplaceOurs);
        let pair = host("/o/report_files/2-a.txt.html");
        if change == "another size" {
            vfs.insert_file(&pair, &b"shorter"[..], MemMeta::default())
                .unwrap();
        } else {
            let bytes = vfs.read_back(&pair).unwrap();
            let folder_spec = plan.links.as_ref().unwrap().folder.clone();
            let spec = vfs
                .child(&folder_spec, &FileName::from_os(OsStr::new("2-a.txt.html")))
                .unwrap();
            vfs.remove(&spec).unwrap();
            vfs.insert_file(
                &host("/o/report_files/2-b.txt.html"),
                bytes.to_vec(),
                MemMeta::default(),
            )
            .unwrap();
        }
        let before = vfs.dump();
        let err = fx
            .service
            .report_execute_on(&vfs, fx.folder.source(), &req, plan, None, &control)
            .unwrap_err();
        assert!(
            err.to_string().contains("changed since the plan was made"),
            "{change}: {err}"
        );
        assert_eq!(vfs.dump(), before, "{change}");
    }
    // Without a change the same plan runs (P1: the comparison is not inverted).
    let vfs = with_earlier(&fx);
    let outcome = fx.run(&vfs, "/o/report.html").unwrap();
    assert_eq!(outcome.plan.links.unwrap().state, FilesFolder::ReplaceOurs);
}

/// A7.12 (review, M3, M4, M16, M24): folders that are not ours by the marker alone: another report's folder with
/// every file as listed, a marker that names the report in another case on a case-sensitive back-end, a marker that
/// lists a file that is gone, a marker that is a link to a good marker elsewhere.
#[test]
fn a7_12_markers_that_do_not_make_a_folder_ours() {
    let fx = on_mem();
    let ours = mem(CaseSensitivity::Sensitive);
    fx.run(&ours, "/o/report.html").unwrap();
    let marker = text_of(&ours, &host("/o/report_files/.hikasa-report.toml"));
    let pair_b = ours.read_back(&host("/o/report_files/1-b.c.html")).unwrap();
    let pair_a = ours
        .read_back(&host("/o/report_files/2-a.txt.html"))
        .unwrap();
    let both = |v: &MemoryVfs| {
        v.insert_file(
            &host("/o/report_files/1-b.c.html"),
            pair_b.to_vec(),
            MemMeta::default(),
        )
        .unwrap();
        v.insert_file(
            &host("/o/report_files/2-a.txt.html"),
            pair_a.to_vec(),
            MemMeta::default(),
        )
        .unwrap();
    };
    let cases: Vec<(&str, Box<dyn Fn(&MemoryVfs)>)> = vec![
        (
            "another report's folder, every file as listed",
            Box::new(|v: &MemoryVfs| {
                let other = marker.replace("report = \"report.html\"", "report = \"other.html\"");
                assert_ne!(other, marker);
                v.insert_file(
                    &host("/o/report_files/.hikasa-report.toml"),
                    other.into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                both(v);
            }),
        ),
        (
            "the report's name in another case",
            Box::new(|v: &MemoryVfs| {
                let other = marker.replace("report = \"report.html\"", "report = \"REPORT.html\"");
                v.insert_file(
                    &host("/o/report_files/.hikasa-report.toml"),
                    other.into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                both(v);
            }),
        ),
        (
            "a listed file that is gone",
            Box::new(|v: &MemoryVfs| {
                v.insert_file(
                    &host("/o/report_files/.hikasa-report.toml"),
                    marker.clone().into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_file(
                    &host("/o/report_files/1-b.c.html"),
                    pair_b.to_vec(),
                    MemMeta::default(),
                )
                .unwrap();
            }),
        ),
        (
            "a marker that is a link",
            Box::new(|v: &MemoryVfs| {
                v.insert_file(
                    &host("/o/elsewhere.toml"),
                    marker.clone().into_bytes(),
                    MemMeta::default(),
                )
                .unwrap();
                v.insert_dir(&host("/o/report_files"), MemMeta::default())
                    .unwrap();
                v.insert_symlink(
                    &host("/o/report_files/.hikasa-report.toml"),
                    &host("/o/elsewhere.toml"),
                    MemMeta::default(),
                )
                .unwrap();
                both(v);
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
        let after: Vec<MemDump> = vfs
            .dump()
            .into_iter()
            .filter(|d| d.path.contains("report_files") && !d.path.contains("report_files-2"))
            .collect();
        assert_eq!(before, after, "{what}: the folder is as it was");
    }
}

/// A7.12 (review, M2, `cfg(unix)` disk): an earlier report's pair file with its size but another modification time
/// makes the folder not ours.
#[cfg(unix)]
#[test]
fn a7_12_a_file_with_another_time_is_not_ours() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    std::fs::create_dir_all(root.join("out")).unwrap();
    let service = service(root);
    let folder = compare(&service, root);
    let target = ReportTarget::File(root.join("out/report.html").into_os_string());
    let router = service.vfs();
    let mut req = linked(&service, &target);
    run(
        &service,
        &*router,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let pair = root.join("out/report_files/2-a.txt.html");
    let modified = std::fs::metadata(&pair).unwrap().modified().unwrap();
    std::fs::File::options()
        .write(true)
        .open(&pair)
        .unwrap()
        .set_modified(modified - std::time::Duration::from_secs(10))
        .unwrap();
    let mut req = linked(&service, &target);
    let outcome = run(
        &service,
        &*router,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let links = outcome.plan.links.as_ref().unwrap();
    assert_eq!(links.state, FilesFolder::New);
    assert!(
        links.folder.canonical().ends_with("report_files-2"),
        "{}",
        links.folder.canonical()
    );
    assert_eq!(
        std::fs::metadata(&pair).unwrap().modified().unwrap(),
        modified - std::time::Duration::from_secs(10)
    );
}

/// A7.12 (review, M25, Linux disk): a file whose name is not UTF-8 makes a folder not ours, even when a marker lists
/// its lossy spelling with its size and time; the foreign file stays where it is.
#[cfg(target_os = "linux")]
#[test]
fn a7_12_a_name_that_is_not_utf8_is_never_ours() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    std::fs::create_dir_all(root.join("out")).unwrap();
    let service = service(root);
    let folder = compare(&service, root);
    let target = ReportTarget::File(root.join("out/report.html").into_os_string());
    let router = service.vfs();
    let mut req = linked(&service, &target);
    run(
        &service,
        &*router,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let files = root.join("out/report_files");
    let foreign = files.join(OsStr::from_bytes(b"x\xff.html"));
    std::fs::write(&foreign, b"theirs").unwrap();
    let meta = std::fs::metadata(&foreign).unwrap();
    let nanos = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let marker = files.join(".hikasa-report.toml");
    let mut text = std::fs::read_to_string(&marker).unwrap();
    text.push_str(&format!(
        "\n[[file]]\nname = \"x\u{fffd}.html\"\nsize = {}\nmodified = {nanos}\n",
        meta.len()
    ));
    std::fs::write(&marker, text).unwrap();
    let mut req = linked(&service, &target);
    let outcome = run(
        &service,
        &*router,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let links = outcome.plan.links.as_ref().unwrap();
    assert!(
        links.folder.canonical().ends_with("report_files-2"),
        "{}",
        links.folder.canonical()
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), b"theirs");
}

/// A7.12 (review, M5, N8): a folder that appears at a name the run looked at and found free is never replaced: at the
/// files folder's name before step 5 (a `New` plan), at the hidden old folder's name before step 4's rename. The run
/// fails and the back-end is as before, the folder that appeared included.
#[test]
fn a7_12_a_folder_that_appears_is_never_replaced() {
    let fx = on_mem();
    // Step 5: the plan is `New`; an empty folder appears at report_files.
    let vfs = mem(CaseSensitivity::Sensitive);
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&fx.service, &target);
    let control = job(&fx.service, JobKind::Report);
    let plan = fx
        .service
        .report_plan_on(&vfs, fx.folder.source(), &mut req, &target, None, &control)
        .unwrap();
    assert_eq!(plan.links.as_ref().unwrap().state, FilesFolder::New);
    vfs.insert_dir(&host("/o/report_files"), MemMeta::default())
        .unwrap();
    let before = vfs.dump();
    let result = fx
        .service
        .report_execute_on(&vfs, fx.folder.source(), &req, plan, None, &control);
    assert!(result.is_err(), "{result:?}");
    assert_eq!(vfs.dump(), before);
    // Step 4: `.report_files.hikasa-old` appears right after the look that found it free.
    let vfs = with_earlier(&fx);
    let before = vfs.dump();
    let mut hooked = Hooked::new(&vfs);
    hooked.appears = Some(host("/o/.report_files.hikasa-old"));
    let result = run_with(&fx, &hooked, &job(&fx.service, JobKind::Report));
    assert!(result.is_err(), "{result:?}");
    let mut after = vfs.dump();
    after.retain(|d| d.path != host("/o/.report_files.hikasa-old"));
    assert_eq!(after, before, "only the folder that appeared is new");
    assert!(
        vfs.dump()
            .iter()
            .any(|d| d.path == host("/o/.report_files.hikasa-old")),
        "and it stays"
    );
}

/// A7.12 (review, M9): the hidden folders a crash left (`.report_files.hikasa-tmp`, `.report_files.hikasa-old`) are
/// not touched: the run picks other hidden names and leaves them byte for byte.
#[test]
fn a7_12_hidden_folders_a_crash_left_are_not_touched() {
    let fx = on_mem();
    let vfs = with_earlier(&fx);
    for leftover in [
        "/o/.report_files.hikasa-tmp/1-b.c.html",
        "/o/.report_files.hikasa-old/.hikasa-report.toml",
    ] {
        vfs.insert_file(&host(leftover), &b"from a crash"[..], MemMeta::default())
            .unwrap();
    }
    let leftovers = |vfs: &MemoryVfs| -> Vec<MemDump> {
        vfs.dump()
            .into_iter()
            .filter(|d| {
                d.path.ends_with(".hikasa-tmp")
                    || d.path.ends_with(".hikasa-old")
                    || d.path.contains(".hikasa-tmp/")
                    || d.path.contains(".hikasa-old/")
                    || d.path
                        .contains(&format!(".hikasa-tmp{}", std::path::MAIN_SEPARATOR))
                    || d.path
                        .contains(&format!(".hikasa-old{}", std::path::MAIN_SEPARATOR))
            })
            .collect()
    };
    let before = leftovers(&vfs);
    assert_eq!(before.len(), 4, "{before:?}");
    let outcome = fx.run(&vfs, "/o/report.html").unwrap();
    assert_eq!(outcome.plan.links.unwrap().state, FilesFolder::ReplaceOurs);
    assert_eq!(leftovers(&vfs), before);
    assert!(
        !vfs.dump()
            .iter()
            .any(|d| d.path.contains("hikasa-tmp-2") || d.path.contains("hikasa-old-2")),
        "this run's own hidden folders are gone"
    );
}

/// A7.12 (review, M11): a pair file whose commit lands and then fails is `failed`, and the file that landed is
/// removed: the files folder holds exactly the marker and the pairs it lists.
#[test]
fn a7_12_a_pair_that_landed_and_failed_is_removed() {
    let fx = on_mem();
    let vfs = mem(CaseSensitivity::Sensitive);
    let mut hooked = Hooked::new(&vfs);
    hooked.commit_then_fail = Some(host("/o/.report_files.hikasa-tmp/2-a.txt.html"));
    let outcome = run_with(&fx, &hooked, &job(&fx.service, JobKind::Report)).unwrap();
    let failed: Vec<&str> = outcome
        .pairs
        .iter()
        .filter(|p| p.status == PairStatus::Failed)
        .map(|p| p.path.as_str())
        .collect();
    assert_eq!(failed, ["a.txt"]);
    let held: BTreeSet<String> = paths(&vfs.dump())
        .into_iter()
        .filter(|p| p.starts_with(&host("/o/report_files/")))
        .collect();
    let want: BTreeSet<String> = [
        "/o/report_files/.hikasa-report.toml",
        "/o/report_files/1-b.c.html",
    ]
    .iter()
    .map(|p| host(p))
    .collect();
    assert_eq!(held, want);
}

/// A7.12 (review, P5): a cancel that lands after the main report's last byte (its writer's flush) is seen before
/// step 4: nothing changes. And (N4) the same for a file report without links: the target keeps what it had.
#[test]
fn a7_12_a_cancel_after_the_last_byte_is_seen_before_step_4() {
    let fx = on_mem();
    let vfs = with_earlier(&fx);
    let before = vfs.dump();
    let token = CancelToken::new();
    let mut hooked = Hooked::new(&vfs);
    hooked.cancel_on_flush = Some((host("/o/report.html"), token.clone()));
    let control = JobControl {
        cancel: token,
        ..job(&fx.service, JobKind::Report)
    };
    let result = run_with(&fx, &hooked, &control);
    assert!(matches!(result, Err(ApiError::Cancelled)), "{result:?}");
    assert_eq!(vfs.dump(), before);
    no_hidden(&vfs);
    // N4: a text report into an existing file.
    let dir = tempfile::tempdir().unwrap();
    let resp = text_pair(&fx.service, dir.path());
    let vfs = mem(CaseSensitivity::Sensitive);
    vfs.insert_file(&host("/o/r.out"), &b"before"[..], MemMeta::default())
        .unwrap();
    let before = vfs.dump();
    let token = CancelToken::new();
    let mut hooked = Hooked::new(&vfs);
    hooked.cancel_on_flush = Some((host("/o/r.out"), token.clone()));
    let control = JobControl {
        cancel: token,
        ..job(&fx.service, JobKind::Report)
    };
    let target = file_target(&host("/o/r.out"));
    let mut req = text_request(&fx.service, "unified", &[], &target).unwrap();
    let result = text_run(&fx.service, &hooked, &resp, &mut req, &target, &control);
    assert!(matches!(result, Err(ApiError::Cancelled)), "{result:?}");
    assert_eq!(vfs.dump(), before);
}

/// A7.12 (review, M20): a cancel between pairs stops the next pair before it is compared or written (one thread, so
/// the order is fixed): no pair file is opened after the cancel.
#[test]
fn a7_12_a_cancel_between_pairs_stops_the_next_pair() {
    let dir = tempfile::tempdir().unwrap();
    trees(dir.path(), PAIRS);
    let init = ServiceInit {
        config_dir: Some(dir.path().join("config")),
        threads: std::num::NonZeroUsize::new(1),
        ..ServiceInit::new(
            facts(dir.path()),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    let folder = compare(&service, dir.path());
    let vfs = mem(CaseSensitivity::Sensitive);
    vfs.set_journal(true);
    vfs.set_journal_writes(true);
    let token = CancelToken::new();
    // The first pair's writer cancels when flushed: the second pair must not start.
    let mut hooked = Hooked::new(&vfs);
    hooked.cancel_on_flush = Some((
        host("/o/.report_files.hikasa-tmp/1-b.c.html"),
        token.clone(),
    ));
    let control = JobControl {
        cancel: token,
        ..job(&service, JobKind::Report)
    };
    let target = file_target(&host("/o/report.html"));
    let mut req = linked(&service, &target);
    let result = run(&service, &hooked, &folder, &mut req, &target, &control);
    assert!(matches!(result, Err(ApiError::Cancelled)), "{result:?}");
    let opened: Vec<String> = vfs
        .journal()
        .into_iter()
        .filter(|(op, path)| *op == VfsOp::Write && path.contains("2-a.txt"))
        .map(|(_, path)| path)
        .collect();
    assert!(opened.is_empty(), "the second pair was written: {opened:?}");
    no_hidden(&vfs);
}

/// A7.12 (review, M12): pruning keeps a print folder exactly seven days old by the clock.
#[test]
fn a7_16_a_print_folder_exactly_seven_days_old_stays() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    let cache = service.config().cache_dir.join("print");
    // The clock is 2026-10-21T12:00:00Z.
    for name in ["20261014-120000-1", "20261014-115959-1"] {
        std::fs::create_dir_all(cache.join(name)).unwrap();
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
    assert!(
        cache.join("20261014-120000-1").exists(),
        "seven days to the second stays"
    );
    assert!(!cache.join("20261014-115959-1").exists());
}

/// A clock that moves one second at each reading.
struct TickingClock(std::sync::atomic::AtomicI64);

impl hikasa_session::Clock for TickingClock {
    fn now(&self) -> jiff::Zoned {
        let s = self.0.fetch_add(1, Ordering::SeqCst);
        jiff::Timestamp::from_second(s)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
    }
}

/// A7.17 (review, N9): one instant for a folder report and its pair reports, also when the clock moves between them
/// (no `SOURCE_DATE_EPOCH`).
#[test]
fn a7_17_one_instant_when_the_clock_moves() {
    let dir = tempfile::tempdir().unwrap();
    trees(dir.path(), PAIRS);
    let init = ServiceInit {
        config_dir: Some(dir.path().join("config")),
        ..ServiceInit::new(
            facts(dir.path()),
            Arc::new(TickingClock(std::sync::atomic::AtomicI64::new(
                1_792_584_000,
            ))),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
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
    let stamps = |path: &str| -> BTreeSet<String> {
        let text = text_of(&vfs, &host(path));
        text.match_indices("2026-10-")
            .filter_map(|(at, _)| text.get(at..at + 19))
            .map(str::to_owned)
            .collect()
    };
    let main = stamps("/o/report.html");
    assert_eq!(main.len(), 1, "{main:?}");
    for pair in ["/o/report_files/1-b.c.html", "/o/report_files/2-a.txt.html"] {
        assert_eq!(stamps(pair), main, "{pair}");
    }
}

/// A7.15 (review, M6, M7, M8): the clipboard's cap holds the text and the HTML flavours together; a report over it is
/// `too_large` with nothing returned; a report of exactly the cap fits (the cap lowered through the test hook).
#[test]
fn a7_15_the_clipboard_cap_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let resp = text_pair(&service, dir.path());
    let vfs = mem(CaseSensitivity::Sensitive);
    let clip = |service: &Service, layout: &str, palette: &[&str]| {
        let mut req = text_request(service, layout, palette, &ReportTarget::Clipboard).unwrap();
        text_run(
            service,
            &vfs,
            &resp,
            &mut req,
            &ReportTarget::Clipboard,
            &job(service, JobKind::Report),
        )
    };
    let full = clip(&service, "side-by-side", &["html-color"])
        .unwrap()
        .clipboard
        .unwrap();
    let (html, text) = (
        full.html.clone().unwrap().len() as u64,
        full.text.len() as u64,
    );
    assert!(html > text && text > 0);
    // Both flavours fit exactly; one byte less is too large, also when the HTML alone fits.
    service.report_hooks().set_clipboard_cap(html + text);
    assert_eq!(
        clip(&service, "side-by-side", &["html-color"])
            .unwrap()
            .clipboard
            .unwrap(),
        full
    );
    service.report_hooks().set_clipboard_cap(html + text - 1);
    let err = clip(&service, "side-by-side", &["html-color"]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge, "{err}");
    service.report_hooks().set_clipboard_cap(html - 1);
    let err = clip(&service, "side-by-side", &["html-color"]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge, "{err}");
    // A text report of exactly the cap fits.
    service.report_hooks().set_clipboard_cap(0);
    let plain = clip(&service, "unified", &[]).unwrap().clipboard.unwrap();
    service
        .report_hooks()
        .set_clipboard_cap(plain.text.len() as u64);
    assert_eq!(
        clip(&service, "unified", &[]).unwrap().clipboard.unwrap(),
        plain
    );
    service.report_hooks().set_clipboard_cap(0);
    assert_eq!(service.report_hooks().clipboard_cap(), CLIPBOARD_REPORT_MAX);
}

/// A7.12 (review): the pair comparisons stay within the content budget: with a budget of 1 MiB and pairs of 600 KiB
/// and a byte each, one comparison holds its share at a time, and never more than the budget.
#[test]
fn a7_12_pair_comparisons_stay_within_the_content_budget() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let big = |c: u8| vec![c; 300 << 10];
    let files: Vec<(String, Vec<u8>, Vec<u8>)> = (0..3)
        .map(|i| {
            (format!("f{i}.txt"), big(b'a'), {
                let mut r = big(b'a');
                r.push(b'b');
                r
            })
        })
        .collect();
    let borrowed: Vec<(&str, Option<&[u8]>, Option<&[u8]>)> = files
        .iter()
        .map(|(n, l, r)| (n.as_str(), Some(l.as_slice()), Some(r.as_slice())))
        .collect();
    trees(root, &borrowed);
    write(
        &root.join("config"),
        "options.toml",
        b"schema_version = 1\n[tweaks.file_operations]\ncontent_memory_budget_mib = 1\n",
    );
    let service = service(root);
    assert_eq!(
        service.options().content_budget_bytes(),
        1 << 20,
        "the options file is read"
    );
    let folder = compare(&service, root);
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
    assert_eq!(outcome.pairs.len(), 3);
    let (capacity, peak) = service.report_hooks().budget();
    assert_eq!(capacity, 1 << 20);
    assert_eq!(
        peak,
        (600 << 10) + 1,
        "a pair holds its two sides, one pair at a time"
    );
    assert!(peak <= capacity);
    assert_eq!(service.report_hooks().pairs_at_once(), 1);
}

/// A7.19 (review): `state.toml` changed between the load and the save of `remember_report_choices` goes through
/// `state_update`'s retry: both the other change and the choices are kept.
#[test]
fn a7_19_a_change_between_load_and_save_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    service
        .state_update(&mut |state| state.set_dialog_text("options", "page", "tabs"))
        .unwrap();
    let path = dir.path().join("config/state.toml");
    let other = service.clone();
    service
        .report_hooks()
        .before_choices_save(Box::new(move || {
            other
                .state_update(&mut |state| {
                    state.set_dialog_text("options", "page", "colours, a longer value")
                })
                .unwrap();
        }));
    let choices = ReportChoices {
        layout: Some("interleaved".into()),
        ..ReportChoices::default()
    };
    service
        .remember_report_choices(ReportKind::Text, &choices)
        .unwrap();
    assert_eq!(service.report_choices(ReportKind::Text), choices);
    let mut kept = None;
    service
        .state_update(&mut |state| kept = state.dialog_text("options", "page").map(str::to_owned))
        .unwrap();
    assert_eq!(
        kept.as_deref(),
        Some("colours, a longer value"),
        "{}",
        std::fs::read_to_string(&path).unwrap()
    );
}

/// A7.12 (review, N10): `write_output_with` syncs a replaced file (`FileOnly`) and not a new one (`None`).
#[test]
fn a7_14_write_output_with_durabilities() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let vfs = mem(CaseSensitivity::Sensitive);
    vfs.insert_file(&host("/o/old.txt"), &b"before"[..], MemMeta::default())
        .unwrap();
    let control = job(&service, JobKind::Report);
    for name in ["/o/new.txt", "/o/old.txt"] {
        service
            .write_output_with_on(&vfs, OsStr::new(&host(name)), &control, &mut |out| {
                out.write_all(b"report")
                    .map_err(|e| ApiError::Internal(e.to_string()))
            })
            .unwrap();
        assert_eq!(text_of(&vfs, &host(name)), "report");
    }
    assert_eq!(
        vfs.durabilities(),
        [
            (host("/o/new.txt"), Durability::None),
            (host("/o/old.txt"), Durability::FileOnly)
        ]
    );
}

/// A7.18 (review, N6): `--lines` is refused for a folder report by name, before anything is compared.
#[test]
fn a7_18_lines_are_refused_for_a_folder_report_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let flags = ReportFlags {
        lines: Some("1-2".into()),
        ..ReportFlags::default()
    };
    let err = service
        .report_request(
            ReportKind::Folder,
            "side-by-side",
            &flags,
            &ReportTarget::Stdout,
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(err.to_string().contains("--lines"), "{err}");
}

/// A7.12 (review, case-insensitive disks: macOS and Windows): an existing `REPORT_FILES` that is not ours makes
/// `_files-2` and stays as it is; a report of `REPORT.html` is ours for `report.html`.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn a7_12_case_insensitive_names_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    trees(root, PAIRS);
    write(&root.join("out"), "REPORT_FILES/theirs.txt", b"theirs");
    let service = service(root);
    let folder = compare(&service, root);
    let router = service.vfs();
    let at = |name: &str| ReportTarget::File(root.join("out").join(name).into_os_string());
    let target = at("report.html");
    let mut req = linked(&service, &target);
    let outcome = run(
        &service,
        &*router,
        &folder,
        &mut req,
        &target,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let links = outcome.plan.links.as_ref().unwrap();
    assert!(
        links.folder.canonical().ends_with("report_files-2"),
        "{}",
        links.folder.canonical()
    );
    assert_eq!(
        std::fs::read(root.join("out/REPORT_FILES/theirs.txt")).unwrap(),
        b"theirs"
    );
    let out2 = root.join("out2");
    std::fs::create_dir_all(&out2).unwrap();
    let upper = ReportTarget::File(out2.join("REPORT.html").into_os_string());
    let mut req = linked(&service, &upper);
    run(
        &service,
        &*router,
        &folder,
        &mut req,
        &upper,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    let lower = ReportTarget::File(out2.join("report.html").into_os_string());
    let mut req = linked(&service, &lower);
    let outcome = run(
        &service,
        &*router,
        &folder,
        &mut req,
        &lower,
        &job(&service, JobKind::Report),
    )
    .unwrap();
    assert_eq!(
        outcome.plan.links.as_ref().unwrap().state,
        FilesFolder::ReplaceOurs,
        "{:?}",
        outcome.warnings
    );
}

/// A7.12 (review): pairs that cannot share the budget never hang the run, however the pool's threads interleave with
/// the comparisons' own parallel work: eight pairs of 600 KiB under 1 MiB on three threads, twelve times, each within a
/// minute (a watchdog fails the test instead of hanging it). The budget is taken off the pool (`run_pairs`).
#[test]
fn a7_12_pairs_under_a_tight_budget_never_hang() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let files: Vec<(String, Vec<u8>, Vec<u8>)> = (0..8)
        .map(|i| {
            let left = vec![b'a'; 300 << 10];
            let mut right = left.clone();
            right.push(b'b');
            (format!("f{i}.txt"), left, right)
        })
        .collect();
    let borrowed: Vec<(&str, Option<&[u8]>, Option<&[u8]>)> = files
        .iter()
        .map(|(n, l, r)| (n.as_str(), Some(l.as_slice()), Some(r.as_slice())))
        .collect();
    trees(&root, &borrowed);
    write(
        &root.join("config"),
        "options.toml",
        b"schema_version = 1\n[tweaks.file_operations]\ncontent_memory_budget_mib = 1\n",
    );
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let service = service(&root);
        let folder = compare(&service, &root);
        for _ in 0..12 {
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
            assert_eq!(outcome.pairs.len(), 8);
            assert_eq!(service.report_hooks().pairs_at_once(), 1);
            done.send(()).unwrap();
        }
    });
    for round in 0..12 {
        finished
            .recv_timeout(std::time::Duration::from_secs(60))
            .unwrap_or_else(|e| panic!("round {round} did not end: {e}"));
    }
}

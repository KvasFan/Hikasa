//! M2 W10c acceptance A10.26, its Service part (TC-052, D-288; contract §1.10.21) on disk: the child's row found by its
//! `ParentLink` in an open parent; with the parent closed, the parent session (named, auto-saved, folder-sync) compared
//! again in a job and the row found in it; `text_copy_file_steps` (the source's edits saved first, the target's lost,
//! W6's copy plan equal to `folder op copy --to <other> --path <row>`'s, the next pair, a read-only target side refused)
//! and `text_copy_file_execute` (save, copy, refresh, next; a step that asks what no one answers stops the chain).

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{job, os, service, write};
use hikasa_core::{ErrorKind, JobKind, Pair, RelPath, Side};
use hikasa_folder::ops::{ConfirmMode, FolderOp, QuestionKind, ReadOnlySides};
use hikasa_folder::select::SideSet;
use hikasa_folder::view::NextFileOptions;
use hikasa_folder::{
    Category, DisplayOptions, FolderDisplayFilter, FolderMode, NodeId, NullObserver, TreeRead,
};
use hikasa_session::api::{
    CopyFileExecute, CopyFileRequest, CopyFileStep, FolderCompareRequest, FolderOpRequest,
    LiveOptions, LiveTextView, OpOverrides, OpRequest, PreparedFolderCompare, SelectRequest,
    TextCompareRequest, parent_link_of, parent_row,
};
use hikasa_session::sessions::{ParentLink, Session, SessionRef, SessionRules, SessionType};
use hikasa_session::{ApiError, Service};
use hikasa_text::{EditRequest, EditableDocument, GroupHint, TextPos};
use hikasa_vfs::ops::OverwritePolicy;

// ---- helpers -------------------------------------------------------------------------------------------------------

/// `root/l` and `root/r`: pre-order `sub/`, `sub/b.txt` (different), `a.txt` (different), `c.txt` (same), `d.txt`
/// (different), `e.txt` (left only).
fn tree(root: &Path) -> (PathBuf, PathBuf) {
    let l = root.join("l");
    let r = root.join("r");
    for (side, a, b, d) in [
        (&l, "left a\n", "b1\n", "d left\n"),
        (&r, "right a\n", "b22\n", "d right\n"),
    ] {
        write(side, "a.txt", a.as_bytes());
        write(side, "sub/b.txt", b.as_bytes());
        write(side, "c.txt", b"same\n");
        write(side, "d.txt", d.as_bytes());
    }
    write(&l, "e.txt", b"only\n");
    (l, r)
}

fn folder_request(l: &Path, r: &Path) -> FolderCompareRequest {
    FolderCompareRequest {
        left: os(l),
        right: os(r),
        ..FolderCompareRequest::default()
    }
}

fn compared(service: &Service, l: &Path, r: &Path) -> PreparedFolderCompare {
    let control = job(service, JobKind::FolderCompare);
    let prepared = service
        .folder_prepare_in(&folder_request(l, r), &control)
        .unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    prepared
}

fn row(read: &TreeRead<'_>, path: &str) -> NodeId {
    read.node_at(&RelPath::parse(path).unwrap()).unwrap()
}

fn path_of(read: &TreeRead<'_>, node: Option<NodeId>) -> Option<String> {
    node.map(|n| read.path(n).as_str().to_owned())
}

fn link(session: &str, path: &str) -> ParentLink {
    ParentLink {
        session: session.to_owned(),
        left: path.to_owned(),
        right: path.to_owned(),
    }
}

fn live(service: &Service, l: &Path, r: &Path, opts: LiveOptions) -> LiveTextView {
    let req = TextCompareRequest {
        left: os(l),
        right: os(r),
        ..TextCompareRequest::default()
    };
    service
        .text_compare_live(&req, opts, &job(service, JobKind::TextCompare), None)
        .unwrap()
}

fn type_x(doc: &mut EditableDocument) {
    let at = TextPos::new(0, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: "X".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

fn next_options() -> NextFileOptions {
    NextFileOptions {
        limit_to_folder: false,
        wrap: false,
        display: DisplayOptions {
            filter: FolderDisplayFilter::All,
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
        },
        ignore_unimportant: false,
    }
}

fn copy_request<'a>(
    parent: &'a PreparedFolderCompare,
    row: NodeId,
    from: Side,
    view: &'a LiveTextView,
    read_only: ReadOnlySides,
) -> CopyFileRequest<'a> {
    CopyFileRequest {
        parent,
        row,
        from,
        docs: Pair::new(&view.docs.left, &view.docs.right),
        read_only,
        next: next_options(),
        save: None,
    }
}

fn execute<'a>(
    parent: &'a PreparedFolderCompare,
    row: NodeId,
    mode: ConfirmMode,
) -> CopyFileExecute<'a> {
    CopyFileExecute {
        parent,
        row,
        from: Side::Left,
        read_only: ReadOnlySides::default(),
        next: next_options(),
        copy_mode: mode,
    }
}

fn folder_session(l: &Path, r: &Path, kind: SessionType) -> Session {
    let mut session = Session::builtin(kind);
    session.specs.left = l.display().to_string();
    session.specs.right = r.display().to_string();
    session
}

// ---- the parent's row ------------------------------------------------------------------------------------------------

#[test]
fn a10_26_the_childs_row_is_found_by_its_parent_link() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    let parent = compared(&service, &l, &r);
    let read = parent.model().read();
    for path in ["sub/b.txt", "a.txt", "c.txt", "e.txt"] {
        let found = parent_row(&read, &link("p", path)).unwrap();
        assert_eq!(read.path(found).as_str(), path);
        // The link a child of that row stores names it again.
        let back = parent_link_of(&read, found, "p").unwrap();
        assert_eq!(parent_row(&read, &back), Some(found));
    }
    // An orphan's link has one path; the other side's is empty.
    let orphan = parent_link_of(&read, row(&read, "e.txt"), "p").unwrap();
    assert_eq!((orphan.left.as_str(), orphan.right.as_str()), ("e.txt", ""));
    assert_eq!(parent_link_of(&read, NodeId::ROOT, "p"), None);
    // A path the parent no longer has.
    assert_eq!(parent_row(&read, &link("p", "gone.txt")), None);
    drop(read);
    let err = service
        .text_parent_row(&parent, &link("p", "gone.txt"))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(
        service
            .text_parent_row(&parent, &link("p", "sub/b.txt"))
            .unwrap(),
        row(&parent.model().read(), "sub/b.txt")
    );
}

#[test]
fn a10_26_with_the_parent_closed_the_session_is_compared_again() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    let observer = NullObserver;
    // A named folder session, read-only on the right.
    let mut session = folder_session(&l, &r, SessionType::FolderCompare);
    session.specs.read_only_right = true;
    let reference = SessionRef::parse("work/parent").unwrap();
    service.session_save(&reference, &session, false).unwrap();
    let control = job(&service, JobKind::FolderCompare);
    let again = service
        .text_parent_compare(&link("work/parent", "sub/b.txt"), &control, &observer)
        .unwrap();
    assert!(again.response.comparison.is_complete());
    let read = again.prepared.model().read();
    assert_eq!(path_of(&read, again.row), Some("sub/b.txt".into()));
    assert_eq!(
        again.read_only,
        ReadOnlySides {
            left: false,
            right: true
        }
    );
    assert_eq!(
        read.category(again.row.unwrap(), false),
        Category::Different
    );
    drop(read);
    // An auto-saved parent (a view that was never named).
    let id = service
        .session_autosave(None, &folder_session(&l, &r, SessionType::FolderCompare))
        .unwrap()
        .unwrap();
    let again = service
        .text_parent_compare(
            &link(&format!("_autosaved/{id}"), "a.txt"),
            &control,
            &observer,
        )
        .unwrap();
    assert_eq!(
        path_of(&again.prepared.model().read(), again.row),
        Some("a.txt".into())
    );
    // A folder-sync parent: the comparison its view shows.
    let sync = folder_session(&l, &r, SessionType::FolderSync);
    assert!(matches!(sync.rules, SessionRules::Sync(_)));
    service
        .session_save(&SessionRef::parse("sync").unwrap(), &sync, false)
        .unwrap();
    let again = service
        .text_parent_compare(&link("sync", "d.txt"), &control, &observer)
        .unwrap();
    assert_eq!(
        path_of(&again.prepared.model().read(), again.row),
        Some("d.txt".into())
    );
    // The row is gone: the comparison still comes back, without it.
    let again = service
        .text_parent_compare(&link("work/parent", "gone.txt"), &control, &observer)
        .unwrap();
    assert_eq!(again.row, None);
    // A text session is no parent; a missing session is not found.
    let mut text = Session::builtin(SessionType::TextCompare);
    text.specs.left = l.join("a.txt").display().to_string();
    text.specs.right = r.join("a.txt").display().to_string();
    service
        .session_save(&SessionRef::parse("text").unwrap(), &text, false)
        .unwrap();
    let err = service
        .text_parent_compare(&link("text", "a.txt"), &control, &observer)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let err = service
        .text_parent_compare(&link("missing", "a.txt"), &control, &observer)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
}

// ---- Copy File and Open Next Difference ----------------------------------------------------------------------------------

#[test]
fn a10_26_copy_file_steps_save_the_source_discard_the_target_and_plan_w6s_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    let parent = compared(&service, &l, &r);
    let a = row(&parent.model().read(), "a.txt");
    let mut view = live(
        &service,
        &l.join("a.txt"),
        &r.join("a.txt"),
        LiveOptions::default(),
    );
    type_x(&mut view.docs.left);
    type_x(&mut view.docs.right);
    let control = job(&service, JobKind::FolderOp);
    let steps = service
        .text_copy_file_steps(
            &copy_request(&parent, a, Side::Left, &view, ReadOnlySides::default()),
            &control,
        )
        .unwrap();
    // The source's edits are saved first, in place.
    let save = steps.save_source.as_ref().unwrap();
    assert_eq!(save.items.len(), 1);
    assert_eq!(save.items[0].side, Some(Side::Left));
    // The target's are lost.
    assert!(steps.discard_target);
    // W6's plan, as `folder op copy --to right --path a.txt` plans it through the Service.
    let cli = FolderOpRequest {
        op: OpRequest::Op(FolderOp::Copy {
            to: Side::Right,
            overwrite: OverwritePolicy::Always,
        }),
        select: SelectRequest {
            paths: vec![("a.txt".to_owned(), SideSet::empty())],
            ..SelectRequest::default()
        },
        read_only: ReadOnlySides::default(),
        overrides: OpOverrides::default(),
    };
    let w6 = service
        .folder_op_plan(parent.model(), &cli, &control)
        .unwrap();
    assert_eq!(steps.copy, w6);
    assert_eq!(steps.copy.plan.items.len(), 1);
    // The next pair after a.txt: c.txt is the same, d.txt differs.
    assert_eq!(
        path_of(&parent.model().read(), steps.next),
        Some("d.txt".into())
    );
    // Nothing was written by planning.
    assert_eq!(std::fs::read(l.join("a.txt")).unwrap(), b"left a\n");
    assert_eq!(std::fs::read(r.join("a.txt")).unwrap(), b"right a\n");

    // Executed: save, copy, refresh, next.
    let mut source = view.docs.left.clone();
    let report = service
        .text_copy_file_execute(
            steps,
            &execute(&parent, a, ConfirmMode::YesToAll),
            &mut source,
            &control,
            &NullObserver,
        )
        .unwrap();
    assert!(report.ok(), "{report:?}");
    assert!(report.save.as_ref().unwrap().ok());
    assert!(report.copy.as_ref().unwrap().ok());
    assert!(report.refresh.is_some());
    assert_eq!(std::fs::read(l.join("a.txt")).unwrap(), b"Xleft a\n");
    assert_eq!(std::fs::read(r.join("a.txt")).unwrap(), b"Xleft a\n");
    assert!(
        !source.is_modified(),
        "the save was applied to the document"
    );
    let read = parent.model().read();
    let copied = report.row.unwrap();
    assert_eq!(read.path(copied).as_str(), "a.txt");
    assert_eq!(read.category(copied, false), Category::Same);
    assert_eq!(path_of(&read, report.next), Some("d.txt".into()));
}

#[test]
fn a10_26_copy_file_without_edits_and_from_the_right() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    let parent = compared(&service, &l, &r);
    let b = row(&parent.model().read(), "sub/b.txt");
    let view = live(
        &service,
        &l.join("sub/b.txt"),
        &r.join("sub/b.txt"),
        LiveOptions::default(),
    );
    let control = job(&service, JobKind::FolderOp);
    let steps = service
        .text_copy_file_steps(
            &copy_request(&parent, b, Side::Right, &view, ReadOnlySides::default()),
            &control,
        )
        .unwrap();
    assert!(steps.save_source.is_none());
    assert!(!steps.discard_target);
    assert_eq!(
        path_of(&parent.model().read(), steps.next),
        Some("a.txt".into())
    );
    let mut source = view.docs.right.clone();
    let report = service
        .text_copy_file_execute(
            steps,
            &CopyFileExecute {
                from: Side::Right,
                ..execute(&parent, b, ConfirmMode::YesToAll)
            },
            &mut source,
            &control,
            &NullObserver,
        )
        .unwrap();
    assert!(report.ok(), "{report:?}");
    assert!(report.save.is_none());
    assert_eq!(std::fs::read(l.join("sub/b.txt")).unwrap(), b"b22\n");
    // A side that is neither left nor right copies nothing.
    let err = service
        .text_copy_file_steps(
            &copy_request(&parent, b, Side::Center, &view, ReadOnlySides::default()),
            &control,
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn a10_26_a_read_only_target_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    let parent = compared(&service, &l, &r);
    let a = row(&parent.model().read(), "a.txt");
    let mut view = live(
        &service,
        &l.join("a.txt"),
        &r.join("a.txt"),
        LiveOptions::default(),
    );
    type_x(&mut view.docs.left);
    let control = job(&service, JobKind::FolderOp);
    // The parent's right side is read-only (D-184).
    let read_only = ReadOnlySides {
        left: false,
        right: true,
    };
    let err = service
        .text_copy_file_steps(
            &copy_request(&parent, a, Side::Left, &view, read_only),
            &control,
        )
        .unwrap_err();
    assert!(
        matches!(err, ApiError::ReadOnlySide { side: Side::Right }),
        "{err:?}"
    );
    // The other way is allowed: the left side is writable.
    let steps = service
        .text_copy_file_steps(
            &copy_request(&parent, a, Side::Right, &view, read_only),
            &control,
        )
        .unwrap();
    assert!(steps.discard_target, "the left pane's edits would be lost");
    // The right pane opened read-only (`--ro2`) is a read-only side too.
    let mut ro = live(
        &service,
        &l.join("a.txt"),
        &r.join("a.txt"),
        LiveOptions {
            read_only: ReadOnlySides {
                left: false,
                right: true,
            },
            ..LiveOptions::default()
        },
    );
    type_x(&mut ro.docs.left);
    let err = service
        .text_copy_file_steps(
            &copy_request(&parent, a, Side::Left, &ro, ReadOnlySides::default()),
            &control,
        )
        .unwrap_err();
    assert!(
        matches!(err, ApiError::ReadOnlySide { side: Side::Right }),
        "{err:?}"
    );
    // Nothing was written.
    assert_eq!(std::fs::read(l.join("a.txt")).unwrap(), b"left a\n");
    assert_eq!(std::fs::read(r.join("a.txt")).unwrap(), b"right a\n");
}

#[test]
fn a10_26_a_question_left_open_stops_the_chain_after_the_save() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    // The right file is newer: W6 asks before it replaces it.
    let newer = SystemTime::now() + Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(r.join("d.txt"))
        .unwrap()
        .set_modified(newer)
        .unwrap();
    let parent = compared(&service, &l, &r);
    let d = row(&parent.model().read(), "d.txt");
    let mut view = live(
        &service,
        &l.join("d.txt"),
        &r.join("d.txt"),
        LiveOptions::default(),
    );
    type_x(&mut view.docs.left);
    let control = job(&service, JobKind::FolderOp);
    let steps = service
        .text_copy_file_steps(
            &copy_request(&parent, d, Side::Left, &view, ReadOnlySides::default()),
            &control,
        )
        .unwrap();
    let asks = service.folder_confirmation(&steps.copy, None);
    assert!(
        asks.open_questions()
            .contains_key(&QuestionKind::OverwriteNewer),
        "the copy asks about the newer target"
    );
    let mut source = view.docs.left.clone();
    let report = service
        .text_copy_file_execute(
            steps,
            &execute(&parent, d, ConfirmMode::Prompt),
            &mut source,
            &control,
            &NullObserver,
        )
        .unwrap();
    assert_eq!(report.stopped, Some(CopyFileStep::Copy));
    assert!(!report.ok());
    assert!(report.error.is_some());
    // The save ran (it is the first step), the copy did not.
    assert_eq!(std::fs::read(l.join("d.txt")).unwrap(), b"Xd left\n");
    assert_eq!(std::fs::read(r.join("d.txt")).unwrap(), b"d right\n");
    assert!(report.copy.is_none());
    assert_eq!(report.next, None);
}

#[test]
fn a10_26_a_declined_copy_stops_the_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let (l, r) = tree(tmp.path());
    let newer = SystemTime::now() + Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(r.join("d.txt"))
        .unwrap()
        .set_modified(newer)
        .unwrap();
    let parent = compared(&service, &l, &r);
    let d = row(&parent.model().read(), "d.txt");
    let view = live(
        &service,
        &l.join("d.txt"),
        &r.join("d.txt"),
        LiveOptions::default(),
    );
    let control = job(&service, JobKind::FolderOp);
    let steps = service
        .text_copy_file_steps(
            &copy_request(&parent, d, Side::Left, &view, ReadOnlySides::default()),
            &control,
        )
        .unwrap();
    let mut source = view.docs.left.clone();
    let report = service
        .text_copy_file_execute(
            steps,
            &execute(&parent, d, ConfirmMode::NoToAll),
            &mut source,
            &control,
            &NullObserver,
        )
        .unwrap();
    assert_eq!(report.stopped, Some(CopyFileStep::Copy), "{report:?}");
    assert!(report.copy.is_some(), "the copy ran with its item declined");
    assert!(report.refresh.is_none());
    assert_eq!(std::fs::read(r.join("d.txt")).unwrap(), b"d right\n");
}

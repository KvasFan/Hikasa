//! A9.12 (M2 W9b, D-244, D-246): a preview is the dry run. For random twin trees on disk (the Service's router serves
//! local files) with W6's generators (a name in two cases, NFC/NFD names, newer files, orphans, read-only files),
//! random selections and every verb, `OpPreview::line(plan, i)` and the summary line equal the human output of
//! `folder_op_dry_run` that `folder op <verb> … --dry-run` prints, line for line, its counts add up to the dry run's
//! summary and its warnings are the dry run's; the same for `SyncPreview` against `folder_sync_dry_run` for every
//! preset. The preview's own memory is at most 16 bytes an item (a counting allocator, 100,000 items).
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace lint is `deny`,
// so this test binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use common::{facts, fixed_clock, job, os, write};
use hikasa_core::{FileTime, JobKind, Side};
use hikasa_folder::NullObserver;
use hikasa_folder::ops::{FolderOp, OpPlan, PathStructure, Recycle, RenameMask, TouchFrom};
use hikasa_folder::select::SideSet;
use hikasa_folder::sync::{SyncOptions, SyncPreset};
use hikasa_session::api::human::{folder_op_lines, folder_sync_lines};
use hikasa_session::api::{
    CountKind, FolderCompareRequest, FolderOpRequest, FolderSyncRequest, OpOverrides, OpRequest,
    PreparedFolderCompare, SelectRequest, parse_touch_time,
};
use hikasa_session::{Service, ServiceInit};
use hikasa_vfs::ops::OverwritePolicy;
use proptest::prelude::*;

/// Counts live heap bytes (the §6.1 exemption for `unsafe` in test binaries). Under nextest every test runs in a
/// process of its own, so nothing else allocates while the memory test counts.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: every method forwards to the system allocator with the caller's arguments unchanged, so the `GlobalAlloc`
// contract (valid layouts, pointers returned by this allocator) is upheld by `System`; the counter is only bookkeeping.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; the caller guarantees a non-zero-sized layout.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by `alloc`/`realloc` of this allocator (that is, of `System`) with `layout`.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr`/`layout` come from this allocator; `new_size` is the caller's valid new size.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(
                new_size as isize - layout.size() as isize,
                Ordering::Relaxed,
            );
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn service(root: &std::path::Path) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(2),
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

use std::sync::Arc;

fn prepared(
    service: &Service,
    root: &std::path::Path,
    sync: Option<&SyncOptions>,
) -> PreparedFolderCompare {
    let request = FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        ..FolderCompareRequest::default()
    };
    let control = job(service, JobKind::FolderCompare);
    let prepared = match sync {
        Some(options) => service.folder_sync_prepare(&request, options, &control),
        None => service.folder_prepare_in(&request, &control),
    }
    .unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    prepared
}

/// The names, and their second spelling (W6's generators: case, NFC/NFD).
const PATHS: [(&str, &str); 8] = [
    ("a.txt", "A.TXT"),
    ("b.dat", "b.dat"),
    ("sub/c.txt", "sub/C.txt"),
    ("sub/d.txt", "sub/d.txt"),
    ("sub/deep/e.txt", "sub/deep/e.txt"),
    ("z.txt", "z.txt"),
    ("caf\u{e9}.txt", "cafe\u{301}.txt"),
    ("sub/n\u{e9}e.dat", "sub/ne\u{301}e.dat"),
];

#[derive(Debug, Clone)]
struct Entry {
    path: usize,
    left: Option<u8>,
    right: Option<u8>,
    spell: (bool, bool),
}

fn entries() -> impl Strategy<Value = Vec<Entry>> {
    proptest::collection::vec(
        (
            0..PATHS.len(),
            proptest::option::of(0u8..3),
            proptest::option::of(0u8..3),
            any::<(bool, bool)>(),
        ),
        1..9,
    )
    .prop_map(|list| {
        let mut seen = std::collections::BTreeSet::new();
        list.into_iter()
            .filter(|(i, ..)| seen.insert(*i))
            .map(|(path, left, right, spell)| Entry {
                path,
                left,
                right,
                spell,
            })
            .collect()
    })
}

fn build(root: &std::path::Path, tree: &[Entry]) {
    std::fs::create_dir_all(root.join("l")).unwrap();
    std::fs::create_dir_all(root.join("r")).unwrap();
    for entry in tree {
        for (side, version, other) in [
            ("l", entry.left, entry.spell.0),
            ("r", entry.right, entry.spell.1),
        ] {
            let Some(v) = version else { continue };
            let (first, second) = PATHS[entry.path];
            let path = if other { second } else { first };
            let file = write(
                root,
                &format!("{side}/{path}"),
                format!("{first}{}\n", "x".repeat(usize::from(v))).as_bytes(),
            );
            let time = std::time::UNIX_EPOCH
                + std::time::Duration::from_secs(1_000_000 + u64::from(v) * 10);
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
    }
}

/// The masks a selection draws from (`--select`).
const MASKS: [&str; 6] = [
    "all",
    "left.all",
    "right.files",
    "changed",
    "orphan",
    "newer.files",
];

/// Every verb of `folder op` that plans, with its request (`target` is a folder outside the bases).
fn verbs(target: &std::path::Path) -> Vec<(&'static str, OpRequest)> {
    let both = SideSet::LEFT | SideSet::RIGHT;
    vec![
        (
            "copy right",
            OpRequest::Op(FolderOp::Copy {
                to: Side::Right,
                overwrite: OverwritePolicy::Always,
            }),
        ),
        (
            "copy left newer",
            OpRequest::Op(FolderOp::Copy {
                to: Side::Left,
                overwrite: OverwritePolicy::Newer,
            }),
        ),
        (
            "move right skip",
            OpRequest::Op(FolderOp::Move {
                to: Side::Right,
                overwrite: OverwritePolicy::Skip,
            }),
        ),
        ("exchange", OpRequest::Op(FolderOp::Exchange)),
        (
            "delete",
            OpRequest::Op(FolderOp::Delete {
                sides: both,
                recycle: Recycle::default(),
            }),
        ),
        (
            "delete bin",
            OpRequest::Op(FolderOp::Delete {
                sides: SideSet::LEFT,
                recycle: Recycle {
                    use_bin: true,
                    fallback: true,
                },
            }),
        ),
        (
            "rename",
            OpRequest::Op(FolderOp::Rename {
                sides: both,
                mask: RenameMask::Dos("*.bak".to_owned()),
            }),
        ),
        (
            "rename regex",
            OpRequest::Op(FolderOp::Rename {
                sides: SideSet::RIGHT,
                mask: RenameMask::Regex {
                    old: "(.*)".to_owned(),
                    new: "x_$1".to_owned(),
                },
            }),
        ),
        (
            "touch at",
            OpRequest::Op(FolderOp::Touch {
                sides: both,
                from: TouchFrom::At(FileTime::from_unix(2_000_000, 0).unwrap()),
            }),
        ),
        (
            "touch other",
            OpRequest::Op(FolderOp::Touch {
                sides: SideSet::RIGHT,
                from: TouchFrom::OtherSide { to: Side::Right },
            }),
        ),
        (
            "touch now",
            OpRequest::TouchNow {
                sides: SideSet::LEFT,
            },
        ),
        (
            "mkdir",
            OpRequest::MkdirIn {
                sides: both,
                parent: String::new(),
                name: "new".to_owned(),
                folder: true,
            },
        ),
        (
            "copyto",
            OpRequest::CopyTo {
                sides: both,
                target: os(target),
                structure: PathStructure::Relative,
                overwrite: OverwritePolicy::Fail,
            },
        ),
        (
            "moveto",
            OpRequest::MoveTo {
                sides: SideSet::LEFT,
                target: os(&target.join("m")),
                structure: PathStructure::None,
                overwrite: OverwritePolicy::Always,
            },
        ),
    ]
}

/// The preview's lines and counts against the dry run of the same plan.
fn check_op(service: &Service, plan: &OpPlan) -> Result<(), TestCaseError> {
    let preview = service.folder_op_preview(plan);
    let dry = service.folder_op_dry_run(plan.clone());
    let dto = dry.dto();
    let lines = folder_op_lines(&dto, dry.verb);
    prop_assert_eq!(preview.len() + 1, lines.len());
    for (i, line) in lines.iter().enumerate() {
        prop_assert_eq!(&preview.line(plan, i), line, "line {}", i);
    }
    prop_assert_eq!(preview.summary(), dto.summary);
    prop_assert_eq!(preview.warnings(), &dto.warnings[..]);
    let counted: u64 = preview.counts().iter().map(|c| c.items).sum();
    prop_assert_eq!(counted, dto.summary.planned);
    let failed: u64 = preview
        .counts()
        .iter()
        .filter(|c| c.kind == CountKind::Failed)
        .map(|c| c.items)
        .sum();
    prop_assert_eq!(failed, dto.summary.failed);
    let backups: Vec<(String, String)> = preview
        .backups()
        .iter()
        .map(|(f, b)| (f.canonical(), b.canonical()))
        .collect();
    let dto_backups: Vec<(String, String)> = dto
        .backups
        .iter()
        .map(|b| (b.file.clone(), b.backup.clone()))
        .collect();
    prop_assert_eq!(backups, dto_backups);
    for (i, item) in dto.items.iter().enumerate() {
        let at = preview.item(i).unwrap();
        prop_assert_eq!(at.status, item.status);
        prop_assert_eq!(at.reason, item.reason);
        prop_assert_eq!(preview.conflicts(plan, i).names(), item.conflicts.clone());
    }
    Ok(())
}

/// The cases of a property: `default`, or `PROPTEST_CASES / divisor` (a case builds two trees on disk).
fn cases(default: u32, divisor: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(default, |n: u32| (n / divisor).max(1))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(96, 8)))]

    #[test]
    fn p_op_preview_is_the_dry_run(
        tree in entries(),
        masks in proptest::collection::vec(0..MASKS.len(), 1..3),
        verb in 0usize..14,
        hidden in any::<bool>(),
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        build(root, &tree);
        let service = service(root);
        let prepared = prepared(&service, root, None);
        let (_, op) = verbs(&root.join("out")).swap_remove(verb);
        let request = FolderOpRequest {
            op,
            select: SelectRequest {
                masks: masks.into_iter().map(|i| MASKS[i].to_owned()).collect(),
                include_hidden: Some(hidden),
                ..SelectRequest::default()
            },
            read_only: Default::default(),
            overrides: OpOverrides::default(),
        };
        let control = job(&service, JobKind::FolderOp);
        // A refused plan (nothing selected, a name that needs one row) has no preview.
        if let Ok(plan) = service.folder_op_plan(prepared.model(), &request, &control) {
            check_op(&service, &plan)?;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(32, 16)))]

    #[test]
    fn p_sync_preview_is_the_dry_run(
        tree in entries(),
        preset in 0usize..5,
        just in proptest::option::of(0..MASKS.len()),
        overriding in any::<bool>(),
        folders in any::<bool>(),
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        build(root, &tree);
        // Both `sub` folders, also where one holds no file: a mirror empties it entirely (a nested mount that failed).
        if folders {
            std::fs::create_dir_all(root.join("l/sub")).unwrap();
            std::fs::create_dir_all(root.join("r/sub")).unwrap();
        }
        let service = service(root);
        let mut options = SyncOptions {
            preset: SyncPreset::ALL[preset],
            ..SyncOptions::default()
        };
        if overriding {
            options.overrides.insert(
                "sub".to_owned(),
                hikasa_folder::sync::SyncAction::Leave,
            );
        }
        let prepared = prepared(&service, root, Some(&options));
        let request = FolderSyncRequest {
            options,
            select: just.map(|i| SelectRequest {
                masks: vec![MASKS[i].to_owned()],
                ..SelectRequest::default()
            }),
            ..FolderSyncRequest::default()
        };
        let control = job(&service, JobKind::FolderSync);
        let Ok(plan) = service.folder_sync_plan(prepared.model(), &request, &control) else {
            return Ok(());
        };
        let preview = service.folder_sync_preview(&plan);
        check_op(&service, &plan.op)?;
        let read = prepared.model().read();
        let dry = service.folder_sync_dry_run(plan.clone());
        let dto = dry.dto(&read, false);
        let lines = folder_sync_lines(&dto);
        prop_assert_eq!(preview.len(), lines.len());
        for (i, line) in lines.iter().enumerate() {
            prop_assert_eq!(&preview.line(&plan, &read, i), line, "line {}", i);
            prop_assert!(preview.kind(&plan, i).is_some());
        }
        prop_assert!(preview.kind(&plan, lines.len()).is_none());
        prop_assert_eq!(preview.counts(), dto.counts);
        prop_assert_eq!(preview.warnings(), &dto.warnings[..]);
    }
}

#[test]
fn the_preview_keeps_at_most_16_bytes_an_item() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for i in 0..20 {
        write(root, &format!("l/f{i:02}.txt"), b"left");
        write(root, &format!("r/f{i:02}.txt"), b"right!");
    }
    let service = service(root);
    let prepared = prepared(&service, root, None);
    let request = FolderOpRequest {
        op: OpRequest::Op(FolderOp::Copy {
            to: Side::Right,
            overwrite: OverwritePolicy::Always,
        }),
        select: SelectRequest {
            masks: vec!["all".to_owned()],
            ..SelectRequest::default()
        },
        read_only: Default::default(),
        overrides: OpOverrides::default(),
    };
    let control = job(&service, JobKind::FolderOp);
    let mut plan = service
        .folder_op_plan(prepared.model(), &request, &control)
        .unwrap();
    // 100,000 items, every one replacing: the small plan repeated.
    let n = 100_000;
    let items = plan.plan.items.clone();
    let meta = plan.meta.clone();
    plan.plan.items = items.iter().cycle().take(n).cloned().collect();
    plan.meta = meta.iter().cycle().take(n).cloned().collect();
    let before = LIVE.load(Ordering::SeqCst);
    let preview = service.folder_op_preview(&plan);
    let after = LIVE.load(Ordering::SeqCst);
    let kept = after - before;
    assert_eq!(preview.len(), n);
    assert!(
        kept <= 16 * n as isize,
        "the preview keeps {kept} bytes for {n} items ({} an item)",
        kept as f64 / n as f64
    );
    // The lines are the plan's, made on demand.
    assert!(
        preview.line(&plan, 0).starts_with("R replace "),
        "{}",
        preview.line(&plan, 0)
    );
    assert_eq!(preview.summary_line(), format!("would copy {n} items"));
    drop(preview);
}

#[test]
fn touch_times_in_the_local_zone_and_rfc_3339() {
    let utc = hikasa_session::api::human::TimeZone::UTC;
    let at = |text: &str, zone: &hikasa_session::api::human::TimeZone| {
        parse_touch_time(text, zone).map(|t| t.unix_seconds())
    };
    assert_eq!(at("1970-01-02 00:00:00", &utc).unwrap(), 86_400);
    assert_eq!(at(" 1970-01-02T00:00:00 ", &utc).unwrap(), 86_400);
    assert_eq!(at("1970-01-02 00:01", &utc).unwrap(), 86_460);
    assert_eq!(at("1970-01-02", &utc).unwrap(), 86_400);
    assert_eq!(at("1970-01-02T00:00:00Z", &utc).unwrap(), 86_400);
    // An explicit offset wins over the zone (the CLI's `--time` rule).
    let kathmandu = hikasa_session::api::human::TimeZone::fixed(
        jiff::tz::Offset::from_seconds(5 * 3600 + 45 * 60).unwrap(),
    );
    assert_eq!(at("1970-01-02T00:00:00Z", &kathmandu).unwrap(), 86_400);
    assert_eq!(at("1970-01-02 05:45:00", &kathmandu).unwrap(), 86_400);
    // A DST gap: 02:30 does not exist on 2026-03-29 in Berlin; taken after the gap (03:30 CEST = 01:30 UTC).
    if let Ok(berlin) = hikasa_session::api::human::TimeZone::get("Europe/Berlin") {
        let gap = at("2026-03-29 02:30:00", &berlin).unwrap();
        assert_eq!(gap, at("2026-03-29T01:30:00Z", &utc).unwrap());
    }
    // The same rules in a zone given as a POSIX rule (no time zone database needed), and the fold (R16): 02:30 on
    // 2026-10-25 happens twice in Central Europe; the earlier one is taken (CEST, 00:30 UTC), jiff's *compatible*.
    let cet = hikasa_session::api::human::TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
    assert_eq!(
        at("2026-03-29 02:30:00", &cet).unwrap(),
        at("2026-03-29T01:30:00Z", &utc).unwrap(),
        "a gap: after it"
    );
    assert_eq!(
        at("2026-10-25 02:30:00", &cet).unwrap(),
        at("2026-10-25T00:30:00Z", &utc).unwrap(),
        "a fold: the earlier"
    );
    assert_eq!(
        at("2026-10-25 03:30:00", &cet).unwrap(),
        at("2026-10-25T02:30:00Z", &utc).unwrap(),
        "after the fold"
    );
    for bad in ["", "yesterday", "2026-13-01 00:00:00", "2026-01-01 25:00"] {
        let error = parse_touch_time(bad, &utc).unwrap_err();
        assert!(error.to_string().contains("YYYY-MM-DD HH:MM:SS"), "{error}");
    }
}

#[test]
fn a_folder_emptied_entirely_and_a_conflict_are_previewed_as_the_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"left");
    write(root, "r/a.txt", b"right side");
    std::fs::create_dir_all(root.join("l/sub")).unwrap();
    write(root, "r/sub/c.txt", b"only right");
    let service = service(root);
    for preset in [SyncPreset::MirrorRight, SyncPreset::UpdateBoth] {
        let options = SyncOptions {
            preset,
            ..SyncOptions::default()
        };
        let prepared = prepared(&service, root, Some(&options));
        let request = FolderSyncRequest {
            options,
            ..FolderSyncRequest::default()
        };
        let control = job(&service, JobKind::FolderSync);
        let plan = service
            .folder_sync_plan(prepared.model(), &request, &control)
            .unwrap();
        let preview = service.folder_sync_preview(&plan);
        let read = prepared.model().read();
        let lines = folder_sync_lines(&service.folder_sync_dry_run(plan.clone()).dto(&read, false));
        let shown: Vec<String> = (0..preview.len())
            .map(|i| preview.line(&plan, &read, i))
            .collect();
        assert_eq!(shown, lines, "{preset:?}");
        if preset == SyncPreset::MirrorRight {
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with("# emptied entirely: sub")),
                "{lines:?}"
            );
        }
    }
}

/// R18: a count line shows its bytes from one byte on.
#[test]
fn a_count_line_shows_its_bytes_from_one_byte() {
    use hikasa_session::api::PreviewCount;
    let line = |bytes| {
        PreviewCount {
            kind: CountKind::Action("copy"),
            items: 1,
            bytes,
        }
        .text()
    };
    assert_eq!(line(0), "copy: 1 items");
    assert_eq!(line(1), "copy: 1 items, 1 bytes");
    assert_eq!(line(2), "copy: 1 items, 2 bytes");
}

/// R17: a row the comparison could not tell is marked unresolved in the sync preview (a folder pair not scanned:
/// *auto scan subfolders* off in a Folder Compare, as Synchronize ▸ plans on the view's model as it stands).
#[test]
fn an_unresolved_row_is_marked_in_the_sync_preview() {
    use hikasa_session::api::SyncLine;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "l/a.txt", b"left");
    write(root, "r/a.txt", b"right side");
    write(root, "l/sub/c.txt", b"one");
    write(root, "r/sub/c.txt", b"two!");
    let service = service(root);
    let request = FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        patch: hikasa_folder::FolderRulesPatch {
            auto_scan_subfolders: Some(false),
            ..Default::default()
        },
        ..FolderCompareRequest::default()
    };
    let control = job(&service, JobKind::FolderCompare);
    let prepared = service.folder_prepare_in(&request, &control).unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    let options = SyncOptions {
        preset: SyncPreset::UpdateRight,
        ..SyncOptions::default()
    };
    let sync = FolderSyncRequest {
        options,
        ..FolderSyncRequest::default()
    };
    let control = job(&service, JobKind::FolderSync);
    let plan = service
        .folder_sync_plan(prepared.model(), &sync, &control)
        .unwrap();
    assert!(plan.counts.unresolved > 0, "{:?}", plan.counts);
    let preview = service.folder_sync_preview(&plan);
    let read = prepared.model().read();
    let marked: Vec<String> = (0..preview.len())
        .filter(|&i| {
            matches!(
                preview.kind(&plan, i),
                Some(SyncLine::Row {
                    unresolved: true,
                    ..
                })
            )
        })
        .map(|i| preview.line(&plan, &read, i))
        .collect();
    assert!(!marked.is_empty(), "the unresolved rows are marked");
    assert!(marked.iter().all(|l| l.contains("sub")), "{marked:?}");
    let resolved = (0..preview.len()).any(|i| {
        matches!(
            preview.kind(&plan, i),
            Some(SyncLine::Row {
                unresolved: false,
                ..
            })
        )
    });
    assert!(resolved, "a.txt is not unresolved");
}

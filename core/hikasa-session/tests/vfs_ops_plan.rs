//! M2 W9c acceptance A9.19 (core, D-251): a drop executes the plan it asked about. `vfs_op_execute(vfs_op_plan(r))`
//! writes what `vfs_run(r)` writes and reports the same items, for copies and moves of random trees into a folder that
//! holds some of their names, under every overwrite policy (two identical sandboxes, one run each way). `vfs_plan` and
//! `vfs_run` keep W3's behaviour: their goldens are W3's tests (`clipboard.rs`, `cli`'s `vfs` goldens).

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::{job, os};
use hikasa_core::JobKind;
use hikasa_session::api::{VfsOpDto, VfsOpRequest};
use hikasa_vfs::ops::{CopyOptions, MoveOptions, OverwritePolicy};
use proptest::prelude::*;

fn cases(default: u32, divisor: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(default, |n: u32| (n / divisor).max(1))
}

/// One top-level entry: a file (content, age) or a folder of files.
#[derive(Debug, Clone)]
enum Entry {
    File(u8, u8),
    Folder(Vec<(String, u8, u8)>),
}

const NAMES: [&str; 4] = ["a", "b.txt", "c", "d.bin"];

fn entry() -> impl Strategy<Value = Entry> {
    prop_oneof![
        (0u8..3, 0u8..4).prop_map(|(c, t)| Entry::File(c, t)),
        proptest::collection::vec(
            (
                prop::sample::select(&["x", "y.txt", "z"][..]),
                0u8..3,
                0u8..4
            ),
            0..3
        )
        .prop_map(|files| {
            let mut seen = BTreeMap::new();
            for (name, c, t) in files {
                seen.insert(name.to_owned(), (c, t));
            }
            Entry::Folder(seen.into_iter().map(|(n, (c, t))| (n, c, t)).collect())
        }),
    ]
}

fn side(at_least: usize) -> impl Strategy<Value = BTreeMap<usize, Entry>> {
    proptest::collection::btree_map(0usize..NAMES.len(), entry(), at_least..4)
}

fn put(path: &Path, content: u8, age: u8) {
    std::fs::write(path, vec![b'0' + content; usize::from(content) + 1]).unwrap();
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + u64::from(age) * 100);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

fn build(root: &Path, tree: &BTreeMap<usize, Entry>) {
    std::fs::create_dir_all(root).unwrap();
    for (k, entry) in tree {
        let path = root.join(NAMES[*k]);
        match entry {
            Entry::File(c, t) => put(&path, *c, *t),
            Entry::Folder(files) => {
                std::fs::create_dir_all(&path).unwrap();
                for (name, c, t) in files {
                    put(&path.join(name), *c, *t);
                }
            }
        }
    }
}

/// Every entry below `root`: path, kind and the bytes of a file.
fn listing(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.push((format!("{rel}/"), None));
                stack.push(path);
            } else {
                out.push((rel, Some(std::fs::read(&path).unwrap())));
            }
        }
    }
    out.sort();
    out
}

/// The DTO's items and summary with the sandbox's root written as `<root>`.
fn report(dto: &VfsOpDto, root: &Path) -> String {
    let root = root.to_string_lossy().into_owned();
    let items: Vec<String> = dto
        .items
        .iter()
        .map(|i| {
            // The texts as they are (not `{:?}`, which escapes a Windows path's backslashes, so `<root>` would not
            // replace it).
            format!(
                "{} {:?} {} {} {:?} {} {:?} {:?} {:?}",
                i.action,
                i.kind,
                i.source.as_deref().unwrap_or("-"),
                i.target,
                i.size,
                i.replaces,
                i.status,
                i.reason,
                i.error.as_ref().map(|e| e.kind)
            )
        })
        .collect();
    format!("{}\n{:?}\n{}", items.join("\n"), dto.summary, dto.cancelled).replace(&root, "<root>")
}

fn request(
    root: &Path,
    moving: bool,
    sources: &[usize],
    overwrite: OverwritePolicy,
) -> VfsOpRequest {
    let sources = sources
        .iter()
        .map(|k| os(&root.join("src").join(NAMES[*k])))
        .collect();
    let mut target = os(&root.join("dst"));
    target.push(std::path::MAIN_SEPARATOR_STR);
    if moving {
        VfsOpRequest::Move {
            sources,
            target,
            options: MoveOptions {
                overwrite,
                ..MoveOptions::default()
            },
        }
    } else {
        VfsOpRequest::Copy {
            sources,
            target,
            options: CopyOptions {
                recursive: true,
                overwrite,
                ..CopyOptions::default()
            },
        }
    }
}

fn sandbox(
    base: &Path,
    name: &str,
    src: &BTreeMap<usize, Entry>,
    dst: &BTreeMap<usize, Entry>,
) -> PathBuf {
    let root = base.join(name);
    build(&root.join("src"), src);
    build(&root.join("dst"), dst);
    root
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(48, 16)))]

    #[test]
    fn p_the_executed_plan_is_vfs_run(
        src in side(1),
        dst in side(0),
        moving in any::<bool>(),
        pick in proptest::collection::vec(any::<bool>(), NAMES.len()),
        overwrite in prop::sample::select(&[
            OverwritePolicy::Fail,
            OverwritePolicy::Skip,
            OverwritePolicy::Always,
            OverwritePolicy::Newer,
        ][..]),
    ) {
        // The picked entries, else the first one: never an empty drop (no case is rejected).
        let mut sources: Vec<usize> = src.keys().copied().filter(|k| pick[*k]).collect();
        if sources.is_empty() {
            sources.extend(src.keys().next().copied());
        }
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let direct = sandbox(&base, "direct", &src, &dst);
        let planned = sandbox(&base, "planned", &src, &dst);
        let svc = common::service(&base);
        let control = || job(&svc, JobKind::VfsWrite);
        let ran = svc.vfs_run(&request(&direct, moving, &sources, overwrite), &control());
        let before = listing(&planned);
        let plan = svc.vfs_op_plan(&request(&planned, moving, &sources, overwrite), &control());
        match (ran, plan) {
            (Ok(ran), Ok(plan)) => {
                // The preview is the dry run of the same plan and writes nothing.
                prop_assert_eq!(listing(&planned), before);
                prop_assert!(plan.preview.dry_run);
                prop_assert_eq!(plan.preview.items.len(), plan.plan.items.len());
                let done = svc.vfs_op_execute(plan.plan, &control()).unwrap();
                prop_assert_eq!(report(&done, &planned), report(&ran, &direct));
                prop_assert_eq!(listing(&planned), listing(&direct));
            }
            (Err(a), Err(b)) => prop_assert_eq!(
                a.to_string().replace(&*direct.to_string_lossy(), "<root>"),
                b.to_string().replace(&*planned.to_string_lossy(), "<root>")
            ),
            (ran, plan) => prop_assert!(false, "one way failed: {:?} / {:?}", ran.err(), plan.err()),
        }
    }
}

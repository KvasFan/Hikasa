//! D-164's measure of the copy path, by hand and in a release build (`--ignored --nocapture`): a stamped batch that
//! copies 10,000 files of 4 KiB in 100 folders, local → local, targets absent, against a plain recursive copy that keeps
//! times, without `fsync` or a temporary name, median of three each, on one file system. G7 itself is measured at the
//! end of W6c through the Service (§1.6.22); this one shows what `ops` adds. It asserts nothing.

mod common;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hikasa_core::{Ctl, LocalPath, Spec};
use hikasa_vfs::ops::{
    self, Action, BatchOptions, ExecOptions, Expect, Fence, ItemStamp, OpKind, Plan, PlanItem,
    PlanOptions,
};
use hikasa_vfs::{EntryKind, FollowLinks, LocalVfs, Vfs};

const FOLDERS: usize = 100;
const FILES: usize = 100;

fn tree(root: &Path) {
    let content = vec![7u8; 4096];
    for folder in 0..FOLDERS {
        let dir = root.join(format!("d{folder:03}"));
        fs::create_dir_all(&dir).unwrap();
        for file in 0..FILES {
            fs::write(dir.join(format!("f{file:03}.bin")), &content).unwrap();
        }
    }
}

fn plain(from: &Path, to: &Path) {
    for folder in fs::read_dir(from).unwrap() {
        let folder = folder.unwrap();
        let dir = to.join(folder.file_name());
        fs::create_dir(&dir).unwrap();
        for item in fs::read_dir(folder.path()).unwrap() {
            let item = item.unwrap();
            let target = dir.join(item.file_name());
            fs::copy(item.path(), &target).unwrap();
            let time = item.metadata().unwrap().modified().unwrap();
            File::options()
                .write(true)
                .open(&target)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
    }
}

fn spec(path: PathBuf) -> Spec {
    Spec::Local(LocalPath::new(path).unwrap())
}

/// A batch as a folder plan is: a `create_dir` for each folder, a stamped `copy` for each file.
fn prepared(vfs: &LocalVfs, from: &Path, to: &Path) -> Plan {
    let mut items = Vec::new();
    for folder in 0..FOLDERS {
        let name = format!("d{folder:03}");
        let (source, target) = (from.join(&name), to.join(&name));
        items.push(PlanItem::planned(
            Action::CreateDir,
            EntryKind::Dir,
            None,
            spec(target.clone()),
            0,
        ));
        for file in 0..FILES {
            let name = format!("f{file:03}.bin");
            let source = spec(source.join(&name));
            let entry = vfs.stat(&source, FollowLinks::No).unwrap();
            let mut item = PlanItem::planned(
                Action::Copy,
                EntryKind::File,
                Some(source),
                spec(target.join(&name)),
                1,
            );
            item.size = Some(entry.size);
            item.expect = Some(Expect {
                source: Some(ItemStamp {
                    kind: entry.kind,
                    size: Some(entry.size),
                    mtime: entry.mtime,
                }),
                target: None,
            });
            items.push(item);
        }
    }
    Plan {
        op: OpKind::Batch,
        options: PlanOptions::Batch(BatchOptions::default()),
        items,
        warnings: Vec::new(),
    }
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
#[ignore = "a measure, by hand, in a release build"]
fn d164_copy_path_against_a_plain_copy() {
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let from = root.join("from");
    tree(&from);
    let vfs = LocalVfs::default();
    let (mut baseline, mut batch) = (Vec::new(), Vec::new());
    for round in 0..3 {
        let to = root.join(format!("plain{round}"));
        fs::create_dir(&to).unwrap();
        let start = Instant::now();
        plain(&from, &to);
        baseline.push(start.elapsed());
        fs::remove_dir_all(&to).unwrap();

        let to = root.join(format!("batch{round}"));
        fs::create_dir(&to).unwrap();
        let plan = prepared(&vfs, &from, &to);
        let options = ExecOptions {
            fence: Some(Fence {
                bases: vec![spec(from.clone()), spec(to.clone())],
                writes: true,
            }),
            ..ExecOptions::default()
        };
        let start = Instant::now();
        let report = ops::execute_with(&vfs, &vfs, plan, &options, &Ctl::detached());
        batch.push(start.elapsed());
        assert!(report.ok());
        fs::remove_dir_all(&to).unwrap();
    }
    let (plain, ours) = (median(baseline.clone()), median(batch.clone()));
    println!(
        "plain {plain:?} ({baseline:?}), batch {ours:?} ({batch:?}), {:.2} ×",
        ours.as_secs_f64() / plain.as_secs_f64()
    );
}

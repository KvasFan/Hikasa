//! Gate G7 copy (§1.6.22, D-192), by hand and in a release build (`--ignored --nocapture`): `folder_op_execute` of a
//! prepared plan that copies 10,000 files of 4 KiB in 100 folders, local → local, targets absent, against a plain recursive
//! copy in the same test (`fs::copy` + `File::set_modified`, no `fsync`, no temporary name), median of three each, on one
//! file system (the temporary folder). The comparison and the plan are made before the clock starts: they are not part
//! of the gate (G2 has the comparison). Threshold: at most 1.5 ×. A failure is reported with its figures, not relaxed.

mod common;

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, Instant};

use common::{facts, fixed_clock, job, os};
use hikasa_core::{JobKind, Side};
use hikasa_folder::NullObserver;
use hikasa_folder::ops::{FolderOp, ReadOnlySides};
use hikasa_session::api::{
    FolderCompareRequest, FolderOpRequest, OpOverrides, OpRequest, SelectRequest,
};
use hikasa_session::{Service, ServiceInit};
use hikasa_vfs::ops::OverwritePolicy;

const FOLDERS: usize = 100;
const FILES: usize = 100;
/// G7: the copy path against a plain copy.
const GATE: f64 = 1.5;

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

/// What a plain recursive copy that keeps times does.
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

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

fn service(root: &Path) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(4),
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            std::sync::Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

#[test]
#[ignore = "gate G7, by hand, in a release build"]
fn g7_copy_of_10000_files_through_the_service() {
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let from = root.join("l");
    tree(&from);
    let service = service(&root);
    let (mut baseline, mut ours) = (Vec::new(), Vec::new());
    for round in 0..3 {
        let to = root.join(format!("plain{round}"));
        fs::create_dir(&to).unwrap();
        let start = Instant::now();
        plain(&from, &to);
        baseline.push(start.elapsed());
        fs::remove_dir_all(&to).unwrap();

        let to = root.join("r");
        fs::create_dir(&to).unwrap();
        let request = FolderCompareRequest {
            left: os(&from),
            right: os(&to),
            ..FolderCompareRequest::default()
        };
        let control = job(&service, JobKind::FolderCompare);
        let prepared = service.folder_prepare_in(&request, &control).unwrap();
        service
            .folder_run(&prepared, &control, &NullObserver)
            .unwrap();
        let op = FolderOpRequest {
            op: OpRequest::Op(FolderOp::Copy {
                to: Side::Right,
                overwrite: OverwritePolicy::Always,
            }),
            select: SelectRequest {
                masks: vec!["all".to_owned()],
                ..SelectRequest::default()
            },
            read_only: ReadOnlySides::default(),
            overrides: OpOverrides {
                backup: Some(false),
                ..OpOverrides::default()
            },
        };
        let control = job(&service, JobKind::FolderOp);
        let plan = service
            .folder_op_plan(prepared.model(), &op, &control)
            .unwrap();
        assert_eq!(plan.plan.items.len(), FOLDERS * (FILES + 1));
        let start = Instant::now();
        let outcome = service.folder_op_execute(plan, &control).unwrap();
        ours.push(start.elapsed());
        assert!(outcome.ok(), "{:?}", outcome.report.summary);
        assert_eq!(outcome.report.summary.done as usize, FOLDERS * (FILES + 1));
        drop(prepared);
        fs::remove_dir_all(&to).unwrap();
    }
    let (plain, path) = (median(baseline.clone()), median(ours.clone()));
    let ratio = path.as_secs_f64() / plain.as_secs_f64();
    println!(
        "G7 copy: plain {plain:?} ({baseline:?}), folder_op_execute {path:?} ({ours:?}), {ratio:.2} × (gate ≤ {GATE} ×)"
    );
    assert!(
        ratio <= GATE,
        "G7 copy failed: {ratio:.2} × a plain copy (gate ≤ {GATE} ×)"
    );
}

//! Bench `sync_plan_100k` (§1.6.22, D-192, gate G7 plan): `plan_sync` of a compared tree of 100,000 files on each side in
//! `MemoryVfs` (1,000 folders × 100 files), half of them differing, Mirror to Right. The comparison is made once and is
//! not part of the gate (its cost is G2's); the plan must take less than 0.5 s, median of five, in a release build.

use std::time::{Duration, Instant};

use criterion::Criterion;
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, LocalPath, NullProgress, Spec};
use hikasa_folder::ops::{OpSettings, ReadOnlySides};
use hikasa_folder::sync::{SyncOptions, SyncPreset, SyncRequest, plan_sync, sync_folder_rules};
use hikasa_folder::{
    ContentEngines, FolderComparison, FolderEnv, FolderModel, FolderRules, NullObserver,
    compare_into,
};
use hikasa_vfs::{MemMeta, MemoryVfs, StreamOptions};

const FOLDERS: usize = 1_000;
const FILES: usize = 100;
/// G7: a sync plan of 100,000 files in less than this.
const GATE: Duration = Duration::from_millis(500);

fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(std::path::PathBuf::from(host(path))).unwrap())
}

/// Both sides with 100,000 files; every second file differs (newer on the left, another size).
fn build() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let old = MemMeta {
        mtime: FileTime::from_unix(1_790_000_000, 0),
        ..MemMeta::default()
    };
    let new = MemMeta {
        mtime: FileTime::from_unix(1_790_100_000, 0),
        ..MemMeta::default()
    };
    let same: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"content"[..]);
    let other: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"changed content"[..]);
    for base in ["/l", "/r"] {
        vfs.insert_dir(&host(base), old.clone()).unwrap();
        for d in 0..FOLDERS {
            let dir = format!("{base}/folder{d:04}");
            vfs.insert_dir(&host(&dir), old.clone()).unwrap();
            for f in 0..FILES {
                let differs = f % 2 == 1;
                let (bytes, meta) = if differs && base == "/l" {
                    (std::sync::Arc::clone(&other), new.clone())
                } else {
                    (std::sync::Arc::clone(&same), old.clone())
                };
                vfs.insert_file(&host(&format!("{dir}/file{f:03}.dat")), bytes, meta)
                    .unwrap();
            }
        }
    }
    vfs
}

fn compare(vfs: &MemoryVfs, rules: &FolderRules) -> FolderComparison {
    let engines = ContentEngines::with_stream_options(StreamOptions::default(), None);
    let now = jiff::Timestamp::from_second(1_792_584_000)
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC);
    let cancel = CancelToken::never();
    let progress = NullProgress;
    let env = FolderEnv {
        vfs,
        content: &engines,
        pool: None,
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &NullObserver,
        now: &now,
        content_budget: 1 << 30,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
    compare_into(&model, &env).unwrap()
}

fn main() {
    let vfs = build();
    let options = SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    };
    let rules = sync_folder_rules(&FolderRules::default(), &options).unwrap();
    let cmp = compare(&vfs, &rules);
    let read = cmp.read();
    let request = SyncRequest {
        options: &options,
        just_selection: None,
        visible: None,
        include_hidden: false,
        iu: false,
        read_only: ReadOnlySides::default(),
        allow_empty_source: false,
        settings: OpSettings::default(),
    };
    let mut times = Vec::new();
    let mut items = 0;
    for _ in 0..5 {
        let start = Instant::now();
        let plan = plan_sync(&read, &vfs, &request, &Ctl::detached()).unwrap();
        times.push(start.elapsed());
        items = plan.op.plan.items.len();
        assert_eq!(plan.counts.copy_right, (FOLDERS * FILES / 2) as u64);
    }
    times.sort();
    let median = times[times.len() / 2];
    println!(
        "sync_plan_100k: {} rows, {items} items; median of five {median:?} ({times:?}); gate < {GATE:?}",
        read.len() - 1
    );
    assert!(median < GATE, "G7 plan: {median:?} is not below {GATE:?}");
    let mut c = Criterion::default()
        .configure_from_args()
        .sample_size(10)
        .measurement_time(Duration::from_secs(20));
    c.bench_function("sync_plan_100k/mirror_right", |b| {
        b.iter(|| {
            plan_sync(&read, &vfs, &request, &Ctl::detached()).map(|plan| plan.op.plan.items.len())
        });
    });
    c.final_summary();
}

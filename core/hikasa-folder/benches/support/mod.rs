//! Shared helpers of the folder benches: deterministic tree generation on disk (seeded xorshift64*, §6.1) and a
//! comparison runner.
//!
//! The 100k trees are written below `$HIKASA_BENCH_DIR` (default: `<temp dir>/hikasa-bench-folder`) and reused by
//! later runs (a `.complete` marker); delete that folder when done.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use hikasa_core::{CancelToken, Ctl, LocalPath, NullProgress, Spec};
use hikasa_folder::{
    ContentEngines, FolderComparison, FolderEnv, FolderModel, FolderRules, ScanObserver,
};
use hikasa_vfs::{StreamOptions, Vfs};

/// Seeded xorshift64*.
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> XorShift {
        XorShift(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// The bench root folder.
pub fn bench_root() -> PathBuf {
    std::env::var_os("HIKASA_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("hikasa-bench-folder"))
}

/// Folders per side (besides the base) and files per folder of the 100k trees.
pub const FOLDERS: usize = 10_000;
pub const FILES_PER_FOLDER: usize = 10;
/// Branching factor of the folder heap: folder `i` (1-based) lives in folder `(i - 1) / 6` (0 = the base), so
/// the depth stays ≤ 8.
const BRANCHING: usize = 6;

/// Relative path of folder `i` (0 = the base).
pub fn folder_path(i: usize) -> PathBuf {
    let mut chain = Vec::new();
    let mut k = i;
    while k != 0 {
        chain.push(k);
        k = (k - 1) / BRANCHING;
    }
    chain.iter().rev().map(|k| format!("d{k:05}")).collect()
}

/// Generates (or reuses) a pair of 100,000-file trees with 10,000 folders per side: `identical`, or with 1 % of
/// the right files changed (`one_percent`: half in size, half in modification time).
pub fn tree_100k(kind: &str) -> PathBuf {
    let dir = bench_root().join(format!("tree-100k-{kind}"));
    if dir.join(".complete").exists() {
        return dir;
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mtime = filetime::FileTime::from_unix_time(1_790_000_000, 0);
    let mut rng = XorShift::new(0x5EED_F01D);
    for side in ["l", "r"] {
        let base = dir.join(side);
        std::fs::create_dir_all(&base).unwrap();
        for i in 1..=FOLDERS {
            std::fs::create_dir_all(base.join(folder_path(i))).unwrap();
        }
    }
    let mut changed = 0usize;
    for i in 1..=FOLDERS {
        let folder = folder_path(i);
        for f in 0..FILES_PER_FOLDER {
            let name = format!("f{f:02}.txt");
            let len = 32 + (rng.next() % 96) as usize;
            let content = vec![b'a' + (f as u8 % 26); len];
            let differs = kind == "one_percent" && rng.next().is_multiple_of(100);
            for side in ["l", "r"] {
                let path = dir.join(side).join(&folder).join(&name);
                let mut bytes = content.clone();
                let mut t = mtime;
                if differs && side == "r" {
                    if changed.is_multiple_of(2) {
                        bytes.push(b'!');
                    } else {
                        t = filetime::FileTime::from_unix_time(1_790_000_000 + 3_600, 0);
                    }
                }
                std::fs::write(&path, &bytes).unwrap();
                filetime::set_file_mtime(&path, t).unwrap();
            }
            if differs {
                changed += 1;
            }
        }
    }
    std::fs::write(dir.join(".complete"), format!("{changed} changed\n")).unwrap();
    dir
}

/// A local spec.
pub fn spec(path: &Path) -> Spec {
    Spec::Local(LocalPath::new(path.to_path_buf()).unwrap())
}

/// The pool the service would use (`available_parallelism` threads; `$HIKASA_BENCH_THREADS` overrides, e.g. 8
/// for the reference laptop of §0.8).
pub fn service_pool() -> rayon::ThreadPool {
    let threads = std::env::var("HIKASA_BENCH_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(8, |n| n.get()));
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("hikasa-worker-{i}"))
        .build()
        .unwrap()
}

/// Runs one comparison of `left` and `right`.
#[allow(clippy::too_many_arguments)]
pub fn compare(
    vfs: &dyn Vfs,
    left: Spec,
    right: Spec,
    rules: &FolderRules,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
    observer: &dyn ScanObserver,
) -> (std::sync::Arc<FolderModel>, Option<FolderComparison>) {
    let engines = ContentEngines::with_stream_options(StreamOptions::default(), None);
    let now = jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC);
    let progress = NullProgress;
    let env = FolderEnv {
        vfs,
        content: &engines,
        pool,
        ctl: Ctl {
            cancel,
            progress: &progress,
        },
        observer,
        now: &now,
        content_budget: 1 << 30,
    };
    let model = FolderModel::new(left, right, rules.clone()).unwrap();
    let cmp = hikasa_folder::compare_into(&model, &env).ok();
    (model, cmp)
}

/// Gates are asserted unless `HIKASA_GATES=report` (a noisy developer machine): then they are only printed.
pub fn check_gate(gate: &str, ok: bool, detail: &str) {
    let report_only = std::env::var("HIKASA_GATES").is_ok_and(|v| v == "report");
    println!("{gate}: {detail} — {}", if ok { "pass" } else { "FAIL" });
    if !report_only {
        assert!(
            ok,
            "{gate} failed: {detail} (set HIKASA_GATES=report to only print)"
        );
    }
}

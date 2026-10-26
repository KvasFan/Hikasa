//! Bench `crc32_1gb` (gate G3, §6.4: CRC32 at ≥ 90 % of the sequential-read throughput of the same file).
//!
//! A sparse 1 GiB file is created in `HIKASA_BENCH_DIR` (default: the system temp folder) and deleted at the
//! end. Every measured function processes the whole file (reported as bytes per second):
//!
//! - `sequential_read`: the baseline, reading the file with 1 MiB `read` calls on one thread and nothing else;
//! - `crc32/auto`: `crc32_source` with the default policy (memory-mapped, 8 MiB chunks) and `threads = 4`, the
//!   configuration `hikasa-session` uses (`min(4, pool size)`, §5.1);
//! - `crc32/auto_1t`: the same on the calling thread only (`threads = 1`, the engine default);
//! - `crc32/stream`: `crc32_source` with `MmapPolicy::Never` (the streaming reference path, 1 MiB buffers).
//!
//! G3 compares `crc32/auto` with `sequential_read` (warm cache, §6.4). As in `binary_compare_2gb`, one mapped
//! pass touches every page before measuring, so both paths read the same resident pages. Each criterion sample
//! is the fastest iteration of the sample times the iteration count (see `min_time`). For the gate, run on a
//! disk file system (`HIKASA_BENCH_DIR`), not tmpfs: a page-cache `read` of a disk file system is the faster
//! baseline.

use std::fs::File;
use std::hint::black_box;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use hikasa_core::{Ctl, LocalPath, Spec};
use hikasa_vfs::{FollowLinks, LocalVfs, MmapPolicy, StreamOptions, Vfs, VfsSource, crc32_source};

/// File length: the gate size under `cargo bench` (which passes `--bench`); 64 MiB when the binary runs in
/// criterion's test mode (`cargo test --benches`), so a test run stays cheap.
fn file_len() -> u64 {
    if std::env::args().any(|arg| arg == "--bench") {
        1 << 30
    } else {
        64 << 20
    }
}

fn bench_dir() -> PathBuf {
    std::env::var_os("HIKASA_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Threads of the G3 configuration (`hikasa-session` passes `min(4, pool size)`).
const G3_THREADS: usize = 4;

fn crc(path: &Path, mmap: MmapPolicy, threads: usize) -> u32 {
    let vfs = LocalVfs::default();
    let spec = Spec::Local(LocalPath::new(path.canonicalize().unwrap()).unwrap());
    let entry = vfs.stat(&spec, FollowLinks::Yes).unwrap();
    let source = VfsSource::new(&vfs, spec, entry);
    let opts = StreamOptions {
        mmap,
        threads,
        ..StreamOptions::default()
    };
    crc32_source(&source, &opts, &Ctl::detached()).unwrap()
}

fn read_all(path: &Path, buf: &mut [u8]) -> u64 {
    let mut file = File::open(path).unwrap();
    let mut total = 0u64;
    loop {
        let n = file.read(buf).unwrap();
        if n == 0 {
            return total;
        }
        total += n as u64;
    }
}

/// Runs `f` `iters` times and returns `iters` × the fastest run: a min-estimator. The gate runs on machines that
/// are shared with other jobs, where bursts of contention make mean times unreliable; every measured function
/// (the baseline included) is timed the same way.
fn min_time(iters: u64, mut f: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..iters {
        let start = Instant::now();
        f();
        best = best.min(start.elapsed());
    }
    best.mul_f64(iters as f64)
}

fn bench(c: &mut Criterion) {
    let len = file_len();
    let dir = tempfile::Builder::new()
        .prefix("hikasa-g3-")
        .tempdir_in(bench_dir())
        .unwrap();
    let path = dir.path().join("crc.bin");
    File::create(&path).unwrap().set_len(len).unwrap();
    let auto = StreamOptions::default().mmap;
    // Warm-up (see the module documentation); also checks both paths against each other.
    let expected = crc(&path, auto, G3_THREADS);
    assert_eq!(crc(&path, auto, 1), expected);
    assert_eq!(crc(&path, MmapPolicy::Never, 1), expected);

    let mut group = c.benchmark_group("crc32_1gb");
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(15));
    group.throughput(Throughput::Bytes(len));

    let mut buf = vec![0u8; StreamOptions::DEFAULT_BUFFER_SIZE];
    group.bench_function("sequential_read", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                black_box(read_all(&path, &mut buf));
            })
        })
    });
    group.bench_function("crc32/auto", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || assert_eq!(crc(&path, auto, G3_THREADS), expected))
        })
    });
    group.bench_function("crc32/auto_1t", |bench| {
        bench.iter_custom(|iters| min_time(iters, || assert_eq!(crc(&path, auto, 1), expected)))
    });
    group.bench_function("crc32/stream", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                assert_eq!(crc(&path, MmapPolicy::Never, 1), expected)
            })
        })
    });
    group.finish();
    drop(dir);
}

criterion_group!(benches, bench);
criterion_main!(benches);

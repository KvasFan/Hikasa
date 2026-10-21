//! Bench `binary_compare_2gb` (gate G3, §6.4: binary compare streams at ≥ 90 % of the sequential-read
//! throughput of the same files).
//!
//! Two sparse 2 GiB files are created in `HIKASA_BENCH_DIR` (default: the system temp folder) and deleted at
//! the end. Every measured function processes both files completely (4 GiB per iteration, reported as bytes
//! per second):
//!
//! - `sequential_read`: the baseline, reading both files with 1 MiB `read` calls on one thread and nothing else;
//! - `identical/auto`: `compare_sources` with the default policy (memory-mapped, 8 MiB chunks) and
//!   `threads = 4`, the configuration `hikasa-session` uses (`min(4, pool size)`, §5.1);
//! - `identical/auto_1t`: the same on the calling thread only (`threads = 1`, the engine default);
//! - `identical/stream`: `MmapPolicy::Never` (the streaming reference path, 1 MiB buffers);
//! - `last_byte/{auto,stream}`: `auto` (4 threads) and `stream` after the last byte of the second file was
//!   changed.
//!
//! G3 compares `identical/auto` with `sequential_read` (warm cache, §6.4). Before measuring, one mapped pass
//! touches every page: on file systems that keep sparse holes unallocated in the page cache (tmpfs), the first
//! mapping of a hole allocates a page, while `read` of a hole copies a shared zero page; after the warm-up both
//! paths read the same resident pages. For the gate, run on a disk file system (`HIKASA_BENCH_DIR`), not tmpfs:
//! a page-cache `read` of a disk file system is the faster baseline.
//!
//! Each criterion sample is the fastest iteration of the sample times the iteration count (see `min_time`).

use std::fs::{File, OpenOptions};
use std::hint::black_box;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use criterion::{Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use hikasa_core::{Ctl, LocalPath, Spec};
use hikasa_vfs::{
    BinaryOutcome, FollowLinks, LocalVfs, MmapPolicy, StreamOptions, Vfs, VfsSource,
    compare_sources,
};

/// File length: the gate size under `cargo bench` (which passes `--bench`); 64 MiB when the binary runs in
/// criterion's test mode (`cargo test --benches`), so a test run stays cheap.
fn file_len() -> u64 {
    if std::env::args().any(|arg| arg == "--bench") {
        2 << 30
    } else {
        64 << 20
    }
}

fn bench_dir() -> PathBuf {
    std::env::var_os("HIKASA_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// A sparse file of `len` zero bytes.
fn sparse(path: &Path, len: u64) {
    let file = File::create(path).unwrap();
    file.set_len(len).unwrap();
}

fn spec(path: &Path) -> Spec {
    Spec::Local(LocalPath::new(path.canonicalize().unwrap()).unwrap())
}

fn source<'a>(vfs: &'a LocalVfs, path: &Path) -> VfsSource<'a> {
    let spec = spec(path);
    let entry = vfs.stat(&spec, FollowLinks::Yes).unwrap();
    VfsSource::new(vfs, spec, entry)
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

/// Threads of the G3 configuration (`hikasa-session` passes `min(4, pool size)`).
const G3_THREADS: usize = 4;

fn compare(a: &Path, b: &Path, mmap: MmapPolicy, threads: usize) -> BinaryOutcome {
    let vfs = LocalVfs::default();
    let (sa, sb) = (source(&vfs, a), source(&vfs, b));
    let opts = StreamOptions {
        mmap,
        threads,
        ..StreamOptions::default()
    };
    compare_sources(&sa, &sb, &opts, &Ctl::detached()).unwrap()
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
    let a = dir.path().join("a.bin");
    let b = dir.path().join("b.bin");
    sparse(&a, len);
    sparse(&b, len);
    let auto = StreamOptions::default().mmap;
    // Warm-up: touch every page through the mapped path (see the module documentation).
    assert_eq!(compare(&a, &b, auto, G3_THREADS), BinaryOutcome::Same);

    let mut group = c.benchmark_group("binary_compare_2gb");
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(20));
    group.throughput(Throughput::Bytes(2 * len));

    let mut buf = vec![0u8; StreamOptions::DEFAULT_BUFFER_SIZE];
    group.bench_function("sequential_read", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                black_box(read_all(&a, &mut buf) + read_all(&b, &mut buf));
            })
        })
    });
    group.bench_function("identical/auto", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                assert_eq!(compare(&a, &b, auto, G3_THREADS), BinaryOutcome::Same)
            })
        })
    });
    group.bench_function("identical/auto_1t", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                assert_eq!(compare(&a, &b, auto, 1), BinaryOutcome::Same)
            })
        })
    });
    group.bench_function("identical/stream", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                assert_eq!(compare(&a, &b, MmapPolicy::Never, 1), BinaryOutcome::Same)
            })
        })
    });

    // Change the last byte of the second file.
    {
        let mut file = OpenOptions::new().write(true).open(&b).unwrap();
        file.seek(SeekFrom::Start(len - 1)).unwrap();
        file.write_all(&[1]).unwrap();
    }
    let expected = BinaryOutcome::Different {
        first_difference: Some(len - 1),
    };
    group.bench_function("last_byte/auto", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                assert_eq!(compare(&a, &b, auto, G3_THREADS), expected)
            })
        })
    });
    group.bench_function("last_byte/stream", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                assert_eq!(compare(&a, &b, MmapPolicy::Never, 1), expected)
            })
        })
    });
    group.finish();
    drop(dir);
}

criterion_group!(benches, bench);
criterion_main!(benches);

//! Bench `archive_crc_1gb` (gate G6 CRC, contract M2 §1.5.16, D-155): `ArchiveVfs::prepare` of a 1 GiB `tar.gz` of one
//! member, repeating pseudo-random text with a ratio of about 4:1, the CRC in the index, against decoding the same
//! stream to a sink with `MultiGzDecoder`. The gate: `index/prepare` ≤ 1.3 × `raw_decode`.
//!
//! The fixture is generated in `HIKASA_BENCH_DIR` (default: the system temp folder) and deleted at the end. A
//! sample is the fastest iteration times the iteration count, as in `crc32_1gb`: the gate runs on machines that are
//! shared with other jobs, and both functions are timed the same way. The ratio is printed at the end.

use std::fs::File;
use std::hint::black_box;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use hikasa_core::{Ctl, LocalPath, Spec};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, LocalVfs, Vfs,
};

/// Member length: the gate size under `cargo bench`; 64 MiB in criterion's test mode.
fn member_len() -> u64 {
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

/// One buffer of pseudo-random lowercase text over a small alphabet, so deflate reaches about 4:1.
fn text_block() -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..1 << 20)
        .map(|at: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            if at % 64 == 63 {
                b'\n'
            } else {
                b'a' + (state % 7) as u8
            }
        })
        .collect()
}

/// Writes the fixture: a tar of one member through gzip level 6.
fn fixture(path: &Path, len: u64) {
    let file = File::create(path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::new(6));
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(len);
    header.set_mode(0o644);
    header.set_mtime(1_790_000_000);
    let block = text_block();
    let mut left = len;
    let content = std::iter::from_fn(|| {
        (left > 0).then(|| {
            let take = left.min(block.len() as u64);
            left -= take;
            std::io::Cursor::new(block[..take as usize].to_vec())
        })
    })
    .fold(Box::new(std::io::empty()) as Box<dyn Read>, |all, part| {
        Box::new(all.chain(part))
    });
    builder
        .append_data(&mut header, "member.txt", content)
        .unwrap();
    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap().flush().unwrap();
}

/// The raw decode: the same stream to a sink.
fn raw_decode(path: &Path, buf: &mut [u8]) -> u64 {
    let file = BufReader::with_capacity(128 << 10, File::open(path).unwrap());
    let mut decoder = flate2::bufread::MultiGzDecoder::new(file);
    let mut total = 0u64;
    loop {
        let n = decoder.read(buf).unwrap();
        if n == 0 {
            return total;
        }
        total += n as u64;
    }
}

/// `prepare` on a fresh back-end: the whole index pass.
fn prepare(spec: &Spec, cache: &Path) {
    let masks = ArchiveMasks::new([(ArchiveFormat::TarGz, "*.tar.gz".to_owned())]).unwrap();
    let vfs = ArchiveVfs::new(
        Arc::new(LocalVfs::default()) as Arc<dyn Vfs>,
        ArchiveConfig {
            masks,
            limits: ArchiveLimits {
                ratio_max: 0,
                ..ArchiveLimits::default()
            },
            clock: Arc::new(|| jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC)),
            cache_root: cache.to_path_buf(),
        },
    );
    vfs.prepare(spec, &Ctl::detached()).unwrap();
}

/// Runs `f` `iters` times and returns `iters` × the fastest run.
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
    let len = member_len();
    let dir = tempfile::Builder::new()
        .prefix("hikasa-g6-")
        .tempdir_in(bench_dir())
        .unwrap();
    let path = dir.path().canonicalize().unwrap().join("big.tar.gz");
    fixture(&path, len);
    let compressed = std::fs::metadata(&path).unwrap().len();
    println!("fixture: {len} bytes, {compressed} compressed");
    let spec = Spec::archive_member(Spec::Local(LocalPath::new(path.clone()).unwrap()), "");
    let cache = dir.path().join("cache");
    let mut buf = vec![0u8; 128 << 10];

    let mut group = c.benchmark_group("archive_crc_1gb");
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.measurement_time(Duration::from_secs(30));
    group.throughput(Throughput::Bytes(len));
    group.bench_function("raw_decode", |bench| {
        bench.iter_custom(|iters| {
            min_time(iters, || {
                black_box(raw_decode(&path, &mut buf));
            })
        })
    });
    group.bench_function("index/prepare", |bench| {
        bench.iter_custom(|iters| min_time(iters, || prepare(&spec, &cache)))
    });
    group.finish();

    // The gate's own figure: the best of five runs of each, on the same file.
    let best = |f: &mut dyn FnMut()| {
        (0..5)
            .map(|_| {
                let start = Instant::now();
                f();
                start.elapsed()
            })
            .min()
            .unwrap()
    };
    let raw = best(&mut || {
        black_box(raw_decode(&path, &mut buf));
    });
    let index = best(&mut || prepare(&spec, &cache));
    println!(
        "G6 CRC: raw decode {:.3} s, index {:.3} s, ratio {:.3} (gate 1.3)",
        raw.as_secs_f64(),
        index.as_secs_f64(),
        index.as_secs_f64() / raw.as_secs_f64()
    );
    drop(dir);
}

criterion_group!(benches, bench);
criterion_main!(benches);

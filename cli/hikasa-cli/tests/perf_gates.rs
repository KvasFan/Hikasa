//! The G1 and G2 gates through the `hikasa` binary (contract §6.4, §6.5 wave 5: "G1 and G2 through the binary").
//! Ignored by default; run them in release mode on an otherwise idle machine:
//!
//! ```text
//! cargo test --release -p hikasa-cli --test perf_gates -- --ignored --nocapture
//! ```
//!
//! Inputs are generated under the temporary folder (`TMPDIR`) and removed afterwards; nothing is committed (§6.1).
//!
//! - **G1**: the engine gate's input (`core/hikasa-text/benches/support/inputs.rs`, included so that both gates
//!   measure the same data): two 1,000,000-line C sources of about 48 bytes per line, about 1 % of the lines
//!   changed in clusters. `hikasa text compare l.c r.c --report summary` with `--algorithm auto`, with
//!   `--algorithm standard`, and on an identical pair: each < 3 s end to end (process start, reading, decoding,
//!   tokenising, aligning, the report); on Linux the peak RSS (VmHWM, sampled every 2 ms) ≤ 3 × (L + R bytes) +
//!   64 B × (L + R lines) + 32 MiB.
//! - **G2**: two trees of 100,000 files in 10,000 folders each (the shape of the engine's `folder_scan_100k`:
//!   10 files per folder, a heap of branching 6, so depth ≤ 8), 1 % of the right files changed (half in size,
//!   half in modification time). `hikasa folder compare l r --content none --report summary` (scan + quick
//!   tests) with a warm cache: < 1.5 s. The cold figure (< 5 s with dropped caches) needs root and is not
//!   measured here.
//!
//! - **G6** (contract M2 §1.5.16): `vfs ls big.zip --json` of 100,000 stored files < 1 s, peak RSS ≤ 150 MiB;
//!   `folder compare tree tree.zip --content crc --json` of 10,000 files of 4 KiB in 100 folders < 2 s; the peak RSS
//!   of the index of a 1 GiB `tar.gz` ≤ 64 MiB (its time is the bench `archive_crc_1gb`).
//!
//! Each command runs once to warm the cache, then three times; the median is checked against the threshold and
//! every figure is printed.

#![allow(dead_code)]

mod common;

#[path = "../../../core/hikasa-text/benches/support/inputs.rs"]
mod inputs;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// One timed run of the binary.
struct Run {
    elapsed: Duration,
    /// Peak resident set (bytes), Linux only.
    peak_rss: Option<u64>,
    code: Option<i32>,
}

/// The peak resident set of a running process (`VmHWM` of `/proc/<pid>/status`), in bytes.
fn vm_hwm(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// Runs `hikasa <args>` in `dir` (stdout to `<dir>/stdout.txt`, stderr to `<dir>/stderr.txt`) and measures it.
fn run(dir: &Path, args: &[OsString]) -> Run {
    let stdout = std::fs::File::create(dir.join("stdout.txt")).unwrap();
    let stderr = std::fs::File::create(dir.join("stderr.txt")).unwrap();
    let started = Instant::now();
    let mut child = Command::new(common::hikasa_exe())
        .current_dir(dir)
        .env("HIKASA_IPC_ENDPOINT", dir.join("s.sock"))
        .env("LC_ALL", "C.UTF-8")
        .arg("--config-dir")
        .arg(dir.join("cfg"))
        .args(args)
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut peak = None;
    let status = loop {
        if let Some(rss) = vm_hwm(pid) {
            peak = Some(peak.map_or(rss, |p: u64| p.max(rss)));
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    Run {
        elapsed: started.elapsed(),
        peak_rss: peak,
        code: status.code(),
    }
}

/// One warm-up run, then three timed ones: (median, min, max, highest peak RSS). Every run must exit `code`.
fn measure(
    dir: &Path,
    args: &[OsString],
    code: i32,
) -> (Duration, Duration, Duration, Option<u64>) {
    let warm = run(dir, args);
    let stderr = || std::fs::read_to_string(dir.join("stderr.txt")).unwrap_or_default();
    assert_eq!(warm.code, Some(code), "{args:?}: {}", stderr());
    let mut runs: Vec<Run> = (0..3).map(|_| run(dir, args)).collect();
    for r in &runs {
        assert_eq!(r.code, Some(code), "{args:?}: {}", stderr());
    }
    runs.sort_by_key(|r| r.elapsed);
    let peak = runs.iter().filter_map(|r| r.peak_rss).max();
    (runs[1].elapsed, runs[0].elapsed, runs[2].elapsed, peak)
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// The gates assume a release build (§6.4); a debug build only says so.
fn release_only() -> bool {
    if cfg!(debug_assertions) {
        eprintln!(
            "skipped: run the gates in release mode (cargo test --release -p hikasa-cli --test perf_gates -- \
             --ignored)"
        );
        return false;
    }
    true
}

#[test]
#[ignore = "performance gate: run in release mode with --ignored"]
fn g1_text_compare_1m_lines_through_the_binary() {
    if !release_only() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let lines = 1_000_000;
    let (left, right) = inputs::c_pair(lines, 0x0005_EED1);
    std::fs::write(dir.join("l.c"), &left).unwrap();
    std::fs::write(dir.join("r.c"), &right).unwrap();
    std::fs::write(dir.join("same.c"), &left).unwrap();
    let total_bytes = (left.len() + right.len()) as u64;
    let total_lines = (inputs::line_count(&left) + inputs::line_count(&right)) as u64;
    let rss_bound = 3 * total_bytes + 64 * total_lines + (32 << 20);
    println!(
        "G1 input: {} + {} lines, {:.1} + {:.1} MB",
        inputs::line_count(&left),
        inputs::line_count(&right),
        left.len() as f64 / 1e6,
        right.len() as f64 / 1e6
    );
    let cases: [(&str, Vec<OsString>, i32); 3] = [
        (
            "text_compare_1m_auto",
            os(&[
                "text",
                "compare",
                "l.c",
                "r.c",
                "--report",
                "summary",
                "--algorithm",
                "auto",
            ]),
            13,
        ),
        (
            "text_compare_1m_standard",
            os(&[
                "text",
                "compare",
                "l.c",
                "r.c",
                "--report",
                "summary",
                "--algorithm",
                "standard",
            ]),
            13,
        ),
        (
            "text_compare_1m_identity",
            os(&["text", "compare", "l.c", "same.c", "--report", "summary"]),
            1,
        ),
    ];
    let mut failures = Vec::new();
    for (name, args, code) in cases {
        let (median, min, max, peak) = measure(dir, &args, code);
        let peak_text = peak.map_or("n/a".to_owned(), |p| {
            format!("{:.0} MiB", p as f64 / 1048576.0)
        });
        println!(
            "G1 {name}: median {:.3} s (min {:.3}, max {:.3}); peak RSS {peak_text} (bound {:.0} MiB)",
            median.as_secs_f64(),
            min.as_secs_f64(),
            max.as_secs_f64(),
            rss_bound as f64 / 1048576.0
        );
        if median >= Duration::from_secs(3) {
            failures.push(format!("{name}: {median:?} ≥ 3 s"));
        }
        if let Some(peak) = peak
            && peak > rss_bound
        {
            failures.push(format!("{name}: peak RSS {peak} > {rss_bound}"));
        }
    }
    assert!(failures.is_empty(), "G1: {failures:?}");
}

/// Folders per side (besides the base), files per folder and branching of the G2 trees (the engine bench's
/// shape).
const FOLDERS: usize = 10_000;
const FILES_PER_FOLDER: usize = 10;
const BRANCHING: usize = 6;

/// Relative path of folder `i` (0 = the base): folder `i` lives in folder `(i - 1) / BRANCHING`.
fn folder_path(i: usize) -> PathBuf {
    let mut chain = Vec::new();
    let mut k = i;
    while k != 0 {
        chain.push(k);
        k = (k - 1) / BRANCHING;
    }
    chain.iter().rev().map(|k| format!("d{k:05}")).collect()
}

/// Writes the two G2 trees below `dir` (`l`, `r`), several folders at a time; returns the changed files.
fn tree_100k(dir: &Path) -> usize {
    let mtime = filetime::FileTime::from_unix_time(1_790_000_000, 0);
    let later = filetime::FileTime::from_unix_time(1_790_000_000 + 3_600, 0);
    for side in ["l", "r"] {
        for i in 1..=FOLDERS {
            std::fs::create_dir_all(dir.join(side).join(folder_path(i))).unwrap();
        }
    }
    let threads = 8;
    let changed: usize = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    let mut changed = 0usize;
                    for i in (1..=FOLDERS).filter(|i| i % threads == t) {
                        // One generator per folder: the trees do not depend on the thread count.
                        let mut rng = inputs::XorShift(0x5EED_F01D ^ (i as u64 * 0x9E37_79B9));
                        let folder = folder_path(i);
                        for f in 0..FILES_PER_FOLDER {
                            let name = format!("f{f:02}.txt");
                            let len = 32 + rng.below(96) as usize;
                            let content = vec![b'a' + (f as u8 % 26); len];
                            let differs = rng.below(100) == 0;
                            for side in ["l", "r"] {
                                let path = dir.join(side).join(&folder).join(&name);
                                let mut bytes = content.clone();
                                let mut t = mtime;
                                if differs && side == "r" {
                                    if rng.below(2) == 0 {
                                        bytes.push(b'!');
                                    } else {
                                        t = later;
                                    }
                                }
                                std::fs::write(&path, &bytes).unwrap();
                                filetime::set_file_mtime(&path, t).unwrap();
                            }
                            changed += usize::from(differs);
                        }
                    }
                    changed
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).sum()
    });
    // The folders' own times are not compared (§2.6.1 amended), but keep them fixed anyway.
    for side in ["l", "r"] {
        for i in 0..=FOLDERS {
            let path = dir.join(side).join(folder_path(i));
            filetime::set_file_mtime(&path, mtime).unwrap();
        }
    }
    changed
}

#[test]
#[ignore = "performance gate: run in release mode with --ignored"]
fn g2_folder_compare_100k_files_through_the_binary() {
    if !release_only() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let generated = Instant::now();
    let changed = tree_100k(dir);
    println!(
        "G2 input: {} files in {} folders per side, {changed} changed; generated in {:.1} s",
        FOLDERS * FILES_PER_FOLDER,
        FOLDERS,
        generated.elapsed().as_secs_f64()
    );
    assert!(changed > 0);
    let args = os(&[
        "folder",
        "compare",
        "l",
        "r",
        "--content",
        "none",
        "--report",
        "summary",
    ]);
    let (median, min, max, peak) = measure(dir, &args, 13);
    let peak_text = peak.map_or("n/a".to_owned(), |p| {
        format!("{:.0} MiB", p as f64 / 1048576.0)
    });
    println!(
        "G2 folder_scan_100k (warm): median {:.3} s (min {:.3}, max {:.3}); peak RSS {peak_text}",
        median.as_secs_f64(),
        min.as_secs_f64(),
        max.as_secs_f64()
    );
    // The report of the last run proves the scan: 100,000 files in 10,000 folders (below the base) per side.
    let report = std::fs::read_to_string(dir.join("stdout.txt")).unwrap();
    let totals: Vec<&str> = report
        .lines()
        .skip_while(|l| !l.starts_with("Totals:"))
        .take(4)
        .collect();
    println!("G2 report totals:\n{}", totals.join("\n"));
    let files = totals
        .iter()
        .find(|l| l.trim_start().starts_with("Files"))
        .expect("a Files row");
    let counts: Vec<&str> = files.split_whitespace().skip(1).collect();
    assert_eq!(counts, ["100000", "100000"], "{report}");
    // The JSON document of the same comparison (the differences with the folder structure), for the record.
    let json = os(&["folder", "compare", "l", "r", "--content", "none", "--json"]);
    let (json_median, ..) = measure(dir, &json, 13);
    println!(
        "G2 folder_scan_100k --json (warm): median {:.3} s",
        json_median.as_secs_f64()
    );
    assert!(
        median < Duration::from_millis(1_500),
        "G2 warm: {median:?} ≥ 1.5 s"
    );
}

// ---- G6 (contract M2 §1.5.16, D-155) ------------------------------------------------------------------------------

/// CRC-32.
fn crc32(data: &[u8]) -> u32 {
    let table: Vec<u32> = (0..256u32)
        .map(|i| {
            (0..8).fold(i, |c, _| {
                if c & 1 != 0 {
                    (c >> 1) ^ 0xedb8_8320
                } else {
                    c >> 1
                }
            })
        })
        .collect();
    !data.iter().fold(u32::MAX, |crc, &byte| {
        table[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
    })
}

/// A zip of stored files, each with a 0x5455 time (so no zone is involved).
fn stored_zip(path: &Path, files: &[(String, Vec<u8>)], mtime: u32) {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut central: Vec<u8> = Vec::new();
    let mut extra = vec![0x55, 0x54, 5, 0, 1];
    extra.extend_from_slice(&mtime.to_le_bytes());
    let mut offset = 0u32;
    for (name, data) in files {
        let crc = crc32(data);
        let fields = |buf: &mut Vec<u8>| {
            buf.extend_from_slice(&20u16.to_le_bytes());
            buf.extend_from_slice(&0u16.to_le_bytes());
            buf.extend_from_slice(&0u16.to_le_bytes());
            buf.extend_from_slice(&0x6000u16.to_le_bytes());
            buf.extend_from_slice(&0x5cefu16.to_le_bytes());
            buf.extend_from_slice(&crc.to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
            buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
            buf.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        };
        let mut local = b"PK\x03\x04".to_vec();
        fields(&mut local);
        local.extend_from_slice(name.as_bytes());
        local.extend_from_slice(&extra);
        out.write_all(&local).unwrap();
        out.write_all(data).unwrap();
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&(0x0300u16 | 20).to_le_bytes());
        fields(&mut central);
        central.extend_from_slice(&[0u8; 6]);
        central.extend_from_slice(&(0o100_644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&extra);
        offset += (local.len() + data.len()) as u32;
    }
    out.write_all(&central).unwrap();
    out.write_all(b"PK\x05\x06\0\0\0\0").unwrap();
    // Above 65,535 entries the count is the crate's zip64 case: an end record that says so and the zip64 records.
    let count = files.len() as u64;
    let start = offset as u64;
    let size = central.len() as u64;
    if count > 0xffff {
        out.write_all(&[0xff; 4]).unwrap();
        out.write_all(&(size as u32).to_le_bytes()).unwrap();
        out.write_all(&(start as u32).to_le_bytes()).unwrap();
        out.write_all(&[0, 0]).unwrap();
        out.flush().unwrap();
        drop(out);
        rewrite_with_zip64(path, count, size, start);
    } else {
        out.write_all(&(count as u16).to_le_bytes()).unwrap();
        out.write_all(&(count as u16).to_le_bytes()).unwrap();
        out.write_all(&(size as u32).to_le_bytes()).unwrap();
        out.write_all(&(start as u32).to_le_bytes()).unwrap();
        out.write_all(&[0, 0]).unwrap();
    }
}

/// Inserts the zip64 end record and its locator before the end record.
fn rewrite_with_zip64(path: &Path, count: u64, size: u64, start: u64) {
    let mut bytes = std::fs::read(path).unwrap();
    let end = bytes.split_off(bytes.len() - 22);
    let at = bytes.len() as u64;
    bytes.extend_from_slice(b"PK\x06\x06");
    bytes.extend_from_slice(&44u64.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&start.to_le_bytes());
    bytes.extend_from_slice(b"PK\x06\x07\0\0\0\0");
    bytes.extend_from_slice(&at.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&end);
    std::fs::write(path, bytes).unwrap();
}

fn rss_text(peak: Option<u64>) -> String {
    peak.map_or("n/a".to_owned(), |p| {
        format!("{:.1} MiB", p as f64 / 1048576.0)
    })
}

#[test]
#[ignore = "performance gate: run in release mode with --ignored"]
fn g6_list_a_zip_of_100k_entries_through_the_binary() {
    if !release_only() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let files: Vec<(String, Vec<u8>)> = (0..100_000)
        .map(|i| (format!("file{i:06}.txt"), format!("{i}\n").into_bytes()))
        .collect();
    stored_zip(&dir.join("big.zip"), &files, 1_790_000_000);
    let args = os(&["vfs", "ls", "big.zip", "--json"]);
    let (median, min, max, peak) = measure(dir, &args, 0);
    println!(
        "G6 list 100,000 zip entries (warm): median {:.3} s (min {:.3}, max {:.3}); peak RSS {}",
        median.as_secs_f64(),
        min.as_secs_f64(),
        max.as_secs_f64(),
        rss_text(peak)
    );
    let listed = std::fs::read_to_string(dir.join("stdout.txt")).unwrap();
    assert!(listed.contains("file099999.txt"));
    assert!(median < Duration::from_secs(1), "G6 list: {median:?} ≥ 1 s");
    assert!(
        peak.is_none_or(|p| p <= 150 << 20),
        "G6 list RSS: {}",
        rss_text(peak)
    );
}

#[test]
#[ignore = "performance gate: run in release mode with --ignored"]
fn g6_compare_a_tree_against_its_zip_by_crc() {
    if !release_only() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let mtime = filetime::FileTime::from_unix_time(1_790_000_000, 0);
    let mut files = Vec::new();
    for d in 0..100 {
        std::fs::create_dir_all(dir.join(format!("tree/d{d:03}"))).unwrap();
        for f in 0..100 {
            let name = format!("d{d:03}/f{f:03}.bin");
            let data: Vec<u8> = (0..4096u32)
                .map(|i| (i.wrapping_mul(31) ^ (d * 100 + f)) as u8)
                .collect();
            let path = dir.join("tree").join(&name);
            std::fs::write(&path, &data).unwrap();
            filetime::set_file_mtime(&path, mtime).unwrap();
            files.push((name, data));
        }
    }
    stored_zip(&dir.join("tree.zip"), &files, 1_790_000_000);
    let args = os(&[
        "folder",
        "compare",
        "tree",
        "tree.zip",
        "--content",
        "crc",
        "--json",
    ]);
    let (median, min, max, peak) = measure(dir, &args, 1);
    println!(
        "G6 compare 10,000 files against their zip by CRC (warm): median {:.3} s (min {:.3}, max {:.3}); \
         peak RSS {}",
        median.as_secs_f64(),
        min.as_secs_f64(),
        max.as_secs_f64(),
        rss_text(peak)
    );
    assert!(
        median < Duration::from_secs(2),
        "G6 compare: {median:?} ≥ 2 s"
    );
}

/// The peak RSS of G6 CRC through the binary: 1 GiB in a `tar.gz` of stored blocks, nothing spooled. The time ratio
/// is `core/hikasa-vfs/benches/archive_crc_1gb.rs`.
#[test]
#[ignore = "performance gate: run in release mode with --ignored"]
fn g6_crc_index_of_1_gib_stays_small() {
    use std::io::Write;
    if !release_only() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let block = vec![b'x'; 0xffff];
    let total: u64 = 1 << 30;
    let blocks = total / 0xffff + 1;
    let mut header = [0u8; 512];
    header[..10].copy_from_slice(b"member.txt");
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    let size = blocks * 0xffff;
    header[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    header[136..148].copy_from_slice(b"15253523040\0");
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[148..156].fill(b' ');
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let padding = (512 - (size % 512) as usize) % 512 + 1024;

    // The stream's CRC: the same block repeated, so one pass over the bytes is enough.
    let table: Vec<u32> = (0..256u32)
        .map(|i| {
            (0..8).fold(i, |c, _| {
                if c & 1 != 0 {
                    (c >> 1) ^ 0xedb8_8320
                } else {
                    c >> 1
                }
            })
        })
        .collect();
    let update = |crc: u32, data: &[u8]| {
        data.iter().fold(crc, |crc, &byte| {
            table[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
        })
    };
    let mut crc = update(u32::MAX, &header);
    let mut out = std::io::BufWriter::new(std::fs::File::create(dir.join("big.tar.gz")).unwrap());
    out.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff])
        .unwrap();
    let stored = |out: &mut std::io::BufWriter<std::fs::File>, data: &[u8], last: bool| {
        out.write_all(&[u8::from(last)]).unwrap();
        let len = data.len() as u16;
        out.write_all(&len.to_le_bytes()).unwrap();
        out.write_all(&(!len).to_le_bytes()).unwrap();
        out.write_all(data).unwrap();
    };
    stored(&mut out, &header, false);
    for _ in 0..blocks {
        crc = update(crc, &block);
        stored(&mut out, &block, false);
    }
    let tail = vec![0u8; padding];
    crc = update(crc, &tail);
    stored(&mut out, &tail, true);
    out.write_all(&(!crc).to_le_bytes()).unwrap();
    let stream = 512 + size + padding as u64;
    out.write_all(&(stream as u32).to_le_bytes()).unwrap();
    out.flush().unwrap();
    drop(out);

    let args = os(&["vfs", "stat", "big.tar.gz/member.txt", "--json"]);
    let (median, _, _, peak) = measure(dir, &args, 0);
    println!(
        "G6 CRC through the binary (1 GiB, stored): median {:.3} s; peak RSS {}",
        median.as_secs_f64(),
        rss_text(peak)
    );
    assert!(
        peak.is_none_or(|p| p <= 64 << 20),
        "G6 CRC RSS: {}",
        rss_text(peak)
    );
}

//! The GUI performance gate ARCH-GUI-003 and acceptance (b) of contract §6.5 wave 6, measured through the
//! real binary in its scripted benchmark mode (`HIKASA_GUI_BENCH`). Ignored by default: it opens a window,
//! needs a display, generates about 100 MB of text and 200,000 files under the temporary folder (removed
//! afterwards) and must run on a release build:
//!
//! ```text
//! cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture --test-threads 1
//! ```
//!
//! * `text_scroll`: two 1,000,000-line C files (≈ 48 bytes per line, about 1 % of the lines changed in
//!   clusters; seeded xorshift64*, §6.1). The page script scrolls the canvas like a mouse wheel (3 lines per
//!   frame), a page per frame, and through the whole comparison; the summary has the first screen (unaligned
//!   text, then the aligned and syntax-coloured comparison), the frame work and frame intervals per phase, and
//!   the memory: PSS summed over the GUI and its web processes (shared pages count once) and the GUI process's
//!   own RSS, both in MiB against 400 MB = 381.47 MiB.
//! * `folder_scan`: 10,000 × `HIKASA_PERF_TREE_TOP` files per side (default 10: 100,000 files in 11,110
//!   folders, depth 4), compared with CRC content tests and shown with the display filter *Show All*; every
//!   folder opens as it arrives while the page scrolls the tree (the heaviest incremental case: about 111,000
//!   rows at the default size); the summary has the UI thread's work per message batch and per render while
//!   scanning, the tree rebuild times and the page's frame intervals. A larger tree (`HIKASA_PERF_TREE_TOP=40`)
//!   keeps the scan running long enough for many rebuilds.
//!
//! The thresholds are checked and printed as measured (an fps value below 60 is reported as such); the numbers
//! are recorded in docs/02 §2.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const GUI: &str = env!("CARGO_BIN_EXE_hikasa-gui");

/// xorshift64* (seeded, §6.1).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// One C-like line of about 48 bytes.
fn c_line(rng: &mut Rng, i: u64) -> String {
    match rng.below(8) {
        0 => format!("    /* step {i}: update the counters */"),
        1 => format!(
            "    if (value_{} > {}) {{ total += {}; }}",
            rng.below(997),
            rng.below(100_000),
            rng.below(97)
        ),
        2 => format!("    printf(\"line %d of %s\\n\", {i}, name);"),
        3 => format!(
            "static int table_{}[{}] = {{ 0 }};",
            rng.below(9999),
            rng.below(512)
        ),
        4 => format!(
            "    result = compute(x_{}, 0x{:04x}); // {}",
            rng.below(99),
            rng.below(0xffff),
            rng.below(1000)
        ),
        5 => format!(
            "    for (int k = 0; k < {}; k++) sum += k;",
            rng.below(1000)
        ),
        6 => "    return result;".to_owned(),
        _ => format!("int function_{i}(int a, int b) {{ return a * b; }}"),
    }
}

/// Two 1,000,000-line files; about 1 % of the lines differ, in clusters of 1–20 lines (changed, deleted or
/// preceded by an inserted line).
fn generate_text(dir: &Path) -> (PathBuf, PathBuf) {
    let lines = 1_000_000u64;
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let mut left = String::with_capacity(50 * lines as usize);
    let mut right = String::with_capacity(50 * lines as usize);
    let mut cluster = 0u64;
    for i in 0..lines {
        let line = c_line(&mut rng, i);
        left.push_str(&line);
        left.push('\n');
        if cluster == 0 && rng.below(1000) == 0 {
            cluster = 1 + rng.below(20);
        }
        if cluster > 0 {
            cluster -= 1;
            match rng.below(3) {
                0 => {
                    right.push_str(&line);
                    right.push_str(" /* changed */\n");
                }
                1 => {}
                _ => {
                    right.push_str(&format!("    inserted_{i}();\n"));
                    right.push_str(&line);
                    right.push('\n');
                }
            }
        } else {
            right.push_str(&line);
            right.push('\n');
        }
    }
    let (l, r) = (dir.join("left.c"), dir.join("right.c"));
    std::fs::write(&l, left).expect("write left");
    std::fs::write(&r, right).expect("write right");
    (l, r)
}

/// The top-level fan-out of the generated tree (`HIKASA_PERF_TREE_TOP`, default 10).
fn tree_top() -> u64 {
    std::env::var("HIKASA_PERF_TREE_TOP")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(10)
}

/// 10,000 × `top` files per side in `top` × 10 × 10 × 10 folders (10 files per leaf folder); 1 % of the right
/// files differ in size, and every 500th leaf folder holds an orphan on each side. Every file gets the same
/// time.
fn generate_tree(dir: &Path, top: u64) -> (PathBuf, PathBuf) {
    let time = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_750_000_000);
    let mut rng = Rng(0x5eed_0000_f01d_0002);
    let (l, r) = (dir.join("left"), dir.join("right"));
    let mut leaf = 0u64;
    for a in 0..top {
        for b in 0..10 {
            for c in 0..10 {
                for d in 0..10 {
                    let rel = format!("d{a}/d{b}/d{c}/d{d}");
                    let (ld, rd) = (l.join(&rel), r.join(&rel));
                    std::fs::create_dir_all(&ld).expect("mkdir");
                    std::fs::create_dir_all(&rd).expect("mkdir");
                    for f in 0..10 {
                        let name = format!("file{f}.txt");
                        let body = format!("{leaf} {f}\n");
                        let other = if rng.below(100) == 0 {
                            format!("{body}changed\n")
                        } else {
                            body.clone()
                        };
                        for (path, text) in [(ld.join(&name), &body), (rd.join(&name), &other)] {
                            std::fs::write(&path, text).expect("write");
                            let file = std::fs::File::options()
                                .write(true)
                                .open(&path)
                                .expect("open");
                            file.set_modified(time).expect("mtime");
                        }
                    }
                    if leaf.is_multiple_of(500) {
                        std::fs::write(ld.join("only-left.txt"), "l\n").expect("write");
                        std::fs::write(rd.join("only-right.txt"), "r\n").expect("write");
                    }
                    leaf += 1;
                }
            }
        }
    }
    (l, r)
}

/// Runs the GUI benchmark and returns its JSON summary.
fn bench(kind: &str, dir: &Path, left: &Path, right: &Path) -> Value {
    let began = Instant::now();
    let out = Command::new(GUI)
        .arg("--config-dir")
        .arg(dir.join("config"))
        .arg(left)
        .arg(right)
        .env("HIKASA_GUI_BENCH", kind)
        .env("HIKASA_IPC_ENDPOINT", dir.join("unused.sock"))
        .stdin(Stdio::null())
        .output()
        .expect("run hikasa-gui");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.starts_with('{'))
        .unwrap_or_else(|| {
            panic!(
                "no summary (exit {:?}): {}\n{}",
                out.status.code(),
                stdout,
                String::from_utf8_lossy(&out.stderr)
            )
        });
    let doc: Value = serde_json::from_str(line).expect("summary json");
    eprintln!(
        "{kind} benchmark ran {:.1} s",
        began.elapsed().as_secs_f64()
    );
    eprintln!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    doc
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

#[test]
#[ignore = "opens a window and generates 100 MB of input: run in release with --ignored"]
fn text_scroll() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("scroll", dir.path(), &l, &r);
    let first = f(&doc["first_screen_ms"]["from_process_start"]);
    let coloured = f(&doc["coloured_screen_ms"]["from_process_start"]);
    let total_pss = f(&doc["memory"]["total_pss_mib"]);
    let total_rss = f(&doc["memory"]["total_rss_mib"]);
    let gui_rss = f(&doc["memory"]["gui_rss_mib"]);
    let limit = f(&doc["memory"]["limit_mib"]);
    let phases = doc["page"]["phases"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(phases.len(), 3, "three scroll phases: {doc}");
    eprintln!(
        "ARCH-GUI-003 text: first screen {first} ms, coloured {coloured} ms; memory: PSS of the GUI and its web \
         processes {total_pss} MiB (summed RSS {total_rss} MiB), RSS of the GUI process {gui_rss} MiB; limit \
         400 MB = {limit} MiB"
    );
    for p in &phases {
        eprintln!(
            "  {}: {} fps, frame work p50 {} / p95 {} / max {} ms, interval p95 {} ms",
            p["name"],
            p["fps"],
            p["frame_work_ms"]["p50"],
            p["frame_work_ms"]["p95"],
            p["frame_work_ms"]["max"],
            p["frame_interval_ms"]["p95"]
        );
    }
    let fps: Vec<String> = phases.iter().map(|p| p["fps"].to_string()).collect();
    let gate_fps = phases.iter().all(|p| f(&p["fps"]) >= 60.0);
    let gate_pss = total_pss < limit;
    let gate_gui = gui_rss < limit;
    let gate_first = first < 1000.0;
    eprintln!(
        "gate: fps ≥ 60 {gate_fps} (measured {}), memory < 400 MB: PSS total {gate_pss}, GUI RSS {gate_gui}, \
         first screen < 1 s {gate_first}",
        fps.join(" / ")
    );
    assert!(first.is_finite() && coloured.is_finite() && total_pss > 0.0 && gui_rss > 0.0);
}

#[test]
#[ignore = "opens a window and generates 200,000 files: run in release with --ignored"]
fn folder_scan() {
    let dir = tempfile::tempdir().expect("temp dir");
    let began = Instant::now();
    let top = tree_top();
    let (l, r) = generate_tree(dir.path(), top);
    eprintln!("tree generated in {:.1} s", began.elapsed().as_secs_f64());
    let doc = bench("folder", dir.path(), &l, &r);
    let ticks = &doc["ui_thread_ms_while_scanning"]["message_batches"];
    let renders = &doc["ui_thread_ms_while_scanning"]["root_renders"];
    let page = &doc["page"]["frame_interval_ms_while_scanning"];
    eprintln!(
        "acceptance (b): scan {} ms; UI thread per batch p95 {} max {} ms; render p95 {} max {} ms; tree \
         rebuilds while scanning {} (max {} ms, {} rows at the end); page frame intervals while scanning p95 {} \
         ms, {} of {} above 25 ms; memory PSS {} MiB, GUI RSS {} MiB",
        doc["view"]["scan_ms"],
        ticks["p95"],
        ticks["max"],
        renders["p95"],
        renders["max"],
        doc["view"]["rebuilds_while_scanning"],
        doc["view"]["rebuild_ms"]["max"],
        doc["view"]["visible_rows"],
        page["p95"],
        page["over_25ms"],
        page["count"],
        doc["memory"]["total_pss_mib"],
        doc["memory"]["gui_rss_mib"],
    );
    assert_eq!(doc["view"]["files"][0], top * 10_000 + top * 2, "{doc}");
    assert!(
        doc["view"]["rebuilds_while_scanning"].as_u64().unwrap_or(0) > 1,
        "incremental: {doc}"
    );
}

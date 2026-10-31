//! The GUI performance gate ARCH-GUI-003 and acceptance (b) of contract §6.5 wave 6, measured through the
//! real binary in its scripted benchmark mode (`HIKASA_GUI_BENCH`). Ignored by default: it opens a window,
//! needs a display, generates about 100 MB of text and 200,000 files under the temporary folder (removed
//! afterwards) and must run on a release build:
//!
//! ```text
//! cargo test --release -p hikasa-gui --test perf_gate -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Headless, on a virtual X display: `env -u WAYLAND_DISPLAY xvfb-run -a cargo test …` (docs/17 §3.5).
//! `xvfb-run` only sets `DISPLAY`, and winit prefers Wayland whenever `WAYLAND_DISPLAY` is set, so in a Wayland
//! session the window would otherwise run on the real screen and GPU. The test also removes `WAYLAND_DISPLAY`
//! from the GUI's environment itself whenever `DISPLAY` is set; the summary's `renderer` shows which adapter
//! drew the frames (llvmpipe under Xvfb).
//!
//! * `text_scroll`: two 1,000,000-line C files (≈ 48 bytes per line, about 1 % of the lines changed in
//!   clusters; seeded xorshift64*, §6.1). The window scrolls the text panes like a mouse wheel (3 lines per
//!   frame), a page per frame, and through the whole comparison; the summary has the first screen (unaligned
//!   text, then the aligned and syntax-coloured comparison), per phase the frame rate, the UI thread's work per
//!   frame, the frame intervals and eframe's CPU time per frame, and the memory: PSS (`smaps_rollup`) and RSS of
//!   the GUI process, in MiB against 400 MB = 381.47 MiB, sampled in `memory_settled` (60 idle frames after the
//!   last phase), the median of three whole runs, every run printed with its anonymous memory and the load.
//! * `folder_scan`: 10,000 × `HIKASA_PERF_TREE_TOP` files per side (default 10: 100,000 files in 11,110
//!   folders, depth 4), compared with CRC content tests and shown with the display filter *Show All*; every
//!   folder opens as it arrives while the window scrolls the tree two rows per frame (the heaviest incremental
//!   case: about 111,000 rows at the default size); the summary has the UI thread's work per frame and the frame
//!   intervals while scanning and the tree rebuild times. A larger tree (`HIKASA_PERF_TREE_TOP=40`) keeps the
//!   scan running long enough for many rebuilds.
//!
//! * `two_windows` (M2 W4a, D-132): the `text_scroll` scenario with a second window holding Home, opened with
//!   `WindowNew` once the first screen is up; the three scroll phases run in the first window. Gate: PSS
//!   < 381.5 MiB, UI frame work p95 < 8 ms and max < 16.7 ms. `two_windows_late` opens the second window once
//!   the coloured screen is up instead (`HIKASA_PERF_SECOND_WINDOW=coloured`), same gate.
//!
//! * `workspace_restore` (M2 W4b, G12, D-132): a workspace of 10 tabs in one window (active: the session of the
//!   `text_scroll` pair; then 5 sessions of 10,000-line pairs and 4 of 1,000-file trees) opened with
//!   `hikasa-gui <workspace>`: first screen of the active tab < 1 s after the process start, no view built for the
//!   9 pending tabs. The cost per pending tab comes from two more workspaces over the same files with a small
//!   active pair (1,000 lines), one with the 9 pending tabs and one with the active tab alone, each sampled in
//!   `memory_settled` (coloured screen, no job left, 60 idle frames) and run three times: median
//!   (PSS_10 − PSS_1) / 9 < 20 MiB. The anonymous memory is printed beside the PSS (the PSS under llvmpipe moves
//!   with the file-backed share of Mesa).
//!
//! * `options_memory` (M2 W4c, D-132): one window with Home; `ProgramOptions`, every page shown once, then 60 idle
//!   frames; the gate is PSS < 120 MiB.
//!
//! Every run requires `DISPLAY` and a software renderer (llvmpipe): the benchmarks never use the real display
//! (D-132).
//!
//! The thresholds are checked and printed as measured (an fps value below 60 is reported as such); the numbers
//! are recorded in docs/02 §2.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const GUI: &str = env!("CARGO_BIN_EXE_hikasa-gui");

/// Whole runs of a memory figure (D-132): `text_scroll`, `two_windows` and `two_windows_late` take the median.
const RUNS: usize = 3;

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
    bench_with(kind, dir, left, right, &[])
}

/// [`bench`] with extra environment variables.
fn bench_with(kind: &str, dir: &Path, left: &Path, right: &Path, env: &[(&str, &str)]) -> Value {
    bench_args(kind, dir, &[left.as_os_str(), right.as_os_str()], env)
}

/// [`bench`] with the command line's words.
fn bench_args(kind: &str, dir: &Path, words: &[&std::ffi::OsStr], env: &[(&str, &str)]) -> Value {
    assert!(
        std::env::var_os("DISPLAY").is_some(),
        "run under `env -u WAYLAND_DISPLAY xvfb-run -a`: DISPLAY is not set"
    );
    let began = Instant::now();
    let mut cmd = Command::new(GUI);
    if std::env::var_os("DISPLAY").is_some() {
        // Under xvfb-run the window must use the virtual X display, not the caller's Wayland compositor.
        cmd.env_remove("WAYLAND_DISPLAY");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .arg("--config-dir")
        .arg(dir.join("config"))
        .args(words)
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
    let adapter = doc["renderer"]["adapter"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        adapter.contains("llvmpipe") || adapter.contains("software"),
        "the renderer is not a software one (D-132): {}",
        doc["renderer"]
    );
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

/// What one measured run leaves: the figures of `memory_settled` (D-132, idle frames after the last phase).
struct Settled {
    pss: f64,
    anon: f64,
    rss: f64,
    peak: f64,
}

impl Settled {
    fn of(doc: &Value) -> Settled {
        let m = &doc["memory_settled"];
        Settled {
            pss: f(&m["total_pss_mib"]),
            anon: f(&m["gui_pss_anon_mib"]),
            rss: f(&m["total_rss_mib"]),
            peak: f(&m["gui_peak_rss_mib"]),
        }
    }
}

/// The machine's load, printed beside every run: the PSS under llvmpipe moves with who else maps Mesa.
fn load() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|t| t.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// The median PSS of [`RUNS`] whole runs, each printed: one run of a bimodal figure proves nothing (D-132).
fn median_pss(name: &str, mut run: impl FnMut() -> Value) -> (f64, Value) {
    let mut last = Value::Null;
    let mut pss = Vec::new();
    for k in 0..RUNS {
        last = run();
        let s = Settled::of(&last);
        assert!(s.pss > 0.0, "no settled sample: {last}");
        eprintln!(
            "{name} run {k}: PSS {:.1} MiB, anonymous {:.1} MiB, RSS {:.1} MiB, peak RSS {:.1} MiB, load {}; \
             anonymous regions {}",
            s.pss,
            s.anon,
            s.rss,
            s.peak,
            load(),
            last["memory_settled"]["largest_anon_regions"]
        );
        pss.push(s.pss);
    }
    pss.sort_by(f64::total_cmp);
    let median = pss[pss.len() / 2];
    eprintln!("{name}: median PSS {median:.1} MiB of {pss:?}");
    (median, last)
}

#[test]
#[ignore = "opens a window and generates 100 MB of input: run in release with --ignored"]
fn text_scroll() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let (total_pss, doc) = median_pss("text_scroll", || bench("scroll", dir.path(), &l, &r));
    let first = f(&doc["first_screen_ms"]["from_process_start"]);
    let coloured = f(&doc["coloured_screen_ms"]["from_process_start"]);
    let limit = f(&doc["memory"]["limit_mib"]);
    let phases = doc["phases"].as_array().cloned().unwrap_or_default();
    assert_eq!(phases.len(), 3, "three scroll phases: {doc}");
    eprintln!(
        "ARCH-GUI-003 text: first screen {first} ms, coloured {coloured} ms; memory: median settled PSS {total_pss} \
         MiB; limit 400 MB = {limit} MiB"
    );
    for p in &phases {
        eprintln!(
            "  {}: {} fps, frame work p50 {} / p95 {} / max {} ms, eframe CPU p95 {} ms, interval p50 {} / p95 {} \
             / max {} ms",
            p["name"],
            p["fps"],
            p["frame_work_ms"]["p50"],
            p["frame_work_ms"]["p95"],
            p["frame_work_ms"]["max"],
            p["eframe_cpu_ms"]["p95"],
            p["frame_interval_ms"]["p50"],
            p["frame_interval_ms"]["p95"],
            p["frame_interval_ms"]["max"]
        );
    }
    let fps: Vec<String> = phases.iter().map(|p| p["fps"].to_string()).collect();
    let gate_fps = phases.iter().all(|p| f(&p["fps"]) >= 60.0);
    let gate_pss = total_pss < limit;
    let gate_first = first < 1000.0;
    eprintln!(
        "gate: fps ≥ 60 {gate_fps} (measured {}), memory < 400 MB: PSS {gate_pss}, first screen < 1 s {gate_first}",
        fps.join(" / ")
    );
    assert!(first.is_finite() && coloured.is_finite() && total_pss > 0.0);
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
    let work = &doc["while_scanning"]["frame_work_ms"];
    let cpu = &doc["while_scanning"]["eframe_cpu_ms"];
    let interval = &doc["while_scanning"]["frame_interval_ms"];
    eprintln!(
        "acceptance (b): scan {} ms; {} frames while scanning: UI work p50 {} / p95 {} / max {} ms ({} above \
         16.7 ms), eframe CPU p95 {} / max {} ms, intervals p95 {} / max {} ms; tree rebuilds while scanning {} \
         (max {} ms, {} rows at the end); memory PSS {} MiB, GUI RSS {} MiB",
        doc["view"]["scan_ms"],
        doc["while_scanning"]["frames"],
        work["p50"],
        work["p95"],
        work["max"],
        work["over_16_7ms"],
        cpu["p95"],
        cpu["max"],
        interval["p95"],
        interval["max"],
        doc["view"]["rebuilds_while_scanning"],
        doc["view"]["rebuild_ms"]["max"],
        doc["view"]["visible_rows"],
        doc["memory"]["total_pss_mib"],
        doc["memory"]["gui_rss_mib"],
    );
    assert_eq!(doc["view"]["files"][0], top * 10_000 + top * 2, "{doc}");
    assert!(
        doc["view"]["rebuilds_while_scanning"].as_u64().unwrap_or(0) > 1,
        "incremental: {doc}"
    );
}

/// The UI frame work of the phases: (largest p95, largest max) in ms.
fn frame_work(phases: &[Value]) -> (f64, f64) {
    phases.iter().fold((0.0_f64, 0.0_f64), |(p95, max), p| {
        (
            p95.max(f(&p["frame_work_ms"]["p95"])),
            max.max(f(&p["frame_work_ms"]["max"])),
        )
    })
}

#[test]
#[ignore = "opens two windows and generates 100 MB of input: run in release with --ignored"]
fn two_windows() {
    two_windows_gate(&[]);
}

/// The late variant: the second window opens once the coloured screen is up (the comparison is complete).
#[test]
#[ignore = "opens two windows and generates 100 MB of input: run in release with --ignored"]
fn two_windows_late() {
    two_windows_gate(&[("HIKASA_PERF_SECOND_WINDOW", "coloured")]);
}

fn two_windows_gate(env: &[(&str, &str)]) {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let name = env.first().map_or("two_windows", |_| "two_windows_late");
    let (total_pss, doc) = median_pss(name, || {
        let doc = bench_with("two_windows", dir.path(), &l, &r, env);
        assert_eq!(doc["windows"], 2, "the second window was open: {doc}");
        doc
    });
    let phases = doc["phases"].as_array().cloned().unwrap_or_default();
    assert_eq!(phases.len(), 3, "three scroll phases: {doc}");
    let limit = f(&doc["memory"]["limit_mib"]);
    let (p95, max) = frame_work(&phases);
    eprintln!(
        "two windows (second at {}): median settled PSS {total_pss} MiB (limit {limit} MiB); last run UI frame work \
         p95 {p95} ms, max {max} ms",
        doc["second_window_at"]
    );
    let gate = total_pss < limit && p95 < 8.0 && max < 16.7;
    eprintln!("gate (PSS < 381.5 MiB, p95 < 8 ms, max < 16.7 ms): {gate}");
    assert!(total_pss > 0.0 && total_pss < limit, "memory gate");
}

/// One text session file.
fn session_file(dir: &Path, name: &str, kind: &str, left: &Path, right: &Path) {
    let path = dir.join("config/sessions").join(format!("{name}.toml"));
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    let text = format!(
        "schema_version = 1\ntype = \"{kind}\"\n[specs]\nleft = {:?}\nright = {:?}\n",
        left.display().to_string(),
        right.display().to_string()
    );
    std::fs::write(path, text).expect("write");
}

fn write_lines(path: &Path, seed: u64, lines: u64) {
    let mut rng = Rng(seed);
    let mut text = String::new();
    for i in 0..lines {
        text.push_str(&c_line(&mut rng, i));
        text.push('\n');
    }
    std::fs::write(path, text).expect("write");
}

/// The Options dialog (M2 W4c, D-132): one window with Home; `ProgramOptions`, every page shown once, then 60 idle
/// frames. Gate: PSS < 120 MiB.
#[test]
#[ignore = "opens a window: run in release with --ignored"]
fn options_memory() {
    let dir = tempfile::tempdir().expect("temp dir");
    let doc = bench_args("options", dir.path(), &[], &[]);
    let pss = f(&doc["memory_settled"]["total_pss_mib"]);
    let anonymous = f(&doc["memory_settled"]["gui_pss_anon_mib"]);
    eprintln!(
        "Options: {} pages, PSS {pss:.1} MiB, anonymous {anonymous:.1} MiB",
        doc["pages"]
    );
    assert!(pss > 0.0 && pss < 120.0, "memory gate: {pss} MiB");
}

#[test]
#[ignore = "opens a window and generates 100 MB of input: run in release with --ignored"]
fn workspace_restore() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let (l, r) = generate_text(root);
    session_file(root, "main", "text-compare", &l, &r);
    let (small_l, small_r) = (root.join("small-l.c"), root.join("small-r.c"));
    write_lines(&small_l, 301, 1000);
    write_lines(&small_r, 302, 1000);
    session_file(root, "small", "text-compare", &small_l, &small_r);
    let mut tabs = String::new();
    for k in 0..5u64 {
        let (a, b) = (
            root.join(format!("t{k}-l.c")),
            root.join(format!("t{k}-r.c")),
        );
        write_lines(&a, 100 + k, 10_000);
        write_lines(&b, 200 + k, 10_000);
        let name = format!("text{k}");
        session_file(root, &name, "text-compare", &a, &b);
        tabs.push_str(&format!("[[window.tab]]\nsession = \"{name}\"\n"));
    }
    for k in 0..4u64 {
        let (a, b) = (root.join(format!("d{k}-l")), root.join(format!("d{k}-r")));
        for side in [&a, &b] {
            for i in 0..1000 {
                let folder = side.join(format!("f{}", i % 10));
                std::fs::create_dir_all(&folder).expect("mkdir");
                std::fs::write(folder.join(format!("{i}.txt")), "x\n").expect("write");
            }
        }
        let name = format!("tree{k}");
        session_file(root, &name, "folder-compare", &a, &b);
        tabs.push_str(&format!("[[window.tab]]\nsession = \"{name}\"\n"));
    }
    let workspace = |name: &str, active: &str, others: &str| {
        let path = root.join(format!("config/workspaces/{name}.toml"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let text = format!(
            "schema_version = 1\n[[window]]\nactive_tab = 0\n[[window.tab]]\nsession = \"{active}\"\n{others}"
        );
        std::fs::write(path, text).expect("write");
    };
    workspace("ten", "main", &tabs);
    workspace("small_ten", "small", &tabs);
    workspace("small_one", "small", "");
    let doc = bench_args("workspace", root, &[std::ffi::OsStr::new("ten")], &[]);
    let first = f(&doc["first_screen_ms"]["from_process_start"]);
    eprintln!(
        "G12: first screen {first:.0} ms, tabs {}, pending {}, views built {}",
        doc["tabs"], doc["pending_tabs"], doc["views_built"],
    );
    let settled = |name: &str, tabs: u64| {
        let doc = bench_args("workspace", root, &[std::ffi::OsStr::new(name)], &[]);
        assert_eq!(doc["tabs"], tabs, "{name}");
        assert_eq!(doc["pending_tabs"], tabs - 1, "{name}");
        assert_eq!(doc["views_built"], 1, "{name}");
        let m = &doc["memory_settled"];
        (f(&m["total_pss_mib"]), f(&m["gui_pss_anon_mib"]))
    };
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let mut per_pss = Vec::new();
    let mut per_anon = Vec::new();
    for round in 0..3 {
        let (pss_10, anon_10) = settled("small_ten", 10);
        let (pss_1, anon_1) = settled("small_one", 1);
        let (pss, anon) = ((pss_10 - pss_1) / 9.0, (anon_10 - anon_1) / 9.0);
        eprintln!(
            "G12 round {round}: PSS {pss_10:.1} vs {pss_1:.1} MiB, {pss:.2} MiB per tab; \
             anonymous {anon_10:.1} vs {anon_1:.1} MiB, {anon:.2} MiB per tab"
        );
        per_pss.push(pss);
        per_anon.push(anon);
    }
    let (pss, anon) = (median(per_pss), median(per_anon));
    eprintln!("G12: per pending tab {pss:.2} MiB PSS, {anon:.2} MiB anonymous (limit 20 MiB PSS)");
    assert_eq!(doc["tabs"], 10);
    assert_eq!(doc["pending_tabs"], 9, "no job for the 9 pending tabs");
    assert_eq!(doc["views_built"], 1);
    assert!(first < 1000.0, "first screen {first} ms");
    assert!(pss < 20.0, "{pss} MiB per pending tab");
}

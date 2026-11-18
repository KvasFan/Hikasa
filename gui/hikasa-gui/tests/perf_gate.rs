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
//!   frames; the gate is PSS < 120 MiB. `options_memory_named_font` (D-160) is the same with the theme's editor
//!   font set to `HIKASA_PERF_FONT` (default `DejaVu Sans Mono`, a font of the machine): the cost of a named font.
//!
//! * `table_scroll` (Table Compare phase 3, plan W13): the G9 pair (1,000,000 × 10 CSV, about 82 MB a side, 1 %
//!   changed cells, 0.1 % rows deleted and inserted; `core/hikasa-table/tests/gates.rs`'s generator) opened in the
//!   Table Compare grid (all rows shown) and scrolled like `text_scroll`; gate: every phase ≥ 60 fps and the settled
//!   PSS < 400 MB (median of three runs).
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
    // The gate itself (phase-3 review F13): the run fails when it is not met.
    assert!(gate_fps, "fps gate not met: {}", fps.join(" / "));
    assert!(
        gate_pss,
        "memory gate not met: {total_pss} MiB ≥ {limit} MiB"
    );
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
    let own = f(&doc["memory_settled"]["own_mib"]);
    eprintln!(
        "Options: {} pages, PSS {pss:.1} MiB, anonymous {anonymous:.1}, own {own:.1}",
        doc["pages"]
    );
    assert!(pss > 0.0 && pss < 120.0, "memory gate: {pss} MiB");
}

/// `options_memory` with a named font of the machine (D-160): what one family costs. Same gate.
#[test]
#[ignore = "opens a window and needs a font of the machine: run in release with --ignored"]
fn options_memory_named_font() {
    let family = std::env::var("HIKASA_PERF_FONT").unwrap_or_else(|_| "DejaVu Sans Mono".into());
    let dir = tempfile::tempdir().expect("temp dir");
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).expect("mkdir");
    std::fs::write(
        config.join("theme.toml"),
        format!("schema_version = 1\n[fonts.editor]\nfamily = \"{family}\"\n"),
    )
    .expect("write");
    let doc = bench_args("options", dir.path(), &[], &[]);
    let pss = f(&doc["memory_settled"]["total_pss_mib"]);
    let anonymous = f(&doc["memory_settled"]["gui_pss_anon_mib"]);
    let own = f(&doc["memory_settled"]["own_mib"]);
    eprintln!("Options with {family}: PSS {pss:.1} MiB, anonymous {anonymous:.1}, own {own:.1}");
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

/// `n` empty files `file_00000…` in a new folder `name` of `dir` (G15 completion, browse).
fn generate_flat(dir: &Path, name: &str, n: u64) -> PathBuf {
    let folder = dir.join(name);
    std::fs::create_dir_all(&folder).expect("mkdir");
    for i in 0..n {
        std::fs::write(folder.join(format!("file_{i:05}")), b"").expect("write");
    }
    folder
}

/// Prints and returns the frame work of one labelled action of a W9a run.
fn action(doc: &Value, label: &str) -> (f64, f64) {
    let a = &doc["actions"][label];
    eprintln!(
        "  {label}: {} frames, work p50 {} / p95 {} / max {} ms",
        a["count"], a["p50"], a["p95"], a["max"]
    );
    (f(&a["p95"]), f(&a["max"]))
}

#[test]
#[ignore = "opens a window and generates 200,000 files: run in release with --ignored"]
fn folder_columns() {
    let dir = tempfile::tempdir().expect("temp dir");
    let top = tree_top();
    let (l, r) = generate_tree(dir.path(), top);
    let doc = bench("folder_columns", dir.path(), &l, &r);
    let work = &doc["while_scanning"]["frame_work_ms"];
    let pss = f(&doc["memory"]["total_pss_mib"]);
    eprintln!(
        "G15 tree (every column, Size descending): scan {} ms, {} frames while scanning, UI work p95 {} / max {} ms ({} \
         above 16.7 ms); PSS {pss} MiB; load {}",
        doc["view"]["scan_ms"],
        doc["while_scanning"]["frames"],
        work["p95"],
        work["max"],
        work["over_16_7ms"],
        load()
    );
    eprintln!(
        "gate: max ≤ 16.7 ms {}, p95 < 8 ms {}, PSS < 381.5 MiB {}",
        f(&work["max"]) <= 1000.0 / 60.0,
        f(&work["p95"]) < 8.0,
        pss < 381.5
    );
    assert_eq!(doc["view"]["files"][0], top * 10_000 + top * 2, "{doc}");
}

#[test]
#[ignore = "opens a window and generates 200,000 files: run in release with --ignored"]
fn select() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_tree(dir.path(), tree_top());
    let doc = bench("select", dir.path(), &l, &r);
    eprintln!("G15 select (load {}):", load());
    // Select All Files is an extra figure; the gate's commands are Select All, Invert Selection and Shift+Left.
    let _ = action(&doc, "select_all_files");
    let commands: Vec<f64> = ["select_all", "invert_selection", "shift_left"]
        .iter()
        .map(|label| action(&doc, label).1)
        .collect();
    let (down_p95, down_max) = action(&doc, "shift_down");
    eprintln!(
        "gate: each command ≤ 16.7 ms {}, Shift+Down p95 < 4 ms {} (max {down_max} ms)",
        commands.iter().all(|w| *w <= 1000.0 / 60.0),
        down_p95 < 4.0
    );
    assert_eq!(doc["actions"]["shift_down"]["count"], 300, "{doc}");
}

#[test]
#[ignore = "opens a window and generates 10,000 files: run in release with --ignored"]
fn completion() {
    let dir = tempfile::tempdir().expect("temp dir");
    let l = generate_flat(dir.path(), "left", 10_000);
    let r = generate_flat(dir.path(), "right", 0);
    let doc = bench("completion", dir.path(), &l, &r);
    let list = f(&doc["done_ms"]);
    eprintln!(
        "G15 completion (load {}): the list {list} ms after the last key",
        load()
    );
    let (_, typing_max) = action(&doc, "typing");
    action(&doc, "waiting");
    eprintln!(
        "gate: the list ≤ 200 ms {}, no frame above 16.7 ms while typing {}",
        list <= 200.0,
        typing_max <= 1000.0 / 60.0
    );
    assert!(list.is_finite(), "the list never showed: {doc}");
}

#[test]
#[ignore = "opens a window and generates 100,000 files: run in release with --ignored"]
fn browse() {
    let dir = tempfile::tempdir().expect("temp dir");
    let l = generate_flat(dir.path(), "left", 100_000);
    let r = generate_flat(dir.path(), "right", 0);
    let doc = bench("browse", dir.path(), &l, &r);
    let listed = f(&doc["done_ms"]);
    eprintln!(
        "G15 browse (load {}): listed {listed} ms after it opened; PSS {} MiB",
        load(),
        doc["memory"]["total_pss_mib"]
    );
    let (_, scroll_max) = action(&doc, "scrolling");
    eprintln!(
        "gate: listed ≤ 1 s {}, scrolling frame work max < 16.7 ms {}",
        listed <= 1000.0,
        scroll_max < 1000.0 / 60.0
    );
    assert_eq!(doc["actions"]["scrolling"]["count"], 600, "{doc}");
}

#[test]
#[ignore = "opens a window and generates 200,000 files: run in release with --ignored"]
fn preview() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_tree(dir.path(), tree_top());
    let doc = bench("preview", dir.path(), &l, &r);
    let first = f(&doc["done_ms"]);
    let view = f(&doc["memory_settled"]["total_pss_mib"]);
    let open = f(&doc["memory"]["total_pss_mib"]);
    eprintln!(
        "G15 preview (load {}): {} lines; the preview's first frame {first} ms after Copy to Right (the plan job {} ms \
         of it); PSS {view} MiB before, {open} MiB while open (+{} MiB)",
        load(),
        doc["preview_lines"],
        doc["plan_job_ms"],
        open - view
    );
    let (_, scroll_max) = action(&doc, "scrolling");
    eprintln!(
        "gate: first frame < 1 s {}, scrolling frame work max < 16.7 ms {}, PSS while open ≤ view + 96 MiB {}",
        first < 1000.0,
        scroll_max < 1000.0 / 60.0,
        open - view <= 96.0
    );
    assert_eq!(doc["actions"]["scrolling"]["count"], 600, "{doc}");
}

#[test]
#[ignore = "opens a window and generates 200,000 files: run in release with --ignored"]
fn sync_view() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_tree(dir.path(), tree_top());
    let doc = bench_args(
        "sync_view",
        dir.path(),
        &[l.as_os_str(), r.as_os_str(), std::ffi::OsStr::new("--sync")],
        &[],
    );
    let plan = f(&doc["plan_ms"]);
    let replan = f(&doc["override_ms"]);
    eprintln!(
        "G15 sync (load {}): plan shown {plan} ms after the comparison ended; an override re-planned and repainted in \
         {replan} ms; PSS {} MiB",
        load(),
        doc["memory"]["total_pss_mib"]
    );
    let (_, scroll_max) = action(&doc, "scrolling");
    eprintln!(
        "gate: plan ≤ 0.5 s {}, override ≤ 0.5 s {}, scrolling frame work max < 16.7 ms {}",
        plan <= 500.0,
        replan <= 500.0,
        scroll_max < 1000.0 / 60.0
    );
    assert_eq!(doc["actions"]["scrolling"]["count"], 600, "{doc}");
}

/// One row of the table gate's CSV (the G9 generator of `core/hikasa-table/tests/gates.rs`): an id, 4 numbers, 2
/// dates, 3 words.
fn table_row(i: u64, rng: &mut Rng, out: &mut String) {
    use std::fmt::Write as _;
    let _ = writeln!(
        out,
        "{i},{}.{:02},{},{}.5,{},2024-{:02}-{:02},2025-01-{:02} 10:{:02},w{},item {},code{}",
        rng.below(100_000),
        rng.below(100),
        rng.below(1_000_000),
        rng.below(1000),
        rng.below(50),
        1 + rng.below(12),
        1 + rng.below(28),
        1 + rng.below(28),
        rng.below(60),
        rng.below(5000),
        rng.below(100_000),
        rng.below(10)
    );
}

/// The G9 pair: 1,000,000 × 10, the right side with 1 % changed cells, 0.1 % deleted and 0.1 % inserted rows.
fn generate_table(dir: &Path) -> (PathBuf, PathBuf) {
    let header = "id,a,b,c,d,day,stamp,w,item,code\n";
    let (mut l, mut r) = (header.to_owned(), header.to_owned());
    let mut rng = Rng(0x5EED_0009);
    for i in 0..1_000_000u64 {
        let mut line = String::with_capacity(96);
        table_row(i, &mut rng, &mut line);
        l.push_str(&line);
        match rng.below(1000) {
            0 => {}
            1 => {
                r.push_str(&line);
                table_row(2_000_000 + i, &mut rng, &mut r);
            }
            2..=11 => r.push_str(&line.replacen(",w", ",W", 1)),
            _ => r.push_str(&line),
        }
    }
    let (left, right) = (dir.join("left.csv"), dir.join("right.csv"));
    std::fs::write(&left, l).expect("write");
    std::fs::write(&right, r).expect("write");
    (left, right)
}

/// The Table Compare gate (plan W13, m2-table-contract §6.6): `table_scroll` opens the G9 pair (1,000,000 × 10 CSV,
/// about 82 MB a side) in the grid and scrolls it like `text_scroll` (3 rows a frame, a page a frame, the whole sheet);
/// gate: every phase ≥ 60 fps, the settled PSS < 400 MB (median of three runs). The comparison's own figures are G9's
/// (`docs/02`).
#[test]
#[ignore = "opens a window and generates 160 MB of input: run in release with --ignored"]
fn table_scroll() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_table(dir.path());
    let (total_pss, doc) = median_pss("table_scroll", || bench("table_scroll", dir.path(), &l, &r));
    let first = f(&doc["first_screen_ms"]["from_process_start"]);
    let limit = f(&doc["memory"]["limit_mib"]);
    let phases = doc["phases"].as_array().cloned().unwrap_or_default();
    assert_eq!(phases.len(), 3, "three scroll phases: {doc}");
    eprintln!(
        "W13 table gate: first screen {first} ms (the comparison included); memory: median settled PSS {total_pss} \
         MiB; limit 400 MB = {limit} MiB; rows {}",
        doc["view"]["rows"]
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
    eprintln!(
        "gate: fps ≥ 60 {gate_fps} (measured {}), memory < 400 MB: PSS {gate_pss}",
        fps.join(" / ")
    );
    assert!(first.is_finite() && total_pss > 0.0);
    assert!(
        doc["view"]["rows"].as_u64().unwrap_or(0) > 1_000_000,
        "{doc}"
    );
    // The gate itself (phase-3 review F7): the run fails when it is not met.
    assert!(gate_fps, "fps gate not met: {}", fps.join(" / "));
    assert!(
        gate_pss,
        "memory gate not met: {total_pss} MiB ≥ {limit} MiB"
    );
}

// ---- G16 (M2 W11a, contract §1.11.30, D-322): typing, lines, burst, undo and ARCH-GUI-003 edited --------------------

/// Prints the key frames of each G16 row and returns (max, p95) per row.
fn g16_rows(doc: &Value) -> Vec<(String, f64, f64)> {
    let rows = doc["edit"]["key_frames_ms"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for (label, s) in rows {
        eprintln!(
            "  {label}: key frames {} — p50 {} / p95 {} / max {} ms ({} over 16.7 ms)",
            s["count"], s["p50"], s["p95"], s["max"], s["over_16_7ms"]
        );
        out.push((label, f(&s["max"]), f(&s["p95"])));
    }
    for (label, s) in doc["edit"]["install_after_last_key_ms"]
        .as_object()
        .cloned()
        .unwrap_or_default()
    {
        eprintln!(
            "  {label}: install after the last key p50 {} / max {} ms",
            s["p50"], s["max"]
        );
    }
    eprintln!(
        "  generation misses {}, recompare jobs at once ≤ {}, checks {}",
        doc["edit"]["generation_misses"], doc["edit"]["max_recompare_jobs"], doc["edit"]["checks"]
    );
    out
}

#[test]
#[ignore = "opens a window on a 1,000,000-line pair: run in release with --ignored under xvfb"]
fn text_typing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("text_typing", dir.path(), &l, &r);
    eprintln!("G16 typing (type, changed, lines, burst):");
    let rows = g16_rows(&doc);
    assert!(!rows.is_empty(), "{doc}");
    let mut failed = Vec::new();
    for (label, max, p95) in &rows {
        let gated =
            matches!(label.as_str(), "type" | "changed" | "burst") || label.starts_with("lines");
        if gated && (*max >= 1000.0 / 60.0 || (*p95 >= 8.0 && !label.starts_with("burst"))) {
            failed.push(format!("{label}: max {max} ms, p95 {p95} ms"));
        }
    }
    for (label, s) in doc["edit"]["install_after_last_key_ms"]
        .as_object()
        .cloned()
        .unwrap_or_default()
    {
        if matches!(label.as_str(), "type" | "changed") && f(&s["max"]) > 150.0 {
            failed.push(format!(
                "{label}: install {} ms after the last key",
                s["max"]
            ));
        }
    }
    assert_eq!(
        doc["edit"]["generation_misses"], 0,
        "the typed character is in the key frame's slice"
    );
    assert!(doc["edit"]["max_recompare_jobs"].as_u64().unwrap_or(9) <= 1);
    let checks = doc["edit"]["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        checks.iter().all(|c| c["equal"] == true),
        "burst exactness: {checks:?}"
    );
    assert!(failed.is_empty(), "G16 rows not met: {failed:?}");
}

#[test]
#[ignore = "opens a window on a 1,000,000-line pair: run in release with --ignored under xvfb"]
fn text_undo() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("text_undo", dir.path(), &l, &r);
    eprintln!("G16 undo:");
    let rows = g16_rows(&doc);
    let undo = rows.iter().find(|(l, ..)| l == "undo").expect("undo row");
    let checks = doc["edit"]["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        checks.iter().all(|c| c["equal"] == true),
        "the original comparison after the undos: {checks:?}"
    );
    assert!(undo.1 < 1000.0 / 60.0, "undo frames max {} ms", undo.1);
}

#[test]
#[ignore = "opens a window on a 1,000,000-line pair: run in release with --ignored under xvfb"]
fn text_edited() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("text_edited", dir.path(), &l, &r);
    eprintln!("ARCH-GUI-003 edited:");
    let rows = g16_rows(&doc);
    let limit = f(&doc["memory"]["limit_mib"]);
    let mut failed = Vec::new();
    for sample in doc["edit"]["memory"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let pss = f(&sample["memory"]["total_pss_mib"]);
        eprintln!(
            "  memory at {}: PSS {pss} MiB, peak RSS {} MiB (limit {limit} MiB)",
            sample["at"], sample["memory"]["gui_peak_rss_mib"]
        );
        if pss >= limit {
            failed.push(format!("PSS {pss} MiB at {}", sample["at"]));
        }
    }
    for (label, max, p95) in &rows {
        let scroll =
            label.ends_with("_wheel") || label.ends_with("_page") || label.ends_with("_sweep");
        if scroll && (*max >= 1000.0 / 60.0 || *p95 >= 8.0) {
            failed.push(format!("{label}: max {max} ms, p95 {p95} ms"));
        }
    }
    let first = f(&doc["first_screen_ms"]["from_process_start"]);
    eprintln!("  first screen {first} ms");
    let checks = doc["edit"]["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(checks.iter().all(|c| c["equal"] == true), "{checks:?}");
    assert!(first < 1000.0, "first screen {first} ms");
    assert!(failed.is_empty(), "ARCH-GUI-003 edited not met: {failed:?}");
}

/// The job times of an editing benchmark (`edit.job_ms`), printed.
fn job_ms(doc: &Value, job: &str) -> f64 {
    let ms = doc["edit"]["job_ms"]
        .as_array()
        .and_then(|a| a.iter().find(|j| j["job"] == job))
        .map_or(f64::NAN, |j| f(&j["ms"]));
    eprintln!("  {job}: job {ms} ms; found {}", doc["edit"]["found"]);
    ms
}

/// The rows of `labels` with a frame above 16.7 ms (and, with `p95`, a p95 at or above 8 ms).
fn over(rows: &[(String, f64, f64)], labels: &[&str], p95: bool) -> Vec<String> {
    rows.iter()
        .filter(|(l, ..)| labels.contains(&l.as_str()))
        .filter(|(_, max, q)| *max >= 1000.0 / 60.0 || (p95 && *q >= 8.0))
        .map(|(l, max, q)| format!("{l}: max {max} ms, p95 {q} ms"))
        .collect()
}

#[test]
#[ignore = "opens a window on a 1,000,000-line pair: run in release with --ignored under xvfb"]
fn text_wrap() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("text_wrap", dir.path(), &l, &r);
    eprintln!("G16 wrap (load {}):", load());
    let rows = g16_rows(&doc);
    let failed = over(&rows, &["wrap_wheel", "wrap_page", "wrap_sweep"], true);
    assert!(
        rows.iter().any(|(l, ..)| l == "wrap_sweep"),
        "the phases ran: {doc}"
    );
    assert!(failed.is_empty(), "G16 wrap not met: {failed:?}");
}

#[test]
#[ignore = "opens a window on a 1,000,000-line pair: run in release with --ignored under xvfb"]
fn text_find() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("text_find", dir.path(), &l, &r);
    eprintln!("G16 find (load {}):", load());
    let rows = g16_rows(&doc);
    let none = job_ms(&doc, "find_none");
    let all = job_ms(&doc, "select_all");
    let failed = over(&rows, &["find_none", "select_all"], false);
    assert!(none.is_finite() && all.is_finite(), "{doc}");
    assert!(failed.is_empty(), "G16 find frames: {failed:?}");
}

/// G7's shape for Find in Files: 10,000 text files in 100 folders, 24 lines each, 10 of them holding the word: 100,000
/// hits.
fn generate_find_tree(root: &Path) {
    for d in 0..100 {
        let dir = root.join(format!("d{d:03}"));
        std::fs::create_dir_all(&dir).expect("mkdir");
        for k in 0..100 {
            let mut text = String::new();
            for line in 0..24 {
                if line % 12 < 5 {
                    text.push_str(&format!("    call(pin_{line}, {k}); // the needle {d}\n"));
                } else {
                    text.push_str(&format!("    value = compute({line}, {k}, {d});\n"));
                }
            }
            std::fs::write(dir.join(format!("f{k:03}.c")), text).expect("write");
        }
    }
}

#[test]
#[ignore = "opens a window and generates 10,000 files: run in release with --ignored under xvfb"]
fn find_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    let tree = dir.path().join("tree");
    generate_find_tree(&tree);
    let l = dir.path().join("a.c");
    let r = dir.path().join("b.c");
    std::fs::write(&l, "int a;\n").expect("write");
    std::fs::write(&r, "int b;\n").expect("write");
    let root = tree.to_string_lossy().into_owned();
    let doc = bench_with(
        "find_files",
        dir.path(),
        &l,
        &r,
        &[("HIKASA_PERF_FIND_ROOT", root.as_str())],
    );
    eprintln!("G16 find files (load {}):", load());
    let rows = g16_rows(&doc);
    let ms = job_ms(&doc, "files_stream");
    let found = doc["edit"]["found"]
        .as_array()
        .and_then(|a| a.first())
        .map_or(0, |x| x["hits"].as_u64().unwrap_or(0));
    assert!(ms.is_finite());
    assert_eq!(found, 100_000, "the hits streamed into the panel");
    let failed = over(&rows, &["files_stream", "files_scroll"], false);
    assert!(failed.is_empty(), "G16 find files frames: {failed:?}");
}

#[test]
#[ignore = "opens a window on a 1,000,000-line pair: run in release with --ignored under xvfb"]
fn text_save() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (l, r) = generate_text(dir.path());
    let doc = bench("text_save", dir.path(), &l, &r);
    eprintln!("G16 save (load {}):", load());
    let rows = g16_rows(&doc);
    let ms = job_ms(&doc, "save");
    let failed = over(&rows, &["save"], false);
    assert!(ms.is_finite(), "{doc}");
    let saved = std::fs::read_to_string(&l).expect("read");
    assert!(saved.contains('x'), "the edits were saved");
    assert!(failed.is_empty(), "G16 save frames: {failed:?}");
}

// ---- G10 in the window (M2 W13a, contract §1.13.21, D-372) --------------------------------------------------------

/// Where the large G10 inputs go: `HIKASA_GATE_DIR` (default `/var/tmp/hikasa-work/w13a/gates`).
fn hex_gate_dir() -> PathBuf {
    let dir = std::env::var_os("HIKASA_GATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/tmp/hikasa-work/w13a/gates"))
        .join("hikasa-gui-hex");
    std::fs::create_dir_all(&dir).expect("gate dir");
    dir
}

/// W12's G10 pair: two 4 GiB files, the right with 1,000 seeded differing runs of 1–64 bytes (written once, in 64 MiB
/// pieces).
fn hex_big_pair(dir: &Path) -> (PathBuf, PathBuf) {
    use std::io::Write as _;
    const GIB: u64 = 1 << 30;
    let len = 4 * GIB;
    let (l, r) = (dir.join("4g-left"), dir.join("4g-right"));
    let mut rng = Rng(0xA11CE);
    let mut runs: Vec<(u64, u64)> = (0..1000)
        .map(|_| (rng.below(len - 64), 1 + rng.below(64)))
        .collect();
    runs.sort_unstable();
    for (path, patch) in [(&l, false), (&r, true)] {
        if std::fs::metadata(path).is_ok_and(|m| m.len() == len) {
            continue;
        }
        let mut f = std::io::BufWriter::new(std::fs::File::create(path).expect("create"));
        let mut rgen = Rng(1);
        let mut at = 0u64;
        while at < len {
            let n = (64u64 << 20).min(len - at) as usize;
            let mut piece = Vec::with_capacity(n);
            while piece.len() < n {
                piece.extend_from_slice(&rgen.next().to_le_bytes());
            }
            piece.truncate(n);
            if patch {
                let end = at + n as u64;
                for &(start, k) in &runs {
                    for p in start..start + k {
                        if (at..end).contains(&p) {
                            piece[(p - at) as usize] ^= 0x5A;
                        }
                    }
                }
            }
            f.write_all(&piece).expect("write");
            at += n as u64;
        }
        f.flush().expect("flush");
    }
    (l, r)
}

/// A pair of `len` bytes, the right one with 1,000 seeded edits (insertions, deletions, replacements of 1–4,096
/// bytes): W12's G10 fast/complete inputs.
fn hex_edited_pair(dir: &Path, len: u64, name: &str) -> (PathBuf, PathBuf) {
    let (l, r) = (
        dir.join(format!("{name}-left")),
        dir.join(format!("{name}-right")),
    );
    if l.exists() && r.exists() {
        return (l, r);
    }
    let mut rgen = Rng(7);
    let mut base = Vec::with_capacity(len as usize + 8);
    while (base.len() as u64) < len {
        base.extend_from_slice(&rgen.next().to_le_bytes());
    }
    base.truncate(len as usize);
    let mut rng = Rng(11);
    let mut at: Vec<u64> = (0..1000).map(|_| rng.below(len - 8192)).collect();
    at.sort_unstable();
    let mut out = Vec::with_capacity(base.len() + 1000 * 4096);
    let mut pos = 0usize;
    for (k, &p) in at.iter().enumerate() {
        let p = p as usize;
        if p < pos {
            continue;
        }
        out.extend_from_slice(&base[pos..p]);
        pos = p;
        let n = 1 + rng.below(4096) as usize;
        match k % 3 {
            0 => {
                for _ in 0..n {
                    out.push(rng.next() as u8);
                }
            }
            1 => pos = (p + n).min(base.len()),
            _ => {
                let end = (p + n).min(base.len());
                out.extend(base[p..end].iter().map(|b| !b));
                pos = end;
            }
        }
    }
    out.extend_from_slice(&base[pos..]);
    std::fs::write(&l, &base).expect("write");
    std::fs::write(&r, &out).expect("write");
    (l, r)
}

/// The words that open a pair in Hex Compare with a mode.
fn hex_words<'a>(l: &'a Path, r: &'a Path, fv: &'a std::ffi::OsStr) -> Vec<&'a std::ffi::OsStr> {
    vec![fv, l.as_os_str(), r.as_os_str()]
}

/// G10 first screen (`hex_open`): the 4 GiB pair, Fast (the default) and None: the first painted rows < 1 s after the
/// process start; the landing reported.
#[test]
#[ignore = "opens a window and writes two 4 GiB files: run in release with --ignored"]
fn hex_open() {
    let gates = hex_gate_dir();
    let (l, r) = hex_big_pair(&gates);
    let dir = tempfile::tempdir().expect("temp dir");
    let fv = std::ffi::OsStr::new("--fv=Hex Compare");
    for mode in ["fast", "none"] {
        std::fs::create_dir_all(dir.path().join("config/sessions/_defaults")).expect("mkdir");
        std::fs::write(
            dir.path()
                .join("config/sessions/_defaults/hex-compare.toml"),
            format!("type = \"hex-compare\"\n[rules.hex]\nalignment = \"{mode}\"\n"),
        )
        .expect("defaults");
        let doc = bench_args("hex_open", dir.path(), &hex_words(&l, &r, fv), &[]);
        let first = f(&doc["hex"]["first_rows_ms_from_process_start"]);
        eprintln!(
            "G10 first screen ({mode}): first rows {first} ms from the process start, {} ms from the view's open; \
             landed {} ms from the open; load {}",
            doc["hex"]["first_rows_ms_from_view_open"],
            doc["hex"]["landed_ms_from_view_open"],
            load()
        );
        assert!(first < 1000.0, "G10 first screen not met: {first} ms");
    }
}

/// G10 scroll and memory (`hex_scroll`): after Fast landed, the three phases and the three again under *Differences*
/// with 3 context rows; frame work p95 < 8 ms, max < 16.7 ms; no placeholder in the first two phases and none later
/// than 2 frames after a jump; PSS < 381.5 MiB at the end.
#[test]
#[ignore = "opens a window and writes two 4 GiB files: run in release with --ignored"]
fn hex_scroll() {
    let gates = hex_gate_dir();
    let (l, r) = hex_big_pair(&gates);
    let dir = tempfile::tempdir().expect("temp dir");
    let fv = std::ffi::OsStr::new("--fv=Hex Compare");
    let doc = bench_args("hex_scroll", dir.path(), &hex_words(&l, &r, fv), &[]);
    let phases = doc["hex"]["phases"].as_array().cloned().unwrap_or_default();
    assert_eq!(phases.len(), 6, "{doc}");
    let mut ok = true;
    for (k, p) in phases.iter().enumerate() {
        let (p95, max) = (f(&p["frame_work_ms"]["p95"]), f(&p["frame_work_ms"]["max"]));
        let ph = p["placeholder_frames"].as_u64().unwrap_or(u64::MAX);
        let late = p["placeholder_frames_later_than_2_after_a_jump"]
            .as_u64()
            .unwrap_or(u64::MAX);
        eprintln!(
            "  {}: frame work p50 {} / p95 {p95} / max {max} ms ({} over 16.7 ms); placeholder frames {ph}, later than 2 \
             frames after a jump {late}",
            p["name"], p["frame_work_ms"]["p50"], p["frame_work_ms"]["over_16_7ms"]
        );
        ok &= p95 < 8.0 && max < 16.7;
        ok &= if k % 3 == 2 { late == 0 } else { ph == 0 };
    }
    let m = &doc["hex"]["memory_settled"];
    let pss = f(&m["total_pss_mib"]);
    eprintln!(
        "G10 memory: PSS {pss} MiB (limit 381.5), peak RSS {} MiB, swapped {}, load {}",
        m["gui_peak_rss_mib"],
        m["gui_swap_mib"],
        load()
    );
    assert!(pss < 381.5, "G10 memory not met: {pss} MiB");
    assert!(ok, "G10 scroll not met (figures above)");
}

/// G10 align (`hex_align`): Fast → Complete on the 256 MiB pair and None → Fast on the 1 GiB pair, scrolling 3 rows a
/// frame while each runs: no frame above 16.7 ms; the landing time reported (against the headless figure).
#[test]
#[ignore = "opens a window and writes 1.3 GiB of input: run in release with --ignored"]
fn hex_align() {
    let gates = hex_gate_dir();
    let dir = tempfile::tempdir().expect("temp dir");
    let fv = std::ffi::OsStr::new("--fv=Hex Compare");
    let mut ok = true;
    for (len, name, from, to) in [
        (256u64 << 20, "256m", "fast", "complete"),
        (1u64 << 30, "1g", "none", "fast"),
    ] {
        let (l, r) = hex_edited_pair(&gates, len, name);
        std::fs::create_dir_all(dir.path().join("config/sessions/_defaults")).expect("mkdir");
        std::fs::write(
            dir.path()
                .join("config/sessions/_defaults/hex-compare.toml"),
            format!("type = \"hex-compare\"\n[rules.hex]\nalignment = \"{from}\"\n"),
        )
        .expect("defaults");
        let doc = bench_args(
            "hex_align",
            dir.path(),
            &hex_words(&l, &r, fv),
            &[("HIKASA_BENCH_HEX_MODE", to)],
        );
        let w = &doc["hex"]["frame_work_while_aligning_ms"];
        eprintln!(
            "G10 align {name} {from} → {to}: landed {} ms after the switch; frame work p95 {} / max {} ms ({} over \
             16.7 ms); load {}",
            doc["hex"]["realign_ms"],
            w["p95"],
            w["max"],
            w["over_16_7ms"],
            load()
        );
        ok &= f(&w["max"]) < 16.7;
    }
    assert!(ok, "G10 align: a frame above 16.7 ms");
}

/// G10 find (`hex_find`): Find Next of an 8-byte pattern absent from the left 4 GiB file, then of one in its last
/// 4 KiB; no frame above 16.7 ms while searching; the result times reported; Esc cancels.
#[test]
#[ignore = "opens a window and writes two 4 GiB files: run in release with --ignored"]
fn hex_find() {
    let gates = hex_gate_dir();
    let (l, r) = hex_big_pair(&gates);
    // An 8-byte pattern from the left file's last 4 KiB.
    let tail = {
        use std::io::{Read as _, Seek as _};
        let mut f = std::fs::File::open(&l).expect("open");
        f.seek(std::io::SeekFrom::End(-2048)).expect("seek");
        let mut b = [0u8; 8];
        f.read_exact(&mut b).expect("read");
        b.iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let fv = std::ffi::OsStr::new("--fv=Hex Compare");
    let doc = bench_args(
        "hex_find",
        dir.path(),
        &hex_words(&l, &r, fv),
        &[
            ("HIKASA_BENCH_HEX_FIND", "00 11 22 33 44 55 66 77"),
            ("HIKASA_BENCH_HEX_FIND2", tail.as_str()),
        ],
    );
    let mut ok = true;
    for s in doc["hex"]["searches"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        eprintln!(
            "G10 find {}: result {} ms; frame work p95 {} / max {} ms; load {}",
            s["pattern"],
            s["result_ms"],
            s["frame_work_ms"]["p95"],
            s["frame_work_ms"]["max"],
            load()
        );
        ok &= f(&s["frame_work_ms"]["max"]) < 16.7;
    }
    eprintln!("Esc: the search stopped in {} ms", doc["hex"]["cancel_ms"]);
    assert!(ok, "G10 find: a frame above 16.7 ms");
}

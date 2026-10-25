//! The scripted performance mode of ARCH-GUI-003 (`HIKASA_GUI_BENCH=scroll|folder`, see `main`): the window
//! opens the command line's view, the page script drives it (scrolling the text canvas through the whole
//! comparison, or scrolling the folder tree while the scan runs) and records its frame times; this module adds
//! the first-screen marks, the UI thread's per-frame work and the memory of the GUI process and its web
//! content processes (PSS summed, RSS of the GUI process), prints one JSON summary on stdout and quits.

use std::time::{Duration, Instant};

use dioxus::prelude::{ReadableExt, WritableExt};
use hikasa_session::ipc::ViewId;
use serde_json::{Value, json};

use crate::app::{UiMsg, ViewModel};
use crate::mailbox::Mailbox;
use crate::ui::Ui;

/// Which benchmark runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchKind {
    /// Text Compare: first screen and scrolling (ARCH-GUI-003 (a)).
    Scroll,
    /// Folder Compare: incremental rendering while scanning (acceptance (b)).
    Folder,
}

/// Benchmark state (UI thread).
#[derive(Debug)]
pub struct BenchState {
    kind: BenchKind,
    mailbox: Mailbox<UiMsg>,
    process_start: Instant,
    view: Option<ViewId>,
    view_opened: Option<Instant>,
    page_ready: Option<Instant>,
    first_screen: Option<Instant>,
    ready_screen: Option<Instant>,
    started_script: bool,
    stop_sent: Option<Instant>,
    scan_done: Option<Instant>,
    ticks: Vec<f64>,
    ticks_busy: Vec<f64>,
    renders: Vec<f64>,
    renders_busy: Vec<f64>,
    slices: Vec<f64>,
    script: Option<Value>,
    memory: Option<Value>,
    extra: Option<Value>,
}

impl BenchState {
    /// A benchmark started at `process_start`.
    pub fn new(kind: BenchKind, process_start: Instant, mailbox: Mailbox<UiMsg>) -> BenchState {
        BenchState {
            kind,
            mailbox,
            process_start,
            view: None,
            view_opened: None,
            page_ready: None,
            first_screen: None,
            ready_screen: None,
            started_script: false,
            stop_sent: None,
            scan_done: None,
            ticks: Vec::new(),
            ticks_busy: Vec::new(),
            renders: Vec::new(),
            renders_busy: Vec::new(),
            slices: Vec::new(),
            script: None,
            memory: None,
            extra: None,
        }
    }

    /// The view under test, opened now.
    pub fn set_view(&mut self, view: ViewId) {
        self.view = Some(view);
        self.view_opened = Some(Instant::now());
    }

    /// One UI-thread message batch took `d`.
    pub fn record_tick(&mut self, d: Duration, _changed: bool) {
        let ms = d.as_secs_f64() * 1000.0;
        if self.ticks.len() < 1_000_000 {
            self.ticks.push(ms);
            if self.scan_done.is_none() && self.kind == BenchKind::Folder {
                self.ticks_busy.push(ms);
            }
        }
    }

    /// One render of the root component took `d`.
    pub fn record_render(&mut self, d: Duration) {
        let ms = d.as_secs_f64() * 1000.0;
        if self.renders.len() < 1_000_000 {
            self.renders.push(ms);
            if self.scan_done.is_none() && self.kind == BenchKind::Folder {
                self.renders_busy.push(ms);
            }
        }
    }

    /// Building one slice of rows (Rust side of a page request) took `d`.
    pub fn record_slice(&mut self, d: Duration) {
        if self.slices.len() < 1_000_000 {
            self.slices.push(d.as_secs_f64() * 1000.0);
        }
    }

    fn since_start(&self, at: Option<Instant>) -> Value {
        at.map_or(Value::Null, |t| {
            json!(round(
                t.duration_since(self.process_start).as_secs_f64() * 1000.0
            ))
        })
    }

    fn since_view(&self, at: Option<Instant>) -> Value {
        match (self.view_opened, at) {
            (Some(v), Some(t)) => json!(round(t.duration_since(v).as_secs_f64() * 1000.0)),
            _ => Value::Null,
        }
    }

    /// The JSON summary (printed when the process quits).
    pub fn final_report(&self) -> Option<String> {
        let mut report = json!({
            "bench": match self.kind { BenchKind::Scroll => "text_scroll", BenchKind::Folder => "folder_scan" },
            "memory": self.memory.clone().unwrap_or(Value::Null),
            "ui_thread_ms": {
                "message_batches": stats(&self.ticks),
                "root_renders": stats(&self.renders),
            },
            "page": self.script.clone().unwrap_or(Value::Null),
        });
        if let Some(obj) = report.as_object_mut() {
            match self.kind {
                BenchKind::Scroll => {
                    obj.insert(
                        "page_ready_ms".into(),
                        json!({
                            "from_process_start": self.since_start(self.page_ready),
                            "from_view_open": self.since_view(self.page_ready),
                        }),
                    );
                    obj.insert("slice_build_ms".into(), stats(&self.slices));
                    obj.insert(
                        "first_screen_ms".into(),
                        json!({
                            "from_process_start": self.since_start(self.first_screen),
                            "from_view_open": self.since_view(self.first_screen),
                        }),
                    );
                    obj.insert(
                        "coloured_screen_ms".into(),
                        json!({
                            "from_process_start": self.since_start(self.ready_screen),
                            "from_view_open": self.since_view(self.ready_screen),
                        }),
                    );
                }
                BenchKind::Folder => {
                    obj.insert(
                        "scan_ms_from_view_open".into(),
                        self.since_view(self.scan_done),
                    );
                    obj.insert(
                        "ui_thread_ms_while_scanning".into(),
                        json!({
                            "message_batches": stats(&self.ticks_busy),
                            "root_renders": stats(&self.renders_busy),
                        }),
                    );
                }
            }
            if let Some(extra) = &self.extra {
                obj.insert("view".into(), extra.clone());
            }
        }
        serde_json::to_string(&report).ok()
    }
}

fn round(ms: f64) -> f64 {
    (ms * 100.0).round() / 100.0
}

/// Count, mean, p50, p95, p99, max and the share above one 60 Hz frame (16.7 ms) of `samples` (ms).
pub fn stats(samples: &[f64]) -> Value {
    if samples.is_empty() {
        return json!({ "count": 0 });
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let pick = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let over = sorted.iter().filter(|&&x| x > 1000.0 / 60.0).count();
    json!({
        "count": sorted.len(),
        "mean": round(sorted.iter().sum::<f64>() / sorted.len() as f64),
        "p50": round(pick(0.5)),
        "p95": round(pick(0.95)),
        "p99": round(pick(0.99)),
        "max": round(pick(1.0)),
        "over_16_7ms": over,
    })
}

/// The page drew a screen of `view` in `phase` (1: the loaded text, unaligned; 2: the aligned comparison with
/// syntax colours).
pub fn drawn(ui: Ui, view: u64, phase: u32) {
    {
        let mut bench = ui.bench;
        let mut guard = bench.write();
        let Some(b) = guard.as_mut() else {
            return;
        };
        if b.view != Some(ViewId(view)) {
            return;
        }
        let now = Instant::now();
        if b.page_ready.is_none() {
            b.page_ready = Some(now);
        }
        if phase >= 1 && b.first_screen.is_none() {
            b.first_screen = Some(now);
        }
        if phase >= 2 && b.ready_screen.is_none() {
            b.ready_screen = Some(now);
        }
    }
    // The UI may be idle now (no job, no tick): start the script from here.
    after_frame(ui);
}

/// The page script finished: keep its results, measure memory and quit.
pub fn results(ui: Ui, body: &[u8]) {
    let script: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let extra = view_summary(ui);
    {
        let mut bench = ui.bench;
        let mut guard = bench.write();
        let Some(b) = guard.as_mut() else {
            return;
        };
        b.script = Some(script);
        b.memory = Some(memory());
        b.extra = extra;
    }
    ui.act(|app| app.shutdown());
}

/// Facts about the view under test (rows, timings, rebuilds).
fn view_summary(ui: Ui) -> Option<Value> {
    let view = ui.bench.read().as_ref()?.view?;
    let app = ui.app.read();
    let tab = app.tab(view)?;
    Some(match &tab.model {
        ViewModel::Text(m) => {
            let (loaded, ready) = m.timings();
            let rows = m.response().map_or(0, |r| r.comparison.row_count());
            let lines = m.response().map_or((0, 0), |r| {
                (
                    r.comparison.document(hikasa_core::Side::Left).line_count(),
                    r.comparison.document(hikasa_core::Side::Right).line_count(),
                )
            });
            json!({
                "rows": rows,
                "lines": [lines.0, lines.1],
                "format": m.response().map(|r| r.format.name.clone()),
                "algorithm": m.response().map(|r| format!("{:?}", r.comparison.algorithm())),
                "documents_loaded_ms": loaded.map(|d| d.as_millis() as u64),
                "comparison_done_ms": ready.map(|d| d.as_millis() as u64),
            })
        }
        ViewModel::Folder(m) => {
            let stats = m.rebuild_stats();
            let files = m.prepared().map(|p| {
                let read = p.model().read();
                let s = read.stats(false);
                (s.left.files, s.right.files)
            });
            json!({
                "visible_rows": stats.rows,
                "rebuilds_while_scanning": stats.during_scan,
                "rebuild_ms": self::stats(&stats.samples),
                "scan_events": m.events(),
                "files": files.map(|f| [f.0, f.1]),
                "scan_ms": m.finished_after().map(|d| d.as_millis() as u64),
            })
        }
        ViewModel::Home(_) => Value::Null,
    })
}

/// Called after every UI message batch: starts the page script when the view is ready, stops the folder
/// script after the scan.
pub fn after_frame(ui: Ui) {
    let (kind, view, started, ready, stop_sent, scan_done) = {
        let bench = ui.bench.read();
        let Some(b) = bench.as_ref() else {
            return;
        };
        (
            b.kind,
            b.view,
            b.started_script,
            b.ready_screen.is_some(),
            b.stop_sent,
            b.scan_done,
        )
    };
    let Some(view) = view else {
        return;
    };
    match kind {
        BenchKind::Scroll => {
            if ready && !started {
                let _ = dioxus::document::eval(&format!(
                    "window.hikasa && window.hikasa.benchScroll({})",
                    view.0
                ));
                mark_started(ui);
            }
        }
        BenchKind::Folder => {
            if !started {
                let _ = dioxus::document::eval(&format!(
                    "window.hikasa && window.hikasa.benchFolder({})",
                    view.0
                ));
                mark_started(ui);
                return;
            }
            let busy = ui
                .app
                .read()
                .tab(view)
                .is_some_and(|t| matches!(&t.model, ViewModel::Folder(m) if m.busy()));
            let mut bench = ui.bench;
            if !busy
                && scan_done.is_none()
                && let Some(b) = bench.write().as_mut()
            {
                b.scan_done = Some(Instant::now());
                let _ =
                    dioxus::document::eval("window.hikasa && window.hikasa.benchFolderScanDone()");
                // The ticker stops with the scan: wake the UI once more to stop the page script.
                let mailbox = b.mailbox.clone();
                let _ = std::thread::Builder::new()
                    .name("hikasa-gui-bench".into())
                    .spawn(move || {
                        std::thread::sleep(Duration::from_millis(600));
                        mailbox.post(UiMsg::Wake);
                    });
            }
            // Let the last rebuilds reach the page, then stop the script.
            if let Some(done) = scan_done
                && stop_sent.is_none()
                && done.elapsed() >= Duration::from_millis(500)
            {
                let _ = dioxus::document::eval("window.hikasa && window.hikasa.benchFolderStop()");
                if let Some(b) = bench.write().as_mut() {
                    b.stop_sent = Some(Instant::now());
                }
            }
        }
    }
}

fn mark_started(ui: Ui) {
    let mut bench = ui.bench;
    if let Some(b) = bench.write().as_mut() {
        b.started_script = true;
    }
}

/// The ARCH-GUI-003 memory limit: 400 MB (decimal) in MiB.
pub const LIMIT_MIB: f64 = 400.0 * 1_000_000.0 / 1_048_576.0;

/// Memory of this process and of its descendants (the web content and network processes), in MiB
/// (1 MiB = 1,048,576 bytes). The multi-process figure is the sum of the *proportional* set sizes (PSS,
/// `/proc/<pid>/smaps_rollup`): pages shared between the processes (the WebKit and GTK libraries, shared
/// buffers) count once in total, split between their users; summed RSS would count them once per process.
/// RSS is kept alongside, and the GUI process's own RSS is reported separately.
pub fn memory() -> Value {
    let me = std::process::id();
    let own = status_of(me);
    let mut children = Vec::new();
    let (mut pss, mut rss, mut hwm) = (own.pss_kib, own.rss_kib, own.hwm_kib);
    for pid in descendants(me) {
        let s = status_of(pid);
        pss += s.pss_kib;
        rss += s.rss_kib;
        hwm += s.hwm_kib;
        children.push(json!({
            "pid": pid,
            "name": s.name,
            "pss_mib": mib(s.pss_kib),
            "rss_mib": mib(s.rss_kib),
            "peak_rss_mib": mib(s.hwm_kib),
        }));
    }
    json!({
        "unit": "MiB",
        "limit_mib": round(LIMIT_MIB),
        "gui_rss_mib": mib(own.rss_kib),
        "gui_peak_rss_mib": mib(own.hwm_kib),
        "gui_pss_mib": mib(own.pss_kib),
        "total_pss_mib": mib(pss),
        "total_rss_mib": mib(rss),
        "total_peak_rss_mib": mib(hwm),
        "processes": children,
    })
}

fn mib(kib: u64) -> f64 {
    round(kib as f64 / 1024.0)
}

#[derive(Default)]
struct ProcStatus {
    name: String,
    rss_kib: u64,
    hwm_kib: u64,
    pss_kib: u64,
}

/// `Name`, `VmRSS` and `VmHWM` from `/proc/<pid>/status`, `Pss` from `/proc/<pid>/smaps_rollup`.
fn status_of(pid: u32) -> ProcStatus {
    let mut out = ProcStatus::default();
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
        parse_status(&text, &mut out);
    }
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        out.pss_kib = kib_field(&text, "Pss:").unwrap_or(0);
    }
    out
}

fn parse_status(text: &str, out: &mut ProcStatus) {
    if let Some(name) = text.lines().find_map(|l| l.strip_prefix("Name:")) {
        out.name = name.trim().to_owned();
    }
    out.rss_kib = kib_field(text, "VmRSS:").unwrap_or(0);
    out.hwm_kib = kib_field(text, "VmHWM:").unwrap_or(0);
}

/// The kB value of the line starting with `key` (`Pss:   1234 kB`).
fn kib_field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

/// Every descendant process of `root` (Linux `/proc`; empty elsewhere).
fn descendants(root: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut parent_of = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // `pid (comm) state ppid …`: comm may contain spaces, so read after the last ')'.
        let ppid = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u32>().ok());
        if let Some(ppid) = ppid {
            parent_of.push((pid, ppid));
        }
    }
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for &(pid, ppid) in &parent_of {
            if ppid == p && pid != root && !out.contains(&pid) {
                out.push(pid);
                frontier.push(pid);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_of_samples() {
        let v = stats(&[1.0, 2.0, 3.0, 4.0, 20.0]);
        assert_eq!(v["count"], 5);
        assert_eq!(v["p50"], 3.0);
        assert_eq!(v["max"], 20.0);
        assert_eq!(v["over_16_7ms"], 1);
        assert_eq!(stats(&[])["count"], 0);
    }

    #[test]
    fn memory_of_this_process() {
        let m = memory();
        if cfg!(target_os = "linux") {
            assert!(m["gui_rss_mib"].as_f64().unwrap_or(0.0) > 0.0, "{m}");
            let pss = m["gui_pss_mib"].as_f64().unwrap_or(0.0);
            assert!(
                pss > 0.0 && pss <= m["gui_rss_mib"].as_f64().unwrap_or(0.0),
                "{m}"
            );
        }
        assert!((LIMIT_MIB - 381.47).abs() < 0.01);
    }

    #[test]
    fn proc_fields_parse() {
        let rollup = "55d0-7ffc ---p 00000000 00:00 0  [rollup]\nRss:              164000 kB\nPss:              117760 kB\n";
        assert_eq!(kib_field(rollup, "Pss:"), Some(117_760));
        assert_eq!(kib_field(rollup, "Pss_Anon:"), None);
        let mut s = ProcStatus::default();
        parse_status(
            "Name:\tWebKitWebProcess\nVmHWM:\t  2048 kB\nVmRSS:\t  1024 kB\n",
            &mut s,
        );
        assert_eq!(
            (s.name.as_str(), s.rss_kib, s.hwm_kib),
            ("WebKitWebProcess", 1024, 2048)
        );
        assert_eq!(mib(117_760), 115.0);
    }
}

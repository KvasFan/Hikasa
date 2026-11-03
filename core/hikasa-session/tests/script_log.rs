//! A8.9 (the log, D-224) and A8.15's stream row (D-231): the line format at each level, the file of `log` (created,
//! truncated, appended, switched, refused when not local, none in a dry run), the end lines with a fixed and an advancing
//! clock, and a run that logs over a million lines with a bounded peak of live heap (a counting allocator: the lines are
//! never kept).
#![allow(unsafe_code)]

mod common;
#[path = "script_common/mod.rs"]
mod script_common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use hikasa_core::ExitCode;
use hikasa_session::api::script::{ScriptEvent, ScriptRunRequest};
use hikasa_session::{Clock, Service, ServiceInit};
use script_common::{Ui, World, yes};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call goes to `System` with the same arguments; only the counters are added.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: forwarded to `System` with the caller's contract.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is forwarded unchanged.
        let block = unsafe { System.alloc(layout) };
        if !block.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        block
    }

    // SAFETY: forwarded to `System` with the caller's contract.
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is forwarded unchanged.
        unsafe { System.dealloc(block, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// One measurement at a time: the counters are global.
static MEASURE: Mutex<()> = Mutex::new(());

fn world() -> World {
    let w = World::new();
    w.file("l/a.txt", "left a\n", 2_000_000);
    w.file("r/a.txt", "right a, longer\n", 1_000_000);
    w.file("l/b.txt", "b\n", 1_000_000);
    w
}

#[test]
fn a8_9_levels_and_the_line_format() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let w = world();
    let log = w.path("x.log");
    let source = format!(
        "log verbose {}\nload l r\nselect left.all.files\ncopy left->right\nlog none\nbeep\nsnapshot left\n",
        log.display()
    );
    let (outcome, ui) = w.run(&source);
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    let file = std::fs::read_to_string(&log).unwrap();
    let shown: Vec<&str> = ui.lines.iter().map(String::as_str).collect();
    let script = w.path("s.txt").display().to_string();
    let start = format!("info - script {script}");
    let mut expected = vec![start.as_str(), "run L1 log verbose"];
    // The `run` line of line 1 holds the expanded path; compare its start.
    assert!(shown[1].starts_with(expected[1]), "{shown:?}");
    expected.remove(1);
    assert_eq!(shown[0], expected[0]);
    assert!(shown.contains(&"run L2 load l r"), "{shown:?}");
    assert!(shown.contains(&"info L3 2 items selected"), "{shown:?}");
    assert!(
        shown.iter().any(|l| l.starts_with("detail L4 R replace ")),
        "verbose details: {shown:?}"
    );
    assert!(
        shown
            .iter()
            .any(|l| l.starts_with("info L4 copied 2 of 2 items")),
        "{shown:?}"
    );
    // `log none`: errors only from line 5 on (its own run line is still at the old level).
    assert!(!shown.contains(&"run L6 beep"), "{shown:?}");
    assert!(shown.contains(&"error L7 snapshot is not available in this version (snapshots arrive with a later release)"));
    // The file has every line from the `log` line on, with the date and time of the fixed clock (UTC).
    let first = file.lines().next().unwrap();
    assert!(first.starts_with("2026-10-21 12:00:00 "), "{first}");
    assert!(file.lines().all(|l| l.starts_with("2026-10-21 12:00:00 ")));
    assert!(file.contains(" info L4 copied 2 of 2 items"));
    // From `log none` on, errors only: the end line is an `info` line and is not written.
    assert!(
        file.ends_with("2026-10-21 12:00:00 error L7 snapshot is not available in this version (snapshots arrive with a later release)\n"),
        "{file}"
    );
}

#[test]
fn a8_9_files_created_truncated_appended_switched_and_refused() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let w = world();
    let a = w.path("a.log");
    let b = w.path("b.log");
    std::fs::write(&a, "old\n").unwrap();
    std::fs::write(&b, "kept\n").unwrap();
    let source = format!(
        "log {}\nbeep\nlog append:{}\nbeep\n",
        a.display(),
        b.display()
    );
    let (_, ui) = w.run(&source);
    let a_text = std::fs::read_to_string(&a).unwrap();
    let b_text = std::fs::read_to_string(&b).unwrap();
    assert!(!a_text.contains("old"), "truncated: {a_text}");
    assert!(
        a_text.contains("info L2 beep") && !a_text.contains("L4"),
        "{a_text}"
    );
    assert!(
        b_text.starts_with("kept\n") && b_text.contains("info L4 beep"),
        "{b_text}"
    );
    assert!(ui.lines.iter().any(|l| l == "info L4 beep"));
    // A remote spec is refused; the stream goes on.
    let (outcome, ui) = w.run("log sftp://host/x.log\nbeep\n");
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(
        ui.lines
            .contains(&"error L1 a script log must be a local file".to_owned()),
        "{:?}",
        ui.lines
    );
    assert!(ui.lines.contains(&"info L2 beep".to_owned()));
    // A dry run writes no file.
    let c = w.path("c.log");
    let mut ui = Ui::default();
    let req = ScriptRunRequest {
        dry_run: true,
        ..yes()
    };
    let outcome = w.run_with(&format!("log {}\nbeep\n", c.display()), req, &mut ui);
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert!(!c.exists());
    assert!(
        ui.lines
            .iter()
            .any(|l| l.contains("[log_not_written] log file not written in a dry run")),
        "{:?}",
        ui.lines
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a8_9_a_write_fault_gives_the_file_up_and_the_stream_goes_on() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let w = world();
    let (outcome, ui) = w.run("log /dev/full\nbeep\nbeep\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        ui.lines
            .iter()
            .filter(|l| l.as_str() == "info L3 beep")
            .count(),
        1
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.code.as_str() == "log_not_written")
    );
}

/// A clock that advances 61 seconds at each reading.
struct Advancing(AtomicI64);

impl Clock for Advancing {
    fn now(&self) -> jiff::Zoned {
        let secs = self.0.fetch_add(61, Ordering::Relaxed);
        jiff::Timestamp::from_second(secs)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC)
    }
}

#[test]
fn a8_9_the_minute_form_of_the_end_line() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = common::short_tempdir();
    let init = ServiceInit {
        config_dir: Some(dir.path().join("config")),
        ..ServiceInit::new(
            common::facts(dir.path()),
            Arc::new(Advancing(AtomicI64::new(1_792_584_000))),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    let w = World { dir, service };
    let (_, ui) = w.run("beep\n");
    let end = ui.lines.last().unwrap();
    assert!(
        end.starts_with("info - Script completed in ") && end.ends_with(" s (exit 0)"),
        "{end}"
    );
    assert!(end.contains(" min "), "{end}");
}

/// One counting interface: the log lines it was given, and the peak of live heap when the run finished (before the
/// Service turns the interpreter's records into its own).
struct Counter(u64, usize);

impl hikasa_session::api::script::HostUi for Counter {
    fn event(&mut self, event: hikasa_session::api::script::ScriptEvent) {
        match event {
            hikasa_session::api::script::ScriptEvent::Log { .. } => self.0 += 1,
            hikasa_session::api::script::ScriptEvent::Finished { .. } => {
                self.1 = PEAK.load(Ordering::Relaxed);
            }
            _ => {}
        }
    }
    fn can_ask(&self) -> bool {
        false
    }
    fn ask_continue(&mut self, _: u32, _: &str) -> bool {
        false
    }
    fn beep(&mut self) {}
}

impl hikasa_session::api::script::ScriptUi for Counter {
    fn answer(&mut self, _: &mut hikasa_folder::ops::Confirmation, _: &hikasa_core::CancelToken) {}
    fn clipboard(
        &mut self,
        _: &hikasa_session::clipboard::ClipboardPayload,
    ) -> Result<(), hikasa_session::ApiError> {
        Ok(())
    }
}

/// The peak of live heap of a run of `source` above the heap before it, and the lines its interface got.
fn peak_of_run(w: &World, source: &str) -> (usize, u64) {
    let text = w.text(source);
    let check = w
        .service
        .script_check(&text, Some(&[]), &Default::default());
    let script = check.script.unwrap();
    let mut ui = Counter(0, 0);
    let req = ScriptRunRequest::default();
    let job = common::job(&w.service, hikasa_core::JobKind::Script);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let outcome = w
        .service
        .script_run(&script, &text, &req, &mut ui, None, &job);
    let peak = ui.1.saturating_sub(base);
    drop(outcome);
    (peak, ui.0)
}

/// A8.15, the stream row of §1.8.17: a run whose log has over a million lines keeps none of them. The same script at
/// level `none` (its lines dropped at the sink) has the same records; the run that logs every line peaks within 4 MiB of
/// it, so the log path holds no line after it was written.
#[test]
fn a8_15_a_million_log_lines_stream() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new();
    let body = "beep\n".repeat(500_000);
    let (quiet, few) = peak_of_run(&w, &format!("log none\n{body}"));
    let (logged, lines) = peak_of_run(&w, &format!("log normal\n{body}"));
    assert!(lines > 1_000_000, "{lines} lines");
    assert!(few < 10, "{few} lines at level none");
    assert!(
        logged.saturating_sub(quiet) <= 4 << 20,
        "peak {logged} bytes logging {lines} lines, {quiet} bytes at level none"
    );
}

/// §1.8.9, OPT-021: `[tweaks.scripts] beep_when_finished` beeps once at the end of a run (any exit code), never in
/// silent mode, not at all when off; a terminal decides only whether the CLI writes BEL (`beep` lines unchanged).
#[test]
fn beep_when_finished_beeps_once_at_the_end_unless_silent() {
    let mut w = world();
    let script = "load l r\nselect all\n";
    let (_, ui) = w.run(script);
    assert_eq!(ui.beeps, 0, "off by default");
    std::fs::create_dir_all(w.dir.path().join("config")).unwrap();
    std::fs::write(
        w.dir.path().join("config/options.toml"),
        "[tweaks.scripts]\nbeep_when_finished = true\n",
    )
    .unwrap();
    w.service = common::service(w.dir.path());
    let (outcome, ui) = w.run(script);
    assert_eq!(ui.beeps, 1, "{:?}", ui.lines);
    assert!(matches!(
        ui.events.last(),
        Some(ScriptEvent::Finished { .. })
    ));
    let (_, ui) = w.run("beep\nload l nothere\n");
    assert_eq!(
        ui.beeps, 2,
        "the beep command, then the end (a failed run too)"
    );
    let mut ui = Ui::default();
    let silent = ScriptRunRequest {
        silent: true,
        ..yes()
    };
    w.run_with(script, silent, &mut ui);
    assert_eq!(ui.beeps, 0, "never in silent mode");
    let _ = outcome;
}

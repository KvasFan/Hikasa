//! The W13c benchmark modes of §1.13.21 (M2 W13c, D-372), driven by `tests/perf_gate.rs` through
//! `HIKASA_GUI_BENCH`:
//!
//! * `views_mixed` (ARCH-GUI-003 views): the command line's hex pair opens first; then a Text Compare tab of the pair
//!   in `HIKASA_BENCH_MIXED_TEXT` and a Picture Compare tab of the pair in `HIKASA_BENCH_MIXED_PICTURE` (each two paths
//!   joined as `PATH` joins them), each shown once (the picture tab last), then 60 idle frames; the memory is sampled
//!   at the end with its validity.
//! * `report_window` (G13 in the window): the command line's text pair; once compared, the report dialog writes the
//!   side-by-side HTML report to `HIKASA_BENCH_REPORT_FILE`; the UI thread's work of each frame while it writes.

use std::time::Instant;

use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};
use serde_json::{Value, json};

use crate::app::{AppCore, ViewModel};
use crate::bench::{round, stats};
use crate::report_dialog::{ReportAction, ReportDialog, ReportTo};

/// Which W13c benchmark runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixedKind {
    /// `views_mixed`.
    Views,
    /// `report_window`.
    Report,
}

/// Idle frames at the end of `views_mixed`.
const IDLE_FRAMES: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Waiting for the first view's comparison.
    First,
    /// The text tab opened; waiting for its comparison.
    Text,
    /// The picture tab opened; waiting for its comparison.
    Picture,
    /// Idle frames.
    Idle(u32),
    /// The report dialog writes.
    Writing,
    /// Done.
    Done,
}

/// The state of a W13c benchmark.
#[derive(Debug)]
pub struct MixedBench {
    kind: MixedKind,
    step: Step,
    frames: u32,
    views: Vec<(ViewKind, ViewId)>,
    work_while_writing: Vec<f64>,
    write_started: Option<Instant>,
    write_ms: Option<f64>,
    message: Option<String>,
    sample: Value,
    tabs_shown: Vec<&'static str>,
}

impl MixedBench {
    /// A benchmark of `kind`.
    pub fn new(kind: MixedKind) -> MixedBench {
        MixedBench {
            kind,
            step: Step::First,
            frames: 0,
            views: Vec::new(),
            work_while_writing: Vec::new(),
            write_started: None,
            write_ms: None,
            message: None,
            sample: Value::Null,
            tabs_shown: Vec::new(),
        }
    }

    /// The benchmark's name.
    pub fn name(&self) -> &'static str {
        match self.kind {
            MixedKind::Views => "views_mixed",
            MixedKind::Report => "report_window",
        }
    }

    /// True once the script ended.
    pub fn finished(&self) -> bool {
        self.step == Step::Done
    }

    /// True when the view's comparison is there.
    fn landed(core: &mut AppCore, view: ViewId) -> bool {
        let service = core.service_handle();
        match core.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Text(m)) => m.response().is_some() && !m.busy(),
            Some(ViewModel::Hex(m)) => m.response().is_some(),
            Some(ViewModel::Picture(m)) => m.response(&service).is_some() && !m.busy(),
            _ => false,
        }
    }

    /// Opens the pair of the `PATH`-joined `var` in a view of `kind` (a new tab of the window).
    fn open(core: &mut AppCore, var: &str, kind: ViewKind) -> Option<ViewId> {
        let joined = std::env::var_os(var)?;
        let specs: Vec<String> = std::env::split_paths(&joined)
            .map(|p| p.display().to_string())
            .collect();
        let options = OpenOptions::default();
        let request = core
            .service_handle()
            .open_view_request(kind, &specs, &options)
            .ok()?;
        let target = core.open_target();
        Some(core.open_local(kind, request, &options, specs, target))
    }

    /// The frame's step, before the window draws (the active view is `first`).
    pub fn drive(&mut self, core: &mut AppCore, first: Option<ViewId>) {
        self.frames += 1;
        match self.step {
            Step::First => {
                let Some(view) = first else { return };
                if !Self::landed(core, view) {
                    return;
                }
                let kind = match core.tab_mut(view).and_then(|t| t.model_mut()) {
                    Some(ViewModel::Text(_)) => ViewKind::TextCompare,
                    Some(ViewModel::Picture(_)) => ViewKind::PictureCompare,
                    _ => ViewKind::HexCompare,
                };
                self.views.push((kind, view));
                self.tabs_shown.push("first");
                match self.kind {
                    MixedKind::Views => {
                        if let Some(v) =
                            Self::open(core, "HIKASA_BENCH_MIXED_TEXT", ViewKind::TextCompare)
                        {
                            self.views.push((ViewKind::TextCompare, v));
                            self.tabs_shown.push("text");
                        }
                        self.step = Step::Text;
                    }
                    MixedKind::Report => self.start_report(core),
                }
            }
            Step::Text => {
                let done = self
                    .views
                    .last()
                    .is_none_or(|(_, v)| Self::landed(core, *v));
                if done {
                    if let Some(v) =
                        Self::open(core, "HIKASA_BENCH_MIXED_PICTURE", ViewKind::PictureCompare)
                    {
                        self.views.push((ViewKind::PictureCompare, v));
                        self.tabs_shown.push("picture");
                    }
                    self.step = Step::Picture;
                }
            }
            Step::Picture => {
                let done = self
                    .views
                    .last()
                    .is_none_or(|(_, v)| Self::landed(core, *v));
                if done {
                    self.step = Step::Idle(0);
                }
            }
            Step::Idle(n) if n >= IDLE_FRAMES => {
                self.sample = crate::bench_picture::sample_validity();
                self.step = Step::Done;
            }
            Step::Idle(n) => self.step = Step::Idle(n + 1),
            Step::Writing => {
                let root = core.root();
                let dialog = core
                    .window_mut(root)
                    .and_then(|w| w.dialogs.find_mut::<ReportDialog>());
                if let Some(d) = dialog
                    && !d.running()
                    && d.message.is_some()
                {
                    self.write_ms = self
                        .write_started
                        .map(|t| round(t.elapsed().as_secs_f64() * 1000.0));
                    self.message = d.message.as_ref().map(|m| m.1.clone());
                    self.step = Step::Done;
                }
            }
            Step::Done => {}
        }
    }

    /// `report_window`: opens the report dialog and presses *Write Report* (side-by-side HTML to the file).
    fn start_report(&mut self, core: &mut AppCore) {
        let root = core.root();
        core.execute(root, crate::commands::CommandId::Report);
        let file = std::env::var("HIKASA_BENCH_REPORT_FILE").unwrap_or_default();
        let Some(d) = core
            .window_mut(root)
            .and_then(|w| w.dialogs.find_mut::<ReportDialog>())
        else {
            self.message = Some("no report dialog".into());
            self.step = Step::Done;
            return;
        };
        d.layout = d.vocabulary().layouts[0].word;
        d.output = d.vocabulary().outputs[0].word;
        d.to = ReportTo::File;
        d.path = file;
        let _ = d.flags(ReportAction::Write);
        core.dialog_button(root, crate::dialog::ButtonRole::Ok);
        self.write_started = Some(Instant::now());
        self.step = Step::Writing;
    }

    /// The frame's work (ms of the UI thread).
    pub fn end_frame(&mut self, work: f64) {
        if self.step == Step::Writing {
            self.work_while_writing.push(work);
        }
    }

    /// The summary.
    pub fn report(&self) -> Value {
        json!({
            "views": self.views.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            "tabs_shown": self.tabs_shown,
            "sample": self.sample,
            "frame_work_while_writing_ms": stats(&self.work_while_writing),
            "write_ms": self.write_ms,
            "message": self.message,
            "frames": self.frames,
        })
    }
}

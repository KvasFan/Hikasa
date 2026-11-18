//! The G10 rows of the window (M2 W13a, contract §1.13.21, D-372): the scripted benchmark modes of the Hex Compare
//! view, driven by `tests/perf_gate.rs` through `HIKASA_GUI_BENCH`:
//!
//! * `hex_open`: the first painted rows (provisional) from the process start and from the view's open; the landed
//!   alignment; placeholder frames.
//! * `hex_scroll`: after Fast landed, 3 rows a frame × 600, a page a frame × 600, 300 jumps to seeded places one every
//!   2 frames; the three again under *Differences* with 3 context rows; frame work and placeholder frames per phase;
//!   the memory at the end.
//! * `hex_align`: the mode of `HIKASA_BENCH_HEX_MODE` asked at the landing of the first alignment while the view
//!   scrolls 3 rows a frame; the frame work while it runs and the time to its landing.
//! * `hex_find`: Find Next of `HIKASA_BENCH_HEX_FIND` (hex bytes), then of `HIKASA_BENCH_HEX_FIND2`; the frame work
//!   while searching and the time to each result; then a search stopped by Esc after 5 frames.

use std::collections::HashMap;
use std::time::Instant;

use hikasa_hex::DisplayFilter;
use hikasa_session::ipc::ViewId;
use serde_json::{Value, json};

use crate::app::{AppCore, ViewModel};
use crate::hex::HexViewModel;
use crate::ui::hex::HexUi;

/// The memory sample with the process's swapped pages and the machine's available memory (a sample is valid only
/// without swapped pages and with ≥ 10 GB available, W11a review F9).
fn sample() -> Value {
    let mut m = crate::bench::memory();
    let field = |file: &str, key: &str| -> Option<f64> {
        let text = std::fs::read_to_string(file).ok()?;
        let line = text.lines().find(|l| l.starts_with(key))?;
        line.split([' ', '\t'])
            .filter(|w| !w.is_empty())
            .nth(1)?
            .parse::<f64>()
            .ok()
    };
    if let Some(obj) = m.as_object_mut() {
        obj.insert(
            "gui_swap_mib".into(),
            json!(field("/proc/self/status", "VmSwap:").map(|k| k / 1024.0)),
        );
        obj.insert(
            "available_gib".into(),
            json!(
                field("/proc/meminfo", "MemAvailable:")
                    .map(|k| (k / 1024.0 / 1024.0 * 10.0).round() / 10.0)
            ),
        );
    }
    m
}

/// Which hex benchmark runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexBenchKind {
    /// `hex_open`.
    Open,
    /// `hex_scroll`.
    Scroll,
    /// `hex_align`.
    Align,
    /// `hex_find`.
    Find,
}

/// One scroll phase of `hex_scroll`.
#[derive(Debug, Clone, Copy)]
enum Move {
    Rows(u64),
    Page,
    Jump,
}

const SCROLL_PHASES: [(&str, Move, u32); 3] = [
    ("wheel_3_rows_per_frame", Move::Rows(3), 600),
    ("page_per_frame", Move::Page, 600),
    ("thumb_jumps_every_2_frames", Move::Jump, 600),
];

/// Frames after the landing before a script starts, and at its end.
const WARM_UP: u32 = 30;

/// A phase's samples.
#[derive(Debug, Default)]
struct PhaseRun {
    name: String,
    work: Vec<f64>,
    placeholder_frames: u32,
    /// Placeholder frames later than 2 frames after a jump.
    late_placeholders: u32,
}

/// The state of a hex benchmark.
#[derive(Debug)]
pub struct HexBench {
    kind: HexBenchKind,
    view: Option<ViewId>,
    opened: Option<Instant>,
    first_rows: Option<Instant>,
    landed: Option<Instant>,
    frame: u32,
    phase: Option<(usize, bool, u32, PhaseRun)>,
    phases: Vec<Value>,
    since_jump: u32,
    rng: u64,
    /// `hex_align`: when the mode was asked and when it landed; the work while it ran.
    asked: Option<Instant>,
    realigned: Option<Instant>,
    align_work: Vec<f64>,
    /// `hex_find`: the searches (name, started, ended, work while running).
    searches: Vec<(String, Instant, Option<Instant>, Vec<f64>)>,
    cancel_ms: Option<f64>,
    /// `hex_align`: the landed comparison as `hex compare --json` writes it (the gate compares it with the CLI's).
    landed_json: Value,
    stage: u32,
    done: bool,
    memory_settled: Value,
    memory_first_rows: Value,
}

impl HexBench {
    /// A benchmark of `kind`.
    pub fn new(kind: HexBenchKind) -> HexBench {
        HexBench {
            kind,
            view: None,
            opened: None,
            first_rows: None,
            landed: None,
            frame: 0,
            phase: None,
            phases: Vec::new(),
            since_jump: 0,
            rng: 0x5EED,
            asked: None,
            realigned: None,
            align_work: Vec::new(),
            searches: Vec::new(),
            cancel_ms: None,
            landed_json: Value::Null,
            stage: 0,
            done: false,
            memory_settled: Value::Null,
            memory_first_rows: Value::Null,
        }
    }

    /// The bench's name in the report.
    pub fn name(&self) -> &'static str {
        match self.kind {
            HexBenchKind::Open => "hex_open",
            HexBenchKind::Scroll => "hex_scroll",
            HexBenchKind::Align => "hex_align",
            HexBenchKind::Find => "hex_find",
        }
    }

    /// True when the script ended.
    pub fn finished(&self) -> bool {
        self.done
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn model(core: &mut AppCore, view: ViewId) -> Option<&mut HexViewModel> {
        match core.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Hex(m)) => Some(m),
            _ => None,
        }
    }

    /// The script's step for this frame (before the view draws).
    pub fn drive(&mut self, core: &mut AppCore) {
        if self.view.is_none() {
            let root = core.root();
            if let Some(tab) = core.active_tab(root)
                && matches!(tab.model(), Some(ViewModel::Hex(_)))
            {
                self.view = Some(tab.id);
                self.opened = Some(Instant::now());
            }
            return;
        }
        let Some(view) = self.view else {
            return;
        };
        let Some(m) = Self::model(core, view) else {
            return;
        };
        if self.landed.is_none() {
            if m.alignment().is_some() {
                self.landed = Some(Instant::now());
            } else {
                return;
            }
        }
        self.frame += 1;
        if self.frame < WARM_UP && self.stage < 3 {
            return;
        }
        match self.kind {
            HexBenchKind::Open => {
                if self.frame > 2 * WARM_UP {
                    self.memory_settled = sample();
                    self.done = true;
                }
            }
            HexBenchKind::Scroll => self.drive_scroll(core, view),
            HexBenchKind::Align => self.drive_align(core, view),
            HexBenchKind::Find => self.drive_find(core, view),
        }
    }

    fn drive_scroll(&mut self, core: &mut AppCore, view: ViewId) {
        let total = SCROLL_PHASES.len() * 2;
        if self.phase.is_none() {
            let next = self.phases.len();
            if next >= total {
                if self.stage == 0 {
                    self.memory_settled = sample();
                    self.stage = 1;
                }
                self.done = true;
                return;
            }
            let filtered = next >= SCROLL_PHASES.len();
            if let Some(m) = Self::model(core, view) {
                if filtered {
                    let layout = m.display().layout;
                    let panes = m.display().panes;
                    m.set_layout(layout, true, panes, 3);
                    m.set_filter(DisplayFilter::Differences);
                } else {
                    m.set_filter(DisplayFilter::All);
                }
                m.screen.set_top(0, m.row_count());
            }
            let (name, _, _) = SCROLL_PHASES[next % SCROLL_PHASES.len()];
            let name = if filtered {
                format!("differences_context_3_{name}")
            } else {
                name.to_owned()
            };
            self.phase = Some((
                next,
                filtered,
                0,
                PhaseRun {
                    name,
                    ..PhaseRun::default()
                },
            ));
            return;
        }
        let Some((index, _, frame, _)) = self.phase.as_mut() else {
            return;
        };
        let (_, mv, frames) = SCROLL_PHASES[*index % SCROLL_PHASES.len()];
        *frame += 1;
        let f = *frame;
        if f > frames {
            if let Some((_, filtered, _, run)) = self.phase.take() {
                self.phases.push(json!({
                    "name": run.name,
                    "filtered": filtered,
                    "frames": run.work.len(),
                    "frame_work_ms": crate::bench::stats(&run.work),
                    "placeholder_frames": run.placeholder_frames,
                    "placeholder_frames_later_than_2_after_a_jump": run.late_placeholders,
                }));
            }
            return;
        }
        let jump = matches!(mv, Move::Jump) && f % 2 == 1;
        let r = if jump { self.rand() } else { 0 };
        if jump {
            self.since_jump = 0;
        } else {
            self.since_jump += 1;
        }
        let Some(m) = Self::model(core, view) else {
            return;
        };
        let rows = m.row_count();
        let max = m.screen.max_top(rows);
        // A wrap from the end to the first row is a jump (the filtered views have a few thousand rows).
        let mut wrapped = false;
        match mv {
            Move::Rows(n) => {
                let top = m.screen.top + n;
                wrapped = top > max;
                m.screen.set_top(if wrapped { 0 } else { top }, rows);
            }
            Move::Page => {
                let page = u64::from(m.screen.rows_visible.max(1));
                let top = m.screen.top + page;
                wrapped = top > max;
                m.screen.set_top(if wrapped { 0 } else { top }, rows);
            }
            Move::Jump => {
                if jump {
                    m.screen.set_top(r % (max + 1), rows);
                }
            }
        }
        if wrapped {
            self.since_jump = 0;
        }
    }

    fn drive_align(&mut self, core: &mut AppCore, view: ViewId) {
        let Some(m) = Self::model(core, view) else {
            return;
        };
        if self.asked.is_none() {
            let mode = std::env::var("HIKASA_BENCH_HEX_MODE")
                .ok()
                .and_then(|w| {
                    hikasa_hex::AlignMode::ALL
                        .iter()
                        .copied()
                        .find(|m| m.as_str() == w)
                })
                .unwrap_or(hikasa_hex::AlignMode::Complete);
            m.set_mode(mode);
            self.asked = Some(Instant::now());
            return;
        }
        let rows = m.row_count();
        let top = m.screen.top + 3;
        m.screen
            .set_top(if top > m.screen.max_top(rows) { 0 } else { top }, rows);
        if self.realigned.is_none() && m.aligning().is_none() && m.alignment().is_some() {
            self.realigned = Some(Instant::now());
            self.stage = 0;
            // The landed sections and statistics, in the CLI's JSON form (W13a review F12).
            if let Some(Ok(json)) = m.response().map(|r| r.json()) {
                let v = serde_json::to_value(&json).unwrap_or(Value::Null);
                self.landed_json = json!({
                    "alignment": v["alignment"],
                    "verdict": v["verdict"],
                    "stats": v["stats"],
                    "sections": v["sections"],
                    "sections_shown": v["sections_shown"],
                    "sections_truncated": v["sections_truncated"],
                });
            }
        }
        if self.realigned.is_some() {
            self.stage += 1;
            if self.stage > WARM_UP {
                self.memory_settled = sample();
                self.done = true;
            }
        }
    }

    fn drive_find(&mut self, core: &mut AppCore, view: ViewId) {
        let Some(m) = Self::model(core, view) else {
            return;
        };
        let running = m.searching().is_some();
        if self.stage == 3 && running {
            // Esc 5 frames after the search started: the job is cancelled and dropped at once.
            if self.frame >= 5 {
                let began = Instant::now();
                m.escape();
                self.cancel_ms = Some(began.elapsed().as_secs_f64() * 1000.0);
                self.stage = 4;
            }
            return;
        }
        if let Some(last) = self.searches.last_mut()
            && last.2.is_none()
            && !running
        {
            last.2 = Some(Instant::now());
        }
        if running {
            return;
        }
        match self.stage {
            0 | 1 => {
                let var = if self.stage == 0 {
                    "HIKASA_BENCH_HEX_FIND"
                } else {
                    "HIKASA_BENCH_HEX_FIND2"
                };
                let Ok(pattern) = std::env::var(var) else {
                    self.stage = 2;
                    return;
                };
                m.set_caret(0, false);
                m.open_find();
                m.find.hex = true;
                m.find.wrap = false;
                m.find.side = hikasa_core::Side::Left;
                m.find.pattern = pattern;
                m.validate_find();
                m.find_step(true);
                self.searches
                    .push((var.to_owned(), Instant::now(), None, Vec::new()));
                self.stage += 1;
            }
            2 => {
                // A search stopped by Esc after 5 frames.
                m.set_caret(0, false);
                if let Ok(pattern) = std::env::var("HIKASA_BENCH_HEX_FIND") {
                    m.find.pattern = pattern;
                    m.validate_find();
                }
                m.find_step(true);
                self.stage = 3;
                self.frame = 0;
            }
            _ => {
                self.memory_settled = sample();
                self.done = true;
            }
        }
    }

    /// A frame ended with `work` ms on the UI thread.
    pub fn end_frame(&mut self, work: f64, hexes: &HashMap<ViewId, HexUi>) {
        let Some(view) = self.view else {
            return;
        };
        let Some(st) = hexes.get(&view) else {
            return;
        };
        if st.drawn >= 1 && self.first_rows.is_none() {
            self.first_rows = Some(Instant::now());
            self.memory_first_rows = sample();
        }
        if let Some((_, _, frame, run)) = self.phase.as_mut()
            && *frame > 0
        {
            run.work.push(work);
            if !st.complete {
                run.placeholder_frames += 1;
                if self.since_jump > 2 {
                    run.late_placeholders += 1;
                }
            }
        }
        if self.asked.is_some() && self.realigned.is_none() {
            self.align_work.push(work);
        }
        if let Some(last) = self.searches.last_mut()
            && last.2.is_none()
        {
            last.3.push(work);
        }
    }

    /// The bench's part of the report.
    pub fn report(&self, process_start: Instant) -> Value {
        let ms = |a: Option<Instant>, from: Option<Instant>| match (a, from) {
            (Some(a), Some(f)) => {
                json!((a.duration_since(f).as_secs_f64() * 1000.0 * 10.0).round() / 10.0)
            }
            _ => Value::Null,
        };
        json!({
            "hex": {
                "first_rows_ms_from_process_start": ms(self.first_rows, Some(process_start)),
                "first_rows_ms_from_view_open": ms(self.first_rows, self.opened),
                "landed_ms_from_view_open": ms(self.landed, self.opened),
                "phases": self.phases,
                "realign_ms": ms(self.realigned, self.asked),
                "frame_work_while_aligning_ms": crate::bench::stats(&self.align_work),
                "searches": self.searches.iter().map(|(name, start, end, work)| json!({
                    "pattern": name,
                    "result_ms": ms(*end, Some(*start)),
                    "frame_work_ms": crate::bench::stats(work),
                })).collect::<Vec<_>>(),
                "cancel_ms": self.cancel_ms,
                "landed": self.landed_json,
                "memory_at_first_rows": self.memory_first_rows,
                "memory_settled": self.memory_settled,
            }
        })
    }
}

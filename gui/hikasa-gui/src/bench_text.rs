//! The editing benchmarks of gate G16 (M2 W11a, contract §1.11.30, D-322): `HIKASA_GUI_BENCH=text_typing`,
//! `text_undo` and `text_edited` on the G1 pair. The script injects keys as egui events at the start of a frame (one
//! a frame unless stated) at seeded places, before the panes take the frame's input; the frame work of each *key
//! frame* is recorded under its row's label, with the generation check (the painted slice is the model's), the time
//! from a burst's last key to its recompare's install, and the exactness of the last install (the live comparison
//! equals a fresh comparison of the snapshots, `hikasa.text_compare.v1`).

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use egui::{Event, Key, Modifiers};
use hikasa_core::{JobKind, NullProgress, Pair, Side};
use hikasa_session::ipc::ViewId;
use hikasa_text::edit::TextPos;
use hikasa_text::{RowIdx, RowState};
use serde_json::{Value, json};

use crate::app::AppCore;
use crate::bench::{memory, stats};
use crate::ui::text::TextUi;
use crate::window::ViewModel;

/// Which editing benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    /// G16 type, changed, lines, burst.
    Typing,
    /// G16 undo.
    Undo,
    /// ARCH-GUI-003 edited.
    Edited,
}

/// One step of the script.
#[derive(Debug, Clone)]
enum Action {
    /// The caret to a line of a pane (one frame, not a key frame).
    Place(Side, u32),
    /// An egui event in this frame, its frame recorded under the label.
    Inject(&'static str, Event),
    /// A paste of this text at the caret (the clipboard read is a job; the text applied as its result is).
    Paste(&'static str, String),
    /// Copy Section to Right of the current section.
    CopySection(&'static str),
    /// A burst ended: the time to its install is measured.
    BurstEnd(&'static str),
    /// Frames without input.
    Idle(u32),
    /// Frames until no recompare waits or runs.
    Settle,
    /// Scrolling: `frames` frames of `rows` display positions (0: a page) under the label.
    Scroll(&'static str, u32, f64),
    /// A memory sample under the label.
    Memory(&'static str),
    /// The installed comparison against a fresh comparison of the snapshots.
    CheckFresh(&'static str),
    /// The installed comparison against the one of the open.
    CheckOriginal(&'static str),
}

/// The state of an editing benchmark.
#[derive(Debug)]
pub struct EditBench {
    mode: EditMode,
    view: Option<ViewId>,
    script: VecDeque<Action>,
    built: bool,
    warm: u32,
    /// The label of this frame's key (its work is a key frame's).
    key: Option<&'static str>,
    scroll: Option<&'static str>,
    works: HashMap<&'static str, Vec<f64>>,
    order: Vec<&'static str>,
    generation_misses: u64,
    burst: Option<(&'static str, Instant, u64)>,
    installs: HashMap<&'static str, Vec<f64>>,
    max_jobs: u32,
    checks: Vec<(&'static str, bool)>,
    memory: Vec<(&'static str, Value)>,
    original: Option<Value>,
    done: bool,
    settle_frames: u32,
}

/// The rows the type/changed rows type into: 10 bursts of 20 characters.
const BURSTS: usize = 10;
const BURST_KEYS: usize = 20;
const BURST_IDLE: u32 = 30;
/// The lines row: 20 of each action.
const LINES_EACH: usize = 20;

impl EditBench {
    /// A benchmark of `mode`.
    pub fn new(mode: EditMode) -> EditBench {
        EditBench {
            mode,
            view: None,
            script: VecDeque::new(),
            built: false,
            warm: 0,
            key: None,
            scroll: None,
            works: HashMap::new(),
            order: Vec::new(),
            generation_misses: 0,
            burst: None,
            installs: HashMap::new(),
            max_jobs: 0,
            checks: Vec::new(),
            memory: Vec::new(),
            original: None,
            done: false,
            settle_frames: 0,
        }
    }

    /// The view under test.
    pub fn set_view(&mut self, view: ViewId) {
        self.view = Some(view);
    }

    /// True once the script ran.
    pub fn finished(&self) -> bool {
        self.done
    }

    fn model<'a>(&self, core: &'a mut AppCore) -> Option<&'a mut crate::text::TextViewModel> {
        match core.tab_mut(self.view?).and_then(|t| t.model_mut()) {
            Some(ViewModel::Text(m)) => Some(m),
            _ => None,
        }
    }

    /// Lines of the left side whose rows are `same` (else changed), seeded.
    fn lines_of(m: &crate::text::TextViewModel, same: bool, n: usize, seed: u64) -> Vec<u32> {
        let Some(cmp) = m.comparison() else {
            return Vec::new();
        };
        let rows = cmp.row_count();
        let mut out = Vec::new();
        let mut x = seed | 1;
        let mut tries = 0;
        while out.len() < n && tries < 1_000_000 {
            tries += 1;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let row = (x % u64::from(rows.max(1))) as u32;
            if let Some(r) = cmp.row(RowIdx(row))
                && (r.state == RowState::Same) == same
                && let Some(l) = r.left
            {
                out.push(l.0);
            }
        }
        out
    }

    fn build(&mut self, m: &crate::text::TextViewModel) {
        let text = |c: char| Event::Text(c.to_string());
        let mut s = VecDeque::new();
        let typing = |s: &mut VecDeque<Action>, label: &'static str, lines: &[u32]| {
            for (b, line) in lines.iter().enumerate().take(BURSTS) {
                s.push_back(Action::Place(Side::Left, *line));
                for k in 0..BURST_KEYS {
                    let c = (b'a' + ((b + k) % 26) as u8) as char;
                    s.push_back(Action::Inject(label, text(c)));
                }
                s.push_back(Action::BurstEnd(label));
                s.push_back(Action::Idle(BURST_IDLE));
            }
        };
        let same = Self::lines_of(m, true, BURSTS + 1_100, 0x5EED_0016);
        let changed = Self::lines_of(m, false, BURSTS, 0x5EED_0017);
        let key = |k: Key, mods: Modifiers| Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: mods,
        };
        match self.mode {
            EditMode::Typing => {
                typing(&mut s, "type", &same[..BURSTS]);
                typing(&mut s, "changed", &changed);
                s.push_back(Action::Settle);
                // Lines: Enter, Delete Line, paste of 100 lines, Copy Section to Right, 20 each.
                let paste: String = (0..100).map(|i| format!("pasted line {i}\n")).collect();
                for (i, line) in same.iter().skip(BURSTS).take(LINES_EACH).enumerate() {
                    s.push_back(Action::Place(Side::Left, *line + i as u32));
                    s.push_back(Action::Inject(
                        "lines_enter",
                        key(Key::Enter, Modifiers::NONE),
                    ));
                    s.push_back(Action::Inject(
                        "lines_delete_line",
                        key(Key::K, Modifiers::COMMAND | Modifiers::SHIFT),
                    ));
                    s.push_back(Action::Paste("lines_paste_100", paste.clone()));
                    s.push_back(Action::Idle(2));
                }
                for line in changed.iter().take(LINES_EACH) {
                    s.push_back(Action::Place(Side::Left, *line));
                    s.push_back(Action::CopySection("lines_copy_section"));
                    s.push_back(Action::Idle(2));
                }
                s.push_back(Action::Settle);
                // Burst: 60 characters at one a frame while recompares run, then idle; the result is exact.
                s.push_back(Action::Place(Side::Right, same[BURSTS + 30]));
                for k in 0..60 {
                    s.push_back(Action::Inject(
                        "burst",
                        text((b'a' + (k % 26) as u8) as char),
                    ));
                }
                s.push_back(Action::BurstEnd("burst"));
                s.push_back(Action::Idle(BURST_IDLE));
                s.push_back(Action::Settle);
                s.push_back(Action::CheckFresh("burst"));
            }
            EditMode::Undo => {
                typing(&mut s, "type", &same[..BURSTS]);
                for line in same.iter().skip(BURSTS).take(1_000) {
                    s.push_back(Action::Place(Side::Left, *line));
                    s.push_back(Action::Inject("edits", text('x')));
                }
                s.push_back(Action::Settle);
                for _ in 0..(1_000 + BURSTS) {
                    s.push_back(Action::Inject("undo", key(Key::Z, Modifiers::COMMAND)));
                }
                s.push_back(Action::Idle(BURST_IDLE));
                s.push_back(Action::Settle);
                s.push_back(Action::CheckOriginal("undo"));
            }
            EditMode::Edited => {
                for line in same.iter().take(1_000) {
                    s.push_back(Action::Place(Side::Left, *line));
                    s.push_back(Action::Inject("edits", text('x')));
                    s.push_back(Action::Settle);
                }
                let scroll = |s: &mut VecDeque<Action>, tag: [&'static str; 3]| {
                    s.push_back(Action::Scroll(tag[0], 600, 3.0));
                    s.push_back(Action::Scroll(tag[1], 300, 0.0));
                    s.push_back(Action::Scroll(tag[2], 1_200, -1.0));
                };
                scroll(&mut s, ["edited_wheel", "edited_page", "edited_sweep"]);
                s.push_back(Action::Idle(60));
                s.push_back(Action::Memory("edited"));
                for _ in 0..1_000 {
                    s.push_back(Action::Inject("undo", key(Key::Z, Modifiers::COMMAND)));
                }
                s.push_back(Action::Idle(BURST_IDLE));
                s.push_back(Action::Settle);
                scroll(&mut s, ["undone_wheel", "undone_page", "undone_sweep"]);
                s.push_back(Action::Idle(60));
                s.push_back(Action::Memory("undone"));
                s.push_back(Action::CheckOriginal("undone"));
            }
        }
        self.script = s;
        self.built = true;
    }

    /// The comparison of the view as JSON (no specs), for the exactness checks.
    fn json_of(m: &crate::text::TextViewModel) -> Option<Value> {
        let r = m.response()?.clone();
        let response = hikasa_session::api::TextCompareResponse {
            comparison: std::sync::Arc::clone(m.comparison()?),
            ..r
        };
        let mut v =
            serde_json::to_value(response.json(&hikasa_text::ViewOptions::default())).ok()?;
        if let Some(o) = v.as_object_mut() {
            for k in ["left", "right", "verdict", "exit_code"] {
                o.remove(k);
            }
        }
        Some(v)
    }

    /// Runs this frame's step before the panes draw: an injected event lands in the frame's input.
    pub fn drive(
        &mut self,
        ctx: &egui::Context,
        core: &mut AppCore,
        service: &hikasa_session::Service,
        texts: &mut HashMap<ViewId, TextUi>,
        ready: bool,
    ) {
        self.key = None;
        self.scroll = None;
        if self.done || !ready {
            return;
        }
        let Some(view) = self.view else {
            return;
        };
        let wake_frames = self.warm < 30;
        if wake_frames {
            self.warm += 1;
            return;
        }
        // The running recompares (one at most, D-294) and the burst's install.
        if let Some(m) = self.model(core) {
            let running = u32::from(m.recompare_state().running.is_some());
            self.max_jobs = self.max_jobs.max(running);
            if let Some((label, at, installs)) = self.burst
                && m.installs() > installs
                && m.recompare_state().idle()
            {
                self.installs
                    .entry(label)
                    .or_default()
                    .push(at.elapsed().as_secs_f64() * 1000.0);
                self.burst = None;
            }
            if self.original.is_none() {
                self.original = Self::json_of(m);
            }
            if !self.built {
                if m.comparison().is_none() {
                    return;
                }
                let m: &crate::text::TextViewModel = m;
                let mode = self.mode;
                let mut script = EditBench::new(mode);
                script.build(m);
                self.script = script.script;
                self.built = true;
            }
        }
        let Some(action) = self.script.pop_front() else {
            self.done = true;
            return;
        };
        match action {
            Action::Place(side, line) => {
                if let Some(m) = self.model(core) {
                    m.set_focus(side);
                    let pos = m
                        .doc(side)
                        .map_or(TextPos::new(line, 0), |d| d.clamp(TextPos::new(line, 0)));
                    m.pane_mut(side).set_caret(pos, false);
                    m.caret_moved();
                }
            }
            Action::Inject(label, event) => {
                ctx.input_mut(|i| i.events.push(event));
                self.key = Some(label);
            }
            Action::Paste(label, text) => {
                if let Some(m) = self.model(core) {
                    let side = m.focus();
                    crate::text::clipboard::paste(m, side, &text);
                }
                self.key = Some(label);
            }
            Action::CopySection(label) => {
                if let Some(m) = self.model(core) {
                    m.execute(service, crate::commands::CommandId::CopyRight);
                }
                self.key = Some(label);
            }
            Action::BurstEnd(label) => {
                let installs = self.model(core).map_or(0, |m| m.installs());
                self.burst = Some((label, Instant::now(), installs));
            }
            Action::Idle(n) => {
                if n > 1 {
                    self.script.push_front(Action::Idle(n - 1));
                }
            }
            Action::Settle => {
                let idle = self
                    .model(core)
                    .is_none_or(|m| m.recompare_state().idle() && !m.busy());
                self.settle_frames += 1;
                if !idle && self.settle_frames < 10_000 {
                    self.script.push_front(Action::Settle);
                } else {
                    self.settle_frames = 0;
                }
            }
            Action::Scroll(label, frames, rows) => {
                if let Some(st) = texts.get_mut(&view) {
                    let step = if rows == 0.0 {
                        (st.page_rows - 1.0).max(1.0)
                    } else if rows < 0.0 {
                        (st.max_top() / f64::from(frames)).max(1.0)
                    } else {
                        rows
                    };
                    let mut top = st.top + step;
                    if top > st.max_top() + 0.5 {
                        top = 0.0;
                    }
                    st.top = top.clamp(0.0, st.max_top());
                }
                self.scroll = Some(label);
                if frames > 1 {
                    self.script
                        .push_front(Action::Scroll(label, frames - 1, rows));
                }
            }
            Action::Memory(label) => self.memory.push((label, memory())),
            Action::CheckFresh(label) => {
                let ok = self.model(core).is_some_and(|m| fresh_equal(m, service));
                self.checks.push((label, ok));
            }
            Action::CheckOriginal(label) => {
                let original = self.original.clone();
                let now = self.model(core).and_then(|m| Self::json_of(m));
                self.checks
                    .push((label, original.is_some() && original == now));
            }
        }
    }

    /// The frame's work is known: a key frame's goes to its label, with the generation check.
    pub fn end_frame(&mut self, work: f64, core: &AppCore, texts: &HashMap<ViewId, TextUi>) {
        let label = self.key.or(self.scroll);
        let Some(label) = label else {
            return;
        };
        if !self.order.contains(&label) {
            self.order.push(label);
        }
        self.works.entry(label).or_default().push(work);
        if self.key.is_some()
            && let Some(view) = self.view
            && let Some(Some(ViewModel::Text(m))) = core.tab(view).map(|t| t.model())
            && texts
                .get(&view)
                .is_some_and(|st| st.painted_generation != m.generation())
        {
            self.generation_misses += 1;
        }
    }

    /// The report's part.
    pub fn report(&self) -> Value {
        let mut rows = serde_json::Map::new();
        for label in &self.order {
            rows.insert(
                (*label).to_owned(),
                stats(self.works.get(label).map_or(&[][..], Vec::as_slice)),
            );
        }
        let mut installs = serde_json::Map::new();
        for (label, v) in &self.installs {
            installs.insert((*label).to_owned(), stats(v));
        }
        json!({
            "key_frames_ms": rows,
            "install_after_last_key_ms": installs,
            "generation_misses": self.generation_misses,
            "max_recompare_jobs": self.max_jobs,
            "checks": self.checks.iter().map(|(l, ok)| json!({ "check": l, "equal": ok })).collect::<Vec<_>>(),
            "memory": self.memory.iter().map(|(l, m)| json!({ "at": l, "memory": m })).collect::<Vec<_>>(),
        })
    }
}

/// The installed comparison equals a fresh comparison of the snapshots (`text_relive` compares them from scratch).
fn fresh_equal(m: &crate::text::TextViewModel, service: &hikasa_session::Service) -> bool {
    let (Some(docs), Some(json)) = (m.docs(), EditBench::json_of(m)) else {
        return false;
    };
    let job = service.job(JobKind::TextCompare, std::sync::Arc::new(NullProgress));
    let Ok(relived) = service.text_relive_full(
        Pair {
            left: docs.left,
            right: docs.right,
        },
        m.request(),
        &job,
        None,
    ) else {
        return false;
    };
    let mut fresh =
        serde_json::to_value(relived.response.json(&hikasa_text::ViewOptions::default())).ok();
    if let Some(o) = fresh.as_mut().and_then(Value::as_object_mut) {
        for k in ["left", "right", "verdict", "exit_code"] {
            o.remove(k);
        }
    }
    fresh == Some(json)
}

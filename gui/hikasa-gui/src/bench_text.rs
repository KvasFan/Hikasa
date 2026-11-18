//! The editing benchmarks of gate G16 (M2 W11a, contract §1.11.30, D-322): `HIKASA_GUI_BENCH=text_typing`,
//! `text_undo` and `text_edited` on the G1 pair. The script injects keys as egui events at the start of a frame (one
//! a frame unless stated) at seeded places, before the panes take the frame's input; the frame work of each *key
//! frame* is recorded under its row's label, with the generation check (the painted slice is the model's), the time
//! from a burst's last key to its recompare's install, and the exactness of the last install (the live comparison
//! equals a fresh comparison of the snapshots, `hikasa.text_compare.v1`).
//!
//! M2 W11c adds the rest of the G16 table: `text_wrap` (Word Wrap on, the three scroll phases), `text_find` (Find Next
//! of a word without a hit, then Select All of about 50,000 hits; the frames while the jobs run and each job's time),
//! `find_files` (Find in Files over `HIKASA_PERF_FIND_ROOT` into the results panel, then 600 frames of scrolling it) and
//! `text_save` (1,000 edited lines of the left side, then Save: the frames during the job and its time).

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
    /// G16 wrap (M2 W11c).
    Wrap,
    /// G16 find.
    Find,
    /// G16 find files.
    FindFiles,
    /// G16 save.
    Save,
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
    /// A command of the view's model, its frame recorded under the label (M2 W11c).
    Model(&'static str, crate::commands::CommandId),
    /// A command of the window (Save), its frame recorded under the label.
    Window(&'static str, crate::commands::CommandId),
    /// The find panel's query (regular expression or not) and an action, its frame recorded under the label.
    Find(
        &'static str,
        &'static str,
        bool,
        crate::text::find::FindAction,
    ),
    /// Frames under the label until the view's jobs (a search, a save) have ended; the time from the action is the
    /// label's job time.
    WaitView(&'static str),
    /// Find in Files over `HIKASA_PERF_FIND_ROOT` for the word into the window's results panel.
    FilesStart(&'static str, &'static str),
    /// Frames under the label until the search has ended.
    WaitFiles(&'static str),
    /// Frames of scrolling the results panel (a page a frame, wrapping) under the label.
    FilesScroll(&'static str, u32),
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
    misses_by: HashMap<&'static str, u64>,
    unfocused_keys: u64,
    doc_generations: (u64, u64),
    /// Bursts waiting for their install: label, time of the last key, the installs then.
    bursts: Vec<(&'static str, Instant, u64)>,
    started: u64,
    installs: HashMap<&'static str, Vec<f64>>,
    max_jobs: u32,
    checks: Vec<(&'static str, bool)>,
    memory: Vec<(&'static str, Value)>,
    original: Option<Value>,
    done: bool,
    settle_frames: u32,
    /// The start of the action a `WaitView`/`WaitFiles` waits for (M2 W11c).
    job_started: Option<Instant>,
    /// The job times: label, ms.
    job_ms: Vec<(&'static str, f64)>,
    /// What the last search found (hits), and the results panel's rows.
    found: Vec<(&'static str, u64)>,
    /// The results panel's scroll offset (points).
    files_offset: f32,
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
            misses_by: HashMap::new(),
            unfocused_keys: 0,
            doc_generations: (0, 0),
            bursts: Vec::new(),
            started: 0,
            installs: HashMap::new(),
            max_jobs: 0,
            checks: Vec::new(),
            memory: Vec::new(),
            original: None,
            done: false,
            settle_frames: 0,
            job_started: None,
            job_ms: Vec::new(),
            found: Vec::new(),
            files_offset: 0.0,
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
                // The frames are not paced at 60 Hz here: the next burst waits for this one's install.
                s.push_back(Action::Settle);
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
            EditMode::Wrap => {
                s.push_back(Action::Model(
                    "wrap_on",
                    crate::commands::CommandId::WordWrap,
                ));
                s.push_back(Action::Idle(30));
                s.push_back(Action::Scroll("wrap_wheel", 600, 3.0));
                s.push_back(Action::Scroll("wrap_page", 300, 0.0));
                s.push_back(Action::Scroll("wrap_sweep", 1_200, -1.0));
                s.push_back(Action::Idle(30));
            }
            EditMode::Find => {
                use crate::text::find::FindAction;
                s.push_back(Action::Place(Side::Left, 0));
                s.push_back(Action::Find(
                    "find_none",
                    "zzzz_no_such_word",
                    false,
                    FindAction::Next,
                ));
                s.push_back(Action::WaitView("find_none"));
                s.push_back(Action::Idle(30));
                s.push_back(Action::Place(Side::Left, 0));
                // About 50,000 hits on the left side of the G1 pair (`[1…` and `[2…` sizes of the tables).
                s.push_back(Action::Find(
                    "select_all",
                    "\\[[12]",
                    true,
                    FindAction::SelectAll,
                ));
                s.push_back(Action::WaitView("select_all"));
                s.push_back(Action::Idle(30));
            }
            EditMode::FindFiles => {
                s.push_back(Action::FilesStart("files_stream", "needle"));
                s.push_back(Action::WaitFiles("files_stream"));
                s.push_back(Action::Idle(30));
                s.push_back(Action::FilesScroll("files_scroll", 600));
                s.push_back(Action::Idle(30));
            }
            EditMode::Save => {
                for line in same.iter().take(1_000) {
                    s.push_back(Action::Place(Side::Left, *line));
                    s.push_back(Action::Inject("edits", text('x')));
                }
                s.push_back(Action::Settle);
                s.push_back(Action::Idle(30));
                s.push_back(Action::Place(Side::Left, 0));
                s.push_back(Action::Window("save", crate::commands::CommandId::Save));
                s.push_back(Action::WaitView("save"));
                s.push_back(Action::Idle(30));
            }
        }
        self.script = s;
        self.built = true;
    }

    /// A digest of the view's comparison (every row's lines, state, importance and ignored flag, and the statistics),
    /// for the exactness checks without holding a JSON document of a million rows.
    fn json_of(m: &crate::text::TextViewModel) -> Option<Value> {
        m.comparison().map(|c| json!(digest(c)))
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
            self.started = m.recompare_state().started;
            let installs_now = m.installs();
            let idle = m.recompare_state().idle();
            let installs = &mut self.installs;
            self.bursts.retain(|(label, at, before)| {
                if installs_now > *before && idle {
                    installs
                        .entry(label)
                        .or_default()
                        .push(at.elapsed().as_secs_f64() * 1000.0);
                    false
                } else {
                    true
                }
            });
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
                if let Some(st) = texts.get_mut(&view) {
                    st.focus_request = true;
                }
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
                // The pane's input functions, as `ui/text.rs` calls them for these events (egui's widget focus does
                // not settle on the pane under xvfb without a window manager, so the events are not routed by
                // egui here; the frame's work and the painted slice are the same).
                let _ = ctx;
                if let Some(m) = self.model(core) {
                    apply_event(m, service, &event);
                }
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
                self.bursts.push((label, Instant::now(), installs));
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
            Action::Model(label, id) => {
                if let Some(m) = self.model(core) {
                    m.execute(service, id);
                }
                self.key = Some(label);
            }
            Action::Window(label, id) => {
                let root = core.root();
                let _ = core.execute(root, id);
                self.job_started = Some(Instant::now());
                self.key = Some(label);
            }
            Action::Find(label, query, regex, action) => {
                if let Some(m) = self.model(core) {
                    m.find.query.text = query.to_owned();
                    m.find.query.regex = regex;
                    m.find.query.whole_word = false;
                    m.find.ask = Some(action);
                }
                self.job_started = Some(Instant::now());
                self.key = Some(label);
            }
            Action::WaitView(label) => {
                let busy = self
                    .model(core)
                    .is_some_and(|m| m.find.busy() || m.find.ask.is_some() || m.busy());
                self.settle_frames += 1;
                if busy && self.settle_frames < 100_000 {
                    self.script.push_front(Action::WaitView(label));
                } else {
                    self.settle_frames = 0;
                    if let Some(at) = self.job_started.take() {
                        self.job_ms
                            .push((label, at.elapsed().as_secs_f64() * 1000.0));
                    }
                    // Select All reports the lines it selected.
                    let hits = self
                        .model(core)
                        .map_or(0, |m| m.find.count.unwrap_or_default());
                    self.found.push((label, hits));
                }
                self.scroll = Some(label);
            }
            Action::FilesStart(label, word) => {
                let root = std::env::var_os("HIKASA_PERF_FIND_ROOT").unwrap_or_default();
                let request = hikasa_session::api::TextFindRequest {
                    pattern: word.to_owned(),
                    roots: vec![root],
                    recursive: true,
                    max_matches: Some(200_000),
                    max_matches_per_file: Some(10_000),
                    ..hikasa_session::api::TextFindRequest::default()
                };
                let wake: crate::job::Wake = std::sync::Arc::new(|| {});
                let window = core.root();
                if let Some(w) = core.window_mut(window) {
                    w.find_files.start(service, &wake, request);
                }
                self.job_started = Some(Instant::now());
                self.key = Some(label);
            }
            Action::WaitFiles(label) => {
                let window = core.root();
                let running = core.window(window).is_some_and(|w| w.find_files.running());
                self.settle_frames += 1;
                if running && self.settle_frames < 100_000 {
                    self.script.push_front(Action::WaitFiles(label));
                } else {
                    self.settle_frames = 0;
                    if let Some(at) = self.job_started.take() {
                        self.job_ms
                            .push((label, at.elapsed().as_secs_f64() * 1000.0));
                    }
                    let hits = core
                        .window(window)
                        .map_or(0, |w| w.find_files.hits.len() as u64);
                    self.found.push((label, hits));
                }
                self.scroll = Some(label);
            }
            Action::FilesScroll(label, frames) => {
                let window = core.root();
                if let Some(w) = core.window_mut(window) {
                    let rows = w.find_files.row_count() as f32;
                    let max = (rows * 18.0 - 200.0).max(0.0);
                    self.files_offset += 200.0;
                    if self.files_offset > max {
                        self.files_offset = 0.0;
                    }
                    w.find_files.scroll_request = Some(self.files_offset);
                }
                self.scroll = Some(label);
                if frames > 1 {
                    self.script
                        .push_front(Action::FilesScroll(label, frames - 1));
                }
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
            && texts.get(&view).is_some_and(|st| !st.had_focus)
        {
            self.unfocused_keys += 1;
        }
        if let Some(view) = self.view
            && let Some(Some(ViewModel::Text(m))) = core.tab(view).map(|t| t.model())
        {
            let g = (
                m.doc(Side::Left).map_or(0, |d| d.generation()),
                m.doc(Side::Right).map_or(0, |d| d.generation()),
            );
            self.doc_generations = g;
        }
        if self.key.is_some()
            && let Some(view) = self.view
            && let Some(Some(ViewModel::Text(m))) = core.tab(view).map(|t| t.model())
            && texts
                .get(&view)
                .is_some_and(|st| st.painted_generation != m.generation())
        {
            self.generation_misses += 1;
            *self.misses_by.entry(label).or_default() += 1;
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
            "generation_misses_by_row": self.misses_by.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "max_recompare_jobs": self.max_jobs,
            "recompares_started": self.started,
            "key_frames_without_focus": self.unfocused_keys,
            "doc_generations": [self.doc_generations.0, self.doc_generations.1],
            "bursts_never_installed": self.bursts.len(),
            "checks": self.checks.iter().map(|(l, ok)| json!({ "check": l, "equal": ok })).collect::<Vec<_>>(),
            "job_ms": self.job_ms.iter().map(|(l, ms)| json!({ "job": l, "ms": (ms * 1000.0).round() / 1000.0 })).collect::<Vec<_>>(),
            "found": self.found.iter().map(|(l, n)| json!({ "at": l, "hits": n })).collect::<Vec<_>>(),
            "memory": self.memory.iter().map(|(l, m)| json!({ "at": l, "memory": m })).collect::<Vec<_>>(),
        })
    }
}

/// What the pane does with `event` (typed text, Enter, Delete Line, Undo).
fn apply_event(
    m: &mut crate::text::TextViewModel,
    service: &hikasa_session::Service,
    event: &Event,
) {
    use crate::text::input::{self, Press};
    match event {
        Event::Text(t) => input::text(m, t),
        Event::Key {
            key: Key::Enter, ..
        } => {
            input::key(
                m,
                Press::plain(input::Key::Enter),
                &crate::text::wrap::FixedMetrics::default(),
                10,
            );
        }
        Event::Key { key: Key::K, .. } => {
            m.execute(service, crate::commands::CommandId::DeleteItem);
        }
        Event::Key { key: Key::Z, .. } => {
            m.execute(service, crate::commands::CommandId::Undo);
        }
        _ => {}
    }
}

/// A digest of a comparison's rows and statistics.
fn digest(c: &hikasa_text::TextComparison) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let n = c.row_count();
    n.hash(&mut h);
    for i in 0..n {
        if let Some(r) = c.row(RowIdx(i)) {
            (
                r.left.map(|l| l.0),
                r.right.map(|l| l.0),
                crate::text::state_code(r.state),
                crate::text::importance_code(r.importance),
                r.ignored,
            )
                .hash(&mut h);
        }
    }
    format!("{:?}", c.stats()).hash(&mut h);
    h.finish()
}

/// The installed comparison equals a fresh comparison of the snapshots (`text_relive` compares them from scratch).
fn fresh_equal(m: &crate::text::TextViewModel, service: &hikasa_session::Service) -> bool {
    let Some(docs) = m.docs() else {
        return false;
    };
    let Some(live) = m.comparison().map(|c| digest(c)) else {
        return false;
    };
    let job = service.job(JobKind::TextCompare, std::sync::Arc::new(NullProgress));
    service
        .text_relive_full(
            Pair {
                left: docs.left,
                right: docs.right,
            },
            m.request(),
            &job,
            None,
        )
        .is_ok_and(|relived| digest(relived.live.current()) == live)
}

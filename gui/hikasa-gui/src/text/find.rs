//! The Find and Replace panels of a text view (TC-064; M2 W11b, contract §1.11.16, D-307), toolkit-free: the query
//! being typed, the search as a job on snapshots of the documents (`Finder::next` per document, in a comparison
//! `find_in_comparison_at` over the snapshots and the comparison of the same moment), the hit selected and brought
//! into view, *Select All* as a line set (`Finder::all_in`), Replace (`replace_one`, then Find Next) and Replace All
//! (`replace_all` on clones, the question, then the request applied when the document is still the clone's). The
//! highlights on screen are `Finder::hits_in_line` of the lines on screen only. Every match, replacement and count is
//! core's; `ui/text_panels.rs` draws the panel.

use std::ops::Range;
use std::sync::Arc;

use hikasa_core::{JobKind, Pair, Side};
use hikasa_session::settings::State;
use hikasa_session::settings::state::MruList;
use hikasa_session::{ApiError, Service};
use hikasa_text::Direction;
use hikasa_text::edit::{EditRequest, EditableDocument, LineSet, Selection, TextPos, word_at};
use hikasa_text::find::{
    FindError, FindQuery, FindSides, Finder, Hit, Replacement, find_in_comparison_at, replace_all,
    replace_one,
};
use hikasa_text::{RowIdx, TextComparison};

use super::edit::Origin;
use super::layout::FindShown;
use super::{TextRequest, TextViewModel, index_of};
use crate::job::Job;
use crate::status::Message;

/// The `[dialogs]` id of the panel's options.
pub const FIND_DIALOG: &str = "find";
/// The help topic of the panels.
pub const FIND_TOPIC: &str = "panel-find-replace";
/// The text of a search without a hit.
pub const NOT_FOUND: &str = "Text not found.";
/// The note of a hit reached by wrapping.
pub const PASSED_END: &str = "Passed the end of the file.";
/// The most hits *Select All* collects (D-307).
pub const SELECT_ALL_LIMIT: u64 = 100_000;

/// The options' keys in `[dialogs.find]`.
const KEYS: [&str; 5] = ["match_case", "whole_word", "regex", "wrap", "close"];

/// A search running on snapshots.
enum Running {
    /// Find Next or Find Previous.
    Next {
        /// The documents' generations the snapshots were taken at.
        gens: [u64; 2],
        dir: Direction,
        job: Job<Option<(Side, Hit)>>,
    },
    /// Select All of the focus pane.
    All {
        side: Side,
        stamp: u64,
        job: Job<Vec<Hit>>,
    },
    /// Replace All: per side, the request on the clone, its count and the clone's generation.
    Replace { job: Job<Vec<SidePlan>> },
}

/// A button of the panel (the window runs it at its next poll, with the service).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindAction {
    /// ▼, Enter: Find Next.
    Next,
    /// ▲, Shift+Enter: Find Previous.
    Prev,
    /// *Select All*.
    SelectAll,
    /// *Replace*.
    Replace,
    /// *Replace All*.
    ReplaceAll,
}

/// One side's Replace All plan: the side, its request (none: no hit), its count and the clone's generation.
type SidePlan = (Side, Option<EditRequest>, u64, u64);

/// A Replace All that waits for its question (*Replace N occurrences?*).
#[derive(Debug, Clone)]
pub struct ReplaceAsk {
    /// Per side: the request, its count and the document's generation it was made on.
    pub sides: Vec<(Side, EditRequest, u64, u64)>,
}

/// The find panel's state (one per view).
pub struct FindPanel {
    /// The query (`hikasa_text::find`), as typed.
    pub query: FindQuery,
    /// *Wrap around* (preset from `[next_difference] wrap_around`).
    pub wrap: bool,
    /// The sides searched (Text Compare).
    pub sides: FindSides,
    /// *Close automatically*: the panel closes after a hit.
    pub close_automatically: bool,
    /// The *Replace with* text.
    pub replace: String,
    /// Why the query cannot be searched (core's `invalid_input` text, shown under the field).
    pub error: Option<String>,
    /// The panel's note (*Passed the end of the file.*, *Text not found.*, *N lines selected*, *N replaced*).
    pub note: Option<String>,
    /// The last hit found (its pane and range).
    pub last_hit: Option<(Side, Hit)>,
    /// The last count (Select All's lines, Replace All's occurrences).
    pub count: Option<u64>,
    /// `mru.find`, newest first.
    pub recent: Vec<String>,
    /// `mru.replace`, newest first.
    pub recent_replace: Vec<String>,
    /// The text field asks for the keyboard (the panel was just opened).
    pub focus: bool,
    /// The compiled query (rebuilt when the query changes).
    finder: Option<(FindQuery, Arc<Finder>)>,
    running: Option<Running>,
    /// A Replace All waiting for its answer.
    pub(crate) asking: Option<ReplaceAsk>,
    /// Searches started (tests: a newer request cancels the older).
    pub(crate) started: u64,
    /// The undo group of the last Replace All (both panes of one Replace All share it).
    group: u64,
    /// A button pressed, run at the next poll.
    pub ask: Option<FindAction>,
}

impl Default for FindPanel {
    fn default() -> FindPanel {
        FindPanel {
            query: FindQuery {
                text: String::new(),
                regex: false,
                match_case: false,
                whole_word: false,
            },
            wrap: false,
            sides: FindSides::Both,
            close_automatically: false,
            replace: String::new(),
            error: None,
            note: None,
            last_hit: None,
            count: None,
            recent: Vec::new(),
            recent_replace: Vec::new(),
            focus: false,
            finder: None,
            running: None,
            asking: None,
            started: 0,
            group: 0,
            ask: None,
        }
    }
}

impl FindPanel {
    /// Reads the remembered options and texts (`[dialogs.find]`, `mru.find`, `mru.replace`).
    pub fn read_state(&mut self, state: &State) {
        let flag = |key| state.dialog_text(FIND_DIALOG, key).map(|v| v == "true");
        if let Some(on) = flag("match_case") {
            self.query.match_case = on;
        }
        if let Some(on) = flag("whole_word") {
            self.query.whole_word = on;
        }
        if let Some(on) = flag("regex") {
            self.query.regex = on;
        }
        if let Some(on) = flag("wrap") {
            self.wrap = on;
        }
        if let Some(on) = flag("close") {
            self.close_automatically = on;
        }
        if let Some(sides) = state.dialog_text(FIND_DIALOG, "sides") {
            self.sides = match sides {
                "left" => FindSides::Left,
                "right" => FindSides::Right,
                _ => FindSides::Both,
            };
        }
        self.recent = state.mru(MruList::Find).to_vec();
        self.recent_replace = state.mru(MruList::Replace).to_vec();
    }

    /// The options to remember (`[dialogs.find]`): key and word.
    pub fn remembered(&self) -> Vec<(&'static str, String)> {
        let flags = [
            self.query.match_case,
            self.query.whole_word,
            self.query.regex,
            self.wrap,
            self.close_automatically,
        ];
        let mut out: Vec<(&'static str, String)> = KEYS
            .iter()
            .zip(flags)
            .map(|(k, v)| (*k, v.to_string()))
            .collect();
        let sides = match self.sides {
            FindSides::Left => "left",
            FindSides::Right => "right",
            _ => "both",
        };
        out.push(("sides", sides.to_owned()));
        out
    }

    /// True while a search of the panel runs.
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// True once a query was typed.
    pub fn has_query(&self) -> bool {
        !self.query.text.is_empty()
    }

    /// The compiled query (`None`: empty or invalid; the error is kept in `error`).
    pub fn finder(
        &mut self,
        word_chars: &str,
        limits: &hikasa_formats::RegexLimits,
    ) -> Option<Arc<Finder>> {
        if let Some((q, f)) = &self.finder
            && *q == self.query
        {
            return Some(Arc::clone(f));
        }
        self.finder = None;
        if self.query.text.is_empty() {
            return None;
        }
        match Finder::new(&self.query, word_chars, limits) {
            Ok(f) => {
                let f = Arc::new(f);
                self.finder = Some((self.query.clone(), Arc::clone(&f)));
                self.error = None;
                Some(f)
            }
            Err(e) => {
                self.error = Some(e.to_string());
                None
            }
        }
    }

    /// The compiled query of the last search, if it still is the query typed (the highlights on screen).
    pub fn compiled(&self) -> Option<&Arc<Finder>> {
        self.finder
            .as_ref()
            .filter(|(q, _)| *q == self.query)
            .map(|(_, f)| f)
    }

    /// Stops the running search (a newer request, the view closing).
    pub fn cancel(&mut self) {
        if let Some(r) = self.running.take() {
            match r {
                Running::Next { job, .. } => job.cancel(),
                Running::All { job, .. } => job.cancel(),
                Running::Replace { job } => job.cancel(),
            }
        }
    }
}

fn api(e: FindError) -> ApiError {
    match e {
        FindError::Cancelled => ApiError::Cancelled,
        other => ApiError::InvalidArgument(other.to_string()),
    }
}

/// The documents' generations (a result of another generation is searched again).
fn generations(view: &TextViewModel) -> [u64; 2] {
    [Side::Left, Side::Right].map(|s| view.doc(s).map_or(0, EditableDocument::generation))
}

/// Opens the panel (`Find`, `Replace`): with `init_find_with_current_word` the text starts as the focus pane's
/// selection (one line) or the word at its caret; the field takes the keyboard.
pub fn open(view: &mut TextViewModel, replace: bool) {
    view.layout.find_panel = if replace {
        FindShown::Replace
    } else {
        FindShown::Find
    };
    if view.is_edit() {
        view.find.sides = FindSides::Left;
    }
    if view.tweaks.init_find_with_current_word
        && let Some(text) = current_word(view)
    {
        view.find.query.text = text;
    }
    view.find.focus = true;
    view.find.note = None;
    view.dirty = true;
}

/// The text a new search starts with: the selection when it is on one line, else the word at the caret.
fn current_word(view: &TextViewModel) -> Option<String> {
    let side = view.focus;
    let doc = view.doc(side)?;
    let pane = view.pane(side);
    let sel = pane.selection().range();
    if pane.lines.is_none() && sel.start != sel.end {
        return (sel.start.line == sel.end.line).then(|| doc.text(sel));
    }
    let r = word_at(doc, pane.caret);
    (r.start != r.end).then(|| doc.text(r))
}

/// Closes the panel (Esc); the query stays for F3.
pub fn close(view: &mut TextViewModel) {
    view.layout.find_panel = FindShown::None;
    view.find.focus = false;
    view.dirty = true;
}

/// Where a search of `dir` starts: the selection's end forward, its start backward (a hit selected is passed).
fn from_of(view: &TextViewModel, side: Side, dir: Direction) -> TextPos {
    let r = view.pane(side).selection().range();
    match dir {
        Direction::Forward => r.end,
        Direction::Backward => r.start,
    }
}

/// Find Next (`Forward`) or Find Previous from the focus pane's caret, as a job on snapshots; a newer request cancels
/// the older. True when a search started (an empty or invalid query starts none; the error is in the panel).
pub fn find(view: &mut TextViewModel, service: &Service, dir: Direction) -> bool {
    view.find.cancel();
    view.find.asking = None;
    let side = view.focus;
    let Some(word_chars) = view.doc(side).map(|d| d.options().word_chars.clone()) else {
        return false;
    };
    let Some(finder) = view.find.finder(&word_chars, service.registry().limits()) else {
        view.dirty = true;
        return false;
    };
    let from = from_of(view, side, dir);
    let wrap = view.find.wrap;
    let sides = if view.is_edit() {
        FindSides::Left
    } else {
        view.find.sides
    };
    let gens = generations(view);
    let snap = Snapshot::of(view);
    let row = view.row_of_line(side, from.line.0);
    let job = Job::spawn(
        service,
        JobKind::TextFind,
        "find".into(),
        &view.wake,
        move |_svc, _control| {
            snap.next(&finder, side, row, from, sides, dir, wrap)
                .map_err(api)
        },
    );
    view.find.started += 1;
    view.find.running = Some(Running::Next { gens, dir, job });
    true
}

/// Snapshots of a view's documents and comparison (a search runs on them, never on the live documents).
struct Snapshot {
    docs: [Option<EditableDocument>; 2],
    cmp: Option<Arc<TextComparison>>,
}

impl Snapshot {
    fn of(view: &TextViewModel) -> Snapshot {
        Snapshot {
            docs: [Side::Left, Side::Right].map(|s| view.doc(s).cloned()),
            cmp: view.comparison().cloned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
        &self,
        f: &Finder,
        side: Side,
        row: Option<u32>,
        from: TextPos,
        sides: FindSides,
        dir: Direction,
        wrap: bool,
    ) -> Result<Option<(Side, Hit)>, FindError> {
        match (&self.cmp, &self.docs) {
            (Some(cmp), [Some(l), Some(r)]) => find_in_comparison_at(
                cmp,
                Pair::new(l, r),
                f,
                (RowIdx(row.unwrap_or(0)), side, from.byte),
                sides,
                dir,
                wrap,
            ),
            _ => {
                let Some(doc) = self.docs[index_of(side)].as_ref() else {
                    return Ok(None);
                };
                Ok(f.next(doc, from, dir, wrap)?.map(|h| (side, h)))
            }
        }
    }
}

/// *Select All*: every hit of the focus pane's document (at most [`SELECT_ALL_LIMIT`]) as a job; the pane's selection
/// becomes the line set of the lines holding one.
pub fn select_all(view: &mut TextViewModel, service: &Service) -> bool {
    view.find.cancel();
    let side = view.focus;
    let Some(doc) = view.doc(side).cloned() else {
        return false;
    };
    let Some(finder) = view
        .find
        .finder(&doc.options().word_chars, service.registry().limits())
    else {
        view.dirty = true;
        return false;
    };
    let stamp = doc.generation();
    let job = Job::spawn(
        service,
        JobKind::TextFind,
        "select all hits".into(),
        &view.wake,
        move |_svc, control| {
            finder
                .all_in(doc.document(), SELECT_ALL_LIMIT, &control.ctl())
                .map_err(api)
        },
    );
    view.find.started += 1;
    view.find.running = Some(Running::All { side, stamp, job });
    true
}

/// Replace: the selected hit replaced (`replace_one`; it must still match, else this is Find Next), then Find Next.
pub fn replace(view: &mut TextViewModel, service: &Service) -> bool {
    let side = view.focus;
    let Some(word_chars) = view.doc(side).map(|d| d.options().word_chars.clone()) else {
        return false;
    };
    let Some(finder) = view.find.finder(&word_chars, service.registry().limits()) else {
        view.dirty = true;
        return false;
    };
    let Some(doc) = view.doc(side) else {
        return false;
    };
    let sel = view.pane(side).selection().range();
    let hit = Hit {
        range: sel.clone(),
        wrapped: false,
    };
    let planned = (sel.start != sel.end)
        .then(|| {
            Replacement::new(&view.find.replace, &finder)
                .and_then(|with| replace_one(doc, &finder, &hit, &with))
        })
        .and_then(Result::ok);
    if let Some(req) = planned {
        let editable = doc.editability().editable();
        if editable {
            super::edit::edit(view, side, req, Origin::Single, None);
        } else {
            super::edit::blocked_message(view, side);
        }
    }
    find(view, service, Direction::Forward)
}

/// The sides Replace All acts on: the panel's sides that are editable (a blocked pane is searched, not replaced).
pub fn replace_sides(view: &TextViewModel) -> Vec<Side> {
    let sides = if view.is_edit() {
        FindSides::Left
    } else {
        view.find.sides
    };
    [Side::Left, Side::Right]
        .into_iter()
        .filter(|s| sides.has(*s))
        .filter(|s| view.doc(*s).is_some_and(|d| d.editability().editable()))
        .collect()
}

/// The scope of Replace All in `side`: the selection when it spans lines, else the whole document.
fn scope_of(view: &TextViewModel, side: Side) -> Option<Range<TextPos>> {
    let sel = view.pane(side).selection().range();
    (sel.start.line != sel.end.line).then_some(sel)
}

/// Replace All: `replace_all` on clones of each side as a job; its counts then ask *Replace N occurrences?*.
pub fn replace_all_start(view: &mut TextViewModel, service: &Service) -> bool {
    view.find.cancel();
    view.find.asking = None;
    let sides = replace_sides(view);
    if sides.is_empty() {
        let side = view.focus;
        super::edit::blocked_message(view, side);
        return false;
    }
    let Some(word_chars) = view.doc(sides[0]).map(|d| d.options().word_chars.clone()) else {
        return false;
    };
    let Some(finder) = view.find.finder(&word_chars, service.registry().limits()) else {
        view.dirty = true;
        return false;
    };
    let with = match Replacement::new(&view.find.replace, &finder) {
        Ok(with) => with,
        Err(e) => {
            view.find.error = Some(e.to_string());
            view.dirty = true;
            return false;
        }
    };
    let work: Vec<(Side, EditableDocument, Option<Range<TextPos>>)> = sides
        .iter()
        .filter_map(|s| Some((*s, view.doc(*s)?.clone(), scope_of(view, *s))))
        .collect();
    let job = Job::spawn(
        service,
        JobKind::TextFind,
        "replace all".into(),
        &view.wake,
        move |_svc, control| {
            let ctl = control.ctl();
            let mut out = Vec::new();
            for (side, doc, scope) in work {
                let (req, n) = replace_all(&doc, &finder, &with, scope, &ctl).map_err(api)?;
                out.push((side, req, n, doc.generation()));
            }
            Ok(out)
        },
    );
    view.find.started += 1;
    view.find.running = Some(Running::Replace { job });
    true
}

/// The answer to *Replace N occurrences?*: Yes applies each side's request as one step (a group per Replace All)
/// when the document is still the clone's, else computes it again.
pub fn replace_all_answer(view: &mut TextViewModel, service: &Service, yes: bool) {
    let Some(ask) = view.find.asking.take() else {
        return;
    };
    if !yes {
        view.find.note = None;
        view.dirty = true;
        return;
    }
    if ask.sides.iter().any(|(side, _, _, stamp)| {
        view.doc(*side).map(EditableDocument::generation) != Some(*stamp)
    }) {
        // An edit came in between: the plan is made again (and asked again).
        replace_all_start(view, service);
        return;
    }
    view.find.group += 1;
    let group = view.find.group;
    let mut done = 0u64;
    for (side, req, n, _) in ask.sides {
        let caret = view.pane(side).selection();
        if super::edit::edit(view, side, req, Origin::Group(group), Some(collapse(caret))) {
            done += n;
        }
    }
    view.find.count = Some(done);
    view.find.note = Some(replaced_text(done));
    view.request_window(TextRequest::Message(Message::info(replaced_text(done))));
    view.dirty = true;
}

/// The caret after a Replace All: where the selection started.
fn collapse(sel: Selection) -> Selection {
    Selection::caret(sel.range().start)
}

/// *Replaced N occurrences.*
pub fn replaced_text(n: u64) -> String {
    if n == 1 {
        "Replaced 1 occurrence.".to_owned()
    } else {
        format!("Replaced {n} occurrences.")
    }
}

/// Runs a button of the panel (the drawing layer has no service).
pub fn act(view: &mut TextViewModel, service: &Service, action: FindAction) -> bool {
    match action {
        FindAction::Next => find(view, service, Direction::Forward),
        FindAction::Prev => find(view, service, Direction::Backward),
        FindAction::SelectAll => select_all(view, service),
        FindAction::Replace => replace(view, service),
        FindAction::ReplaceAll => replace_all_start(view, service),
    }
}

/// Polls the panel's search; true when something changed.
pub fn poll(view: &mut TextViewModel, service: &Service) -> bool {
    if let Some(action) = view.find.ask.take() {
        act(view, service, action);
    }
    let Some(running) = view.find.running.as_ref() else {
        return false;
    };
    match running {
        Running::Next { gens, dir, job } => {
            let Some(result) = job.try_result() else {
                return false;
            };
            let (gens, dir) = (*gens, *dir);
            view.find.running = None;
            match result {
                Ok(_) if gens != generations(view) => {
                    // The documents changed while it ran: searched again on the documents of now.
                    find(view, service, dir);
                }
                Ok(Some((side, hit))) => landed(view, side, hit),
                Ok(None) => {
                    view.find.last_hit = None;
                    view.find.note = Some(NOT_FOUND.to_owned());
                    super::nav::not_found(view, true);
                }
                Err(ApiError::Cancelled) => {}
                Err(e) => view.find.error = Some(e.to_string()),
            }
        }
        Running::All { side, stamp, job } => {
            let Some(result) = job.try_result() else {
                return false;
            };
            let (side, stamp) = (*side, *stamp);
            view.find.running = None;
            match result {
                Ok(_) if view.doc(side).map(EditableDocument::generation) != Some(stamp) => {
                    select_all(view, service);
                }
                Ok(hits) => {
                    let mut lines: Vec<Range<u32>> = Vec::new();
                    for h in &hits {
                        let l = h.range.start.line.0;
                        let end = h.range.end.line.0.max(l) + 1;
                        match lines.last_mut() {
                            Some(last) if last.end >= l => last.end = last.end.max(end),
                            _ => lines.push(l..end),
                        }
                    }
                    let count: u64 = lines.iter().map(|r| u64::from(r.end - r.start)).sum();
                    view.find.count = Some(count);
                    if count == 0 {
                        view.find.note = Some(NOT_FOUND.to_owned());
                        super::nav::not_found(view, true);
                    } else {
                        let pane = view.pane_mut(side);
                        pane.lines = Some(LineSet::from_ranges(lines));
                        view.find.note = Some(lines_selected(count));
                        view.set_focus(side);
                        view.caret_moved();
                    }
                }
                Err(ApiError::Cancelled) => {}
                Err(e) => view.find.error = Some(e.to_string()),
            }
        }
        Running::Replace { job } => {
            let Some(result) = job.try_result() else {
                return false;
            };
            view.find.running = None;
            match result {
                Ok(per_side) => {
                    let total: u64 = per_side.iter().map(|(_, _, n, _)| n).sum();
                    let sides: Vec<_> = per_side
                        .into_iter()
                        .filter_map(|(s, r, n, g)| Some((s, r?, n, g)))
                        .collect();
                    if total == 0 {
                        view.find.note = Some(NOT_FOUND.to_owned());
                        super::nav::not_found(view, true);
                    } else {
                        view.find.asking = Some(ReplaceAsk { sides });
                        view.request_window(TextRequest::AskReplaceAll(total));
                    }
                }
                Err(ApiError::Cancelled) => {}
                Err(e) => view.find.error = Some(e.to_string()),
            }
        }
    }
    view.dirty = true;
    true
}

/// *N lines selected*.
pub fn lines_selected(n: u64) -> String {
    if n == 1 {
        "1 line selected".to_owned()
    } else {
        format!("{n} lines selected")
    }
}

/// A hit landed: selected (the caret at its end), its pane focused, brought into view; *passed the end* when it
/// wrapped; the panel closes with *Close automatically*.
fn landed(view: &mut TextViewModel, side: Side, hit: Hit) {
    view.find.note = hit.wrapped.then(|| PASSED_END.to_owned());
    view.set_focus(side);
    let range = hit.range.clone();
    view.pane_mut(side).select(range.start, range.end);
    view.caret_moved();
    if let Some(row) = view.row_of_line(side, range.start.line.0) {
        view.reveal_row(row);
    }
    view.find.last_hit = Some((side, hit));
    if view.find.close_automatically {
        close(view);
    }
}

/// The hits of the query in `line` of `side` (the highlights of the rows on screen; bounded by the screen):
/// `Finder::hits_in_line`, byte ranges of the line.
pub fn hits_on_screen(view: &TextViewModel, side: Side, line: u32) -> Vec<Range<u32>> {
    let Some(f) = view.find.compiled() else {
        return Vec::new();
    };
    let Some(doc) = view.doc(side) else {
        return Vec::new();
    };
    if line >= doc.line_count() {
        return Vec::new();
    }
    let text = doc.line(hikasa_text::LineIdx(line));
    let mut out = Vec::new();
    f.hits_in_line(&text, 0, |r| {
        out.push(r);
        std::ops::ControlFlow::Continue(())
    });
    out
}

/// Remembers the query's text and options (`mru.find`, `mru.replace`, `[dialogs.find]`) in `state`.
pub fn remember(view: &TextViewModel, state: &mut State, max: usize) {
    let f = &view.find;
    if !f.query.text.is_empty() {
        state.push_mru(MruList::Find, &f.query.text, max);
    }
    if view.layout.find_panel == FindShown::Replace && !f.replace.is_empty() {
        state.push_mru(MruList::Replace, &f.replace, max);
    }
    for (key, value) in f.remembered() {
        state.set_dialog_text(FIND_DIALOG, key, &value);
    }
}

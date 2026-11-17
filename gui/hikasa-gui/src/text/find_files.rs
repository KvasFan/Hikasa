//! Find in Files (TC-064; M2 W11b, contract §1.11.17, D-308), toolkit-free: the dialog's model, and the results panel
//! of a window. The search is `Service::text_find` in a job whose sink posts owned events through a mailbox; the panel
//! drains them once per frame into one arena (the previews back to back), so 100,000 hits stay small; the list is
//! virtualized (a row is a file header or a hit, found by a binary search). Opening a hit, Replace in Files and Copy
//! are the window's (`app/text.rs`) over this model. Every hit, count, plan and question is core's.

use std::any::Any;
use std::ffi::OsString;
use std::ops::Range;
use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_session::api::{TextFindOutcome, TextFindReplace, TextFindRequest};
use hikasa_session::settings::state::MruList;
use hikasa_session::settings::{Shortcut, State};
use hikasa_session::{ApiError, Service};
use hikasa_text::find::files::{FilesEvent, FilesSink, FilesSummary, FindArchives, SkipWhy};
use hikasa_text::find::{FindQuery, Finder};
use parking_lot::Mutex;

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep, default_key};
use crate::job::Job;

/// The `[dialogs]` id of the dialog's choices.
pub const DIALOG_ID: &str = "find-in-files";
/// The results panel's height in [`DIALOG_ID`]'s table (points, as text).
pub const PANEL_HEIGHT_KEY: &str = "panel_height";
/// The dialog's help topic.
pub const DIALOG_TOPIC: &str = "dialog-find-in-files";
/// The panel's help topic.
pub const PANEL_TOPIC: &str = "panel-find-in-files";
/// The defaults of *Advanced* (D-308).
pub const MAX_HITS: u64 = 100_000;
/// Hits per file.
pub const MAX_PER_FILE: u32 = 10_000;
/// Largest file searched (bytes).
pub const MAX_FILE_SIZE: u64 = 128 << 20;

/// The Find in Files dialog (`FindInFiles`, Shift+Ctrl+F).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindFilesDialog {
    /// *Find what*.
    pub text: String,
    /// *Match case*.
    pub match_case: bool,
    /// *Whole words*.
    pub whole_word: bool,
    /// *Regular expression*.
    pub regex: bool,
    /// *Look in*: folders and files, one per entry.
    pub look_in: Vec<String>,
    /// *File masks* (FC-050 syntax).
    pub masks: String,
    /// *Include subfolders*.
    pub subfolders: bool,
    /// *Include hidden items*.
    pub hidden: bool,
    /// *Search inside archives*.
    pub archives: bool,
    /// *Encoding*: `None` detects, else a label.
    pub encoding: Option<String>,
    /// *Convert with the format's converter*.
    pub convert: bool,
    /// *Advanced*: max hits, per file, max file size (bytes), as typed.
    pub max_hits: String,
    /// Hits per file.
    pub per_file: String,
    /// Largest file.
    pub max_size: String,
    /// *Replace with* (Replace in Files from the dialog; `None`: a search).
    pub replace: Option<String>,
    /// Why Find is off (core's message in place).
    pub error: Option<String>,
    /// `mru.find`.
    pub recent: Vec<String>,
    /// `mru.find_in_files`.
    pub recent_roots: Vec<String>,
    chosen: Option<TextFindRequest>,
}

impl FindFilesDialog {
    /// The dialog with the remembered choices of `state` and `roots` (the active view's folders) when none is
    /// remembered.
    pub fn new(state: &State, roots: Vec<String>, text: String) -> FindFilesDialog {
        let flag = |key: &str, default: bool| {
            state
                .dialog_text(DIALOG_ID, key)
                .map_or(default, |v| v == "true")
        };
        let word = |key: &str, default: &str| {
            state
                .dialog_text(DIALOG_ID, key)
                .unwrap_or(default)
                .to_owned()
        };
        let recent_roots = state.mru(MruList::FindInFiles).to_vec();
        let look_in = if roots.is_empty() {
            recent_roots.first().cloned().into_iter().collect()
        } else {
            roots
        };
        FindFilesDialog {
            text,
            match_case: flag("match_case", false),
            whole_word: flag("whole_word", false),
            regex: flag("regex", false),
            look_in,
            masks: word("masks", "*"),
            subfolders: flag("subfolders", true),
            hidden: flag("hidden", false),
            archives: flag("archives", false),
            encoding: state
                .dialog_text(DIALOG_ID, "encoding")
                .filter(|e| !e.is_empty())
                .map(str::to_owned),
            convert: flag("convert", false),
            max_hits: word("max_hits", &MAX_HITS.to_string()),
            per_file: word("per_file", &MAX_PER_FILE.to_string()),
            max_size: word("max_size", &MAX_FILE_SIZE.to_string()),
            replace: None,
            error: None,
            recent: state.mru(MruList::Find).to_vec(),
            recent_roots,
            chosen: None,
        }
    }

    /// The request of the fields (`text find`'s words): the query, the roots, the masks and the limits. An empty query,
    /// no root, a bad number or a pattern core refuses is the error shown in place.
    pub fn request(&self, svc: &Service) -> Result<TextFindRequest, String> {
        let query = FindQuery {
            text: self.text.clone(),
            regex: self.regex,
            match_case: self.match_case,
            whole_word: self.whole_word,
        };
        Finder::new(&query, "", svc.registry().limits()).map_err(|e| e.to_string())?;
        let roots: Vec<OsString> = self
            .look_in
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .map(OsString::from)
            .collect();
        if roots.is_empty() {
            return Err("Look in: give a folder or a file.".to_owned());
        }
        let number = |text: &str, what: &str| -> Result<u64, String> {
            text.trim()
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| format!("{what}: `{}` is not a positive number.", text.trim()))
        };
        let masks = self.masks.trim();
        Ok(TextFindRequest {
            pattern: self.text.clone(),
            roots,
            regex: self.regex,
            match_case: self.match_case,
            whole_word: self.whole_word,
            recursive: self.subfolders,
            mask: (!masks.is_empty() && masks != "*").then(|| masks.to_owned()),
            include_hidden: self.hidden,
            archives: if self.archives {
                FindArchives::Opened
            } else {
                FindArchives::Files
            },
            encoding: self.encoding.clone(),
            format: None,
            convert: self.convert,
            max_matches: Some(number(&self.max_hits, "Max hits")?),
            max_matches_per_file: Some(
                u32::try_from(number(&self.per_file, "Hits per file")?).unwrap_or(u32::MAX),
            ),
            max_file_size: Some(number(&self.max_size, "Max file size")?),
            keep_hits: false,
            replace: self.replace.as_ref().map(|text| TextFindReplace {
                text: text.clone(),
                ..TextFindReplace::default()
            }),
        })
    }

    /// The request after Find (once).
    pub fn take_chosen(&mut self) -> Option<TextFindRequest> {
        self.chosen.take()
    }

    /// The choices to remember (`[dialogs.find-in-files]`).
    pub fn remembered(&self) -> Vec<(&'static str, String)> {
        vec![
            ("match_case", self.match_case.to_string()),
            ("whole_word", self.whole_word.to_string()),
            ("regex", self.regex.to_string()),
            ("masks", self.masks.clone()),
            ("subfolders", self.subfolders.to_string()),
            ("hidden", self.hidden.to_string()),
            ("archives", self.archives.to_string()),
            ("encoding", self.encoding.clone().unwrap_or_default()),
            ("convert", self.convert.to_string()),
            ("max_hits", self.max_hits.clone()),
            ("per_file", self.per_file.clone()),
            ("max_size", self.max_size.clone()),
        ]
    }
}

impl DialogModel for FindFilesDialog {
    fn title(&self) -> String {
        "Find in Files".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        DIALOG_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "Find".to_owned(),
                role: ButtonRole::Ok,
                enabled: !self.text.is_empty() && self.look_in.iter().any(|r| !r.trim().is_empty()),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        match self.request(svc) {
            Ok(req) => {
                self.chosen = Some(req);
                self.error = None;
                DialogStep::Close
            }
            Err(e) => {
                self.error = Some(e);
                DialogStep::Stay
            }
        }
    }

    fn on_key(&mut self, key: &Shortcut) -> Option<ButtonRole> {
        default_key(&self.buttons(), key)
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((560.0, 360.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// One file of the results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The canonical spec.
    pub spec: String,
    /// Its encoding's name.
    pub encoding: &'static str,
    /// The index of its first hit.
    pub first_hit: u32,
    /// Its hits.
    pub hits: u32,
    /// It stopped at a limit before its end.
    pub stopped: bool,
}

/// One hit of the results (the preview's text is in the panel's arena).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitEntry {
    /// The file's index.
    pub file: u32,
    /// 1-based physical line.
    pub line: u32,
    /// 1-based column (characters).
    pub column: u32,
    /// Length (characters).
    pub len: u32,
    /// The preview's bytes in the arena.
    pub preview: (u32, u32),
    /// The hit in the preview, in characters.
    pub range: (u32, u32),
    /// The preview is cut.
    pub truncated: bool,
}

/// A row of the virtualized list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListRow {
    /// A file header.
    File(u32),
    /// A hit.
    Hit(u32),
    /// The *Skipped (N)* group header.
    SkippedHeader,
    /// A skipped item.
    Skipped(u32),
}

/// An owned event of the job's sink (the mailbox).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    File {
        spec: String,
        encoding: &'static str,
    },
    Hit {
        line: u32,
        column: u32,
        len: u32,
        preview: String,
        range: Range<u32>,
        truncated: bool,
    },
    Skipped {
        spec: String,
        reason: String,
    },
    Done {
        stopped: bool,
    },
}

/// The sink of the job: owned events into the mailbox, the window woken once per batch.
struct Mailbox {
    events: Arc<Mutex<Vec<Event>>>,
    wake: crate::job::Wake,
    pending: usize,
}

impl FilesSink for Mailbox {
    fn event(&mut self, e: FilesEvent<'_>) {
        let event = match e {
            FilesEvent::File { spec, encoding, .. } => Event::File {
                spec: spec.canonical(),
                encoding: encoding.name(),
            },
            FilesEvent::Hit {
                line,
                column,
                len_chars,
                preview,
                preview_range,
                truncated,
                ..
            } => Event::Hit {
                line,
                column,
                len: len_chars,
                preview: preview.to_owned(),
                range: preview_range,
                truncated,
            },
            FilesEvent::Skipped { spec, reason } => Event::Skipped {
                spec: spec.canonical(),
                reason: skip_text(&reason),
            },
            FilesEvent::Done { stopped, .. } => Event::Done { stopped },
        };
        self.events.lock().push(event);
        self.pending += 1;
        if self.pending >= 256 {
            self.pending = 0;
            (self.wake)();
        }
    }
}

/// The text of a skip reason.
pub fn skip_text(reason: &SkipWhy) -> String {
    match reason {
        SkipWhy::Binary => "binary content".to_owned(),
        SkipWhy::TooLarge => "larger than the limit".to_owned(),
        SkipWhy::Unreadable(e) => format!("cannot be read: {}", e.message),
        SkipWhy::Converted => "its format converts it (Convert is off)".to_owned(),
        SkipWhy::Filtered => "a link to a folder".to_owned(),
    }
}

/// The results panel of a window (one per window; it keeps the last search).
#[derive(Default)]
pub struct FindFilesPanel {
    /// Shown at the bottom of the window.
    pub shown: bool,
    /// The search (its request).
    pub request: Option<TextFindRequest>,
    /// The files with hits, in the stream's order.
    pub files: Vec<FileEntry>,
    /// The hits, file by file.
    pub hits: Vec<HitEntry>,
    /// The previews back to back.
    arena: String,
    /// The skipped files and folders with their reasons.
    pub skipped: Vec<(String, String)>,
    /// The *Skipped* group is open.
    pub skipped_open: bool,
    /// The summary once the search ended.
    pub summary: Option<FilesSummary>,
    /// Why the search failed.
    pub error: Option<String>,
    /// The selected row.
    pub selected: Option<usize>,
    /// The panel's height (points; `state.dialogs."find-in-files".panel_height`).
    pub height: f32,
    running: Option<Job<TextFindOutcome>>,
    mailbox: Arc<Mutex<Vec<Event>>>,
    /// The file being streamed (its hits come next).
    current: Option<u32>,
    /// Events drained so far (tests: batched per frame).
    pub drains: u64,
    /// The preview of a Replace in Files plan (one line per file, its counts).
    pub preview: Vec<String>,
    /// A scroll offset of the list (points) the drawing applies once (G16 find files' scripted scrolling, M2 W11c).
    pub scroll_request: Option<f32>,
}

impl FindFilesPanel {
    /// Starts a search (a running one is cancelled; the panel empties).
    pub fn start(&mut self, service: &Service, wake: &crate::job::Wake, request: TextFindRequest) {
        self.stop();
        *self = FindFilesPanel {
            shown: true,
            height: if self.height > 0.0 {
                self.height
            } else {
                180.0
            },
            skipped_open: false,
            ..FindFilesPanel::default()
        };
        let mailbox = Arc::new(Mutex::new(Vec::new()));
        self.mailbox = Arc::clone(&mailbox);
        self.request = Some(request.clone());
        let sink_wake = Arc::clone(wake);
        self.running = Some(Job::spawn(
            service,
            JobKind::TextFind,
            "find in files".into(),
            wake,
            move |svc, job| {
                let mut sink = Mailbox {
                    events: mailbox,
                    wake: sink_wake,
                    pending: 0,
                };
                svc.text_find(&request, &mut sink, job)
            },
        ));
    }

    /// Stop (G4): the job is cancelled; the hits so far stay.
    pub fn stop(&mut self) {
        if let Some(job) = &self.running {
            job.cancel();
        }
    }

    /// True while the search runs.
    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// *Clear*: the panel empties (the search is stopped).
    pub fn clear(&mut self) {
        self.stop();
        self.running = None;
        let height = self.height;
        let shown = self.shown;
        *self = FindFilesPanel {
            height,
            shown,
            ..FindFilesPanel::default()
        };
    }

    /// Drains the mailbox (once per frame) and polls the job; true when something changed.
    pub fn poll(&mut self) -> bool {
        let events = std::mem::take(&mut *self.mailbox.lock());
        let changed = !events.is_empty();
        if changed {
            self.drains += 1;
        }
        for e in events {
            self.take(e);
        }
        let Some(job) = &self.running else {
            return changed;
        };
        let Some(result) = job.try_result() else {
            return changed;
        };
        self.running = None;
        // Events the job posted after the last drain.
        let late = std::mem::take(&mut *self.mailbox.lock());
        for e in late {
            self.take(e);
        }
        match result {
            Ok(outcome) => {
                let c = &outcome.dto.counts;
                self.summary = Some(FilesSummary {
                    files_searched: c.files_searched,
                    files_matched: c.files_matched,
                    hits: c.hits,
                    skipped: c.skipped,
                    limit_reached: outcome.dto.limit_reached,
                    warnings: outcome.warnings,
                });
            }
            Err(ApiError::Cancelled) => self.error = Some("Stopped.".to_owned()),
            Err(e) => self.error = Some(e.to_string()),
        }
        true
    }

    fn take(&mut self, e: Event) {
        match e {
            Event::File { spec, encoding } => {
                self.current = None;
                // A file is listed when its first hit comes.
                self.files.push(FileEntry {
                    spec,
                    encoding,
                    first_hit: self.hits.len() as u32,
                    hits: 0,
                    stopped: false,
                });
                self.current = Some(self.files.len() as u32 - 1);
            }
            Event::Hit {
                line,
                column,
                len,
                preview,
                range,
                truncated,
            } => {
                let Some(file) = self.current else {
                    return;
                };
                let start = self.arena.len() as u32;
                self.arena.push_str(&preview);
                self.hits.push(HitEntry {
                    file,
                    line,
                    column,
                    len,
                    preview: (start, self.arena.len() as u32),
                    range: (range.start, range.end),
                    truncated,
                });
                if let Some(f) = self.files.get_mut(file as usize) {
                    f.hits += 1;
                }
            }
            Event::Skipped { spec, reason } => self.skipped.push((spec, reason)),
            Event::Done { stopped } => {
                if let Some(file) = self.current.take() {
                    if self.files.get(file as usize).is_some_and(|f| f.hits == 0) {
                        // A file without hits is not listed.
                        self.files.pop();
                    } else if let Some(f) = self.files.get_mut(file as usize) {
                        f.stopped = stopped;
                    }
                }
            }
        }
    }

    /// The preview of hit `k`.
    pub fn preview(&self, k: usize) -> &str {
        self.hits.get(k).map_or("", |h| {
            self.arena
                .get(h.preview.0 as usize..h.preview.1 as usize)
                .unwrap_or("")
        })
    }

    /// The rows of the list: every file header with its hits, then the *Skipped* group.
    pub fn row_count(&self) -> usize {
        let skipped = if self.skipped.is_empty() {
            0
        } else if self.skipped_open {
            1 + self.skipped.len()
        } else {
            1
        };
        self.files.len() + self.hits.len() + skipped
    }

    /// The row at `index` (a binary search over the files).
    pub fn row(&self, index: usize) -> Option<ListRow> {
        let listed = self.files.len() + self.hits.len();
        if index >= listed {
            let k = index - listed;
            if self.skipped.is_empty() {
                return None;
            }
            return match k {
                0 => Some(ListRow::SkippedHeader),
                k if self.skipped_open && k <= self.skipped.len() => {
                    Some(ListRow::Skipped(k as u32 - 1))
                }
                _ => None,
            };
        }
        // File i's header is at row first_hit_i + i.
        let i = self
            .files
            .partition_point(|f| (f.first_hit as usize) <= index)
            .checked_sub(1)?;
        let mut i = i;
        // Rows grow by one per file header: walk back while the header is after `index`.
        while i > 0 && self.files[i].first_hit as usize + i > index {
            i -= 1;
        }
        let head = self.files[i].first_hit as usize + i;
        if index == head {
            Some(ListRow::File(i as u32))
        } else {
            let k = self.files[i].first_hit as usize + (index - head - 1);
            (k < (self.files[i].first_hit + self.files[i].hits) as usize)
                .then_some(ListRow::Hit(k as u32))
        }
    }

    /// The text of a row (`<spec> (N)` for a file, `line:column: preview` for a hit).
    pub fn row_text(&self, row: ListRow) -> String {
        match row {
            ListRow::File(i) => self.files.get(i as usize).map_or_else(String::new, |f| {
                format!("{} ({})", crate::shared::shown_path(&f.spec), f.hits)
            }),
            ListRow::Hit(k) => self.hits.get(k as usize).map_or_else(String::new, |h| {
                format!("{}:{}: {}", h.line, h.column, self.preview(k as usize))
            }),
            ListRow::SkippedHeader => format!("Skipped ({})", self.skipped.len()),
            ListRow::Skipped(k) => self
                .skipped
                .get(k as usize)
                .map_or_else(String::new, |(s, r)| format!("{s}: {r}")),
        }
    }

    /// The selected hit's file, line and column (Enter, double-click opens it).
    pub fn hit_target(&self, k: usize) -> Option<(String, u32, u32, u32)> {
        let h = self.hits.get(k)?;
        let f = self.files.get(h.file as usize)?;
        Some((f.spec.clone(), h.line, h.column, h.len))
    }

    /// The text of hit `k` (its part of the preview; a cut preview gives the part shown), to verify it where it is
    /// opened.
    pub fn hit_text(&self, k: usize) -> Option<String> {
        let h = self.hits.get(k)?;
        let (a, b) = h.range;
        let text: String = self
            .preview(k)
            .chars()
            .skip(a as usize)
            .take(b.saturating_sub(a) as usize)
            .collect();
        (!text.is_empty()).then_some(text)
    }

    /// Copy (Ctrl+C in the panel): the selected line as `<spec>:<line>:<column>: <preview>`; a file header copies its
    /// hits, one a line.
    pub fn copy_text(&self) -> Option<String> {
        let line = |k: usize| {
            let h = self.hits.get(k)?;
            let f = self.files.get(h.file as usize)?;
            Some(format!(
                "{}:{}:{}: {}",
                f.spec,
                h.line,
                h.column,
                self.preview(k)
            ))
        };
        match self.row(self.selected?)? {
            ListRow::Hit(k) => line(k as usize),
            ListRow::File(i) => {
                let f = self.files.get(i as usize)?;
                let lines: Vec<String> = (f.first_hit..f.first_hit + f.hits)
                    .filter_map(|k| line(k as usize))
                    .collect();
                Some(lines.join("\n"))
            }
            _ => None,
        }
    }

    /// The summary line (files searched, matched, hits, *limit reached*).
    pub fn summary_text(&self) -> String {
        if let Some(e) = &self.error {
            return format!(
                "{e} {} hits in {} files so far.",
                self.hits.len(),
                self.files.len()
            );
        }
        match &self.summary {
            Some(s) => format!(
                "{} files searched, {} matched, {} hits{}.",
                s.files_searched,
                s.files_matched,
                s.hits,
                if s.limit_reached {
                    " (limit reached)"
                } else {
                    ""
                }
            ),
            None if self.running() => format!(
                "Searching… {} hits in {} files.",
                self.hits.len(),
                self.files.len()
            ),
            None => String::new(),
        }
    }

    /// The panel's heap (the counting of A11.15 and G16 find files): the arena, the files and the hits by capacity.
    #[cfg(test)]
    pub fn heap_bytes(&self) -> usize {
        self.arena.capacity()
            + self.hits.capacity() * std::mem::size_of::<HitEntry>()
            + self.files.capacity() * std::mem::size_of::<FileEntry>()
            + self.files.iter().map(|f| f.spec.capacity()).sum::<usize>()
            + self
                .skipped
                .iter()
                .map(|(a, b)| a.capacity() + b.capacity())
                .sum::<usize>()
    }

    /// The previews' bytes (what G16's bound adds to 16 MiB).
    #[cfg(test)]
    pub fn preview_bytes(&self) -> usize {
        self.arena.len()
    }
}

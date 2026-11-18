//! The report dialog (M2 W13c, D-366; RPT-001, RPT-002, RPT-003): one toolkit-free model for every view kind that
//! has a report (Text Compare, Folder Compare, Folder Sync, Table Compare, Hex Compare, Picture Compare), over W7's
//! request, plan and targets and the vocabulary of D-367 (`hikasa_report::vocabulary`). The fields become the CLI's
//! REPORT flags ([`ReportDialog::flags`], T11) and every change is checked by `Service::report_request` (pure);
//! *Write Report* writes to a file or the clipboard, *View in Browser* and *Print Preview* write below
//! `<cache>/print/` and open the file with the associated application. `ui/report_dialog.rs` draws it; the window
//! builds it from the active view (`app/report.rs`).

use std::any::Any;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_report::ReportKind;
use hikasa_report::vocabulary::{
    HTML_CUSTOM_WORD, IGNORE_UNIMPORTANT_WORD, LayoutEntry, OptionEntry, ReportVocabulary,
    ViewWords, vocabulary,
};
use hikasa_session::api::{
    FolderCompareRequest, HexCompareResponse, PictureCompareResponse, PreparedFolderCompare,
    ReportChoices, ReportFlags, ReportSource, ReportTarget, SelectRequest, TableCompareResponse,
    TextCompareResponse,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::{ApiError, Service};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep};
use crate::job::{Job, Wake};

/// The help topic of the dialog.
pub const REPORT_TOPIC: &str = "dialog-report";

/// The button of *View in Browser*.
pub const BROWSER_BUTTON: ButtonRole = ButtonRole::Other(1);
/// The button of *Print Preview*.
pub const PRINT_BUTTON: ButtonRole = ButtonRole::Other(2);

/// What a button of the dialog does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportAction {
    /// *Write Report*: to the file or the clipboard.
    Write,
    /// *View in Browser*: a file below `<cache>/print/` opened by the associated application.
    ViewInBrowser,
    /// *Print Preview*: the same with the print block (palette and orientation).
    PrintPreview,
}

impl ReportAction {
    const ALL: [ReportAction; 3] = [
        ReportAction::Write,
        ReportAction::ViewInBrowser,
        ReportAction::PrintPreview,
    ];

    const fn index(self) -> usize {
        match self {
            ReportAction::Write => 0,
            ReportAction::ViewInBrowser => 1,
            ReportAction::PrintPreview => 2,
        }
    }

    /// The `output_to` word remembered after the action.
    fn output_to(self, to: ReportTo) -> &'static str {
        match (self, to) {
            (ReportAction::Write, ReportTo::File) => "file",
            (ReportAction::Write, ReportTo::Clipboard) => "clipboard",
            (ReportAction::ViewInBrowser, _) => "browser",
            (ReportAction::PrintPreview, _) => "printer",
        }
    }
}

/// Where *Write Report* writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTo {
    /// The file of the path field.
    File,
    /// The clipboard.
    Clipboard,
}

/// The comparison a report shows, owned by the dialog (the view's, as it was when the dialog opened).
#[derive(Clone)]
pub enum OwnedReportSource {
    /// A Text Compare view's live comparison (`Service::text_report_source`); `None` while a recompare of edits is
    /// pending (the report waits for its install).
    Text(Option<TextCompareResponse>),
    /// A Folder Compare or Folder Sync view: its request, its prepared comparison (the live model) and its
    /// ignore-unimportant; the job takes `Service::folder_view_response` of the model as it is.
    Folder {
        /// The view's request.
        request: Box<FolderCompareRequest>,
        /// The view's prepared comparison.
        prepared: Box<PreparedFolderCompare>,
        /// The view's ignore-unimportant.
        ignore_unimportant: bool,
    },
    /// A Table Compare view's response.
    Table(Box<TableCompareResponse>),
    /// A Hex Compare view's response (`hex_response` with the view's layout).
    Hex(Box<HexCompareResponse>),
    /// A Picture Compare view's response (`picture_response`).
    Picture(Box<PictureCompareResponse>),
}

impl OwnedReportSource {
    /// The report kind of the source.
    pub fn kind(&self) -> ReportKind {
        match self {
            OwnedReportSource::Text(_) => ReportKind::Text,
            OwnedReportSource::Folder { .. } => ReportKind::Folder,
            OwnedReportSource::Table(_) => ReportKind::Table,
            OwnedReportSource::Hex(_) => ReportKind::Hex,
            OwnedReportSource::Picture(_) => ReportKind::Picture,
        }
    }
}

/// The selection *Just selection* reports.
#[derive(Debug, Clone)]
pub enum ScopeSource {
    /// The focus pane's selected lines, as `--lines <side>:<a>-<b>`.
    Lines(String),
    /// The selected rows of a folder view (`--select`).
    Rows(Box<SelectRequest>),
}

/// What opens the dialog: the view's source and state, the remembered choices and the window's handles.
pub struct ReportInit {
    /// The view.
    pub view: ViewId,
    /// The view's comparison.
    pub source: OwnedReportSource,
    /// The view's state as words (display, context, options the view decides).
    pub view_words: ViewWords,
    /// The view's ignore-unimportant.
    pub ignore_unimportant: bool,
    /// The view's selection, when it has one.
    pub scope: Option<ScopeSource>,
    /// `report_choices(kind)`.
    pub choices: ReportChoices,
    /// The phase-3 keys of the table's dialog (`html`, `folder`), read once (D-366's migration).
    pub legacy: (Option<bool>, Option<PathBuf>),
    /// The home folder (relative paths without a remembered folder).
    pub home: Option<PathBuf>,
    /// Wakes the window when a job ends.
    pub wake: Wake,
    /// The window's ordered clipboard.
    pub clipboard: Arc<hikasa_session::clipboard::OrderedClipboard>,
}

/// What a report job did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Written {
    /// The file was written (bytes).
    File(u64),
    /// The report is on the clipboard.
    Clipboard,
    /// The file below `<cache>/print/` was handed to the associated application.
    Opened(PathBuf),
    /// The file exists and was left alone: the dialog asks first.
    Exists(PathBuf),
}

/// The report dialog (*<Type> Report…*).
pub struct ReportDialog {
    /// The view the report is of.
    pub view: ViewId,
    /// The kind of report.
    pub kind: ReportKind,
    source: OwnedReportSource,
    vocab: &'static ReportVocabulary,
    /// The layout word.
    pub layout: &'static str,
    /// The option words chosen (checks on, one word of each choice), across layouts.
    pub options: BTreeSet<&'static str>,
    /// The display word (the view's filter when the dialog opens; never remembered).
    pub display: Option<&'static str>,
    /// The number of context lines (text: *Context* and the unified and patch layouts).
    pub context: u32,
    /// The folders value (folder reports).
    pub folders: Option<&'static str>,
    /// *Ignore Unimportant* (kinds that take it; the view's when the dialog opens).
    pub ignore_unimportant: bool,
    /// *Just selection*.
    pub just_selection: bool,
    scope: Option<ScopeSource>,
    /// The title (`""`: the default; never remembered).
    pub title: String,
    /// The output word (`html-color`, `html-mono`, `html-custom:`, `text`).
    pub output: &'static str,
    /// The stylesheet path of `html-custom:`.
    pub css: String,
    /// The wrap word (text layouts that wrap).
    pub wrap: Option<&'static str>,
    /// The print palette and orientation words (*Print Preview*).
    pub print: (&'static str, &'static str),
    /// Where *Write Report* writes.
    pub to: ReportTo,
    /// The file as typed (a relative path is in [`ReportDialog::folder`]).
    pub path: String,
    /// The folder of relative paths: the remembered file's, else the home folder (never the current folder).
    pub folder: Option<PathBuf>,
    /// The existing file the next *Replace* writes over.
    pub confirm: Option<PathBuf>,
    /// The result line of the last action.
    pub message: Option<(bool, String)>,
    /// The core's refusal of each action's fields (`report_request`), `None` when accepted.
    refusals: [Option<String>; 3],
    checked: Option<String>,
    waiting: Option<ReportAction>,
    job: Option<(ReportAction, Job<Written>)>,
    requests: Vec<DialogRequest>,
    wake: Wake,
    clipboard: Arc<hikasa_session::clipboard::OrderedClipboard>,
}

/// The entry of `word` among the options of every layout of `vocab`.
fn entry_of(vocab: &'static ReportVocabulary, word: &str) -> Option<&'static OptionEntry> {
    vocab
        .layouts
        .iter()
        .flat_map(|l| l.options.iter())
        .chain(vocab.wrap.iter())
        .find(|o| o.words.contains(&word))
}

/// The listed `'static` word equal to `word` in `words`.
fn listed(words: &[&'static str], word: &str) -> Option<&'static str> {
    words.iter().copied().find(|w| *w == word)
}

impl ReportDialog {
    /// The dialog over a view's comparison: the remembered layout, options, output and file, the view's display,
    /// ignore-unimportant and view-decided options, the defaults of the vocabulary for the rest.
    pub fn new(init: ReportInit) -> ReportDialog {
        let kind = init.source.kind();
        let vocab = vocabulary(kind);
        let choices = &init.choices;
        let layout = choices
            .layout
            .as_deref()
            .and_then(|w| vocab.layout(w))
            .unwrap_or(&vocab.layouts[0])
            .word;
        let mut dialog = ReportDialog {
            view: init.view,
            kind,
            source: init.source,
            vocab,
            layout,
            options: BTreeSet::new(),
            display: None,
            context: 3,
            folders: vocab.folders.and_then(|f| f.default),
            ignore_unimportant: init.ignore_unimportant,
            just_selection: false,
            scope: init.scope,
            title: String::new(),
            output: vocab.outputs[0].word,
            css: String::new(),
            wrap: vocab.wrap.and_then(|w| w.default),
            print: (
                vocab.print[0].default.unwrap_or(vocab.print[0].words[0]),
                vocab.print[1].default.unwrap_or(vocab.print[1].words[0]),
            ),
            to: ReportTo::File,
            path: choices.file.clone().unwrap_or_default(),
            folder: None,
            confirm: None,
            message: None,
            refusals: [None, None, None],
            checked: None,
            waiting: None,
            job: None,
            requests: Vec::new(),
            wake: init.wake,
            clipboard: init.clipboard,
        };
        let remembered = choices.layout.is_some() || !choices.options.is_empty();
        for word in &choices.options {
            if let Some(entry) = entry_of(vocab, word) {
                dialog.choose(entry, word);
            }
        }
        // The table's phase-3 `html = false` is plain text until an output is remembered (D-366's migration).
        if init.legacy.0 == Some(false) {
            dialog.output = dialog.output_word(false).unwrap_or(dialog.output);
        }
        for word in &choices.output_options {
            if let Some(css) = word.strip_prefix(HTML_CUSTOM_WORD) {
                dialog.output = HTML_CUSTOM_WORD;
                css.trim().clone_into(&mut dialog.css);
            } else if let Some(w) = listed(
                &vocab.outputs.iter().map(|o| o.word).collect::<Vec<_>>(),
                word,
            ) {
                dialog.output = w;
            } else if let Some(w) = vocab.wrap.and_then(|e| listed(e.words, word)) {
                dialog.wrap = Some(w);
            } else if let Some(w) = listed(vocab.print[0].words, word) {
                dialog.print.0 = w;
            } else if let Some(w) = listed(vocab.print[1].words, word) {
                dialog.print.1 = w;
            }
        }
        if choices.output_to.as_deref() == Some(ReportAction::Write.output_to(ReportTo::Clipboard))
        {
            dialog.to = ReportTo::Clipboard;
        }
        // The folder of relative paths: the table's phase-3 `folder`, else the remembered file's, else home.
        let remembered_folder = choices
            .file
            .as_deref()
            .map(Path::new)
            .filter(|p| p.is_absolute())
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        dialog.folder = init.legacy.1.or(remembered_folder).or(init.home);
        // Defaults: every choice gets its default word; checks only when nothing is remembered.
        for layout in vocab.layouts {
            for entry in layout.options {
                let chosen = entry.words.iter().any(|w| dialog.options.contains(w));
                if let Some(default) = entry.default
                    && !chosen
                    && (!entry.is_check() || !remembered)
                {
                    dialog.options.insert(default);
                }
            }
        }
        // The view's state.
        dialog.display = init
            .view_words
            .display
            .and_then(|d| {
                listed(
                    &vocab.displays.iter().map(|e| e.word).collect::<Vec<_>>(),
                    d,
                )
            })
            .or_else(|| vocab.displays.first().map(|d| d.word));
        if let Some(n) = init.view_words.context {
            dialog.context = n;
        }
        if let (Some(f), Some(entry)) = (init.view_words.folders, vocab.folders) {
            dialog.folders = listed(entry.words, f).or(dialog.folders);
        }
        for word in &init.view_words.on {
            if let Some(entry) = entry_of(vocab, word) {
                dialog.choose(entry, word);
            }
        }
        for word in &init.view_words.off {
            dialog.options.remove(word);
        }
        dialog
    }

    /// Chooses `word` of `entry`: a choice's other words leave, a check is turned on.
    fn choose(&mut self, entry: &'static OptionEntry, word: &str) {
        let Some(word) = listed(entry.words, word) else {
            return;
        };
        for w in entry.words {
            self.options.remove(w);
        }
        self.options.insert(word);
    }

    /// Sets a check on or off, or chooses a choice's word (the drawing layer's call).
    pub fn set_option(&mut self, entry: &'static OptionEntry, word: Option<&'static str>) {
        match word {
            Some(w) => self.choose(entry, w),
            None => {
                for w in entry.words {
                    self.options.remove(w);
                }
            }
        }
    }

    /// The vocabulary of the dialog's kind.
    pub fn vocabulary(&self) -> &'static ReportVocabulary {
        self.vocab
    }

    /// The current layout's entry.
    pub fn layout_entry(&self) -> &'static LayoutEntry {
        self.vocab
            .layout(self.layout)
            .unwrap_or(&self.vocab.layouts[0])
    }

    /// The output word of the current layout's form: HTML (`html` true) or plain text, when the layout has it.
    fn output_word(&self, html: bool) -> Option<&'static str> {
        let forms = self.layout_entry().forms;
        self.vocab
            .outputs
            .iter()
            .find(|o| o.html == html)
            .map(|o| o.word)
            .filter(|_| if html { forms.html } else { forms.text })
    }

    /// The outputs the current layout offers (none for a layout that implies its form).
    pub fn outputs(&self) -> Vec<&'static hikasa_report::vocabulary::OutputEntry> {
        let forms = self.layout_entry().forms;
        self.vocab
            .outputs
            .iter()
            .filter(|o| if o.html { forms.html } else { forms.text })
            .collect()
    }

    /// True when *Just selection* can be checked: the view has a selection and the layout takes a scope (D-209).
    pub fn can_scope(&self) -> bool {
        self.scope.is_some() && self.layout_entry().scope
    }

    /// True while a report is written.
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// True while the report waits for the view's comparison (a recompare of edits).
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }

    /// True while the source is a Text Compare view's comparison not installed yet.
    pub fn wants_text_source(&self) -> bool {
        matches!(self.source, OwnedReportSource::Text(None))
    }

    /// The text view's comparison once installed (the window's call); a waiting action starts at the next poll.
    pub fn set_text_source(&mut self, response: TextCompareResponse) {
        if matches!(self.source, OwnedReportSource::Text(_)) {
            self.source = OwnedReportSource::Text(Some(response));
        }
    }

    /// The fraction written of a report with pair reports (`None`: unknown or idle).
    pub fn progress(&self) -> Option<f32> {
        self.job.as_ref().and_then(|(_, j)| j.progress().1)
    }

    /// The core's refusal of the fields, which greys the action buttons (`None`: accepted).
    pub fn refusal(&self) -> Option<&str> {
        self.refusals.iter().flatten().next().map(String::as_str)
    }

    /// The core's refusal of one action's fields.
    pub fn refusal_of(&self, action: ReportAction) -> Option<&str> {
        self.refusals[action.index()].as_deref()
    }

    /// The flags of the fields for `action`, as the CLI's REPORT flags would give them (the Service parses the
    /// words, T11).
    pub fn flags(&self, action: ReportAction) -> ReportFlags {
        let layout = self.layout_entry();
        let mut flags = ReportFlags::default();
        flags.words.layout = Some(layout.word.to_owned());
        for entry in layout.options {
            for word in entry.words {
                if self.options.contains(word) {
                    flags.words.options.push((*word).to_owned());
                }
            }
        }
        if layout.display
            && let Some(display) = self.display
        {
            flags.words.options.push(display.to_owned());
        }
        if self.ignore_unimportant && self.vocab.ignore_unimportant {
            flags.words.options.push(IGNORE_UNIMPORTANT_WORD.to_owned());
        }
        if layout.context_lines && self.kind == ReportKind::Text {
            flags.context = Some(self.context);
        }
        if let Some(folders) = self.folders.filter(|_| layout.display) {
            flags.folders = Some(folders.to_owned());
        }
        if !layout.forms.implied() && self.outputs().iter().any(|o| o.word == self.output) {
            flags
                .words
                .output_options
                .push(if self.output == HTML_CUSTOM_WORD {
                    format!("{HTML_CUSTOM_WORD}{}", self.css.trim())
                } else {
                    self.output.to_owned()
                });
        }
        if layout.wrap
            && let Some(wrap) = self.wrap
        {
            flags.words.output_options.push(wrap.to_owned());
        }
        if action == ReportAction::PrintPreview && layout.forms.html {
            flags.words.output_options.push(self.print.0.to_owned());
            flags.words.output_options.push(self.print.1.to_owned());
        }
        let title = self.title.trim();
        if !title.is_empty() {
            flags.words.title = Some(title.to_owned());
        }
        if self.just_selection && self.can_scope() {
            match &self.scope {
                Some(ScopeSource::Lines(lines)) => flags.lines = Some(lines.clone()),
                Some(ScopeSource::Rows(select)) => flags.select = Some((**select).clone()),
                None => {}
            }
        }
        flags
    }

    /// The file the fields name: an absolute path as typed, a relative one in [`ReportDialog::folder`] (never the
    /// GUI's current folder). `-` and `clipboard://` are refused (a window has no standard output; the clipboard is
    /// its own choice).
    pub fn file(&self) -> Result<PathBuf, String> {
        let typed = self.path.trim();
        if typed.is_empty() {
            return Err("Name the file the report is written to.".to_owned());
        }
        if typed == "-" {
            return Err(
                "`-` is the standard output of the command line: name a file, or choose Clipboard."
                    .to_owned(),
            );
        }
        if typed.to_ascii_lowercase().starts_with("clipboard:") {
            return Err("To copy the report, choose Clipboard instead of a file.".to_owned());
        }
        let path = Path::new(typed);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        match &self.folder {
            Some(folder) => Ok(folder.join(path)),
            None => Err("Name the file with its whole path (no home folder is known).".to_owned()),
        }
    }

    /// The target of `action`.
    pub fn target(&self, action: ReportAction) -> Result<ReportTarget, String> {
        match (action, self.to) {
            (ReportAction::ViewInBrowser, _) => Ok(ReportTarget::Browser),
            (ReportAction::PrintPreview, _) => Ok(ReportTarget::Printer),
            (ReportAction::Write, ReportTo::Clipboard) => Ok(ReportTarget::Clipboard),
            (ReportAction::Write, ReportTo::File) => {
                Ok(ReportTarget::File(OsString::from(self.file()?)))
            }
        }
    }

    /// Checks the fields of every action with `Service::report_request` (pure; on the UI thread): a refusal is
    /// the core's message, shown in place, and greys the action. Cheap when nothing changed.
    pub fn check(&mut self, svc: &Service) {
        let key = format!(
            "{:?}|{:?}|{:?}",
            self.flags(ReportAction::PrintPreview).words,
            self.flags(ReportAction::Write).words,
            (
                self.target(ReportAction::Write),
                self.context,
                self.folders,
                self.just_selection
            ),
        );
        if self.checked.as_deref() == Some(key.as_str()) {
            return;
        }
        for action in ReportAction::ALL {
            // A file not named yet is the Write button's own message, not a refusal of the words.
            let target = self.target(action).unwrap_or(ReportTarget::Clipboard);
            self.refusals[action.index()] = svc
                .report_request(self.kind, "side-by-side", &self.flags(action), &target)
                .err()
                .map(|e| e.to_string());
        }
        self.checked = Some(key);
    }

    /// True when the next *Write Report* replaces the existing file the dialog asked about.
    pub fn replacing(&self) -> bool {
        self.to == ReportTo::File && self.confirm.is_some() && self.confirm == self.file().ok()
    }

    /// The choices remembered after `action` (D-208): the layout, the option words (no display word), the output
    /// words, `output_to` and the file as typed.
    pub fn choices(&self, action: ReportAction) -> ReportChoices {
        let mut output_options = Vec::new();
        output_options.push(if self.output == HTML_CUSTOM_WORD {
            format!("{HTML_CUSTOM_WORD}{}", self.css.trim())
        } else {
            self.output.to_owned()
        });
        output_options.extend(self.wrap.map(str::to_owned));
        output_options.push(self.print.0.to_owned());
        output_options.push(self.print.1.to_owned());
        // `html-custom:` takes the rest of a list: last.
        output_options.sort_by_key(|w| w.starts_with(HTML_CUSTOM_WORD));
        let path = self.path.trim();
        ReportChoices {
            layout: Some(self.layout.to_owned()),
            options: self.options.iter().map(|w| (*w).to_owned()).collect(),
            output_options,
            output_to: Some(action.output_to(self.to).to_owned()),
            file: (!path.is_empty()).then(|| path.to_owned()),
        }
    }

    /// Starts `action` (or waits for the view's comparison first).
    fn start(&mut self, action: ReportAction, svc: &Service) {
        self.check(svc);
        if let Some(why) = self.refusal_of(action) {
            self.message = Some((false, why.to_owned()));
            return;
        }
        let target = match self.target(action) {
            Ok(t) => t,
            Err(e) => {
                self.message = Some((false, e));
                return;
            }
        };
        if self.wants_text_source() {
            self.waiting = Some(action);
            self.message = Some((
                true,
                "The report waits for the comparison of the edits.".to_owned(),
            ));
            return;
        }
        self.waiting = None;
        let flags = self.flags(action);
        let kind = self.kind;
        let source = with_ignore(self.source.clone(), self.ignore_unimportant);
        let clipboard = Arc::clone(&self.clipboard);
        let ticket = clipboard.ticket();
        let replace = self.replacing();
        let file = self
            .file()
            .ok()
            .filter(|_| action == ReportAction::Write && self.to == ReportTo::File);
        self.message = None;
        let job = Job::spawn(
            svc,
            JobKind::Report,
            format!("write the {} report", kind.as_str()),
            &self.wake,
            move |svc, job| {
                let mut req = svc.report_request(kind, "side-by-side", &flags, &target)?;
                let folder_response;
                let source = match &source {
                    OwnedReportSource::Text(Some(r)) => ReportSource::Text(r),
                    OwnedReportSource::Text(None) => return Err(ApiError::Cancelled),
                    OwnedReportSource::Table(r) => ReportSource::Table(r),
                    OwnedReportSource::Hex(r) => ReportSource::Hex(r),
                    OwnedReportSource::Picture(r) => ReportSource::Picture(r),
                    OwnedReportSource::Folder {
                        request,
                        prepared,
                        ignore_unimportant,
                    } => {
                        // The model as it is now (after operations, refreshes, Exclude).
                        folder_response = svc.folder_view_response(prepared, *ignore_unimportant);
                        ReportSource::Folder {
                            request,
                            prepared,
                            response: &folder_response,
                        }
                    }
                };
                let plan =
                    svc.report_plan(source, &mut req, &target, flags.select.as_ref(), job)?;
                // An existing file is replaced only after the dialog asked (the plan knows, one `lstat`).
                if plan.replaces
                    && !replace
                    && let Some(file) = file
                {
                    return Ok(Written::Exists(file));
                }
                let outcome = svc.report_execute(source, &req, plan, None, job)?;
                if outcome.cancelled {
                    return Err(ApiError::Cancelled);
                }
                // The adapter places a clipboard report (architecture check 2), in the window's order of writes.
                if let Some(payload) = &outcome.clipboard {
                    clipboard
                        .in_turn(ticket, |c| c.write_payload(payload))
                        .map_err(|e| ApiError::Vfs(e.to_vfs_write()))?;
                    return Ok(Written::Clipboard);
                }
                match outcome.opened {
                    Some(path) => Ok(Written::Opened(path)),
                    None => Ok(Written::File(outcome.bytes)),
                }
            },
        );
        self.job = Some((action, job));
    }

    /// The *Stop* of a running report, or of a waiting one.
    fn stop(&mut self) {
        if let Some((_, job)) = &self.job {
            job.cancel();
        } else if self.waiting.take().is_some() {
            self.message = Some((false, "The report was stopped.".to_owned()));
        }
    }
}

/// `source` under the dialog's *Ignore Unimportant*: the text, table and picture reports take the response's flag
/// (as the CLI's `--ignore-unimportant` sets it), so the response is the one that flag gives; a folder report takes
/// it with the view's model.
fn with_ignore(source: OwnedReportSource, iu: bool) -> OwnedReportSource {
    match source {
        OwnedReportSource::Text(Some(mut r)) if r.ignore_unimportant != iu => {
            r.ignore_unimportant = iu;
            r.verdict = r.comparison.verdict(iu);
            r.exit_code = hikasa_core::ExitCode::for_content(&r.verdict, iu);
            OwnedReportSource::Text(Some(r))
        }
        OwnedReportSource::Table(mut r) if r.ignore_unimportant != iu => {
            r.ignore_unimportant = iu;
            r.kind = r.comparison.kind(r.bytes_identical, iu);
            r.exit_code = hikasa_core::ExitCode::for_kind(r.kind, iu);
            OwnedReportSource::Table(r)
        }
        OwnedReportSource::Folder {
            request, prepared, ..
        } => OwnedReportSource::Folder {
            request,
            prepared,
            ignore_unimportant: iu,
        },
        other => other,
    }
}

impl DialogModel for ReportDialog {
    fn title(&self) -> String {
        let title = match self.kind {
            ReportKind::Text => crate::commands::Scope::Text.type_title(),
            ReportKind::Folder => crate::commands::Scope::Folder.type_title(),
            ReportKind::Table => crate::commands::Scope::Table.type_title(),
            ReportKind::Hex => crate::commands::Scope::Hex.type_title(),
            ReportKind::Picture => crate::commands::Scope::Picture.type_title(),
        };
        format!("{title} Report")
    }

    fn help_topic(&self) -> &'static str {
        REPORT_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let busy = self.running() || self.waiting();
        let button = |label: &str, role, action: Option<ReportAction>, default| DialogButton {
            label: label.to_owned(),
            role,
            enabled: action.is_none_or(|a| !busy && self.refusal_of(a).is_none()),
            default,
        };
        let browser = self.layout_entry().forms.html;
        vec![
            button(
                if self.replacing() {
                    "Replace"
                } else {
                    "Write Report"
                },
                ButtonRole::Ok,
                Some(ReportAction::Write),
                true,
            ),
            DialogButton {
                enabled: browser && !busy && self.refusal_of(ReportAction::ViewInBrowser).is_none(),
                ..button("View in Browser", BROWSER_BUTTON, None, false)
            },
            DialogButton {
                enabled: browser && !busy && self.refusal_of(ReportAction::PrintPreview).is_none(),
                ..button("Print Preview", PRINT_BUTTON, None, false)
            },
            button(
                if busy { "Stop" } else { "Close" },
                ButtonRole::Close,
                None,
                false,
            ),
        ]
    }

    fn button_tip(&self, role: ButtonRole) -> Option<String> {
        let action = match role {
            ButtonRole::Ok => ReportAction::Write,
            BROWSER_BUTTON => ReportAction::ViewInBrowser,
            PRINT_BUTTON => ReportAction::PrintPreview,
            _ => return None,
        };
        if action != ReportAction::Write && !self.layout_entry().forms.html {
            return Some("This layout has no HTML form.".to_owned());
        }
        self.refusal_of(action).map(str::to_owned)
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        let action = match role {
            ButtonRole::Ok => Some(ReportAction::Write),
            BROWSER_BUTTON => Some(ReportAction::ViewInBrowser),
            PRINT_BUTTON => Some(ReportAction::PrintPreview),
            _ => None,
        };
        match action {
            Some(action) if !self.running() && !self.waiting() => {
                self.start(action, svc);
                DialogStep::Stay
            }
            Some(_) => DialogStep::Stay,
            None if self.running() || self.waiting() => {
                self.stop();
                DialogStep::Stay
            }
            None => DialogStep::Close,
        }
    }

    fn poll(&mut self, svc: &Service) -> bool {
        self.check(svc);
        if let Some(action) = self.waiting
            && !self.wants_text_source()
        {
            self.start(action, svc);
            return true;
        }
        let Some(result) = self.job.as_ref().and_then(|(_, j)| j.try_result()) else {
            return false;
        };
        let Some((action, _)) = self.job.take() else {
            return false;
        };
        let file = self.file().ok();
        let ok = matches!(
            result,
            Ok(Written::File(_) | Written::Clipboard | Written::Opened(_))
        );
        self.message = Some(match result {
            Ok(Written::File(bytes)) => {
                self.confirm = None;
                let name = file.map_or_else(String::new, |f| f.display().to_string());
                (
                    true,
                    format!("The report was written to {name} ({bytes} bytes)."),
                )
            }
            Ok(Written::Clipboard) => (true, "The report is on the clipboard.".to_owned()),
            Ok(Written::Opened(path)) => (
                true,
                format!("The report was written to {} and opened.", path.display()),
            ),
            Ok(Written::Exists(path)) => {
                let text = format!(
                    "{} exists. Replace writes the report over it.",
                    path.display()
                );
                self.confirm = Some(path);
                (false, text)
            }
            Err(ApiError::Cancelled) => (false, "The report was stopped.".to_owned()),
            Err(e) => (false, e.to_string()),
        });
        if ok {
            // Remembered after each successful action (D-208), on the window's settings queue.
            let (kind, choices) = (self.kind, self.choices(action));
            if let ReportTo::File = self.to
                && action == ReportAction::Write
                && let Ok(file) = self.file()
            {
                self.folder = file.parent().map(Path::to_path_buf).or(self.folder.take());
            }
            self.requests.push(DialogRequest::Settings {
                title: "remember the report choices".to_owned(),
                body: Box::new(move |svc: &Service| {
                    svc.remember_report_choices(kind, &choices)?;
                    Ok(vec![
                        svc.settings_store()
                            .path(hikasa_session::settings::FileId::State),
                    ])
                }),
            });
        }
        true
    }

    fn busy(&self) -> bool {
        self.running()
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((600.0, 440.0))
    }

    fn take_requests(&mut self) -> Vec<DialogRequest> {
        std::mem::take(&mut self.requests)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

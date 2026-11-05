//! The dialogs of the folder verbs that plan no file operation (M2 W9b, D-245; contract §1.9.13): *Exclude*
//! (FC-052), *Compare Contents* and its results (FC-029 b), *Quick Compare*. Toolkit-free models (check 7): each reads
//! what core resolves in a job (the masks of `folder_exclude_masks`, the file pairs of `folder_selection`, the
//! results of `quick`) and shows it as it comes; the window runs what they answer.

use std::any::Any;
use std::sync::Arc;

use hikasa_core::{ContentKind, ContentMethod, ContentResult, JobKind};
use hikasa_folder::select::{ExcludeWhat, Selection};
use hikasa_folder::{FolderModel, NodeId};
use hikasa_session::api::{ContentsDto, QuickMethod, QuickRequest, SelectRequest};
use hikasa_session::ipc::ViewId;
use hikasa_session::{ApiError, Service};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};
use crate::folder::glyphs::content_mark;
use crate::job::Job;

/// The help topics.
pub const EXCLUDE_TOPIC: &str = "dialog-exclude";
/// Compare Contents and its results.
pub const COMPARE_CONTENTS_TOPIC: &str = "dialog-compare-contents";
/// Quick Compare.
pub const QUICK_COMPARE_TOPIC: &str = "dialog-quick-compare";
/// The `state.dialogs` ids (§1.9.13).
pub const EXCLUDE_DIALOG: &str = "exclude";
/// Compare Contents.
pub const COMPARE_CONTENTS_DIALOG: &str = "compare-contents";
/// Quick Compare.
pub const QUICK_COMPARE_DIALOG: &str = "quick-compare";
/// At most this many pairs in Quick Compare ("… and n more").
pub const QUICK_LIMIT: usize = 1000;

type Wake = Arc<dyn Fn() + Send + Sync>;

fn ok_cancel(ok: &str, enabled: bool) -> Vec<DialogButton> {
    vec![
        DialogButton {
            label: ok.to_owned(),
            role: ButtonRole::Ok,
            enabled,
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

/// The text of a content result (the centre column's tooltip, D-236).
pub fn result_text(kind: ContentKind) -> &'static str {
    content_mark(kind).1
}

// ---- Exclude (FC-052) -------------------------------------------------------------------------------------------

/// *Exclude*: the selected items by their paths, or every item of their types; the masks `folder op exclude` would
/// add are shown before OK; OK adds them to the view's name filters and the view runs again.
pub struct ExcludeDialog {
    /// The view.
    pub view: ViewId,
    model: Arc<FolderModel>,
    select: SelectRequest,
    what: ExcludeWhat,
    job: Option<Job<(Vec<String>, Vec<String>)>>,
    masks: Option<Result<(Vec<String>, Vec<String>), String>>,
    dirty: bool,
    chosen: Option<(Vec<String>, Vec<String>)>,
    wake: Wake,
}

impl ExcludeDialog {
    /// The dialog over the selection `select` of `model`; `what` is the remembered choice.
    pub fn new(
        view: ViewId,
        model: Arc<FolderModel>,
        select: SelectRequest,
        what: ExcludeWhat,
        wake: Wake,
    ) -> ExcludeDialog {
        ExcludeDialog {
            view,
            model,
            select,
            what,
            job: None,
            masks: None,
            dirty: true,
            chosen: None,
            wake,
        }
    }

    /// *Selected items* or *All files of these types*.
    pub fn what(&self) -> ExcludeWhat {
        self.what
    }

    /// Picks what is excluded (the masks are read again).
    pub fn set_what(&mut self, what: ExcludeWhat) {
        if self.what != what {
            self.what = what;
            self.dirty = true;
        }
    }

    /// The masks for files and for folders, or the message of a selection that gives none; `None` while read.
    pub fn masks(&self) -> Option<&Result<(Vec<String>, Vec<String>), String>> {
        self.masks.as_ref()
    }

    /// The masks once OK was pressed.
    pub fn take_chosen(&mut self) -> Option<(Vec<String>, Vec<String>)> {
        self.chosen.take()
    }

    /// The word remembered in `state.dialogs.exclude.what` (core's word of `--what`).
    pub fn what_word(&self) -> &'static str {
        self.what.as_str()
    }
}

impl DialogModel for ExcludeDialog {
    fn title(&self) -> String {
        "Exclude".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        EXCLUDE_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        ok_cancel("OK", !self.dirty && matches!(self.masks, Some(Ok(_))))
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => match &self.masks {
                Some(Ok(masks)) if !self.dirty => {
                    self.chosen = Some(masks.clone());
                    DialogStep::Close
                }
                _ => DialogStep::Stay,
            },
            ButtonRole::Cancel => DialogStep::Close,
            _ => DialogStep::Stay,
        }
    }

    fn poll(&mut self, svc: &Service) -> bool {
        if self.dirty {
            self.dirty = false;
            if let Some(job) = &self.job {
                job.cancel();
            }
            let (model, select, what) = (Arc::clone(&self.model), self.select.clone(), self.what);
            self.masks = None;
            self.job = Some(Job::spawn(
                svc,
                JobKind::FolderOp,
                "exclude masks".into(),
                &self.wake,
                move |svc, _| svc.folder_exclude_masks(&model, &select, what),
            ));
            return true;
        }
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        self.masks = Some(result.map_err(|e| e.to_string()));
        true
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---- Compare Contents (FC-029 b) --------------------------------------------------------------------------------

/// The pairs a content compare reads: how many file pairs, and their bytes (both sides).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PairCount {
    /// File pairs.
    pub pairs: u64,
    /// Bytes on both sides.
    pub bytes: u64,
}

/// The file pairs of a resolved selection (the rows core resolved; each a file on both sides).
pub fn pair_count(model: &FolderModel, rows: &Selection) -> PairCount {
    let read = model.read();
    let mut count = PairCount::default();
    for (node, _) in rows.iter() {
        let Some(view) = read.node(node) else {
            continue;
        };
        if let (Some(left), Some(right)) = (&view.left, &view.right)
            && !view.is_folder()
        {
            count.pairs += 1;
            count.bytes += left.size + right.size;
        }
    }
    count
}

/// *Compare Contents*: CRC, Binary or Rules-based (the session's method by default), *Show results dialog*; the file
/// pairs it reads are counted in a job (the question `confirm_content_compare` asks names them).
pub struct CompareContentsDialog {
    /// The view.
    pub view: ViewId,
    /// The method.
    pub method: ContentMethod,
    /// *Show results dialog*.
    pub show_results: bool,
    count: Option<Result<PairCount, String>>,
    job: Option<Job<PairCount>>,
    chosen: Option<(ContentMethod, bool, PairCount)>,
}

impl CompareContentsDialog {
    /// The dialog over `select` of `model`.
    pub fn new(
        svc: &Service,
        view: ViewId,
        model: Arc<FolderModel>,
        select: SelectRequest,
        method: ContentMethod,
        show_results: bool,
        wake: Wake,
    ) -> CompareContentsDialog {
        let job = Job::spawn(
            svc,
            JobKind::FolderOp,
            "content pairs".into(),
            &wake,
            move |svc, _| {
                let rows = svc.folder_selection(&model, &select)?;
                Ok(pair_count(&model, &rows))
            },
        );
        CompareContentsDialog {
            view,
            method: if method == ContentMethod::None {
                ContentMethod::Binary
            } else {
                method
            },
            show_results,
            count: None,
            job: Some(job),
            chosen: None,
        }
    }

    /// The pairs it reads, once counted.
    pub fn count(&self) -> Option<&Result<PairCount, String>> {
        self.count.as_ref()
    }

    /// The choices once OK was pressed: method, *Show results dialog*, the pairs.
    pub fn take_chosen(&mut self) -> Option<(ContentMethod, bool, PairCount)> {
        self.chosen.take()
    }
}

impl DialogModel for CompareContentsDialog {
    fn title(&self) -> String {
        "Compare Contents".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        COMPARE_CONTENTS_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let ready = matches!(self.count, Some(Ok(c)) if c.pairs > 0);
        ok_cancel("Compare", ready)
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => match self.count {
                Some(Ok(count)) if count.pairs > 0 => {
                    self.chosen = Some((self.method, self.show_results, count));
                    DialogStep::Close
                }
                _ => DialogStep::Stay,
            },
            ButtonRole::Cancel => {
                if let Some(job) = &self.job {
                    job.cancel();
                }
                DialogStep::Close
            }
            _ => DialogStep::Stay,
        }
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        self.count = Some(match result {
            Ok(count) if count.pairs == 0 => Err("No file pair is selected.".to_owned()),
            Ok(count) => Ok(count),
            Err(e) => Err(e.to_string()),
        });
        true
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The results of a content compare: `ContentReport`'s counts and its items (virtualized), as `folder op compare`
/// prints them.
pub struct ContentsResultsDialog {
    dto: ContentsDto,
}

impl ContentsResultsDialog {
    /// The results of `dto`.
    pub fn new(dto: ContentsDto) -> ContentsResultsDialog {
        ContentsResultsDialog { dto }
    }

    /// The counts' lines.
    pub fn counts(&self) -> Vec<String> {
        let c = &self.dto.counts;
        let mut out = Vec::new();
        for (kind, n) in [
            (ContentKind::BinarySame, c.binary_same),
            (ContentKind::RulesSame, c.rules_same),
            (ContentKind::UnimportantOnly, c.unimportant_only),
            (ContentKind::ImportantDifferences, c.important_differences),
            (ContentKind::BinaryDifferent, c.binary_different),
        ] {
            if n > 0 {
                out.push(format!("{}: {n}", result_text(kind)));
            }
        }
        if c.error > 0 {
            out.push(format!("Errors: {}", c.error));
        }
        if self.dto.cancelled {
            out.push("Stopped: the rows not reached are not listed.".to_owned());
        }
        out
    }

    /// The number of items.
    pub fn len(&self) -> usize {
        self.dto.items.len()
    }

    /// True without items.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.dto.items.is_empty()
    }

    /// Item `index`: its path, its result's text, whether it failed.
    pub fn item(&self, index: usize) -> Option<(String, String, bool)> {
        let item = self.dto.items.get(index)?;
        Some(match (&item.error, item.result) {
            (Some(error), _) => (item.path.clone(), error.message.clone(), true),
            (None, Some(kind)) => (item.path.clone(), result_text(kind).to_owned(), false),
            (None, None) => (item.path.clone(), String::new(), false),
        })
    }
}

impl DialogModel for ContentsResultsDialog {
    fn title(&self) -> String {
        "Compare Contents Results".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        COMPARE_CONTENTS_TOPIC
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((600.0, 360.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![DialogButton {
            label: "Close".to_owned(),
            role: ButtonRole::Close,
            enabled: true,
            default: true,
        }]
    }

    fn on_button(&mut self, _role: ButtonRole, _svc: &Service) -> DialogStep {
        DialogStep::Close
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---- Quick Compare ----------------------------------------------------------------------------------------------

/// One pair of Quick Compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickRow {
    /// The row.
    pub node: NodeId,
    /// Its path.
    pub path: String,
    /// Its result (`hikasa quick`'s), or the error message.
    pub result: Result<ContentKind, String>,
}

/// *Quick Compare*: the content result of each selected file pair (at most [`QUICK_LIMIT`]), Binary or Rules-based;
/// *Open* opens the selected pair. One job runs `Service::quick` per pair; a change of method runs them again.
pub struct QuickCompareDialog {
    /// The view.
    pub view: ViewId,
    model: Arc<FolderModel>,
    select: SelectRequest,
    method: QuickMethod,
    ignore_unimportant: bool,
    rows: Vec<QuickRow>,
    more: usize,
    job: Option<Job<(Vec<QuickRow>, usize)>>,
    dirty: bool,
    error: Option<String>,
    /// The row picked in the list.
    pub selected: Option<usize>,
    open: Option<NodeId>,
    wake: Wake,
}

/// The role of *Open*.
pub const OPEN: ButtonRole = ButtonRole::Other(5);

impl QuickCompareDialog {
    /// The dialog over `select` of `model`.
    pub fn new(
        view: ViewId,
        model: Arc<FolderModel>,
        select: SelectRequest,
        method: QuickMethod,
        ignore_unimportant: bool,
        wake: Wake,
    ) -> QuickCompareDialog {
        QuickCompareDialog {
            view,
            model,
            select,
            method,
            ignore_unimportant,
            rows: Vec::new(),
            more: 0,
            job: None,
            dirty: true,
            error: None,
            selected: None,
            open: None,
            wake,
        }
    }

    /// The method.
    pub fn method(&self) -> QuickMethod {
        self.method
    }

    /// Binary or Rules-based (the pairs are compared again).
    pub fn set_method(&mut self, method: QuickMethod) {
        if self.method != method {
            self.method = method;
            self.dirty = true;
        }
    }

    /// The pairs compared, in tree order.
    pub fn rows(&self) -> &[QuickRow] {
        &self.rows
    }

    /// How many selected pairs past the limit were not compared ("… and n more").
    pub fn more(&self) -> usize {
        self.more
    }

    /// True while the pairs are compared.
    pub fn running(&self) -> bool {
        self.dirty || self.job.is_some()
    }

    /// A failure of the whole run (a selection that cannot be resolved).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The row *Open* opens, once pressed.
    pub fn take_open(&mut self) -> Option<NodeId> {
        self.open.take()
    }

    /// The remembered method's word.
    pub fn method_word(&self) -> &'static str {
        self.method.as_str()
    }
}

impl DialogModel for QuickCompareDialog {
    fn title(&self) -> String {
        "Quick Compare".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        QUICK_COMPARE_TOPIC
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((600.0, 360.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "Open".to_owned(),
                role: OPEN,
                enabled: self.selected.is_some_and(|i| i < self.rows.len()),
                default: false,
            },
            DialogButton {
                label: "Close".to_owned(),
                role: ButtonRole::Close,
                enabled: true,
                default: true,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            OPEN => match self.selected.and_then(|i| self.rows.get(i)) {
                Some(row) => {
                    self.open = Some(row.node);
                    DialogStep::Close
                }
                None => DialogStep::Stay,
            },
            ButtonRole::Close | ButtonRole::Cancel => {
                if let Some(job) = &self.job {
                    job.cancel();
                }
                DialogStep::Close
            }
            _ => DialogStep::Stay,
        }
    }

    fn poll(&mut self, svc: &Service) -> bool {
        if self.dirty {
            self.dirty = false;
            if let Some(job) = &self.job {
                job.cancel();
            }
            let (model, select) = (Arc::clone(&self.model), self.select.clone());
            let (method, iu) = (self.method, self.ignore_unimportant);
            self.job = Some(Job::spawn(
                svc,
                JobKind::QuickCompare,
                "quick compare".into(),
                &self.wake,
                move |svc, job| {
                    let rows = svc.folder_selection(&model, &select)?;
                    let pairs: Vec<(NodeId, String, String, String)> = {
                        let read = model.read();
                        rows.iter()
                            .filter_map(|(node, _)| {
                                let view = read.node(node)?;
                                if view.is_folder() || view.left.is_none() || view.right.is_none() {
                                    return None;
                                }
                                Some((
                                    node,
                                    read.path(node).as_str().to_owned(),
                                    read.spec(node, hikasa_core::Side::Left)?.canonical(),
                                    read.spec(node, hikasa_core::Side::Right)?.canonical(),
                                ))
                            })
                            .collect()
                    };
                    let more = pairs.len().saturating_sub(QUICK_LIMIT);
                    let mut out = Vec::new();
                    for (node, path, left, right) in pairs.into_iter().take(QUICK_LIMIT) {
                        if job.cancel.is_cancelled() {
                            return Err(ApiError::Cancelled);
                        }
                        let request = QuickRequest {
                            left: left.into(),
                            right: right.into(),
                            method,
                            ignore_unimportant: iu,
                            ..QuickRequest::default()
                        };
                        let result = match svc.quick(&request, job) {
                            Ok(response) => match response.outcome.result {
                                ContentResult::Error { error } => Err(error.message),
                                other => Ok(other.kind()),
                            },
                            Err(e) => Err(e.to_string()),
                        };
                        out.push(QuickRow { node, path, result });
                    }
                    Ok((out, more))
                },
            ));
            self.rows.clear();
            self.selected = None;
            return true;
        }
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        match result {
            Ok((rows, more)) => {
                self.rows = rows;
                self.more = more;
                self.error = None;
            }
            Err(ApiError::Cancelled) => {}
            Err(e) => self.error = Some(e.to_string()),
        }
        true
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

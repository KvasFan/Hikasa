//! The Quick Compare dialog at startup (M2 W13c, D-368; CLI-005, OPT-001): when the GUI is asked from the command
//! line (the process's arguments, `hikasa gui open a b`, the bare form) to open two files with an inferred view kind
//! and no view words, and `[startup] show_quick_compare_dialog` is on, this modal dialog comes first: both specs
//! with size and modification time, the method (*Binary* / *Rules-based*; a change runs again), `Service::quick`'s
//! result as the folder Quick Compare says it (D-245) and the view *Open View* would open. With
//! `open_view_if_different` a different result opens the view at once. *Close* answers a waiting caller with the quick
//! result's exit code; *Open View* opens the view (the caller waits for it). Toolkit-free; `ui/quick_start.rs`
//! draws it, the window acts when it closes (`app.rs`).

use std::any::Any;
use std::ffi::OsString;
use std::path::PathBuf;

use hikasa_core::{ContentKind, ContentResult, JobKind};
use hikasa_session::api::{QuickMethod, QuickRequest, ViewOutcome, ViewRequest};
use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};
use hikasa_session::settings::options::QuickCompareMethod;
use hikasa_session::{ApiError, Service};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep};
use crate::job::{Job, Wake};

/// The help topic of the dialog.
pub const QUICK_START_TOPIC: &str = "dialog-quick-compare-startup";

/// The *Open View* button.
pub const OPEN_VIEW: ButtonRole = ButtonRole::Other(1);

/// The two specs of an open the dialog may come before: a file view's request (Text, Table, Hex, Picture).
pub fn quick_specs(request: &ViewRequest) -> Option<[OsString; 2]> {
    match request {
        ViewRequest::Text { request, .. } => Some([request.left.clone(), request.right.clone()]),
        ViewRequest::Table { request, .. } => Some([request.left.clone(), request.right.clone()]),
        ViewRequest::Hex { request, .. } => Some([request.left.clone(), request.right.clone()]),
        ViewRequest::Picture { request, .. } => Some([request.left.clone(), request.right.clone()]),
        _ => None,
    }
}

/// The quick method of the option (`[startup] quick_compare_method`).
pub fn method_of(method: QuickCompareMethod) -> QuickMethod {
    match method {
        QuickCompareMethod::Binary => QuickMethod::Binary,
        QuickCompareMethod::RulesBased => QuickMethod::Rules,
    }
}

/// What the dialog decided when it closed.
#[derive(Debug, Clone, PartialEq)]
pub enum QuickChoice {
    /// Open the view (the waiter waits for the view).
    OpenView,
    /// Close: a waiter gets this outcome.
    Close(ViewOutcome),
}

/// The facts of one side: size and modification time, or why not.
pub type Facts = Result<(u64, Option<String>), String>;

/// What the job found.
struct Found {
    facts: [Facts; 2],
    result: Result<(ContentResult, hikasa_core::ExitCode), String>,
}

/// *Quick Compare* at startup.
pub struct QuickStartDialog {
    /// The view id the open was registered under (its waiter waits on it).
    pub view: ViewId,
    /// The view *Open View* opens (`infer_view_kind`, D-369).
    pub kind: ViewKind,
    request: Option<ViewRequest>,
    options: OpenOptions,
    /// The specs as given.
    pub specs: [OsString; 2],
    /// Size and time of each side, once read.
    pub facts: [Option<Facts>; 2],
    /// The method.
    pub method: QuickMethod,
    /// The result (`Service::quick`'s), or the error in place.
    pub result: Option<Result<(ContentResult, hikasa_core::ExitCode), String>>,
    /// *Show this dialog when starting with two files*.
    pub show_at_start: bool,
    open_if_different: bool,
    rerun: bool,
    job: Option<Job<Found>>,
    chosen: Option<QuickChoice>,
    requests: Vec<DialogRequest>,
    wake: Wake,
    /// The options file the check writes (acknowledged with the watch).
    options_path: PathBuf,
}

impl QuickStartDialog {
    /// The dialog over an open of `view` (`request` with `options`); the quick job starts at once.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        svc: &Service,
        startup: &hikasa_session::settings::options::Startup,
        view: ViewId,
        kind: ViewKind,
        request: ViewRequest,
        options: OpenOptions,
        specs: [OsString; 2],
        wake: Wake,
    ) -> QuickStartDialog {
        let mut dialog = QuickStartDialog {
            view,
            kind,
            request: Some(request),
            options,
            specs,
            facts: [None, None],
            method: method_of(startup.quick_compare_method),
            result: None,
            show_at_start: true,
            open_if_different: startup.open_view_if_different,
            rerun: false,
            job: None,
            chosen: None,
            requests: Vec::new(),
            wake,
            options_path: svc
                .settings_store()
                .path(hikasa_session::settings::FileId::Options),
        };
        dialog.run(svc);
        dialog
    }

    /// A new method: the comparison runs again at the next poll (the older job is cancelled).
    pub fn set_method(&mut self, method: QuickMethod) {
        if method != self.method {
            self.method = method;
            self.rerun = true;
        }
    }

    fn run(&mut self, svc: &Service) {
        if let Some(job) = self.job.take() {
            job.cancel();
        }
        self.result = None;
        let (left, right) = (self.specs[0].clone(), self.specs[1].clone());
        let method = self.method;
        self.job = Some(Job::spawn(
            svc,
            JobKind::QuickCompare,
            "quick compare".into(),
            &self.wake,
            move |svc, job| {
                let facts = [&left, &right].map(|s| svc.spec_facts(s).map_err(|e| e.to_string()));
                let request = QuickRequest {
                    left,
                    right,
                    method,
                    ..QuickRequest::default()
                };
                let result = match svc.quick(&request, job) {
                    Ok(r) => Ok((r.outcome.result, r.exit_code)),
                    Err(ApiError::Cancelled) => return Err(ApiError::Cancelled),
                    Err(e) => Err(e.to_string()),
                };
                Ok(Found { facts, result })
            },
        ));
    }

    /// The caption of the view *Open View* opens.
    pub fn kind_title(&self) -> &'static str {
        use crate::commands::Scope;
        match self.kind {
            ViewKind::TableCompare => Scope::Table.type_title(),
            ViewKind::HexCompare => Scope::Hex.type_title(),
            ViewKind::PictureCompare => Scope::Picture.type_title(),
            _ => Scope::Text.type_title(),
        }
    }

    /// True while the comparison runs.
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// The result's kind (`None` while it runs or after an error).
    pub fn kind_of_result(&self) -> Option<ContentKind> {
        match &self.result {
            Some(Ok((result, _))) => Some(result.kind()),
            _ => None,
        }
    }

    /// The result as the folder Quick Compare says it (D-245), or the error.
    pub fn result_text(&self) -> Option<String> {
        match &self.result {
            Some(Ok((result, _))) => match result {
                ContentResult::Error { error } => Some(error.message.clone()),
                other => Some(crate::content_dialogs::result_text(other.kind()).to_owned()),
            },
            Some(Err(e)) => Some(e.clone()),
            None => None,
        }
    }

    /// True when the result says the files are the same (binary or by the rules).
    pub fn same(&self) -> bool {
        matches!(
            self.kind_of_result(),
            Some(ContentKind::BinarySame | ContentKind::RulesSame)
        )
    }

    /// The check *Show this dialog when starting with two files*: written at once (`settings_set`, D-73).
    pub fn set_show_at_start(&mut self, on: bool) {
        if on == self.show_at_start {
            return;
        }
        self.show_at_start = on;
        let path = self.options_path.clone();
        self.requests.push(DialogRequest::Settings {
            title: "set an option".to_owned(),
            body: Box::new(move |svc: &Service| {
                let key = hikasa_session::settings::parse_key(
                    "options.startup.show_quick_compare_dialog",
                )?;
                svc.settings_set(&key, &on.to_string(), false)?;
                Ok(vec![path])
            }),
        });
    }

    /// The decision and the open it acts on (once, when the dialog closed).
    pub fn take_choice(&mut self) -> Option<(QuickChoice, Option<(ViewRequest, OpenOptions)>)> {
        let choice = self.chosen.take()?;
        let open = match choice {
            QuickChoice::OpenView => self
                .request
                .take()
                .map(|r| (r, std::mem::take(&mut self.options))),
            QuickChoice::Close(_) => None,
        };
        Some((choice, open))
    }

    /// The outcome of *Close*: the quick result's (`quick`'s exit codes: 1, 2, 11, 12, 13; 100 for an error).
    fn close_outcome(&self) -> ViewOutcome {
        match &self.result {
            Some(Ok((result, _))) => ViewOutcome::Text {
                result: result.clone(),
                ignore_unimportant: false,
            },
            Some(Err(_)) => ViewOutcome::LoadFailed,
            None => ViewOutcome::NoVerdict,
        }
    }
}

impl DialogModel for QuickStartDialog {
    fn title(&self) -> String {
        "Quick Compare".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        QUICK_START_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let different = self.result.is_some() && !self.same();
        vec![
            DialogButton {
                label: "Open View".to_owned(),
                role: OPEN_VIEW,
                enabled: !self.running(),
                default: different,
            },
            DialogButton {
                label: if self.running() { "Stop" } else { "Close" }.to_owned(),
                role: ButtonRole::Close,
                enabled: true,
                default: !different,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            OPEN_VIEW if !self.running() => {
                self.chosen = Some(QuickChoice::OpenView);
                DialogStep::Close
            }
            OPEN_VIEW => DialogStep::Stay,
            _ if self.running() => {
                if let Some(job) = self.job.take() {
                    job.cancel();
                }
                self.result = Some(Err("The comparison was stopped.".to_owned()));
                DialogStep::Stay
            }
            _ => {
                self.chosen = Some(QuickChoice::Close(self.close_outcome()));
                DialogStep::Close
            }
        }
    }

    fn on_close(&mut self) -> bool {
        self.chosen = Some(QuickChoice::Close(self.close_outcome()));
        false
    }

    fn poll(&mut self, svc: &Service) -> bool {
        if std::mem::take(&mut self.rerun) {
            self.run(svc);
            return true;
        }
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        match result {
            Ok(found) => {
                self.facts = found.facts.map(Some);
                self.result = Some(found.result);
            }
            Err(ApiError::Cancelled) => return true,
            Err(e) => self.result = Some(Err(e.to_string())),
        }
        // A different result opens the view at once (`open_view_if_different`).
        if self.open_if_different && matches!(self.result, Some(Ok(_))) && !self.same() {
            self.chosen = Some(QuickChoice::OpenView);
            self.requests.push(DialogRequest::Close);
        }
        true
    }

    fn busy(&self) -> bool {
        self.running()
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

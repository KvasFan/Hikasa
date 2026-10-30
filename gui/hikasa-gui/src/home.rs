//! The Home view (UI-001): the entry screen of a new tab, with two path fields and Compare. The paths go
//! through the same resolution as the command line (`Service::resolve_open`: specs against the GUI's current
//! folder, the view kind from CLI-002) and the same mapping as an IPC request (`Service::open_view_request`).
//!
//! The UI thread never blocks (contract §1.10): the fields are passed verbatim (spaces are legal in names,
//! §2.1), standard input (`-`) and `clipboard://` are refused at once with an inline message (a window has no
//! standard input to read; the command line can spool it), and the resolution, which stats the paths, runs
//! as a job (`Service::spawn`); the view opens when the job's result arrives.

use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::{JobKind, Spec};
use hikasa_session::api::{GuiOpenRequest, ViewRequest};
use hikasa_session::ipc::{OpenOptions, ViewKind};
use hikasa_session::{ApiError, Service};

use crate::commands::Ctx;
use crate::job::Job;

/// Shown when a field is empty.
pub const NEED_TWO: &str = "Enter two files or two folders.";
/// Shown for `-`.
pub const NO_STDIN: &str = "Standard input (-) cannot be compared from Home: the window has no input to read. \
                            Pass - on the command line instead (hikasa gui open - FILE).";
/// Shown for `clipboard://`.
pub const NO_CLIPBOARD: &str = "clipboard:// cannot be opened in the GUI in this version.";

/// The running resolution of a Compare (compared by identity, so the model stays a plain value for the UI).
#[derive(Clone)]
pub struct Pending(Arc<Job<HomeOpen>>);

impl PartialEq for Pending {
    fn eq(&self, other: &Pending) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Pending {}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Pending").field(&self.0.id()).finish()
    }
}

/// The Home view model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomeModel {
    /// Left path as typed.
    pub left: String,
    /// Right path as typed.
    pub right: String,
    /// The last error (shown under the fields).
    pub error: Option<String>,
    /// The resolution started by Compare, while it runs.
    pending: Option<Pending>,
}

/// What Compare opens.
#[derive(Debug, Clone)]
pub struct HomeOpen {
    /// View kind.
    pub kind: ViewKind,
    /// The view's request.
    pub request: ViewRequest,
    /// Options (titles).
    pub options: OpenOptions,
    /// Canonical absolute specs.
    pub specs: Vec<String>,
}

impl HomeModel {
    /// True when both paths are filled in (blank fields count as empty; the paths themselves are not trimmed).
    pub fn ready(&self) -> bool {
        !self.left.trim().is_empty() && !self.right.trim().is_empty()
    }

    /// True while Compare resolves the paths.
    pub fn resolving(&self) -> bool {
        self.pending.is_some()
    }

    /// True while the resolution has ended but its result is not published yet.
    pub fn settling(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.0.settling())
    }

    /// Fills the command context.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.home_ready = self.ready() && !self.resolving();
        ctx.busy = self.resolving();
        ctx.ready = true;
    }

    /// Starts resolving the two paths (off the UI thread); `wake` is called when the result is there. Refuses
    /// `-` and `clipboard://` at once. The result is picked up by [`HomeModel::poll`].
    pub fn compare(&mut self, service: &Service, wake: Arc<dyn Fn() + Send + Sync>) {
        if self.resolving() {
            return;
        }
        if !self.ready() {
            self.error = Some(NEED_TWO.into());
            return;
        }
        let specs = vec![OsString::from(&self.left), OsString::from(&self.right)];
        // Parsing is pure (no I/O, standard input is not touched): the kinds that a window cannot open are
        // refused before anything runs.
        let ctx = service.spec_context();
        for spec in &specs {
            match Spec::parse_os(spec, &ctx) {
                Ok(Spec::Stdin) => {
                    self.error = Some(NO_STDIN.into());
                    return;
                }
                Ok(Spec::Clipboard) => {
                    self.error = Some(NO_CLIPBOARD.into());
                    return;
                }
                _ => {}
            }
        }
        self.error = None;
        let request = GuiOpenRequest {
            specs,
            ..GuiOpenRequest::default()
        };
        let job = Job::spawn(
            service,
            JobKind::VfsRead,
            "resolve the Home paths".into(),
            &wake,
            move |svc, _job| resolve(svc, &request),
        );
        self.pending = Some(Pending(Arc::new(job)));
    }

    /// A copy of the typed fields (`TabDuplicate` of a Home tab, D-107): no error, nothing running.
    pub fn copy_fields(&self) -> HomeModel {
        HomeModel {
            left: self.left.clone(),
            right: self.right.clone(),
            error: None,
            pending: None,
        }
    }

    /// Picks up the result of a running Compare: `Some` opens the view; an error is kept for display. The
    /// second value is true when the model changed.
    pub fn poll(&mut self) -> (Option<HomeOpen>, bool) {
        let Some(result) = self.pending.as_ref().and_then(|p| p.0.try_result()) else {
            return (None, false);
        };
        self.pending = None;
        match result {
            Ok(open) => (Some(open), true),
            Err(e) => {
                self.error = Some(e.to_string());
                (None, true)
            }
        }
    }
}

/// `resolve_open` + `open_view_request` (on the job thread; also the window's drops, UI-008).
pub fn resolve(service: &Service, request: &GuiOpenRequest) -> Result<HomeOpen, ApiError> {
    let resolved = service.resolve_open(request)?;
    let view = service.open_view_request(resolved.kind, &resolved.specs, &resolved.options)?;
    Ok(HomeOpen {
        kind: resolved.kind,
        request: view,
        options: resolved.options,
        specs: resolved.specs,
    })
}

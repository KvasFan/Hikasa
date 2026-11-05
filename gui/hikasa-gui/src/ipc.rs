//! The GUI's `IpcHandler` (ARCH-090…092, contract §4.4.3 rule 5): it never blocks; `OpenView` is mapped to
//! the view's request with `Service::open_view_from_ipc` (which reads the GUI's own clipboard for a
//! `clipboard://` spec first, M2 W3 D-101), registered (with its waiter) and posted to the UI
//! thread; `LoadSession` reads the session and posts it, `LoadWorkspace` reads the workspace, registers its tabs
//! and posts it (M2 W4b, D-125: the window runs the close actions and restores the tabs lazily, D-126);
//! `ImportPackage` posts the package (M2 W4c, D-127: the window shows the import wizard); `RunScript` takes the script
//! slot (`busy` while another script runs), then a worker locates, reads and checks the script with the client's cwd
//! and environment, posts it and answers (M2 W8b, D-228: the window shows its status window; 105/106 and a relative
//! path are `invalid_request`; the `Hello` lists `run_script.dry_run`); `ListViews` reads the registry; `CloseView` and
//! `Shutdown` are posted.

use std::sync::Arc;

use hikasa_core::ErrorKind;
use hikasa_session::api::ViewRequest;
use hikasa_session::ipc::{
    FEATURE_SCRIPT_DRY_RUN, IpcErrorCode, IpcHandler, OpenOptions, PROTOCOL_VERSION, Request,
    Response, ScriptReply, ScriptWaiter, ServerInfo, ViewId, ViewInfo, ViewKind, ViewWaiter,
};
use hikasa_session::sessions::{SessionName, SessionRef};
use hikasa_session::{ApiError, HIKASA_VERSION, Service};

use crate::app::{OpenedSession, UiMsg, load_named};
use crate::mailbox::Mailbox;
use crate::script_dialog::{BUSY, ScriptClaim, prepare};
use crate::shared::{Shared, view_title};
use crate::workspace::register_tabs;

/// The handler of the GUI's IPC server.
pub struct GuiHandler {
    service: Service,
    shared: Arc<Shared>,
    mailbox: Mailbox<UiMsg>,
}

impl GuiHandler {
    /// Registers a view and posts it to the UI thread; returns its id.
    fn open(
        &self,
        kind: ViewKind,
        specs: Vec<String>,
        title: String,
        request: ViewRequest,
        options: OpenOptions,
        waiter: Option<ViewWaiter>,
    ) -> ViewId {
        let view = self.shared.allocate();
        self.shared.register(
            ViewInfo {
                view,
                kind,
                title,
                specs,
            },
            waiter,
        );
        self.mailbox.post(UiMsg::Open {
            view,
            request: Box::new(request),
            options: Box::new(options),
        });
        view
    }

    /// The session of `LoadSession` (D-89 error codes); its display is cleared, as for a script (SES-010).
    fn session(&self, name: &str) -> Result<OpenedSession, Response> {
        let reference = SessionRef::parse(name).map_err(|e| error_response(&e))?;
        let mut opened = load_named(&self.service, &reference).map_err(|e| error_response(&e))?;
        opened.session = opened.session.for_script();
        Ok(opened)
    }

    /// A handler posting to `mailbox`.
    pub fn new(service: Service, shared: Arc<Shared>, mailbox: Mailbox<UiMsg>) -> GuiHandler {
        GuiHandler {
            service,
            shared,
            mailbox,
        }
    }
}

impl IpcHandler for GuiHandler {
    fn server_info(&self) -> ServerInfo {
        ServerInfo {
            server: format!("hikasa-gui {HIKASA_VERSION}"),
            version: PROTOCOL_VERSION,
        }
    }

    fn handle(&self, request: Request, waiter: Option<ViewWaiter>) -> Response {
        match request {
            Request::OpenView {
                kind,
                specs,
                options,
                wait: _,
            } => {
                if !crate::window::served(kind) {
                    return Response::error(
                        IpcErrorCode::UnsupportedView,
                        hikasa_session::ipc::unserved_view(kind),
                    );
                }
                // `clipboard://` is read from the GUI's own clipboard first (D-101).
                match self.service.open_view_from_ipc(kind, &specs, &options) {
                    Err(e) => Response::error(IpcErrorCode::InvalidRequest, e.to_string()),
                    Ok(view_request) => {
                        // The tab caption's title, so `gui list` shows one title from the start.
                        let spec = |k: usize| specs.get(k).map_or("", String::as_str);
                        let title = view_title(
                            [options.titles[0].as_deref(), options.titles[1].as_deref()],
                            [spec(0), spec(1)],
                        );
                        let view = self.open(kind, specs, title, view_request, options, waiter);
                        Response::ViewOpened { view }
                    }
                }
            }
            Request::LoadSession { name, wait: _ } => match self.session(&name) {
                Ok(opened) => {
                    let view = self.shared.allocate();
                    self.shared.register(
                        ViewInfo {
                            view,
                            kind: opened.view.kind,
                            title: opened.view.title.clone(),
                            specs: opened.view.specs.clone(),
                        },
                        waiter,
                    );
                    self.mailbox.post(UiMsg::OpenSession {
                        view,
                        opened: Box::new(opened),
                    });
                    Response::ViewOpened { view }
                }
                Err(response) => response,
            },
            Request::LoadWorkspace { name } => {
                let load = SessionName::parse(&name)
                    .and_then(|n| self.service.workspace_load(&n))
                    .map_err(|e| error_response(&e));
                match load {
                    Ok(load) => {
                        // The tabs are listed from now on (D-126); the window restores them lazily.
                        let ids = register_tabs(&self.shared, &load);
                        let wanted: Vec<ViewId> = ids.iter().flatten().flatten().copied().collect();
                        let views = self
                            .shared
                            .list()
                            .into_iter()
                            .filter(|v| wanted.contains(&v.view))
                            .collect();
                        self.mailbox.post(UiMsg::LoadWorkspace {
                            load: Box::new(load),
                            ids,
                        });
                        Response::Views { views }
                    }
                    Err(response) => response,
                }
            }
            // D-127: the window shows the wizard; a bad package is its step 1's message, not an IPC error.
            Request::ImportPackage { path } => {
                let path = std::path::PathBuf::from(path);
                if path.is_absolute() {
                    self.mailbox.post(UiMsg::ImportPackage(path));
                    Response::Ok
                } else {
                    Response::error(
                        IpcErrorCode::InvalidRequest,
                        format!("{}: an absolute path is needed", path.display()),
                    )
                }
            }
            Request::ListViews => Response::Views {
                views: self.shared.list(),
            },
            Request::CloseView { view } => {
                if self.shared.contains(view) {
                    self.mailbox.post(UiMsg::Close { view });
                    Response::Ok
                } else {
                    Response::error(IpcErrorCode::NotFound, format!("no view {}", view.0))
                }
            }
            Request::Ping => Response::Pong,
            Request::Shutdown => {
                self.mailbox.post(UiMsg::Quit);
                Response::Ok
            }
            other => Response::error(
                IpcErrorCode::UnsupportedRequest,
                format!("`{}` is not supported in this version", other.type_name()),
            ),
        }
    }

    fn features(&self) -> Vec<String> {
        vec![FEATURE_SCRIPT_DRY_RUN.to_owned()]
    }

    /// [`IpcHandler::handle_script_later`], waited for (direct callers; the server calls the other).
    fn handle_script(&self, request: Request, waiter: Option<ScriptWaiter>) -> Response {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.handle_script_later(
            request,
            waiter,
            ScriptReply::new(move |response| {
                let _ = tx.send(response);
            }),
        );
        rx.recv().unwrap_or_else(|_| {
            Response::error(
                IpcErrorCode::Internal,
                "the request ended without an answer",
            )
        })
    }

    /// `RunScript` (D-228): the slot and the request's words are checked here (`busy`, `invalid_request` for a
    /// relative `path` or `cwd` or a bad confirm mode); the lookup, the read (up to 16 MiB, an archive member's too) and
    /// the check run on a worker, which posts the run and answers (review F7: the handler must not block).
    fn handle_script_later(
        &self,
        request: Request,
        waiter: Option<ScriptWaiter>,
        reply: ScriptReply,
    ) {
        let Request::RunScript {
            path,
            args,
            cwd,
            env,
            wait: _,
            stop_on_error,
            confirm,
            close,
            dry_run,
        } = request
        else {
            reply.send(Response::error(
                IpcErrorCode::InvalidRequest,
                "not a run_script",
            ));
            return;
        };
        let Some(claim) = ScriptClaim::take(&self.shared) else {
            reply.send(Response::error(IpcErrorCode::Busy, BUSY));
            return;
        };
        let invalid = |message: String| Response::error(IpcErrorCode::InvalidRequest, message);
        let confirm = match confirm
            .as_deref()
            .map(hikasa_session::api::parse_confirm_mode)
            .transpose()
        {
            Ok(confirm) => confirm,
            Err(e) => return reply.send(invalid(e.to_string())),
        };
        // §1.8.13: the script's path is absolute (an archive member's canonical spec starts with its archive's).
        if !std::path::Path::new(&path).is_absolute() {
            return reply.send(invalid(format!("{path}: an absolute path is needed")));
        }
        // The run's relative paths mean what they mean in the client's shell.
        let service = match cwd.map(std::path::PathBuf::from) {
            Some(cwd) if cwd.is_absolute() => self.service.with_cwd(cwd),
            Some(cwd) => {
                return reply.send(invalid(format!(
                    "{}: an absolute path is needed",
                    cwd.display()
                )));
            }
            None => self.service.clone(),
        };
        let env = env
            .into_iter()
            .map(|(name, value)| (name, std::ffi::OsString::from(value)))
            .collect();
        let args = args.into_iter().map(std::ffi::OsString::from).collect();
        let mailbox = self.mailbox.clone();
        let worker = move || {
            match prepare(
                &service,
                std::ffi::OsStr::new(&path),
                args,
                Some(env),
                stop_on_error,
                confirm,
                close,
            ) {
                Ok(mut start) => {
                    start.request.dry_run = dry_run;
                    start.waiter = waiter;
                    start.claim = Some(claim);
                    mailbox.post(UiMsg::RunScript(Box::new(start)));
                    reply.send(Response::Ok);
                }
                Err(e) => {
                    // The slot is free before the client hears.
                    drop(claim);
                    reply.send(invalid(e.message()));
                }
            }
        };
        // A worker that cannot start drops the reply: `Error{internal}`.
        if let Err(e) = std::thread::Builder::new()
            .name("hikasa-ipc-script".into())
            .spawn(worker)
        {
            tracing::warn!(target: "hikasa::gui", "cannot start the RunScript worker: {e}");
        }
    }
}

/// The IPC error of a session or workspace that cannot be opened (D-89): an unknown name is `not_found`, a
/// type this GUI cannot show `unsupported_request`, anything else `invalid_request`.
fn error_response(error: &ApiError) -> Response {
    let code = match error.kind() {
        ErrorKind::NotFound => IpcErrorCode::NotFound,
        ErrorKind::Unsupported => IpcErrorCode::UnsupportedRequest,
        _ => IpcErrorCode::InvalidRequest,
    };
    Response::error(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_options_or_spec_names() {
        let specs = ["/w/src/a.c", "/w/dir/"];
        assert_eq!(view_title([None, None], specs), "a.c ⇄ dir");
        assert_eq!(view_title([Some("old"), None], specs), "old ⇄ dir");
        assert_eq!(view_title([None, None], ["/", "C:\\x\\y.txt"]), "/ ⇄ y.txt");
    }
}

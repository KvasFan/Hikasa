//! The GUI's `IpcHandler` (ARCH-090…092, contract §4.4.3 rule 5): it never blocks; `OpenView` is mapped to
//! the view's request with `Service::open_view_from_ipc` (which reads the GUI's own clipboard for a
//! `clipboard://` spec first, M2 W3 D-101), registered (with its waiter) and posted to the UI
//! thread; `LoadSession` reads the session and posts it, `LoadWorkspace` reads the workspace, registers its tabs
//! and posts it (M2 W4b, D-125: the window runs the close actions and restores the tabs lazily, D-126);
//! `ListViews` reads the registry; `CloseView` and `Shutdown` are posted.

use std::sync::Arc;

use hikasa_core::ErrorKind;
use hikasa_session::api::ViewRequest;
use hikasa_session::ipc::{
    IpcErrorCode, IpcHandler, OpenOptions, PROTOCOL_VERSION, Request, Response, ServerInfo, ViewId,
    ViewInfo, ViewKind, ViewWaiter,
};
use hikasa_session::sessions::{SessionName, SessionRef};
use hikasa_session::{ApiError, HIKASA_VERSION, Service};

use crate::app::{OpenedSession, UiMsg, load_named};
use crate::mailbox::Mailbox;
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
                if !kind.is_served_in_m1() {
                    return Response::error(
                        IpcErrorCode::UnsupportedView,
                        format!(
                            "the {} view is not available in this version",
                            kind.as_str()
                        ),
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

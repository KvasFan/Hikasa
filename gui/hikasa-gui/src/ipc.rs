//! The GUI's `IpcHandler` (ARCH-090…092, contract §4.4.3 rule 5): it never blocks; `OpenView` is mapped to
//! the view's request with `Service::open_view_request`, registered (with its waiter) and posted to the UI
//! thread; `LoadSession` and `LoadWorkspace` (M2 W2, D-89) take their views from `Service::session_view` and
//! `Service::workspace_views` and open them the same way (a workspace first closes the open views, SES-040);
//! `ListViews` reads the registry; `CloseView` and `Shutdown` are posted.

use std::sync::Arc;

use hikasa_core::ErrorKind;
use hikasa_session::api::{SessionView, ViewRequest};
use hikasa_session::ipc::{
    IpcErrorCode, IpcHandler, OpenOptions, PROTOCOL_VERSION, Request, Response, ServerInfo, ViewId,
    ViewInfo, ViewKind, ViewWaiter,
};
use hikasa_session::sessions::{SessionName, SessionRef};
use hikasa_session::{ApiError, HIKASA_VERSION, Service};

use crate::app::UiMsg;
use crate::mailbox::Mailbox;
use crate::shared::{Shared, view_title};

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

    /// The view of a session (D-89 error codes).
    fn session(&self, name: &str) -> Result<SessionView, Response> {
        let reference = SessionRef::parse(name).map_err(|e| error_response(&e))?;
        self.service
            .session_view(&reference)
            .map_err(|e| error_response(&e))
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
                match self.service.open_view_request(kind, &specs, &options) {
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
                Ok(v) => {
                    let view = self.open(v.kind, v.specs, v.title, v.request, v.options, waiter);
                    Response::ViewOpened { view }
                }
                Err(response) => response,
            },
            Request::LoadWorkspace { name } => {
                let views = SessionName::parse(&name)
                    .and_then(|n| self.service.workspace_views(&n))
                    .map_err(|e| error_response(&e));
                match views {
                    Ok((views, warnings)) => {
                        for warning in &warnings {
                            tracing::warn!(target: "hikasa::gui", "{}: {}", warning.code, warning.message);
                        }
                        // Loading a workspace closes the open views (SES-040).
                        for open in self.shared.list() {
                            self.mailbox.post(UiMsg::Close { view: open.view });
                        }
                        let mut opened = Vec::new();
                        for v in views {
                            let specs = v.specs.clone();
                            let view = self.open(
                                v.kind,
                                v.specs,
                                v.title.clone(),
                                v.request,
                                v.options,
                                None,
                            );
                            opened.push(ViewInfo {
                                view,
                                kind: v.kind,
                                title: v.title,
                                specs,
                            });
                        }
                        Response::Views { views: opened }
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

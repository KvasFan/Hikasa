//! The GUI's `IpcHandler` (ARCH-090…092, contract §4.4.3 rule 5): it never blocks; `OpenView` is mapped to
//! the view's request with `Service::open_view_request`, registered (with its waiter) and posted to the UI
//! thread; `ListViews` reads the registry; `CloseView` and `Shutdown` are posted.

use std::sync::Arc;

use hikasa_session::ipc::{
    IpcErrorCode, IpcHandler, PROTOCOL_VERSION, Request, Response, ServerInfo, ViewInfo, ViewWaiter,
};
use hikasa_session::{HIKASA_VERSION, Service};

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
                        let view = self.shared.allocate();
                        // The tab caption's title, so `gui list` shows one title from the start.
                        let spec = |k: usize| specs.get(k).map_or("", String::as_str);
                        let title = view_title(
                            [options.titles[0].as_deref(), options.titles[1].as_deref()],
                            [spec(0), spec(1)],
                        );
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
                            request: Box::new(view_request),
                            options: Box::new(options),
                        });
                        Response::ViewOpened { view }
                    }
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

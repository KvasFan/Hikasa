//! The page ↔ Rust channel of the canvas views: the page fetches `/hk/<op>?…` from the webview's own protocol
//! and this handler answers on the UI thread (no DOM node per line, ARCH-GUI-002: the page asks for the slice
//! it is about to draw and Rust computes only that slice).
//!
//! | Op | Answer |
//! |---|---|
//! | `meta?view` | [`TextMeta`](crate::text_view::TextMeta) |
//! | `slice?view&start&count` | [`TextSlice`](crate::text_view::TextSlice) |
//! | `thumb?view&h` | [`ThumbData`](crate::text_view::ThumbData) |
//! | `viewport?view&top&page` | the canvas viewport (no content) |
//! | `click?view&pos` | `{"changed": bool}` (a gap bar expanded) |
//! | `thumbclick?view&row` | `{"pos": n}` |
//! | `drawn?view&phase` | benchmark first-screen marks |
//! | `bench` (body) | benchmark results |

use std::borrow::Cow;

use dioxus::desktop::RequestAsyncResponder;
use dioxus::desktop::wry::http::{HeaderValue, Request, Response, StatusCode, header};
use dioxus::prelude::WritableExt;
use serde::Serialize;

use crate::app::ViewModel;
use crate::text_view::TextViewModel;
use crate::ui::Ui;

/// The query parameters of a request.
struct Query(Vec<(String, String)>);

impl Query {
    fn parse(query: Option<&str>) -> Query {
        Query(
            query
                .unwrap_or("")
                .split('&')
                .filter_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    Some((k.to_owned(), v.to_owned()))
                })
                .collect(),
        )
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.as_str() == key)
            .map(|(_, v)| v.as_str())
    }

    fn u32(&self, key: &str) -> Option<u32> {
        self.get(key)?.parse().ok()
    }

    fn u64(&self, key: &str) -> Option<u64> {
        self.get(key)?.parse().ok()
    }
}

fn json<T: Serialize>(value: &T) -> Response<Cow<'static, [u8]>> {
    match serde_json::to_vec(value) {
        Ok(bytes) => {
            let mut response = Response::new(Cow::Owned(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            response
        }
        Err(e) => status(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn status(code: StatusCode, text: &str) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Owned(text.as_bytes().to_vec()));
    *response.status_mut() = code;
    response
}

fn empty() -> Response<Cow<'static, [u8]>> {
    status(StatusCode::NO_CONTENT, "")
}

/// Runs `f` on the text view `view`; `None` when there is no such view.
fn with_text<R>(ui: Ui, view: u64, f: impl FnOnce(&mut TextViewModel) -> R) -> Option<R> {
    let mut app = ui.app;
    let mut core = app.write();
    let tab = core.tab_mut(hikasa_session::ipc::ViewId(view))?;
    match &mut tab.model {
        ViewModel::Text(m) => Some(f(m)),
        _ => None,
    }
}

/// Answers one request of the page.
pub fn handle(ui: Ui, request: &Request<Vec<u8>>, responder: RequestAsyncResponder) {
    let path = request.uri().path();
    let op = path.trim_start_matches('/').trim_start_matches("hk/");
    let q = Query::parse(request.uri().query());
    let view = q.u64("view").unwrap_or(0);
    let response = match op {
        "meta" => with_text(ui, view, |m| json(&m.meta())),
        "slice" => {
            let start = q.u32("start").unwrap_or(0);
            let count = q.u32("count").unwrap_or(64);
            let began = std::time::Instant::now();
            let response = with_text(ui, view, |m| json(&m.slice(start, count)));
            let mut bench = ui.bench;
            if let Some(b) = bench.write().as_mut() {
                b.record_slice(began.elapsed());
            }
            response
        }
        "thumb" => {
            let height = q.u32("h").unwrap_or(256);
            with_text(ui, view, |m| json(&m.thumbnail(height)))
        }
        "viewport" => {
            let top = q.u32("top").unwrap_or(0);
            let page = q.u32("page").unwrap_or(1);
            with_text(ui, view, |m| {
                m.set_viewport(top, page);
                empty()
            })
        }
        "click" => {
            let pos = q.u32("pos").unwrap_or(0);
            let changed = with_text(ui, view, |m| m.click(pos));
            if changed.is_some() {
                let mut rev = ui.rev;
                rev += 1;
            }
            changed.map(|changed| json(&serde_json::json!({ "changed": changed })))
        }
        "thumbclick" => {
            let row = q.u32("row").unwrap_or(0);
            with_text(ui, view, |m| {
                json(&serde_json::json!({ "pos": m.position_of_row(row) }))
            })
        }
        "drawn" => {
            let phase = q.u32("phase").unwrap_or(0);
            crate::bench::drawn(ui, view, phase);
            Some(empty())
        }
        "bench" => {
            crate::bench::results(ui, request.body());
            Some(empty())
        }
        _ => None,
    };
    responder.respond(response.unwrap_or_else(|| status(StatusCode::NOT_FOUND, "not found")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_parse() {
        let q = Query::parse(Some("view=7&start=120&count=64&x"));
        assert_eq!(q.u64("view"), Some(7));
        assert_eq!(q.u32("start"), Some(120));
        assert_eq!(q.u32("count"), Some(64));
        assert_eq!(q.get("x"), None);
        assert_eq!(Query::parse(None).u32("view"), None);
    }
}

//! The Text Compare pane: file headers, the canvas host (the page script draws the panes, the line numbers,
//! the thumbnail and the scroll bar from the slices it fetches, ARCH-GUI-002) and the status bar.

use dioxus::prelude::*;

use crate::text_view::TextStatus;

/// The Text Compare pane.
#[component]
pub fn TextPane(view: u64, status: TextStatus) -> Element {
    use_drop(move || {
        let _ = document::eval(&format!(
            "window.hikasa && window.hikasa.textUnmount({view})"
        ));
    });
    let [left, right] = &status.panes;
    rsx! {
        div { class: "hk-text",
            div { class: "hk-theads",
                div { class: "hk-thead", title: "{left.title}",
                    span { class: "hk-thead-title", "{left.title}" }
                    span { class: "hk-thead-detail", "{left.detail}" }
                }
                div { class: "hk-thead", title: "{right.title}",
                    span { class: "hk-thead-title", "{right.title}" }
                    span { class: "hk-thead-detail", "{right.detail}" }
                }
            }
            if let Some(error) = &status.error {
                div { class: "hk-error", "{error}" }
            }
            div {
                class: "hk-tbody",
                id: "hk-tb-{view}",
                onmounted: move |_| {
                    let _ = document::eval(&format!("window.hikasa && window.hikasa.textMount({view})"));
                },
                canvas { class: "hk-thumb", id: "hk-tt-{view}" }
                canvas { class: "hk-tcanvas", id: "hk-tc-{view}", tabindex: "0" }
                div { class: "hk-vscroll", id: "hk-tv-{view}",
                    div { class: "hk-vthumb", id: "hk-tk-{view}" }
                }
            }
            div { class: "hk-status",
                span { class: "hk-status-left", "{status.state}" }
                span { class: "hk-status-right", "{status.detail}" }
            }
        }
    }
}

//! The folder tree: a virtualised list (only the rows around the scroll position exist in the page), with
//! colour spots and names per side, the centre state mark, disclosure triangles and selection (FC-030…FC-032,
//! FC-060…FC-062).

use dioxus::prelude::*;

use crate::app::ViewModel;
use crate::commands::{CommandId, KeyOrigin};
use crate::folder_view::{Cell, FolderStatus, FolderWindow, ROW_HEIGHT, RowView};
use crate::ui::Ui;

fn with_folder(
    ui: Ui,
    view: u64,
    f: impl FnOnce(&mut crate::folder_view::FolderViewModel) -> bool,
) {
    let changed = {
        let mut app = ui.app;
        let mut core = app.write();
        match core
            .tab_mut(hikasa_session::ipc::ViewId(view))
            .map(|t| &mut t.model)
        {
            Some(ViewModel::Folder(m)) => f(m),
            _ => false,
        }
    };
    if changed {
        let mut rev = ui.rev;
        rev += 1;
    }
}

/// The Folder Compare pane.
#[component]
pub fn FolderPane(view: u64, window: FolderWindow, status: FolderStatus) -> Element {
    let ui = use_context::<Ui>();
    let height = window.total * ROW_HEIGHT;
    rsx! {
        div { class: "hk-folder",
            div { class: "hk-fbases",
                div { class: "hk-fbase", title: "{status.bases[0]}", "{status.bases[0]}" }
                div { class: "hk-fmark-head" }
                div { class: "hk-fbase", title: "{status.bases[1]}", "{status.bases[1]}" }
            }
            div { class: "hk-fcols",
                {columns()}
                div { class: "hk-fmark-head" }
                {columns()}
            }
            if let Some(error) = &status.error {
                div { class: "hk-error", "{error}" }
            }
            div {
                class: "hk-fscroll",
                id: "hk-fs-{view}",
                tabindex: "0",
                // The tree takes the focus when the view opens, so its keys work at once.
                onmounted: move |evt: MountedEvent| async move {
                    let _ = evt.set_focus(true).await;
                },
                onscroll: move |evt: ScrollEvent| {
                    let top = evt.scroll_top().max(0.0);
                    let visible = (evt.client_height().max(0) as u32) / ROW_HEIGHT + 1;
                    let first = (top / f64::from(ROW_HEIGHT)) as u32;
                    with_folder(ui, view, |m| m.set_scroll(first, visible));
                },
                onkeydown: move |evt: KeyboardEvent| {
                    let delta = match evt.key() {
                        Key::ArrowDown => 1,
                        Key::ArrowUp => -1,
                        Key::PageDown => 20,
                        Key::PageUp => -20,
                        _ => 0,
                    };
                    if delta == 0 {
                        // Enter, Left and Right with the tree focused: Open, Expand, Collapse.
                        crate::ui::key_command(ui, &evt, KeyOrigin::View);
                        return;
                    }
                    if evt.modifiers().is_empty() {
                        evt.prevent_default();
                        ui.act(|app| {
                            if let Some(tab) = app.tab_mut(hikasa_session::ipc::ViewId(view))
                                && let ViewModel::Folder(m) = &mut tab.model
                            {
                                m.move_selection(delta);
                            }
                            app.dirty_effects()
                        });
                    }
                },
                div { class: "hk-fspacer", style: "height: {height}px;",
                    for row in window.rows {
                        FolderRow { key: "{row.node}", view, row }
                    }
                }
            }
            div { class: "hk-status",
                span { class: "hk-status-left", "{status.state}" }
                span { class: "hk-status-right", "{status.detail}" }
            }
        }
    }
}

fn columns() -> Element {
    rsx! {
        div { class: "hk-fside hk-fhead",
            span { class: "hk-col-name", "Name" }
            span { class: "hk-col-size", "Size" }
            span { class: "hk-col-time", "Modified (UTC)" }
        }
    }
}

#[component]
fn FolderRow(view: u64, row: RowView) -> Element {
    let ui = use_context::<Ui>();
    let index = row.index as usize;
    let top = row.index * ROW_HEIGHT;
    let indent = row.depth * 16 + 4;
    let class = if row.selected {
        "hk-frow hk-selected"
    } else {
        "hk-frow"
    };
    let folder = row.folder;
    rsx! {
        div {
            class: "{class}",
            style: "top: {top}px; height: {ROW_HEIGHT}px;",
            title: "{row.errors}",
            onclick: move |_| {
                with_folder(ui, view, |m| {
                    m.select(index);
                    true
                });
            },
            ondoubleclick: move |_| {
                if folder {
                    with_folder(ui, view, |m| {
                        m.toggle(index);
                        true
                    });
                } else {
                    with_folder(ui, view, |m| {
                        m.select(index);
                        true
                    });
                    ui.command(CommandId::Open);
                }
            },
            {side(ui, view, &row, row.left.as_ref(), indent, index)}
            div { class: "hk-fmark", title: "{row.mark_title}", "{row.mark}" }
            {side(ui, view, &row, row.right.as_ref(), indent, index)}
        }
    }
}

fn side(
    ui: Ui,
    view: u64,
    row: &RowView,
    cell: Option<&Cell>,
    indent: u32,
    index: usize,
) -> Element {
    let disclosure = if !row.folder {
        ""
    } else if row.open {
        "▾"
    } else {
        "▸"
    };
    let Some(cell) = cell else {
        return rsx! { div { class: "hk-fside hk-absent" } };
    };
    let folder = row.folder;
    rsx! {
        div { class: "hk-fside",
            span { class: "hk-col-name", style: "padding-left: {indent}px;",
                span {
                    class: "hk-disc",
                    onclick: move |evt: MouseEvent| {
                        if folder {
                            evt.stop_propagation();
                            with_folder(ui, view, |m| {
                                m.toggle(index);
                                true
                            });
                        }
                    },
                    "{disclosure}"
                }
                span { class: "hk-spot {cell.colour}" }
                span { class: "hk-name {cell.colour}", "{cell.name}" }
            }
            span { class: "hk-col-size", "{cell.size}" }
            span { class: "hk-col-time", "{cell.modified}" }
        }
    }
}

//! Draws the status bar with its job indicator and the message panel of a view (D-115).

use egui::{Align, Button, Frame, Layout, Margin, ProgressBar, RichText, Ui};
use hikasa_session::ipc::ViewId;

use crate::status::{JobStatus, JobSummary, Message, MessageKind, StatusClick, StatusItems};
use crate::ui::hint::Hint as _;
use crate::ui::icons::{self, IconId};
use crate::ui::theme::Palette;
use crate::window::{CommandArg, Invocation};

/// What the status bar asked for.
pub enum StatusAction {
    /// Cancel the jobs of a view (a row of the job list).
    Cancel(ViewId),
    /// A clickable state item (the picture view's offset and zoom) asked for a command.
    Run(Invocation),
}

/// The id of the state item `k` of the status bar (tests find its place by it).
pub fn item_id(k: usize) -> egui::Id {
    egui::Id::unique(("hk-status-item", k))
}

/// Draws the status bar: the hint or the view's context text on the left, its state items and the job
/// indicator on the right.
pub fn bar(
    ui: &mut Ui,
    items: &StatusItems,
    hint: Option<&str>,
    jobs: &[JobStatus],
    pal: &Palette,
) -> Vec<StatusAction> {
    let mut out = Vec::new();
    let small = |t: &str| RichText::new(t).size(12.0).color(pal.muted);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !jobs.is_empty() {
                let summary = JobSummary::of(jobs);
                let label = format!(
                    "{} job{}",
                    summary.count,
                    if summary.count == 1 { "" } else { "s" }
                );
                let response = ui
                    .add(Button::new(RichText::new(label).size(12.0)).frame(false))
                    .hint(&summary.text);
                if let Some(f) = summary.fraction {
                    ui.add(ProgressBar::new(f).desired_width(80.0).desired_height(8.0));
                } else {
                    ui.spinner();
                }
                egui::Popup::menu(&response).show(|ui| {
                    ui.set_min_width(320.0);
                    for job in jobs {
                        ui.horizontal(|ui| {
                            let mut text = job.title.clone();
                            if let Some(p) = job.phase {
                                text.push_str(&format!(" — {}", p.as_str().replace('_', " ")));
                            }
                            if let Some(f) = job.fraction {
                                text.push_str(&format!(" ({:.0} %)", f * 100.0));
                            }
                            ui.label(text);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("Cancel").clicked() {
                                    out.push(StatusAction::Cancel(job.view));
                                }
                            });
                        });
                    }
                });
                ui.separator();
            }
            if items.clicks.iter().all(Option::is_none) {
                let right = items.right.join(" · ");
                ui.add(egui::Label::new(small(&right)).truncate());
            } else {
                // Right to left: the last item first. A clickable item is a frameless button.
                for (k, text) in items.right.iter().enumerate().rev() {
                    match items.clicks.get(k).copied().flatten() {
                        None => {
                            ui.label(small(text));
                        }
                        Some(click) => {
                            let response = ui
                                .push_id(item_id(k), |ui| {
                                    ui.add(Button::new(small(text)).frame(false))
                                })
                                .inner;
                            crate::ui::picture::mark(ui.ctx(), item_id(k), response.rect);
                            match click {
                                StatusClick::Run(id) if response.clicked() => {
                                    out.push(StatusAction::Run(Invocation::new(id)));
                                }
                                StatusClick::ZoomList => {
                                    egui::Popup::menu(&response).show(|ui| {
                                        for (i, choice) in
                                            crate::picture::ZoomChoice::LIST.into_iter().enumerate()
                                        {
                                            let item = ui.button(choice.caption());
                                            crate::ui::picture::mark(
                                                ui.ctx(),
                                                item_id(k).with(i),
                                                item.rect,
                                            );
                                            if item.clicked() {
                                                out.push(StatusAction::Run(Invocation::with(
                                                    crate::commands::CommandId::ZoomList,
                                                    CommandArg::ZoomStep(choice),
                                                )));
                                            }
                                        }
                                    });
                                }
                                StatusClick::Run(_) => {}
                            }
                        }
                    }
                    if k > 0 {
                        ui.label(small("·"));
                    }
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(small(hint.unwrap_or(&items.left))).truncate());
            });
        });
    });
    out
}

/// What the message panel asked for.
pub enum MessageAction {
    /// Close the panel.
    Dismiss,
    /// "Do not show again".
    DoNotShow,
}

/// Draws the message panel of a view.
pub fn message(ui: &mut Ui, m: &Message, pal: &Palette) -> Option<MessageAction> {
    let mut out = None;
    let (fill, colour, icon) = match m.kind {
        MessageKind::Info => (pal.chrome, pal.fg, IconId::Info),
        MessageKind::Warning => (pal.warning_bg, pal.warning, IconId::Warning),
        MessageKind::Error => (pal.error_bg, pal.error, IconId::Error),
    };
    Frame::NONE
        .fill(fill)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                icons::paint(ui.painter(), rect, icon, colour);
                ui.label(RichText::new(&m.text).color(pal.fg));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").hint("Close").clicked() {
                        out = Some(MessageAction::Dismiss);
                    }
                    if m.do_not_show.is_some() && ui.small_button("Do not show again").clicked() {
                        out = Some(MessageAction::DoNotShow);
                    }
                });
            });
        });
    out
}

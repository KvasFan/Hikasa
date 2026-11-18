//! The window's side of a Picture Compare view (M2 W13b, §1.13.9…§1.13.13): Info with *Show*, Rules (the Session
//! Settings dialog on its Picture tab), the Zoom list, the toolbar's Tolerance spinner and Blend Percentage slider,
//! Swap Sides (no decode), Open (the browse dialog for the current side), what the view asks of its window (message
//! panel lines, warnings, full screen) and what its dialogs leave when they close. Toolkit-free.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_session::ipc::ViewId;

use super::{AppCore, Effect, ViewModel};
use crate::browse::{BrowseMode, BrowseTarget};
use crate::commands::CommandId;
use crate::notify::{Notice, NoticeKind};
use crate::picture::info::PictureInfoDialog;
use crate::picture::{PictureRequest, PictureViewModel, ZoomChoice};
use crate::status::Message;
use crate::window::{CommandArg, Invocation, WindowId};

impl AppCore {
    /// The active Picture Compare view of a window.
    pub(super) fn active_picture(&self, window: WindowId) -> Option<ViewId> {
        let tab = self.active_tab(window)?;
        matches!(tab.model(), Some(ViewModel::Picture(_))).then_some(tab.id)
    }

    /// A picture view's model.
    pub(crate) fn picture_model(&mut self, view: ViewId) -> Option<&mut PictureViewModel> {
        match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Picture(m)) => Some(m),
            _ => None,
        }
    }

    /// The commands a picture view handles with the window (`None`: not one of them, or no picture view is active).
    pub(super) fn picture_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
    ) -> Option<Vec<Effect>> {
        let view = self.active_picture(window)?;
        let service = self.service.clone();
        match inv.id {
            CommandId::Info => {
                let response = self.picture_model(view)?.response(&service)?;
                let dialog = PictureInfoDialog::new(
                    view,
                    &response,
                    Arc::clone(&self.script_clipboard) as _,
                );
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::Rules => {
                self.edit_view_on(window, Some(crate::picture_tabs::PICTURE));
            }
            CommandId::ZoomList => {
                let choice = match inv.arg {
                    Some(CommandArg::ZoomStep(choice)) => choice,
                    _ => ZoomChoice::Fit,
                };
                self.picture_model(view)?.zoom_to(choice);
            }
            CommandId::Tolerance => {
                if let Some(CommandArg::Number(n)) = inv.arg {
                    let n = n.min(255) as u8;
                    self.picture_model(view)?.set_tolerance(&service, n);
                    self.mark_dirty(view);
                }
            }
            CommandId::BlendPercent => {
                if let Some(CommandArg::Number(n)) = inv.arg {
                    self.picture_model(view)?.set_blend(n.min(100) as u8);
                    self.mark_dirty(view);
                }
            }
            CommandId::Swap => {
                self.picture_model(view)?.swap(&service);
            }
            CommandId::FileFormat => {
                self.edit_view_on(window, Some(crate::picture_tabs::FORMAT));
            }
            // The current picture's file through the window's launcher (W9b's associated application, the file
            // manager, Open With's applications and Text Edit).
            CommandId::AssocApp | CommandId::ExplorerList | CommandId::OpenWithList => {
                let m = self.picture_model(view)?;
                let side = match m.focus {
                    hikasa_picture::Pane::Left => Side::Left,
                    _ => Side::Right,
                };
                let spec = m.loaded().map(|l| match side {
                    Side::Left => l.specs.left.clone(),
                    _ => l.specs.right.clone(),
                })?;
                match (inv.id, &inv.arg) {
                    (CommandId::OpenWithList, Some(CommandArg::OpenWith(item))) => match item {
                        crate::window::OpenWithItem::App(index) => {
                            let name = self
                                .open_with_names()
                                .get(*index)
                                .cloned()
                                .unwrap_or_default();
                            self.launch(crate::app::ops::Launch {
                                view,
                                caption: name.clone(),
                                what: crate::app::ops::LaunchWhat::App(*index, name),
                                items: vec![(spec.clone(), spec)],
                                refresh: None,
                            });
                        }
                        crate::window::OpenWithItem::TextEdit => {
                            self.open_text_edit(window, spec.canonical(), None);
                        }
                        _ => {}
                    },
                    (CommandId::OpenWithList, _) => {}
                    _ => return Some(self.launch_spec(window, view, spec, inv)),
                }
            }
            CommandId::Open | CommandId::OpenLocal => {
                let m = self.picture_model(view)?;
                let side = match m.focus {
                    hikasa_picture::Pane::Left => Side::Left,
                    _ => Side::Right,
                };
                let spec = match side {
                    Side::Left => m.request().left.to_string_lossy().into_owned(),
                    _ => m.request().right.to_string_lossy().into_owned(),
                };
                self.open_browse(
                    window,
                    BrowseMode::File,
                    crate::browse::folder_of(&spec),
                    BrowseTarget::TextOpen {
                        view,
                        side,
                        local: inv.id == CommandId::OpenLocal,
                    },
                );
            }
            // The rule words of the session: a change of the mode, the tolerance, Auto Scale or Ignore Unimportant
            // makes the session dirty (F8 never does, D-364).
            CommandId::DiffTolerance
            | CommandId::DiffRange
            | CommandId::DiffBlend
            | CommandId::AutoScale
            | CommandId::InsigIgnored => {
                self.picture_model(view)?.execute(&service, inv.id);
                self.mark_dirty(view);
            }
            id => {
                if !self.picture_model(view)?.execute(&service, id) {
                    return None;
                }
            }
        }
        Some(Vec::new())
    }

    /// The session of a view has changed rules (`[rules.picture]`).
    fn mark_dirty(&mut self, view: ViewId) {
        if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
            vs.dirty = true;
        }
    }

    /// What a picture view asked of its window: message panel lines, warning notices with their log entries, full
    /// screen (a viewport command), the browse dialog of a side.
    pub(super) fn picture_requests(
        &mut self,
        window: WindowId,
        view: ViewId,
        list: Vec<PictureRequest>,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        for request in list {
            match request {
                PictureRequest::Message(text) => self.set_message(view, Message::info(text)),
                PictureRequest::Warning(text) => {
                    let caption = self.tab(view).map(|t| t.caption()).unwrap_or_default();
                    self.log_entry(
                        hikasa_session::log::LogKind::Warning,
                        &caption,
                        text.clone(),
                    );
                    let notice =
                        Notice::new(format!("picture:{}", view.0), NoticeKind::Warning, text);
                    self.push_notice(window, notice);
                }
                PictureRequest::FullScreen(on) => effects.push(Effect::FullScreen(window, on)),
                PictureRequest::Browse(side) => {
                    let spec = self
                        .picture_model(view)
                        .map(|m| match side {
                            Side::Left => m.request().left.to_string_lossy().into_owned(),
                            _ => m.request().right.to_string_lossy().into_owned(),
                        })
                        .unwrap_or_default();
                    self.open_browse(
                        window,
                        BrowseMode::File,
                        crate::browse::folder_of(&spec),
                        BrowseTarget::TextOpen {
                            view,
                            side,
                            local: false,
                        },
                    );
                }
            }
        }
        effects
    }

    /// A picture dialog left the stack: Info's *Show* fits the view to the bounds. True when `closed` was one.
    pub(super) fn picture_dialog_closed(
        &mut self,
        closed: &mut Box<dyn crate::dialog::DialogModel>,
    ) -> bool {
        let Some(d) = closed.as_any_mut().downcast_mut::<PictureInfoDialog>() else {
            return false;
        };
        if d.show {
            let view = d.view;
            if let Some(m) = self.picture_model(view) {
                m.show_bounds();
            }
        }
        true
    }

    /// The browse answer of Open in a picture view: that side reloads with the file. True when `view` is one.
    pub(super) fn picture_browsed(&mut self, view: ViewId, side: Side, path: String) -> bool {
        let service = self.service.clone();
        match self.picture_model(view) {
            Some(m) => {
                m.set_spec(&service, side, path.into());
                true
            }
            None => false,
        }
    }
}

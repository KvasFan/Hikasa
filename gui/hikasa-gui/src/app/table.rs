//! The window's side of a Table Compare view (m2-table-contract §6.6): Copy of a cell or a row through the window's
//! own clipboard, Rules (the Session Settings dialog on its Sheets tab), Info, Go To, and what their dialogs leave
//! when they close (the report dialog is every view's since M2 W13c, `app/report.rs`).

use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_session::clipboard::ClipboardPayload;
use hikasa_session::ipc::ViewId;

use super::pane::PaneDone;
use super::{AppCore, CoreJob, Effect, ViewModel};
use crate::commands::CommandId;
use crate::job::Job;
use crate::status::Message;
use crate::table_dialogs::GoToDialog;
use crate::window::{Invocation, Tab, WindowId};

impl AppCore {
    /// The active Table Compare view of a window.
    pub(super) fn active_table(&self, window: WindowId) -> Option<ViewId> {
        let tab = self.active_tab(window)?;
        matches!(tab.model(), Some(ViewModel::Table(_))).then_some(tab.id)
    }

    /// The commands a table view handles with the window (`None`: not one of them, or no table view is active).
    pub(super) fn table_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
    ) -> Option<Vec<Effect>> {
        let view = self.active_table(window)?;
        match inv.id {
            CommandId::Copy => self.copy_cell(window, view),
            CommandId::Rules => self.edit_view_on(window, Some(crate::table_tabs::SHEETS)),
            CommandId::Info => {
                let sections = match self.tab(view).and_then(Tab::model) {
                    Some(ViewModel::Table(m)) => m.info_sections(),
                    _ => return Some(Vec::new()),
                };
                let dialog = crate::info_dialog::InfoDialog::table(
                    crate::commands::Scope::Table.type_title(),
                    sections,
                    Arc::clone(&self.script_clipboard) as _,
                );
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::GoTo => {
                let (sheets, active) = match self.tab(view).and_then(Tab::model) {
                    Some(ViewModel::Table(m)) => (m.sheet_names(), m.active_sheet()),
                    _ => return Some(Vec::new()),
                };
                self.push_dialog(window, Box::new(GoToDialog::new(view, sheets, active)));
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// Copy (TB-023's copy): the current cell's text, or the row after a click on its number, on the window's own
    /// clipboard (D-252), in turn with the other writes.
    fn copy_cell(&mut self, window: WindowId, view: ViewId) {
        let text = match self.tab(view).and_then(Tab::model) {
            Some(ViewModel::Table(m)) => m.copy_text(),
            _ => None,
        };
        let Some(text) = text else {
            self.set_message(
                view,
                Message::info("Click a cell or a row number to copy it."),
            );
            return;
        };
        let clipboard = Arc::clone(&self.script_clipboard);
        let ticket = clipboard.ticket();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy the cell".into(),
            &self.wake(),
            move |_, _| {
                clipboard
                    .in_turn(ticket, |c| {
                        c.write_payload(&ClipboardPayload { text, html: None })
                    })
                    .map_err(|e| hikasa_session::ApiError::Vfs(e.to_vfs_write()))?;
                Ok(PaneDone::Named)
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
    }

    /// A table dialog left the stack: Go To moves the view's cursor. True when `closed` was one.
    pub(super) fn table_dialog_closed(
        &mut self,
        closed: &mut Box<dyn crate::dialog::DialogModel>,
    ) -> bool {
        if let Some(d) = closed.as_any_mut().downcast_mut::<GoToDialog>() {
            let view = d.view;
            if let Some(target) = d.take_chosen() {
                let found = match self.tab_mut(view).and_then(|t| t.model_mut()) {
                    Some(ViewModel::Table(m)) => {
                        m.set_sheet(target.sheet);
                        let col = m.column_named(&target.column);
                        if !target.column.is_empty() && col.is_none() {
                            Err(format!("No column is named `{}`.", target.column))
                        } else if m.go_to(None, target.row, col) {
                            Ok(())
                        } else {
                            Err(format!("No row {} is shown on this sheet.", target.row))
                        }
                    }
                    _ => Ok(()),
                };
                if let Err(message) = found {
                    self.set_message(view, Message::warning(message));
                }
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
impl AppCore {
    /// [`AppCore::capture`] for the tests of other modules.
    pub fn capture_for_test(tab: &Tab) -> Option<crate::view_session::ViewCapture> {
        AppCore::capture(tab)
    }
}

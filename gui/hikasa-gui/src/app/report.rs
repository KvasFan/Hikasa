//! The window's side of the report dialog (M2 W13c, D-366): *<Type> Report…* in every view whose type has a report
//! kind builds a [`ReportDialog`] from the active view (its comparison, its display and state, its selection), the
//! remembered choices (`report_choices`, the table's phase-3 keys once) and the home folder; a Text Compare view
//! with edits not compared yet hands the dialog its comparison once the recompare is installed.

use std::sync::Arc;

use hikasa_report::ReportKind;
use hikasa_report::vocabulary::{
    folder_view_words, hex_view_words, picture_view_words, table_view_words, text_view_words,
};
use hikasa_session::api::lines_flag;

use super::{AppCore, Effect, ViewModel};
use crate::commands::CommandId;
use crate::report_dialog::{OwnedReportSource, ReportDialog, ReportInit, ScopeSource};
use crate::status::Message;
use crate::table_dialogs::REPORT_DIALOG;
use crate::window::{Invocation, Tab, WindowId};

impl AppCore {
    /// *<Type> Report…* (`CommandId::Report`) of the active view; `None` for another command.
    pub(super) fn report_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
    ) -> Option<Vec<Effect>> {
        if inv.id != CommandId::Report {
            return None;
        }
        let service = self.service.clone();
        let tab = self.active_tab_mut(window)?;
        let view = tab.id;
        let built = match tab.model_mut() {
            Some(ViewModel::Text(m)) if !m.is_edit() => {
                let ready = m.report_ready();
                m.live_view().map(|live| {
                    let response = ready.then(|| service.text_report_source(live));
                    let opts = m.view_options();
                    (
                        OwnedReportSource::Text(response),
                        text_view_words(opts.filter),
                        opts.ignore_unimportant,
                        m.report_lines()
                            .map(|scope| ScopeSource::Lines(lines_flag(&scope))),
                    )
                })
            }
            Some(ViewModel::Folder(m)) if !m.scanning() => {
                let display = *m.display_options();
                let scope = m.selection_request(false);
                m.prepared().map(|prepared| {
                    (
                        OwnedReportSource::Folder {
                            request: Box::new(m.request().clone()),
                            prepared: Box::new(prepared.clone()),
                            ignore_unimportant: display.ignore_unimportant,
                        },
                        folder_view_words(&display),
                        display.ignore_unimportant,
                        scope.map(|(_, mut select)| {
                            select.include_hidden = None;
                            ScopeSource::Rows(Box::new(select))
                        }),
                    )
                })
            }
            Some(ViewModel::Table(m)) => m.report_response().map(|r| {
                let iu = r.ignore_unimportant;
                (
                    OwnedReportSource::Table(Box::new(r)),
                    table_view_words(m.filter()),
                    iu,
                    None,
                )
            }),
            Some(ViewModel::Hex(m)) => m.response().cloned().map(|r| {
                let d = m.display();
                (
                    OwnedReportSource::Hex(Box::new(r)),
                    hex_view_words(Some(d.filter), d.context, d.addresses),
                    false,
                    None,
                )
            }),
            Some(ViewModel::Picture(m)) => m.response(&service).map(|r| {
                (
                    OwnedReportSource::Picture(Box::new(r)),
                    picture_view_words(m.mode, m.checkerboard),
                    m.ignore_unimportant,
                    None,
                )
            }),
            _ => None,
        };
        let Some((source, words, iu, scope)) = built else {
            self.set_message(
                view,
                Message::info("The report is available once the comparison is complete."),
            );
            return Some(Vec::new());
        };
        let kind = source.kind();
        let choices = service.report_choices(kind);
        // The table's phase-3 keys (`html`, `folder`) are read once; the next remember replaces the table.
        let legacy = if kind == ReportKind::Table {
            let text = |key| {
                self.state
                    .dialog_text(REPORT_DIALOG, key)
                    .map(str::to_owned)
            };
            (
                text("html").map(|w| w == true.to_string()),
                text("folder").map(std::path::PathBuf::from),
            )
        } else {
            (None, None)
        };
        let dialog = ReportDialog::new(ReportInit {
            view,
            source,
            view_words: words,
            ignore_unimportant: iu,
            scope,
            choices,
            legacy,
            home: service.env().home.clone(),
            wake: self.wake(),
            clipboard: Arc::clone(&self.script_clipboard),
        });
        self.push_dialog(window, Box::new(dialog));
        Some(Vec::new())
    }

    /// Hands each report dialog that waits for a Text Compare view's comparison the comparison once the view's
    /// recompare is installed (polled every frame before the dialogs).
    pub(super) fn report_sources(&mut self) {
        let service = self.service.clone();
        let wanted: Vec<_> = self
            .windows
            .iter_mut()
            .enumerate()
            .filter_map(|(w, window)| {
                window
                    .dialogs
                    .find_mut::<ReportDialog>()
                    .filter(|d| d.wants_text_source())
                    .map(|d| (w, d.view))
            })
            .collect();
        for (w, view) in wanted {
            let source = match self.tab_mut(view).and_then(Tab::model_mut) {
                Some(ViewModel::Text(m)) => {
                    if m.report_ready() {
                        m.live_view().map(|live| service.text_report_source(live))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(source) = source
                && let Some(d) = self.windows[w].dialogs.find_mut::<ReportDialog>()
            {
                d.set_text_source(source);
            }
        }
    }
}

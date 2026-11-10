//! The dialogs of a Table Compare view (TB-022, TB-025; m2-table-contract §6.6): *Go To* (a row label, a column and
//! a sheet; Ctrl+G) and *Table Compare Report* (File ▸ Report, T-48: a report of the view's comparison through the
//! CLI's report path, `Service::report_request` with `ReportKind::Table`, `report_plan` and `report_execute`, on a
//! job). Toolkit-free models; `ui/table_dialogs.rs` draws them, the app core acts on them when they close.

use std::any::Any;
use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_report::ReportKind;
use hikasa_session::api::{ReportFlags, ReportSource, ReportTarget, TableCompareResponse};
use hikasa_session::ipc::ViewId;
use hikasa_session::{ApiError, Service};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};
use crate::job::Job;

/// The help topic of the Go To dialog.
pub const GOTO_TOPIC: &str = "dialog-table-goto";
/// The help topic of the report dialog.
pub const REPORT_TOPIC: &str = "dialog-table-report";
/// `state.toml [dialogs."report-table"]`, the remembered choices of the report dialog (T-38's key).
pub const REPORT_DIALOG: &str = "report-table";

fn button(label: &str, role: ButtonRole, enabled: bool, default: bool) -> DialogButton {
    DialogButton {
        label: label.to_owned(),
        role,
        enabled,
        default,
    }
}

/// What Go To asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoToTarget {
    /// The sheet pair.
    pub sheet: usize,
    /// The row label (as the row numbers show it, either side).
    pub row: u64,
    /// The column, a name or letters (`""`: keep the column).
    pub column: String,
}

/// *Go To…* (TB-022): row, column and sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoToDialog {
    /// The view.
    pub view: ViewId,
    /// The sheet pairs offered.
    pub sheets: Vec<String>,
    /// The sheet chosen.
    pub sheet: usize,
    /// The row label as typed.
    pub row: String,
    /// The column as typed.
    pub column: String,
    /// Why OK did not close.
    pub error: Option<String>,
    chosen: Option<GoToTarget>,
}

impl GoToDialog {
    /// The dialog over a view's sheets, the active one chosen.
    pub fn new(view: ViewId, sheets: Vec<String>, sheet: usize) -> GoToDialog {
        GoToDialog {
            view,
            sheets,
            sheet,
            row: String::new(),
            column: String::new(),
            error: None,
            chosen: None,
        }
    }

    /// The target after OK (once).
    pub fn take_chosen(&mut self) -> Option<GoToTarget> {
        self.chosen.take()
    }

    /// The fields as a target: `row` a positive number, or `row:column` in the row field (`12:B`).
    fn target(&self) -> Result<GoToTarget, String> {
        let (row, column) = match self.row.split_once(':') {
            Some((r, c)) if self.column.trim().is_empty() => (r, c),
            _ => (self.row.as_str(), self.column.as_str()),
        };
        let row = row
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("`{}` is not a row number.", row.trim()))?;
        Ok(GoToTarget {
            sheet: self.sheet.min(self.sheets.len().saturating_sub(1)),
            row,
            column: column.trim().to_owned(),
        })
    }
}

impl DialogModel for GoToDialog {
    fn title(&self) -> String {
        "Go To".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        GOTO_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            button("OK", ButtonRole::Ok, !self.row.trim().is_empty(), true),
            button("Cancel", ButtonRole::Cancel, true, false),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        match self.target() {
            Ok(target) => {
                self.chosen = Some(target);
                DialogStep::Close
            }
            Err(e) => {
                self.error = Some(e);
                DialogStep::Stay
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The layouts of a table report (T-38) with their captions.
pub const LAYOUTS: [(&str, &str); 5] = [
    ("side-by-side", "Side by side"),
    ("interleaved", "Interleaved"),
    ("summary", "Summary"),
    ("xml", "XML"),
    ("json", "JSON"),
];

/// The display words of a report (TB-021).
pub const DISPLAYS: [(&str, &str); 4] = [
    ("all", "All rows"),
    ("differences", "Differences"),
    ("same", "Same rows"),
    ("none", "No rows"),
];

/// Where the report goes: a file or the clipboard (never the printer or the browser from here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTo {
    /// The file of the path field.
    File,
    /// The clipboard.
    Clipboard,
}

/// *Table Compare Report…* (File ▸ Report, T-48).
pub struct TableReportDialog {
    response: TableCompareResponse,
    /// The layout word.
    pub layout: &'static str,
    /// HTML (else plain text; `xml` and `json` ignore it).
    pub html: bool,
    /// The display word.
    pub display: &'static str,
    /// Row numbers.
    pub row_numbers: bool,
    /// The title (`""`: the default).
    pub title: String,
    /// The target.
    pub to: ReportTo,
    /// The file path as typed.
    pub path: String,
    /// The last answer (written, or why not).
    pub message: Option<(bool, String)>,
    job: Option<Job<(u64, bool)>>,
    wake: crate::job::Wake,
    clipboard: Arc<hikasa_session::clipboard::OrderedClipboard>,
}

impl TableReportDialog {
    /// The dialog over a view's comparison, with the remembered layout and display.
    pub fn new(
        response: TableCompareResponse,
        remembered: (Option<String>, Option<String>, Option<bool>),
        wake: crate::job::Wake,
        clipboard: Arc<hikasa_session::clipboard::OrderedClipboard>,
    ) -> TableReportDialog {
        let pick = |list: &[(&'static str, &str)], word: Option<String>, default: &'static str| {
            list.iter()
                .map(|(w, _)| *w)
                .find(|w| word.as_deref() == Some(*w))
                .unwrap_or(default)
        };
        let (layout, display, html) = remembered;
        TableReportDialog {
            response,
            layout: pick(&LAYOUTS, layout, "side-by-side"),
            html: html.unwrap_or(true),
            display: pick(&DISPLAYS, display, "differences"),
            row_numbers: true,
            title: String::new(),
            to: ReportTo::File,
            path: String::new(),
            message: None,
            job: None,
            wake,
            clipboard,
        }
    }

    /// True while the report is written.
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// The flags of the fields, as the CLI's REPORT flags would give them (the Service parses the words, T11).
    pub fn flags(&self) -> ReportFlags {
        let mut flags = ReportFlags::default();
        flags.words.layout = Some(self.layout.to_owned());
        if !self.title.trim().is_empty() {
            flags.words.title = Some(self.title.trim().to_owned());
        }
        flags.html = (self.html && !matches!(self.layout, "xml" | "json")).then_some(false);
        flags.display = Some(self.display.to_owned());
        flags.line_numbers = Some(self.row_numbers);
        flags
    }

    /// The target of the fields.
    pub fn target(&self) -> Result<ReportTarget, String> {
        match self.to {
            ReportTo::Clipboard => Ok(ReportTarget::Clipboard),
            ReportTo::File if self.path.trim().is_empty() => {
                Err("Name the file the report is written to.".to_owned())
            }
            // The path is a file spec as typed: `-`, `printer` and `browser` are files here.
            ReportTo::File => Ok(ReportTarget::File(OsString::from(self.path.trim()))),
        }
    }

    /// The words remembered for the next report: layout, display, HTML.
    pub fn remembered(&self) -> [(&'static str, String); 3] {
        [
            ("layout", self.layout.to_owned()),
            ("display", self.display.to_owned()),
            ("html", self.html.to_string()),
        ]
    }

    fn start(&mut self, svc: &Service) {
        let target = match self.target() {
            Ok(t) => t,
            Err(e) => {
                self.message = Some((false, e));
                return;
            }
        };
        let flags = self.flags();
        let response = self.response.clone();
        let clipboard = Arc::clone(&self.clipboard);
        let ticket = clipboard.ticket();
        self.message = None;
        self.job = Some(Job::spawn(
            svc,
            JobKind::Report,
            "write the table report".into(),
            &self.wake,
            move |svc, job| {
                let mut req =
                    svc.report_request(ReportKind::Table, "side-by-side", &flags, &target)?;
                let source = ReportSource::Table(&response);
                let plan = svc.report_plan(source, &mut req, &target, None, job)?;
                let outcome = svc.report_execute(source, &req, plan, None, job)?;
                if outcome.cancelled {
                    return Err(ApiError::Cancelled);
                }
                // The adapter places a clipboard report (architecture check 2), in the window's order of writes.
                let copied = match &outcome.clipboard {
                    Some(payload) => {
                        clipboard
                            .in_turn(ticket, |c| c.write_payload(payload))
                            .map_err(|e| ApiError::Vfs(e.to_vfs_write()))?;
                        true
                    }
                    None => false,
                };
                Ok((outcome.bytes, copied))
            },
        ));
    }
}

impl DialogModel for TableReportDialog {
    fn title(&self) -> String {
        "Table Compare Report".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        REPORT_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            button("Write Report", ButtonRole::Ok, !self.running(), true),
            button(
                if self.running() { "Stop" } else { "Close" },
                ButtonRole::Close,
                true,
                false,
            ),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => {
                self.start(svc);
                DialogStep::Stay
            }
            _ if self.running() => {
                if let Some(job) = &self.job {
                    job.cancel();
                }
                DialogStep::Stay
            }
            _ => DialogStep::Close,
        }
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        self.message = Some(match result {
            Ok((bytes, false)) => (true, format!("The report was written ({bytes} bytes).")),
            Ok((_, true)) => (true, "The report is on the clipboard.".to_owned()),
            Err(ApiError::Cancelled) => (false, "The report was stopped.".to_owned()),
            Err(e) => (false, e.to_string()),
        });
        true
    }

    fn busy(&self) -> bool {
        self.running()
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((520.0, 260.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_to_reads_row_and_column() {
        let mut d = GoToDialog::new(ViewId(1), vec!["a".into(), "b".into()], 1);
        d.row = "12:B".into();
        assert_eq!(
            d.target(),
            Ok(GoToTarget {
                sheet: 1,
                row: 12,
                column: "B".into()
            })
        );
        d.row = " 7 ".into();
        d.column = "price".into();
        assert_eq!(d.target().unwrap().column, "price");
        for bad in ["0", "x", "-3", ""] {
            d.row = bad.into();
            assert!(d.target().is_err(), "{bad}");
        }
    }
}

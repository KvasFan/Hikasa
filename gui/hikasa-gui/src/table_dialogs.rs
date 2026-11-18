//! The dialogs of a Table Compare view (TB-022; m2-table-contract §6.6): *Go To* (a row label, a column and a sheet;
//! Ctrl+G). *Table Compare Report* (T-56) is the report dialog of every view kind since M2 W13c (D-366,
//! `report_dialog.rs`). Toolkit-free models; `ui/table_dialogs.rs` draws them, the app core acts on them when they
//! close.

use std::any::Any;

use hikasa_session::Service;
use hikasa_session::ipc::ViewId;

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};

/// The help topic of the Go To dialog.
pub const GOTO_TOPIC: &str = "dialog-table-goto";
/// `state.toml [dialogs."report-table"]`, the remembered choices of the table's report dialog (T-38's key): its
/// phase-3 keys `html` and `folder` are read once by the report dialog (D-366's migration).
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

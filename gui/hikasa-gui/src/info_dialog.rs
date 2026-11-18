//! The Info dialog (M2 W9a, FC-064; D-242): the statistics of a folder view's comparison, read once when it opens
//! (`TreeRead::stats`, O(1)): file and folder pairs per category, per side files, folders and bytes, the content
//! results, Ignored. *Copy* places the table as text on the clipboard. While the comparison runs the figures are the
//! partial ones and the dialog says so. Toolkit-free; `ui/dialog.rs` draws the table.

use std::any::Any;
use std::sync::Arc;

use hikasa_core::ContentKind;
use hikasa_folder::{Category, FolderStats};
use hikasa_session::Service;
use hikasa_session::clipboard::{ClipboardPayload, ClipboardProvider};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};
use crate::folder::glyphs::{content_mark, state_mark};

/// The help topic of the dialog.
pub const INFO_TOPIC: &str = "dialog-folder-info";

/// The help topic of a Table Compare view's Info.
pub const TABLE_INFO_TOPIC: &str = "dialog-table-info";

/// The Copy button.
pub const COPY: ButtonRole = ButtonRole::Other(0);

/// One line of the table: its label and the figures (one column, or left and right).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoRow {
    /// The label.
    pub label: String,
    /// The figures.
    pub values: Vec<String>,
}

/// A titled part of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoSection {
    /// The title.
    pub title: String,
    /// The column heads (empty: one unnamed column).
    pub heads: Vec<String>,
    /// The lines.
    pub rows: Vec<InfoRow>,
}

/// The Info dialog.
pub struct InfoDialog {
    title: String,
    stats: FolderStats,
    partial: bool,
    clipboard: Arc<dyn ClipboardProvider>,
    /// The answer of the last Copy.
    pub message: Option<String>,
    /// A Folder Sync view's plan counts.
    sync: Option<hikasa_folder::sync::SyncCounts>,
    /// The table of another view type (a Table Compare view's, TB-024), shown instead of the folder figures.
    table: Option<(Vec<InfoSection>, &'static str)>,
    /// What Copy places instead of the table's text (a Hex Compare view's: the core's summary and verdict, D-360).
    copy_text: Option<String>,
}

impl InfoDialog {
    /// The dialog of `stats` for a view of type `type_title` ("Folder Compare"); `partial` while the comparison
    /// runs.
    pub fn new(
        type_title: &str,
        stats: FolderStats,
        partial: bool,
        clipboard: Arc<dyn ClipboardProvider>,
    ) -> InfoDialog {
        InfoDialog {
            title: format!("{type_title} Info"),
            stats,
            partial,
            clipboard,
            message: None,
            sync: None,
            table: None,
            copy_text: None,
        }
    }

    /// The Info dialog of a Table Compare view (TB-024): the view's own sections (the statistics per sheet).
    pub fn table(
        type_title: &str,
        sections: Vec<InfoSection>,
        clipboard: Arc<dyn ClipboardProvider>,
    ) -> InfoDialog {
        InfoDialog::with_sections(type_title, sections, TABLE_INFO_TOPIC, clipboard)
    }

    /// The Info dialog of a view with its own table and help topic (a text view's, M2 W11b, D-315).
    pub fn with_sections(
        type_title: &str,
        sections: Vec<InfoSection>,
        topic: &'static str,
        clipboard: Arc<dyn ClipboardProvider>,
    ) -> InfoDialog {
        InfoDialog {
            table: Some((sections, topic)),
            ..InfoDialog::new(type_title, FolderStats::default(), false, clipboard)
        }
    }

    /// The dialog whose Copy places `text` (a Hex Compare view's summary and verdict, M2 W13a).
    pub fn with_copy_text(mut self, text: String) -> InfoDialog {
        self.copy_text = Some(text);
        self
    }

    /// A Folder Sync view's Info adds the counts of its plan (D-242, FS-005).
    pub fn with_sync(mut self, counts: Option<hikasa_folder::sync::SyncCounts>) -> InfoDialog {
        self.sync = counts;
        self
    }

    /// The figures the dialog shows.
    #[cfg(test)]
    pub fn stats(&self) -> &FolderStats {
        &self.stats
    }

    /// True when the comparison was still running when the dialog opened.
    #[cfg(test)]
    pub fn partial(&self) -> bool {
        self.partial
    }

    /// The note above the table while the figures are partial.
    pub fn note(&self) -> Option<&'static str> {
        self.partial
            .then_some("The comparison is still running: these figures are partial.")
    }

    /// The table.
    pub fn sections(&self) -> Vec<InfoSection> {
        if let Some((sections, _)) = &self.table {
            return sections.clone();
        }
        let s = &self.stats;
        let n = |v: u64| v.to_string();
        let pairs = Category::ALL
            .into_iter()
            .map(|c| InfoRow {
                label: state_mark(c).1.to_owned(),
                values: vec![n(u64::from(s.files.get(c))), n(u64::from(s.folders.get(c)))],
            })
            .collect();
        let sides = vec![
            InfoRow {
                label: "Files".to_owned(),
                values: vec![n(s.left.files), n(s.right.files)],
            },
            InfoRow {
                label: "Folders".to_owned(),
                values: vec![n(s.left.folders), n(s.right.folders)],
            },
            InfoRow {
                label: "Bytes".to_owned(),
                values: vec![n(s.left.bytes), n(s.right.bytes)],
            },
        ];
        let mut content: Vec<InfoRow> = ContentKind::ALL
            .into_iter()
            .map(|k| InfoRow {
                label: content_mark(k).1.to_owned(),
                values: vec![n(u64::from(s.content.get(k)))],
            })
            .collect();
        content.push(InfoRow {
            label: "Not compared (the quick tests sufficed)".to_owned(),
            values: vec![n(u64::from(s.content.not_compared))],
        });
        content.push(InfoRow {
            label: "Ignored".to_owned(),
            values: vec![n(u64::from(s.ignored))],
        });
        let mut sections = vec![
            InfoSection {
                title: "Pairs".to_owned(),
                heads: vec!["Files".to_owned(), "Folders".to_owned()],
                rows: pairs,
            },
            InfoSection {
                title: "Sides".to_owned(),
                heads: vec!["Left".to_owned(), "Right".to_owned()],
                rows: sides,
            },
            InfoSection {
                title: "Content comparisons".to_owned(),
                heads: Vec::new(),
                rows: content,
            },
        ];
        if let Some(c) = &self.sync {
            let row = |label: &str, v: u64| InfoRow {
                label: label.to_owned(),
                values: vec![n(v)],
            };
            sections.push(InfoSection {
                title: "Sync plan".to_owned(),
                heads: Vec::new(),
                rows: vec![
                    row("Copies to the right", c.copy_right),
                    row("Copies to the left", c.copy_left),
                    row("Deletes on the left", c.delete_left),
                    row("Deletes on the right", c.delete_right),
                    row("Left alone", c.leave),
                    row("Conflicts", c.conflicts),
                    row("Unresolved", c.unresolved),
                ],
            });
        }
        sections
    }

    /// The table as text (what Copy places): a title line, the note, then one line per row, the figures
    /// right-aligned in columns.
    pub fn text(&self) -> String {
        let sections = self.sections();
        let label_w = sections
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.label.chars().count()))
            .max()
            .unwrap_or(0);
        let value_w = sections
            .iter()
            .flat_map(|s| {
                s.rows
                    .iter()
                    .flat_map(|r| r.values.iter().map(String::len))
                    .chain(s.heads.iter().map(|h| h.chars().count()))
            })
            .max()
            .unwrap_or(0);
        let mut out = format!("{}\n", self.title);
        if let Some(note) = self.note() {
            out.push_str(note);
            out.push('\n');
        }
        for section in sections {
            out.push('\n');
            out.push_str(&format!("{:label_w$}", section.title));
            for head in &section.heads {
                out.push_str(&format!("  {head:>value_w$}"));
            }
            out.push('\n');
            for row in section.rows {
                out.push_str(&format!("{:label_w$}", row.label));
                for value in &row.values {
                    out.push_str(&format!("  {value:>value_w$}"));
                }
                out.push('\n');
            }
        }
        out
    }
}

impl DialogModel for InfoDialog {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn help_topic(&self) -> &'static str {
        self.table.as_ref().map_or(INFO_TOPIC, |(_, topic)| topic)
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "Copy".to_owned(),
                role: COPY,
                enabled: true,
                default: false,
            },
            DialogButton {
                label: "Close".to_owned(),
                role: ButtonRole::Close,
                enabled: true,
                default: true,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if role != COPY {
            return DialogStep::Close;
        }
        let payload = ClipboardPayload {
            text: self.copy_text.clone().unwrap_or_else(|| self.text()),
            html: None,
        };
        self.message = Some(match self.clipboard.write_payload(&payload) {
            Ok(()) => "Copied to the clipboard.".to_owned(),
            Err(e) => format!("Could not copy: {e}"),
        });
        DialogStep::Stay
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

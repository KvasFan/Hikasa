//! Info, Clear Session and changes on disk of a text view (D-118, FF-004; M2 W11b, contract §1.11.24, D-315),
//! toolkit-free. Info's table is read from the last install (and says *provisional* while edits wait): per side the
//! lines, the encoding with its BOM, the line endings, the format and its reason, the size, the editability and
//! *Modified*; the comparison's statistics, the engine used, the times and the last compare's warnings. A change on
//! disk of an input with edits is a notice with *Reload* and *Keep mine*, never a reload; a change of a format the view
//! uses offers *Reopen*.

use hikasa_core::Side;
use hikasa_text::edit::EditableDocument;

use super::header::{eol_text, format_text};
use super::{TextViewModel, side_of};
use crate::info_dialog::{InfoRow, InfoSection};

/// The help topic of the text views' Info.
pub const TEXT_INFO_TOPIC: &str = "dialog-text-info";

fn row(label: &str, values: Vec<String>) -> InfoRow {
    InfoRow {
        label: label.to_owned(),
        values,
    }
}

/// The line endings' counts of a document (`12 LF, 1 CRLF`).
fn endings(doc: &EditableDocument) -> String {
    let e = doc.document().eol();
    let mut parts = Vec::new();
    if e.lf > 0 {
        parts.push(format!("{} LF", e.lf));
    }
    if e.crlf > 0 {
        parts.push(format!("{} CRLF", e.crlf));
    }
    if e.cr > 0 {
        parts.push(format!("{} CR", e.cr));
    }
    if e.none > 0 {
        parts.push(format!("{} without a terminator", e.none));
    }
    if parts.is_empty() {
        parts.push("no line".to_owned());
    }
    format!("{} ({})", eol_text(doc), parts.join(", "))
}

/// The Info table of a view (TC-035's figures, §1.11.24).
pub fn sections(view: &TextViewModel) -> Vec<InfoSection> {
    let edit = view.is_edit();
    let sides: Vec<Side> = if edit {
        vec![Side::Left]
    } else {
        vec![Side::Left, Side::Right]
    };
    let heads: Vec<String> = if edit {
        Vec::new()
    } else {
        vec!["Left".to_owned(), "Right".to_owned()]
    };
    let per = |f: &dyn Fn(Side, &EditableDocument) -> String| -> Vec<String> {
        sides
            .iter()
            .map(|s| view.doc(*s).map_or_else(String::new, |d| f(*s, d)))
            .collect()
    };
    let sizes: Vec<String> = sides
        .iter()
        .map(|s| {
            let stamp = match (view.live_view(), view.edit_response()) {
                (Some(l), _) => l.stamps.get(*s).copied().flatten(),
                (None, Some(r)) => r.stamp,
                _ => None,
            };
            stamp.map_or_else(|| "—".to_owned(), |st| format!("{} bytes", st.size))
        })
        .collect();
    let files = InfoSection {
        title: if edit { "File" } else { "Files" }.to_owned(),
        heads,
        rows: vec![
            row(
                "Name",
                sides
                    .iter()
                    .map(|s| super::header::title(view, *s))
                    .collect(),
            ),
            row("Lines", per(&|_, d| d.line_count().to_string())),
            row("Encoding", per(&|_, d| super::header::encoding_text(d))),
            row("Line endings", per(&|_, d| endings(d))),
            row("Format", per(&|s, _| format_text(view, s))),
            row("Size on disk", sizes),
            row(
                "Editing",
                per(&|_, d| {
                    let e = d.editability();
                    if e.editable() {
                        "Editable".to_owned()
                    } else {
                        super::header::block_message(&e)
                    }
                }),
            ),
            row(
                "Modified",
                per(&|_, d| if d.is_modified() { "Yes" } else { "No" }.to_owned()),
            ),
        ],
    };
    let mut out = vec![files];
    if let Some(cmp) = view.comparison() {
        let s = cmp.stats();
        let n = |v: u64| v.to_string();
        let provisional = cmp.provisional();
        let mut rows = vec![
            row("Rows", vec![n(s.rows)]),
            row("Same rows", vec![n(s.same_rows)]),
            row("Changed rows", vec![n(s.changed_rows)]),
            row(
                "Orphan rows (left / right)",
                vec![format!("{} / {}", s.left_only_rows, s.right_only_rows)],
            ),
            row(
                "Rows by importance (important / unimportant)",
                vec![format!("{} / {}", s.important_rows, s.unimportant_rows)],
            ),
            row(
                "Sections (important / all)",
                vec![format!("{} / {}", s.important_sections, s.sections)],
            ),
            row("Ignored rows", vec![n(s.ignored_rows)]),
            row("Engine", vec![format!("{:?}", cmp.algorithm())]),
        ];
        if let Some(t) = view.timings_text() {
            rows.push(row("Times", vec![t]));
        }
        if provisional {
            rows.push(row(
                "State",
                vec!["provisional: edits wait for the comparison".to_owned()],
            ));
        }
        out.push(InfoSection {
            title: "Comparison".to_owned(),
            heads: Vec::new(),
            rows,
        });
        if let Some(r) = view.response()
            && !r.warnings.is_empty()
        {
            out.push(InfoSection {
                title: "Warnings of the last compare".to_owned(),
                heads: Vec::new(),
                rows: r
                    .warnings
                    .iter()
                    .map(|w| row(w.code.as_str(), vec![w.message.clone()]))
                    .collect(),
            });
        }
    } else if let Some(t) = view.timings().0 {
        out.push(InfoSection {
            title: "Load".to_owned(),
            heads: Vec::new(),
            rows: vec![row("Load time", vec![format!("{:.2} s", t.as_secs_f64())])],
        });
    }
    out
}

/// The notice of an input changed on disk while its document has edits (D-315): `(key, text)`.
pub fn changed_notice(view: &TextViewModel, side: Side) -> (String, String) {
    let name = super::name_of(&super::header::title(view, side));
    (
        format!("disk:{}", super::index_of(side)),
        format!("{name} changed on disk."),
    )
}

/// The sides whose input is `spec` (a changed path of the watch).
pub fn sides_of(view: &TextViewModel, spec: &str) -> Vec<Side> {
    let inputs = view.inputs();
    (0..2)
        .filter(|k| !inputs[*k].is_empty() && inputs[*k] == spec)
        .map(side_of)
        .filter(|s| view.doc(*s).is_some())
        .collect()
}

/// True when `side`'s document has edits (a change on disk then asks, D-315).
pub fn modified(view: &TextViewModel, side: Side) -> bool {
    view.doc(side).is_some_and(EditableDocument::is_modified)
}

/// The names of the formats the view uses (the format registry's reload offers *Reopen* when one of them changed).
pub fn formats_used(view: &TextViewModel) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(r) = view.response() {
        out.push(r.format.name.clone());
        if let Some(rf) = &r.right_format {
            out.push(rf.name.clone());
        }
    }
    if let Some(r) = view.edit_response() {
        out.push(r.format.name.clone());
    }
    out.dedup();
    out
}

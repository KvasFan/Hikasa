//! The hex view's Info dialog (*Hex Compare Info*, M2 W13a, contract §1.13.8, D-360), toolkit-free: the statistics of
//! the landed alignment (`HexStats`: sizes, same, differing, left-only, right-only bytes, sections), the requested and
//! landed modes, block, skew, window, budget, *degraded*, *sections truncated* and the warnings. *Copy* copies the
//! core's summary line (`HexCompareResponse::human_lines`) and the verdict.

use hikasa_hex::HexAlignOptions;
use hikasa_session::api::HexCompareResponse;

use crate::info_dialog::{InfoRow, InfoSection};

/// The help topic of the dialog.
pub const INFO_TOPIC: &str = "dialog-hex-info";

fn row(label: &str, values: Vec<String>) -> InfoRow {
    InfoRow {
        label: label.to_owned(),
        values,
    }
}

/// The Info sections of a landed comparison with the options it ran with (`requested`: the mode asked).
pub fn sections(
    r: &HexCompareResponse,
    options: &HexAlignOptions,
    requested: hikasa_hex::AlignMode,
) -> Vec<InfoSection> {
    let s = &r.alignment.stats;
    let n = |v: u64| v.to_string();
    let mut out = vec![
        InfoSection {
            title: "Bytes".to_owned(),
            heads: vec!["Left".to_owned(), "Right".to_owned()],
            rows: vec![
                row("Size", vec![n(s.left_bytes), n(s.right_bytes)]),
                row("Same", vec![n(s.same_bytes), n(s.same_bytes)]),
                row(
                    "Differing",
                    vec![n(s.differing_bytes), n(s.differing_bytes)],
                ),
                row(
                    "Only on this side",
                    vec![n(s.left_only_bytes), n(s.right_only_bytes)],
                ),
            ],
        },
        InfoSection {
            title: "Alignment".to_owned(),
            heads: Vec::new(),
            rows: vec![
                row("Sections", vec![n(s.sections)]),
                row("Requested mode", vec![requested.as_str().to_owned()]),
                row("Mode", vec![r.alignment.mode.as_str().to_owned()]),
                row("Block", vec![format!("{} bytes", options.block)]),
                row("Skew", vec![format!("{} bytes", options.skew)]),
                row("Window", vec![format!("{} bytes", options.window)]),
                row("Budget", vec![format!("{} MiB", options.budget_mib)]),
                row(
                    "Degraded",
                    vec![if r.alignment.degraded { "yes" } else { "no" }.to_owned()],
                ),
                row(
                    "Sections truncated",
                    vec![
                        if r.alignment.sections.truncated() {
                            "yes"
                        } else {
                            "no"
                        }
                        .to_owned(),
                    ],
                ),
            ],
        },
    ];
    if !r.warnings.is_empty() {
        out.push(InfoSection {
            title: "Warnings".to_owned(),
            heads: Vec::new(),
            rows: r
                .warnings
                .iter()
                .map(|w| row(w.code.as_str(), vec![w.message.clone()]))
                .collect(),
        });
    }
    out
}

/// What *Copy* places: the core's summary line and the verdict line.
pub fn copy_text(r: &HexCompareResponse) -> String {
    let mut summary = r.clone();
    summary.display = None;
    let mut out = summary.human_lines().unwrap_or_default().join("\n");
    out.push('\n');
    out.push_str(crate::table_view::verdict_text(r.kind));
    out.push('\n');
    out
}

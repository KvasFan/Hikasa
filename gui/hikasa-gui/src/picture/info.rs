//! Picture Compare Info (PX-008; §1.13.13, D-365): `PictureStats` with core's percentages (4 decimals), the distances,
//! the bounds of the non-same pixels with *Show* (fits the viewport to them, **open default**), the area, the scaled
//! side, the options and the warnings; *Copy* copies `PictureCompareResponse::human_lines()` and the verdict line
//! (core's text). Toolkit-free.

use std::any::Any;
use std::sync::Arc;

use hikasa_picture::{DiffMode, PictureStats};
use hikasa_session::Service;
use hikasa_session::api::PictureCompareResponse;
use hikasa_session::clipboard::{ClipboardPayload, ClipboardProvider};
use hikasa_session::ipc::ViewId;

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};
use crate::info_dialog::{InfoRow, InfoSection};

/// The help topic.
pub const TOPIC: &str = "dialog-picture-info";

/// *Copy*.
pub const COPY: ButtonRole = ButtonRole::Other(1);
/// *Show*.
pub const SHOW: ButtonRole = ButtonRole::Other(2);

/// The Info dialog of a Picture Compare view.
pub struct PictureInfoDialog {
    /// The view.
    pub view: ViewId,
    sections: Vec<InfoSection>,
    text: String,
    has_bounds: bool,
    clipboard: Arc<dyn ClipboardProvider>,
    /// The answer of the last Copy.
    pub message: Option<String>,
    /// *Show* was pressed: the window fits the view to the bounds.
    pub show: bool,
}

fn counted(s: &PictureStats, n: u64) -> String {
    format!("{n} ({:.4} %)", s.percent(n))
}

impl PictureInfoDialog {
    /// The dialog of a view's response.
    pub fn new(
        view: ViewId,
        response: &PictureCompareResponse,
        clipboard: Arc<dyn ClipboardProvider>,
    ) -> PictureInfoDialog {
        let c = &response.comparison;
        let s = c.stats();
        let row = |label: &str, value: String| InfoRow {
            label: label.to_owned(),
            values: vec![value],
        };
        let mut pixels = vec![
            row("Covered pixels", s.pixels.to_string()),
            row("Same pixels", counted(s, s.same)),
            row("Similar pixels", counted(s, s.similar)),
            row("Different pixels", counted(s, s.different)),
            row("Left only", counted(s, s.left_only)),
            row("Right only", counted(s, s.right_only)),
            row("Maximum distance", s.max_distance.to_string()),
            row("Mean distance", format!("{:.4}", s.mean_distance())),
        ];
        pixels.push(row(
            "Bounds of the differences",
            match s.bounds {
                Some(b) => format!(
                    "{},{} {}",
                    b.x,
                    b.y,
                    hikasa_report::picture::size_text(b.width, b.height)
                ),
                None => "none".to_owned(),
            },
        ));
        let area = c.area();
        let o = c.options();
        let mut geometry = vec![
            row(
                "Area",
                hikasa_report::picture::size_text(area.width, area.height),
            ),
            row(
                "Offset",
                format!("{}, {}", o.geometry.offset.0, o.geometry.offset.1),
            ),
            row(
                "Orientation (left, right)",
                format!(
                    "{}, {}",
                    o.geometry.orientation.left.as_str(),
                    o.geometry.orientation.right.as_str()
                ),
            ),
            row(
                "Auto Scale",
                if o.geometry.auto_scale { "on" } else { "off" }.to_owned(),
            ),
        ];
        if let Some((side, (w, h))) = c.scaled() {
            geometry.push(row(
                "Scaled",
                format!(
                    "{} to {}",
                    side.as_str(),
                    hikasa_report::picture::size_text(w, h)
                ),
            ));
        }
        let mode = match response.mode {
            DiffMode::Blend { percent } => format!("blend {percent} %"),
            m => m.as_str().to_owned(),
        };
        let options = vec![
            row("Tolerance", o.tolerance.to_string()),
            row("Mode", mode),
            row(
                "Ignore unimportant differences",
                if response.ignore_unimportant {
                    "yes"
                } else {
                    "no"
                }
                .to_owned(),
            ),
            row(
                "Verdict",
                crate::table_view::verdict_text(response.kind).to_owned(),
            ),
        ];
        let mut sections = vec![
            InfoSection {
                title: "Pixels".to_owned(),
                heads: Vec::new(),
                rows: pixels,
            },
            InfoSection {
                title: "Geometry".to_owned(),
                heads: Vec::new(),
                rows: geometry,
            },
            InfoSection {
                title: "Options".to_owned(),
                heads: Vec::new(),
                rows: options,
            },
        ];
        if !response.warnings.is_empty() {
            sections.push(InfoSection {
                title: "Warnings".to_owned(),
                heads: Vec::new(),
                rows: response
                    .warnings
                    .iter()
                    .map(|w| row(w.code.as_str(), w.message.clone()))
                    .collect(),
            });
        }
        let mut lines = response.human_lines();
        lines.push(crate::table_view::verdict_text(response.kind).to_owned());
        PictureInfoDialog {
            view,
            sections,
            text: lines.join("\n") + "\n",
            has_bounds: s.bounds.is_some(),
            clipboard,
            message: None,
            show: false,
        }
    }

    /// The table.
    pub fn sections(&self) -> &[InfoSection] {
        &self.sections
    }

    /// What *Copy* places: `human_lines` and the verdict.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl DialogModel for PictureInfoDialog {
    fn title(&self) -> String {
        "Picture Compare Info".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let b = |label: &str, role, enabled, default| DialogButton {
            label: label.to_owned(),
            role,
            enabled,
            default,
        };
        vec![
            b("Copy", COPY, true, false),
            b("Show", SHOW, self.has_bounds, false),
            b("Close", ButtonRole::Close, true, true),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            COPY => {
                let payload = ClipboardPayload {
                    text: self.text.clone(),
                    html: None,
                };
                self.message = Some(match self.clipboard.write_payload(&payload) {
                    Ok(()) => "Copied to the clipboard.".to_owned(),
                    Err(e) => format!("Could not copy: {e}"),
                });
                DialogStep::Stay
            }
            SHOW => {
                self.show = true;
                DialogStep::Close
            }
            _ => DialogStep::Close,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

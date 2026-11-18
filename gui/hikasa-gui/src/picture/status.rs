//! The status bar of the Picture Compare view (PX-008, UI-030; §1.13.13, D-365): the probe of the area pixel under
//! the pointer in core's words (`probe_line`), the offset (a click resets it: `OffsetReset`), the zoom (a click opens
//! the Zoom list),
//! the mode with its tolerance or blend percentage, the verdict and the jobs' progress. Toolkit-free.

use hikasa_picture::DiffMode;
use hikasa_session::Service;

use super::{PicturePhase, PictureViewModel};
use crate::commands::CommandId;
use crate::status::{StatusClick, StatusItems};

/// The zoom as the status bar shows it (`150%`, `Fit 12.5%`).
pub fn zoom_text(model: &PictureViewModel) -> String {
    let word = hikasa_picture::Zoom::Scale(model.scale()).word();
    match model.viewport.zoom {
        super::viewport::Zoom::Fit => format!("Fit {word}"),
        super::viewport::Zoom::Scale(_) => word,
    }
}

/// The offset item (`Offset 3, -2`).
pub fn offset_text(model: &PictureViewModel) -> String {
    let (dx, dy) = model.options.geometry.offset;
    format!("Offset {dx}, {dy}")
}

/// The mode item (`Tolerance 5`, `Range`, `Blend 40 %`).
pub fn mode_text(model: &PictureViewModel) -> String {
    match model.shown_mode() {
        DiffMode::Tolerance => format!("Tolerance {}", model.options.tolerance),
        DiffMode::Range => "Range".to_owned(),
        DiffMode::Blend { percent } => format!("Blend {percent} %"),
    }
}

/// The status bar's items: the probe on the left; offset, zoom, mode, verdict on the right.
pub fn items(model: &PictureViewModel, service: &Service) -> StatusItems {
    let left = model
        .probe()
        .map(hikasa_report::probe_line)
        .unwrap_or_default();
    let verdict = match model.phase() {
        PicturePhase::Loading => "Decoding…".to_owned(),
        PicturePhase::Comparing => "Comparing…".to_owned(),
        PicturePhase::Failed => model.error().unwrap_or("Failed").to_owned(),
        PicturePhase::Ready => model
            .response(service)
            .map(|r| crate::table_view::verdict_text(r.kind).to_owned())
            .unwrap_or_default(),
    };
    StatusItems {
        left,
        right: vec![
            offset_text(model),
            zoom_text(model),
            mode_text(model),
            verdict,
        ],
        clicks: vec![
            Some(StatusClick::Run(CommandId::OffsetReset)),
            Some(StatusClick::ZoomList),
        ],
    }
}

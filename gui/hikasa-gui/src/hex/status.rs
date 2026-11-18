//! The hex view's status bar (UI-030; M2 W13a, contract §1.13.8, D-360), toolkit-free: the Current Byte Address of
//! the focus side and the other side's byte at the same column (both `nav::address_text`), the caret cell's state, the
//! selection's length, the alignment (*None*, *Fast*, *Complete*, *degraded*, *Aligning… n %*, *Alignment stopped*),
//! the filter with its hidden rows, and the verdict once landed.

use hikasa_core::Side;
use hikasa_hex::{AlignMode, ByteState, DisplayFilter};

use super::{HexPhase, HexViewModel, other};
use crate::status::StatusItems;

/// The words of a cell state.
pub fn state_text(state: ByteState) -> &'static str {
    match state {
        ByteState::Same => "Same",
        ByteState::Different => "Different",
        ByteState::LeftOnly => "Left only",
        ByteState::RightOnly => "Right only",
    }
}

fn mode_text(mode: AlignMode) -> &'static str {
    match mode {
        AlignMode::None => "None",
        AlignMode::Fast => "Fast",
        AlignMode::Complete => "Complete",
    }
}

fn side_word(side: Side) -> &'static str {
    if side == Side::Right { "right" } else { "left" }
}

/// The status bar of a hex view.
pub fn items(m: &HexViewModel) -> StatusItems {
    let left = match m.phase() {
        HexPhase::Opening => "Opening the files…".to_owned(),
        HexPhase::Failed => m.error().map_or_else(String::new, |(_, e)| {
            format!("The files could not be opened: {e}")
        }),
        _ => {
            let focus = m.focus();
            let mut parts = Vec::new();
            if let Some(a) = m.address_of(focus) {
                parts.push(format!("{} {a}", capital(side_word(focus))));
            }
            if let Some(b) = m.address_of(other(focus)) {
                parts.push(format!("{} {b}", side_word(other(focus))));
            }
            parts.join(" · ")
        }
    };
    let mut right = Vec::new();
    if let Some(state) = caret_state(m) {
        right.push(state_text(state).to_owned());
    }
    let sel = m.selection();
    if sel.end - sel.start > 1
        && let Some(o) = m.opened()
    {
        let side = m.focus();
        let store_len = |col: u64| m.byte_at(side, col).map_or(0, |(o, _)| o);
        let bytes = store_len(sel.end).saturating_sub(store_len(sel.start));
        let _ = o;
        right.push(format!(
            "{} {} selected",
            bytes,
            if bytes == 1 { "byte" } else { "bytes" }
        ));
    }
    right.push(alignment_text(m));
    let filter = m.display().filter;
    if filter != DisplayFilter::All {
        let hidden = m.full_row_count().saturating_sub(m.row_count());
        let word = if filter == DisplayFilter::Differences {
            "Differences"
        } else {
            "Same"
        };
        if m.alignment().is_some() {
            right.push(format!("{word}: {hidden} rows hidden"));
        } else {
            right.push(format!("{word}: when complete"));
        }
    }
    if let Some(kind) = m.verdict() {
        right.push(crate::table_view::verdict_text(kind).to_owned());
    }
    StatusItems {
        left,
        right,
        clicks: Vec::new(),
    }
}

/// The alignment item.
pub fn alignment_text(m: &HexViewModel) -> String {
    if let Some((_, fraction)) = m.aligning() {
        return match fraction {
            Some(f) => format!("Aligning… {} %", (f * 100.0).round() as u32),
            None => "Aligning…".to_owned(),
        };
    }
    if m.stopped() && m.alignment().is_none() {
        return "Alignment stopped".to_owned();
    }
    match m.alignment() {
        Some(al) if al.degraded => format!("{} (degraded)", mode_text(al.mode)),
        Some(al) => mode_text(al.mode).to_owned(),
        None => mode_text(m.mode).to_owned(),
    }
}

/// The state of the caret's cell when its row is in the row cache.
pub fn caret_state(m: &HexViewModel) -> Option<ByteState> {
    let caret = m.pane(m.focus()).caret;
    let k = m.row_of_column(caret)?;
    let first = m.first_column_of(k)?;
    if caret < first {
        return None;
    }
    let row = m.screen.cached(m.generation, k)?;
    row.cells.get((caret - first) as usize).map(|c| c.state)
}

fn capital(word: &str) -> String {
    let mut c = word.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

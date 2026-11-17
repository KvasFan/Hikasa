//! The File Info header of a pane (M2 W11a, contract §1.11.11, D-302), toolkit-free: the path (the spec, the
//! `--title1/2` title), the encoding with its BOM, the line endings of the document after the last edit, the format
//! with its reason, and the badges (*Read-only*, *Editing disabled*, *Converted*, *Modified*). Every text comes from
//! core (`EditableDocument::encoding`, the document's `EolSummary`, `Editability::status_text`, the response's format
//! choice); `ui/text_header.rs` draws it and turns its menus into the calls below.

use hikasa_core::Side;
use hikasa_formats::DetectReason;
use hikasa_text::edit::{EditBlock, EditRequest, Editability, EditableDocument};
use hikasa_text::{EolStyle, LineEnding};

use super::edit::{self, Origin};
use super::{TextViewModel, index_of};

/// The header's state of one pane (the path edit's buffer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderState {
    /// The path edit's text while it is being edited (`None`: it shows the spec).
    pub path: Option<String>,
}

/// A badge of the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Badge {
    /// *Read-only* or *Editing disabled* (the status text), with every block as its tooltip; `binary`: the only block
    /// is `Binary`, which offers *Edit anyway* (D-297).
    Blocked {
        /// `Editability::status_text`.
        text: &'static str,
        /// Every block, one a line.
        tooltip: String,
        /// The user may lift it.
        binary: bool,
    },
    /// *Converted* (FF-031): the format converts the file for the comparison.
    Converted,
    /// *Modified*: the pane has edits not saved.
    Modified,
}

/// What the header of one pane shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeaderItems {
    /// The spec (or the title given on the command line).
    pub path: String,
    /// `UTF-8`, `UTF-16LE BOM`, `windows-1252`, …
    pub encoding: String,
    /// The document's encoding is a UTF one (the *Byte Order Mark* check applies).
    pub utf: bool,
    /// The document writes a BOM.
    pub bom: bool,
    /// `LF`, `CRLF`, `CR`, `MIX`, `none`.
    pub eol: &'static str,
    /// `Detected (C, C++, …)`, `Forced (…)`, `Everything Else`.
    pub format: String,
    /// The badges.
    pub badges: Vec<Badge>,
}

/// The title a pane's header shows: the `--title1/2` title, else the document's location (after a Save As, the new
/// one), else the spec of the request.
pub fn title(view: &TextViewModel, side: Side) -> String {
    let k = index_of(side);
    if let Some(t) = &view.titles[k] {
        return t.clone();
    }
    if let Some(spec) = view.doc(side).and_then(EditableDocument::location) {
        return spec.canonical();
    }
    let spec = if k == 1 {
        &view.request.right
    } else {
        &view.request.left
    };
    spec.to_string_lossy().into_owned()
}

/// The encoding's text (`UTF-16LE BOM`).
pub fn encoding_text(doc: &EditableDocument) -> String {
    let (enc, bom) = doc.encoding();
    if bom {
        format!("{} BOM", enc.name())
    } else {
        enc.name().to_owned()
    }
}

/// The line endings' text of the document as it is now.
pub fn eol_text(doc: &EditableDocument) -> &'static str {
    match doc.document().eol().style() {
        EolStyle::Lf => "LF",
        EolStyle::CrLf => "CRLF",
        EolStyle::Cr => "CR",
        EolStyle::Mixed => "MIX",
        EolStyle::None => "none",
    }
}

/// The format's text of a side: its name and why it was chosen.
pub fn format_text(view: &TextViewModel, side: Side) -> String {
    let Some(r) = view.response() else {
        return String::new();
    };
    let choice = match side {
        Side::Right => r.right_format.as_ref().unwrap_or(&r.format),
        _ => &r.format,
    };
    match &choice.reason {
        DetectReason::Fallback => "Everything Else".to_owned(),
        DetectReason::Forced { .. } => format!("Forced ({})", choice.name),
        DetectReason::Mask { .. } => format!("Detected ({})", choice.name),
    }
}

/// The text of one block (the tooltip of a badge, the message of a blocked edit).
pub fn block_text(block: EditBlock) -> &'static str {
    match block {
        EditBlock::LoadFailed => "the file could not be loaded",
        EditBlock::Cancelled => "the comparison was cancelled",
        EditBlock::ReadOnlyLocation => "the file cannot be written",
        EditBlock::ArchiveReadOnly => "the archive member cannot be written",
        EditBlock::Converted => "the format converts the file without a way back",
        EditBlock::Truncated => "the file was too large to load whole",
        EditBlock::ParentReadOnly => "the folder comparison's side is read-only",
        EditBlock::SessionReadOnly => "the session's side is read-only",
        EditBlock::CommandLine => "editing is disabled on the command line",
        EditBlock::Binary => "the file contains binary data",
    }
}

/// The message panel line of a blocked edit: `<status text>: <the blocks, joined>` (D-297).
pub fn block_message(e: &Editability) -> String {
    let blocks: Vec<&str> = e.blocks.iter().map(|b| block_text(*b)).collect();
    let status = match e.status_text() {
        "" => "Not editable",
        s => s,
    };
    format!("{status}: {}", blocks.join(", "))
}

/// The header items of a pane (`None` before the documents exist).
pub fn items(view: &TextViewModel, side: Side) -> Option<HeaderItems> {
    let doc = view.doc(side)?;
    let e = doc.editability();
    let (enc, bom) = doc.encoding();
    let mut badges = Vec::new();
    if !e.editable() {
        badges.push(Badge::Blocked {
            text: e.status_text(),
            tooltip: e
                .blocks
                .iter()
                .map(|b| block_text(*b))
                .collect::<Vec<_>>()
                .join("\n"),
            binary: super::only_binary(doc),
        });
    }
    if e.blocks.contains(&EditBlock::Converted) {
        badges.push(Badge::Converted);
    }
    if doc.is_modified() {
        badges.push(Badge::Modified);
    }
    Some(HeaderItems {
        path: title(view, side),
        encoding: encoding_text(doc),
        utf: !enc.bom().is_empty(),
        bom,
        eol: eol_text(doc),
        format: format_text(view, side),
        badges,
    })
}

/// The detail line of a pane (the M1 header line, now the header's items joined): encoding, line endings, lines.
pub fn detail(view: &TextViewModel, side: Side) -> String {
    if let Some(doc) = view.doc(side) {
        let mut parts = vec![encoding_text(doc), eol_text(doc).to_owned()];
        parts.push(format!("{} lines", doc.line_count()));
        if doc.is_modified() {
            parts.push("Modified".to_owned());
        }
        let status = doc.editability().status_text();
        if !status.is_empty() {
            parts.push(status.to_owned());
        }
        return parts.join(" · ");
    }
    match view.preview() {
        Some((l, r)) => super::doc_detail(if side == Side::Right { r } else { l }),
        None => String::new(),
    }
}

/// *Save with Encoding* (D-302): a `SetEncoding` step on that pane (unmappable characters are the save's question).
/// True when applied.
pub fn save_with_encoding(view: &mut TextViewModel, side: Side, label: &str) -> bool {
    if view.frozen(side) {
        super::input::queue(view, side, super::input::Input::Encoding(label.to_owned()));
        return false;
    }
    set_encoding_now(view, side, label)
}

/// [`save_with_encoding`] on a held pane too: the save's own *Save as UTF-8* step, before it plans again.
pub fn set_encoding_now(view: &mut TextViewModel, side: Side, label: &str) -> bool {
    let Some(encoding) = hikasa_formats::TextEncoding::for_label(label) else {
        return false;
    };
    let bom = view
        .doc(side)
        .is_some_and(|d| d.encoding().1 && !encoding.bom().is_empty());
    edit::edit(
        view,
        side,
        EditRequest::SetEncoding { encoding, bom },
        Origin::Single,
        None,
    )
}

/// *Byte Order Mark* (UTF only): `SetEncoding` with the other BOM. True when applied.
pub fn toggle_bom(view: &mut TextViewModel, side: Side) -> bool {
    if view.frozen(side) {
        super::input::queue(view, side, super::input::Input::Bom);
        return false;
    }
    let Some((encoding, bom)) = view.doc(side).map(EditableDocument::encoding) else {
        return false;
    };
    if encoding.bom().is_empty() {
        return false;
    }
    edit::edit(
        view,
        side,
        EditRequest::SetEncoding {
            encoding,
            bom: !bom,
        },
        Origin::Single,
        None,
    )
}

/// The line-ending menu's conversions (*To Windows/DOS*, *To Unix*, *To Macintosh*): every terminated line of that
/// pane gets `eol` as one step. True when applied.
pub fn set_line_endings(view: &mut TextViewModel, side: Side, eol: LineEnding) -> bool {
    if view.frozen(side) {
        super::input::queue(view, side, super::input::Input::Eols(eol));
        return false;
    }
    let Some(lines) = view.doc(side).map(EditableDocument::line_count) else {
        return false;
    };
    edit::edit(
        view,
        side,
        EditRequest::SetEols {
            lines: 0..lines,
            eol,
        },
        Origin::Single,
        None,
    )
}

/// The encoding labels of the header's menus (FF-032's groups, flattened).
pub const ENCODINGS: &[&str] = &[
    "UTF-8",
    "UTF-16LE",
    "UTF-16BE",
    "UTF-32LE",
    "UTF-32BE",
    "windows-1252",
    "ISO-8859-1",
    "ISO-8859-2",
    "ISO-8859-15",
    "windows-1250",
    "windows-1251",
    "KOI8-R",
    "IBM866",
    "Shift_JIS",
    "EUC-JP",
    "GBK",
    "gb18030",
    "Big5",
    "EUC-KR",
];

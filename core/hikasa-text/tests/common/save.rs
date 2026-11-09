//! Helpers of the save tests (A10.13, A10.14): documents loaded from a `Vfs`, a save planned, confirmed and executed,
//! and the bytes a `SaveContent` stands for.

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::confirm::ConfirmMode;
use hikasa_core::{Ctl, LocalPath, Spec};
use hikasa_formats::{
    CompiledFormat, DecodeOptions, EncodingSetting, FileFormat, FormatId, FormatKind, RegexLimits,
    TextEncoding,
};
use hikasa_text::edit::{EditOptions, Editability, EditableDocument};
use hikasa_text::save::{
    Piece, SaveConfirmation, SaveContent, SavePlan, SavePolicy, SaveReport, SaveRequest,
    SaveSource, execute_save, plan_save,
};
use hikasa_text::{LoadOptions, load_document};
use hikasa_vfs::Vfs;

/// `Spec::Local` of a `/`-separated path, host style.
pub fn spec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(super::host(path))).unwrap())
}

/// A text format with the given encoding setting and the Ctrl-Z rule.
pub fn text_format(encoding: EncodingSetting, ctrl_z: bool) -> CompiledFormat {
    let mut format = FileFormat::new(FormatId::new("t").unwrap(), "T", FormatKind::Text);
    format.encoding = encoding;
    format.text.ctrl_z_eof = ctrl_z;
    CompiledFormat::compile(Arc::new(format), &RegexLimits::default()).unwrap()
}

/// Loads `spec` and makes it editable (the format's CPL, or `cpl`).
pub fn open(
    vfs: &dyn Vfs,
    spec: &Spec,
    format: &CompiledFormat,
    decode: DecodeOptions,
    cpl: Option<u32>,
) -> EditableDocument {
    let opts = LoadOptions {
        decode,
        chars_per_line_limit: cpl,
        ..LoadOptions::default()
    };
    let (doc, _) = load_document(vfs, spec, format, &opts, None, &Ctl::detached()).unwrap();
    let edit = EditOptions {
        chars_per_line_limit: cpl,
        ..EditOptions::default()
    };
    EditableDocument::new(Arc::new(doc), edit, Editability::EDITABLE)
}

/// Decode options that force `encoding`.
pub fn forced(encoding: TextEncoding) -> DecodeOptions {
    DecodeOptions {
        override_encoding: Some(encoding),
        ..DecodeOptions::default()
    }
}

/// Plans the save of `doc` with `req`.
pub fn plan(vfs: &dyn Vfs, doc: &EditableDocument, req: &SaveRequest) -> SavePlan {
    plan_save(vfs, &[(None, doc, req)], &Ctl::detached()).unwrap()
}

/// Every question answered yes.
pub fn confirm_all(plan: SavePlan) -> SavePlan {
    let machine = SaveConfirmation::new(&plan, &SavePolicy::ALL, ConfirmMode::YesToAll);
    machine.finish(plan).unwrap()
}

/// Executes `plan` for `doc` and records the save in it.
pub fn execute(vfs: &dyn Vfs, plan: SavePlan, doc: &mut EditableDocument) -> SaveReport {
    let source = SaveSource::new(vfs, &plan, &[&*doc]);
    let report = execute_save(vfs, plan, source, &Ctl::detached());
    if let Some(Some(saved)) = report.saved.first() {
        doc.saved_to(saved);
    }
    report
}

/// Plan, confirm (all yes), execute, record.
pub fn save(vfs: &dyn Vfs, doc: &mut EditableDocument, req: &SaveRequest) -> SaveReport {
    let planned = confirm_all(plan(vfs, doc, req));
    execute(vfs, planned, doc)
}

/// The bytes `content` stands for, its `Source` pieces read from `source`.
pub fn render(content: &SaveContent, source: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for piece in &content.pieces {
        render_piece(piece, content, source, &mut out);
    }
    out
}

fn render_piece(piece: &Piece, content: &SaveContent, source: &[u8], out: &mut Vec<u8>) {
    match piece {
        Piece::Bom => out.extend_from_slice(content.encoding.bom()),
        Piece::Encoded(bytes) => out.extend_from_slice(bytes),
        Piece::Source { offset, len } => {
            out.extend_from_slice(&source[*offset as usize..(*offset + *len) as usize])
        }
        Piece::Converted(stream) => {
            for made in stream.pieces() {
                render_piece(&made, content, source, out);
            }
        }
    }
}

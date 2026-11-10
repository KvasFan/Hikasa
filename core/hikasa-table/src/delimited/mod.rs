//! Delimited and fixed-width text tables (contract §2): decoding, sniffing, parsing, writing.

mod decode;
mod parse;
mod sniff;
mod write;

pub use write::{WriteLoss, write_delimited};

use hikasa_core::CancelToken;

use crate::error::{TableError, TableWarning, TableWarningCode};
use crate::format::{DelimitedOptions, TableFormat, TableKind};

/// A delimiter as the `--delimiter` flag takes it: `tab`, `space`, or the character quoted.
fn word(c: char) -> String {
    match c {
        '\t' => "tab".to_owned(),
        ' ' => "space".to_owned(),
        c => format!("'{c}'"),
    }
}
use crate::sheet::{SheetBuilder, SourceKind, Workbook};

/// Loads a text table from its bytes (contract §2.2). `name` names the sheet (the file name without folders)
/// and the errors. A `Spreadsheet` kind is refused here (see [`crate::load_bytes`]).
pub fn load_text(
    bytes: Vec<u8>,
    name: &str,
    format: &TableFormat,
    cancel: &CancelToken,
) -> Result<Workbook, TableError> {
    if bytes.len() as u64 > u64::from(u32::MAX) {
        return Err(TableError::TooLarge {
            name: name.to_owned(),
            what: "bytes",
            limit: u64::from(u32::MAX),
        });
    }
    let decoded = decode::decode(bytes, format.encoding.as_deref())
        .map_err(|m| TableError::InvalidInput(format!("{name}: {m}")))?;
    cancel.check()?;
    let mut text = decoded.text;
    if format.ctrl_z_eof
        && let Some(at) = memchr::memchr(0x1A, text.as_bytes())
    {
        text.truncate(at);
    } else if text.ends_with('\u{1A}') {
        // A lone Ctrl+Z at the very end is the old end-of-file mark, never data (F12).
        text.pop();
    }
    if text.len() as u64 > u64::from(u32::MAX) {
        return Err(TableError::TooLarge {
            name: name.to_owned(),
            what: "bytes of decoded text",
            limit: u64::from(u32::MAX),
        });
    }
    // A delimited table with fixed delimiters whose rows will all be one field while another delimiter would split
    // it warns (F3); one whose delimiters are a preference takes the other delimiter.
    let mut suspect = None;
    let kind = match &format.kind {
        TableKind::Detect => sniff::sniff(&text),
        TableKind::Delimited(opts) if opts.sniff => {
            match sniff::other_delimiter(&text, &opts.delimiters) {
                Some(d) => TableKind::Delimited(DelimitedOptions {
                    delimiters: vec![d],
                    ..opts.clone()
                }),
                None => format.kind.clone(),
            }
        }
        TableKind::Delimited(opts) => {
            suspect = sniff::other_delimiter(&text, &opts.delimiters);
            format.kind.clone()
        }
        TableKind::Spreadsheet => {
            return Err(TableError::InvalidInput(format!(
                "{name}: a spreadsheet format needs a workbook"
            )));
        }
        other => other.clone(),
    };
    let sheet_name = name.rsplit(['/', '\\']).next().unwrap_or(name).to_owned();
    let mut builder = SheetBuilder::new(sheet_name.clone(), text, format.regional);
    let mut warnings: Vec<TableWarning> = Vec::new();
    let source = match &kind {
        TableKind::Delimited(opts) => {
            opts.validate()
                .map_err(|m| TableError::InvalidInput(format!("{name}: {m}")))?;
            warnings.extend(parse::parse_delimited(&mut builder, opts, cancel)?);
            SourceKind::Delimited
        }
        TableKind::Fixed(opts) => {
            parse::parse_fixed(&mut builder, opts, cancel)?;
            SourceKind::Fixed
        }
        TableKind::Detect | TableKind::Spreadsheet => SourceKind::Delimited,
    };
    if decoded.malformed > 0 {
        warnings.push(TableWarning::new(
            TableWarningCode::LossyDecode,
            format!(
                "{} malformed byte sequences replaced while decoding as {}",
                decoded.malformed, decoded.encoding
            ),
        ));
    }
    cancel.check()?;
    let sheet = builder.finish(format.first_line);
    if let (Some(d), TableKind::Delimited(opts)) = (suspect, &kind)
        && sheet.row_count() > 0
        && sheet.column_count() <= 1
    {
        let given: Vec<String> = opts.delimiters.iter().map(|c| word(*c)).collect();
        warnings.push(TableWarning::new(
            TableWarningCode::DelimiterMismatch,
            format!(
                "every row has one field with the delimiter {}; the text looks {}-delimited: try --delimiter {}",
                given.join(" "),
                word(d),
                word(d),
            ),
        ));
    }
    let detected = TableFormat {
        kind,
        ..format.clone()
    };
    for w in &mut warnings {
        w.sheet = Some(sheet_name.clone());
    }
    Ok(Workbook {
        sheets: vec![sheet],
        source,
        detected,
        warnings,
        encoding: Some(decoded.encoding),
    })
}

//! The delimited and fixed-width parsers (contract §2.2).

use super::split::{CHECKPOINT_EVERY, Field, Splitter};
use crate::cell::Cell;
use crate::error::{TableError, TableWarning, TableWarningCode};
use crate::format::{DelimitedOptions, FixedOptions};
use crate::sheet::SheetBuilder;
use hikasa_core::CancelToken;

/// Rows between two cancellation checks.
const CHECK_EVERY: u32 = 65_536;

/// Parses `builder`'s arena as delimited text into rows. Returns the warnings.
pub(crate) fn parse_delimited(
    builder: &mut SheetBuilder,
    opts: &DelimitedOptions,
    cancel: &CancelToken,
) -> Result<Vec<TableWarning>, TableError> {
    let text = std::mem::take(builder.arena_mut());
    let result = parse_into(builder, &text, opts, cancel);
    *builder.arena_mut() = text;
    result
}

fn parse_into(
    b: &mut SheetBuilder,
    text: &str,
    opts: &DelimitedOptions,
    cancel: &CancelToken,
) -> Result<Vec<TableWarning>, TableError> {
    let splitter = Splitter::delimited(opts);
    let qs: String = opts.qualifier.map(String::from).unwrap_or_default();
    let len = text.len();
    let mut pos = 0usize;
    let mut line: u32 = 1;
    let mut warnings = Vec::new();
    let mut stray_warned = false;
    let mut rows: u32 = 0;
    b.reserve(len / 8, len / 64);
    while pos < len {
        rows += 1;
        if rows.is_multiple_of(CHECK_EVERY) {
            cancel.check()?;
        }
        let record_line = line;
        b.begin_record(pos);
        let mut fields = splitter.record(text, pos);
        for (index, out) in (0u32..).zip(fields.by_ref()) {
            if index > 0 && index.is_multiple_of(CHECKPOINT_EVERY) {
                b.checkpoint(out.at);
            }
            if out.unterminated {
                warnings.push(TableWarning::new(
                    TableWarningCode::UnterminatedQualifier,
                    format!("a qualified field opened on line {record_line} is not closed"),
                ));
            }
            match out.field {
                Field::Empty => b.push(Cell::EMPTY)?,
                Field::Span(start, end) => b.push_span(start, end)?,
                Field::Unescaped {
                    start,
                    end,
                    doubled,
                    stray,
                    lines,
                } => {
                    let content = &text[start..end];
                    let stray = &text[stray.0..stray.1];
                    let mut value = if doubled {
                        content.replace(&format!("{qs}{qs}"), &qs)
                    } else {
                        content.to_owned()
                    };
                    if !stray.is_empty() {
                        if !stray_warned {
                            stray_warned = true;
                            let line = record_line + lines;
                            warnings.push(TableWarning::new(
                                TableWarningCode::StrayAfterQualifier,
                                format!(
                                    "characters after a closing qualifier on line {line} are kept as data"
                                ),
                            ));
                        }
                        value.push_str(stray);
                    }
                    if value.is_empty() {
                        b.push(Cell::EMPTY)?;
                    } else {
                        b.push_text(&value)?;
                    }
                }
            }
        }
        let (next, lines) = fields.finish();
        pos = next;
        line += lines;
        b.end_row(Some(record_line));
    }
    b.set_splitter(splitter);
    Ok(warnings)
}

/// Parses `builder`'s arena as fixed-width text (§2.2 rule 5).
pub(crate) fn parse_fixed(
    b: &mut SheetBuilder,
    opts: &FixedOptions,
    cancel: &CancelToken,
) -> Result<(), TableError> {
    let text = std::mem::take(b.arena_mut());
    let result = fixed_into(b, &text, opts, cancel);
    *b.arena_mut() = text;
    result
}

fn fixed_into(
    b: &mut SheetBuilder,
    text: &str,
    opts: &FixedOptions,
    cancel: &CancelToken,
) -> Result<(), TableError> {
    let splitter = Splitter::fixed(opts);
    let mut pos = 0usize;
    let mut line: u32 = 0;
    while pos < text.len() {
        line += 1;
        if line.is_multiple_of(CHECK_EVERY) {
            cancel.check()?;
        }
        b.begin_record(pos);
        let mut fields = splitter.record(text, pos);
        for (index, out) in (0u32..).zip(fields.by_ref()) {
            if index > 0 && index.is_multiple_of(CHECKPOINT_EVERY) {
                b.checkpoint(out.at);
            }
            b.push(out.field.cell().unwrap_or(Cell::EMPTY))?;
        }
        b.end_row(Some(line));
        pos = fields.finish().0;
    }
    b.set_splitter(splitter);
    Ok(())
}

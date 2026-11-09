//! First-line detection (contract §2.5, TB-003) and column-kind detection (T-6, TB-005).

use crate::cell::Value;
use crate::format::FirstLine;
use crate::regional::Parsed;
use crate::sheet::{ColumnKind, Sheet};

/// Rows of the body that the header vote looks at.
const VOTE_ROWS: u32 = 50;

/// Kind of one value: typed cells keep their kind; text is parsed with the sheet's regional settings.
pub(crate) fn value_kind(v: &Value<'_>, sheet: &Sheet) -> ColumnKind {
    match v {
        Value::Empty => ColumnKind::Empty,
        Value::Number(_) => ColumnKind::Number,
        Value::Bool(_) => ColumnKind::Bool,
        Value::DateTime(_) => ColumnKind::DateTime,
        Value::Duration(_) => ColumnKind::Duration,
        Value::Error(_) => ColumnKind::Mixed,
        Value::Text(t) => {
            if t.trim().is_empty() {
                return ColumnKind::Empty;
            }
            match sheet.regional().parse_any(t) {
                Some(Parsed::Number(_)) => ColumnKind::Number,
                Some(Parsed::DateTime(_)) => ColumnKind::DateTime,
                Some(Parsed::Bool(_)) => ColumnKind::Bool,
                None => ColumnKind::Text,
            }
        }
    }
}

/// True when the first stored row is a header under `first_line` (§2.5).
pub(crate) fn detect_header(sheet: &Sheet, first_line: FirstLine, width: u32) -> bool {
    let rows = sheet.stored_count();
    match first_line {
        FirstLine::Names => return rows > 0,
        FirstLine::Data => return false,
        FirstLine::Detect => {}
    }
    if rows < 2 || width == 0 {
        return false;
    }
    let head = sheet.stored_row(0);
    if head.is_empty() {
        return false;
    }
    // Every header cell non-empty text that is not a number, date or bool.
    for c in 0..width {
        match head.get(c) {
            Value::Text(t) if !t.trim().is_empty() => {
                if sheet.regional().parse_any(t).is_some() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    let body_end = rows.min(VOTE_ROWS + 1);
    let mut votes: i64 = 0;
    for c in 0..width {
        let header = head.get(c).display().trim().to_owned();
        let mut typed = true;
        let mut any = false;
        let mut length: Option<usize> = None;
        let mut same_length = true;
        let mut occurs = false;
        for r in 1..body_end {
            let v = sheet.stored_row(r).get(c);
            if v.is_blank() {
                continue;
            }
            any = true;
            let kind = value_kind(&v, sheet);
            if matches!(kind, ColumnKind::Text | ColumnKind::Mixed) {
                typed = false;
            }
            let text = v.display();
            let text = text.trim();
            let len = text.chars().count();
            match length {
                None => length = Some(len),
                Some(l) if l != len => same_length = false,
                _ => {}
            }
            if text == header {
                occurs = true;
            }
        }
        if !any {
            continue;
        }
        if occurs {
            votes -= 1;
        } else if typed || (same_length && length != Some(header.chars().count())) {
            votes += 1;
        }
    }
    votes > 0
}

/// Detected kinds of the data columns (header row excluded).
pub(crate) fn column_kinds(sheet: &Sheet, width: u32) -> Vec<ColumnKind> {
    kinds_from(sheet, u32::from(sheet.header().is_some()), width)
}

/// Detected kinds of the columns over the stored rows from `first` on.
pub(crate) fn kinds_from(sheet: &Sheet, first: u32, width: u32) -> Vec<ColumnKind> {
    let mut kinds = vec![ColumnKind::Empty; width as usize];
    for r in first..sheet.stored_count() {
        let row = sheet.stored_row(r);
        for (c, kind) in kinds.iter_mut().enumerate().take(row.len() as usize) {
            if *kind == ColumnKind::Text {
                continue;
            }
            let v = row.get(c as u32);
            let k = value_kind(&v, sheet);
            *kind = kind.join(k);
        }
    }
    kinds
}

//! The statistics CSV (§2.17.4, D-18): one header row and one data row per pair, RFC 4180 (CRLF line ends,
//! fields with `,` `"` CR or LF quoted with doubled `"`). It has no report header: the file is pure CSV.

use std::io::Write;

use hikasa_core::Side;
use hikasa_text::TextComparison;

use crate::error::ReportError;
use crate::escape::csv_field;
use crate::meta::ReportMeta;
use crate::text::spec_text;

/// The header row (§2.17.4).
pub(crate) const CSV_HEADER: &str = "SourceFile,CompareFile,Total,IAdded,IDeleted,IChanged,IOriginal,UAdded,UDeleted,UChanged,UOriginal,Errors";

/// The `Errors` field: the per-side errors of `meta` (`left: …; right: …`), or empty.
fn errors(meta: &ReportMeta) -> String {
    [
        (Side::Left, &meta.errors.left),
        (Side::Right, &meta.errors.right),
    ]
    .iter()
    .filter_map(|(side, e)| e.as_ref().map(|e| format!("{side}: {e}")))
    .collect::<Vec<_>>()
    .join("; ")
}

/// Writes the two rows.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &TextComparison,
    meta: &ReportMeta,
) -> Result<(), ReportError> {
    let s = cmp.stats();
    let (i, u) = (&s.important, &s.unimportant);
    write!(w, "{CSV_HEADER}\r\n")?;
    let numbers = [
        s.rows, i.added, i.deleted, i.changed, i.original, u.added, u.deleted, u.changed,
        u.original,
    ];
    let mut row = String::new();
    row.push_str(&csv_field(&spec_text(cmp.document(Side::Left))));
    row.push(',');
    row.push_str(&csv_field(&spec_text(cmp.document(Side::Right))));
    for n in numbers {
        row.push(',');
        row.push_str(&n.to_string());
    }
    row.push(',');
    row.push_str(&csv_field(&errors(meta)));
    write!(w, "{row}\r\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::Pair;

    #[test]
    fn header_has_twelve_fields() {
        assert_eq!(CSV_HEADER.split(',').count(), 12);
        let meta = ReportMeta {
            title: None,
            generated_at: None,
            generator: String::new(),
            mode: None,
            filters: None,
            errors: Pair::new(Some("not found".into()), Some("x, y".into())),
        };
        assert_eq!(errors(&meta), "left: not found; right: x, y");
    }
}

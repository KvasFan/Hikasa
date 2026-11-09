//! Writing delimited text (contract §2.4): the inverse of the parser, used by saves (TB-023) and P-DELIM.

use crate::format::DelimitedOptions;

/// A field that the options cannot represent (no qualifier, and the field needs one).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("data lost because of the format: row {row}, field {field} cannot be written without a qualifier")]
pub struct WriteLoss {
    /// 0-based row.
    pub row: usize,
    /// 0-based field.
    pub field: usize,
}

/// Writes `rows` with the first delimiter; records end with `\n` (§2.4).
pub fn write_delimited<S: AsRef<str>>(
    rows: &[Vec<S>],
    opts: &DelimitedOptions,
) -> Result<String, WriteLoss> {
    let delim = opts.delimiters.first().copied().unwrap_or(',');
    let mut out = String::new();
    for (r, row) in rows.iter().enumerate() {
        for (f, field) in row.iter().enumerate() {
            if f > 0 {
                out.push(delim);
            }
            let field = field.as_ref();
            let needs = field
                .chars()
                .any(|c| opts.delimiters.contains(&c) || Some(c) == opts.qualifier || c == '\r' || c == '\n')
                || (opts.trim_whitespace
                    && (field.starts_with([' ', '\t']) || field.ends_with([' ', '\t'])))
                || (row.len() == 1 && field.is_empty())
                || (opts.qualifier.is_some_and(|q| field.starts_with(q)));
            if needs {
                let Some(q) = opts.qualifier else {
                    return Err(WriteLoss { row: r, field: f });
                };
                out.push(q);
                for c in field.chars() {
                    if c == q {
                        out.push(q);
                    }
                    out.push(c);
                }
                out.push(q);
            } else {
                out.push_str(field);
            }
        }
        out.push('\n');
    }
    Ok(out)
}

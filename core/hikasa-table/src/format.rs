//! Table formats (contract §2.1, TB-001…TB-003): [`TableFormat`], [`TableKind`], [`DelimitedOptions`],
//! [`FixedOptions`], [`FirstLine`].

use serde::{Deserialize, Serialize};

use crate::regional::Regional;

/// *First line contains* (TB-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstLine {
    /// Decide per sheet (§2.5).
    #[default]
    Detect,
    /// Column names.
    Names,
    /// Cell data.
    Data,
}

impl FirstLine {
    /// Parses `detect`, `names`, `data` (ASCII case-insensitive).
    pub fn parse(word: &str) -> Result<FirstLine, String> {
        match word.to_ascii_lowercase().as_str() {
            "detect" => Ok(FirstLine::Detect),
            "names" | "header" => Ok(FirstLine::Names),
            "data" => Ok(FirstLine::Data),
            _ => Err(format!(
                "unknown first-line choice `{word}` (expected detect, names or data)"
            )),
        }
    }

    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            FirstLine::Detect => "detect",
            FirstLine::Names => "names",
            FirstLine::Data => "data",
        }
    }
}

/// Delimited options (TB-002).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DelimitedOptions {
    /// Any of these separates fields (never the qualifier, CR or LF).
    pub delimiters: Vec<char>,
    /// Text qualifier (`None`: no quoting).
    pub qualifier: Option<char>,
    /// A run of delimiters separates once; leading and trailing runs separate nothing.
    pub consecutive_as_one: bool,
    /// Spaces and tabs around unqualified fields and around qualifiers are part of the delimiter.
    pub trim_whitespace: bool,
}

impl Default for DelimitedOptions {
    /// Comma, `"`, no options.
    fn default() -> DelimitedOptions {
        DelimitedOptions::with(',')
    }
}

impl DelimitedOptions {
    /// One delimiter, qualifier `"`, no options.
    pub fn with(delimiter: char) -> DelimitedOptions {
        DelimitedOptions {
            delimiters: vec![delimiter],
            qualifier: Some('"'),
            consecutive_as_one: false,
            trim_whitespace: false,
        }
    }

    /// Checks the options: at least one delimiter; none is CR, LF or the qualifier.
    pub fn validate(&self) -> Result<(), String> {
        if self.delimiters.is_empty() {
            return Err("no delimiter".to_owned());
        }
        for d in &self.delimiters {
            if matches!(d, '\r' | '\n') || Some(*d) == self.qualifier {
                return Err(format!("`{}` cannot be a delimiter", d.escape_default()));
            }
        }
        if matches!(self.qualifier, Some('\r' | '\n')) {
            return Err("a line break cannot be the qualifier".to_owned());
        }
        Ok(())
    }
}

/// Fixed-width options (TB-002).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FixedOptions {
    /// Column widths in characters; text past the last width is one more column.
    pub widths: Vec<u32>,
}

/// The table type (TB-001 *Type*, plus the built-in spreadsheet conversion of TB-004).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TableKind {
    /// Sniff delimited or fixed (T-18); workbooks are recognized by content.
    #[default]
    Detect,
    /// Delimited text.
    Delimited(DelimitedOptions),
    /// Fixed-width text.
    Fixed(FixedOptions),
    /// A workbook (xlsx, xlsm, xlsb, xls, ods).
    Spreadsheet,
}

impl TableKind {
    /// Stable name of the type.
    pub const fn as_str(&self) -> &'static str {
        match self {
            TableKind::Detect => "detect",
            TableKind::Delimited(_) => "delimited",
            TableKind::Fixed(_) => "fixed",
            TableKind::Spreadsheet => "spreadsheet",
        }
    }
}

/// The words of a table type (`--type`, contract §5.7): `detect`, `delimited`, `fixed`, `spreadsheet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableKindWord {
    /// `detect`.
    Detect,
    /// `delimited` (`csv`).
    Delimited,
    /// `fixed` (`fixed-width`).
    Fixed,
    /// `spreadsheet` (`workbook`).
    Spreadsheet,
}

impl TableKindWord {
    /// Parses a type word (ASCII case-insensitive, `_` = `-`).
    pub fn parse(word: &str) -> Result<TableKindWord, String> {
        match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "detect" | "auto" => Ok(TableKindWord::Detect),
            "delimited" | "csv" => Ok(TableKindWord::Delimited),
            "fixed" | "fixed-width" => Ok(TableKindWord::Fixed),
            "spreadsheet" | "workbook" => Ok(TableKindWord::Spreadsheet),
            _ => Err(format!(
                "unknown table type `{word}` (expected detect, delimited, fixed or spreadsheet)"
            )),
        }
    }

    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            TableKindWord::Detect => "detect",
            TableKindWord::Delimited => "delimited",
            TableKindWord::Fixed => "fixed",
            TableKindWord::Spreadsheet => "spreadsheet",
        }
    }
}

/// The character a word names: one character, or `tab` (`\t`), `space`, `comma`, `semicolon`, `colon`, `pipe`,
/// `nbsp` (U+00A0), `nnbsp` (U+202F), `u+XXXX`.
fn char_word(word: &str) -> Option<char> {
    let mut chars = word.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(c);
    }
    let lower = word.to_ascii_lowercase();
    Some(match lower.as_str() {
        "tab" | "\\t" => '\t',
        "space" => ' ',
        "comma" => ',',
        "semicolon" => ';',
        "colon" => ':',
        "pipe" => '|',
        "nbsp" => '\u{a0}',
        "nnbsp" => '\u{202f}',
        _ => {
            let hex = lower.strip_prefix("u+")?;
            char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
        }
    })
}

/// Parses delimiters (`--delimiter`, contract §5.7): a character word (`,`, `tab`, `\t`, `space`, `pipe`, …),
/// or several characters written together (`,;`), each a delimiter. CR, LF and duplicates are refused.
pub fn parse_delimiters(text: &str) -> Result<Vec<char>, String> {
    if text.is_empty() {
        return Err("empty delimiter (expected a character, tab, space, pipe, …)".to_owned());
    }
    let chars: Vec<char> = match char_word(text) {
        Some(c) => vec![c],
        None if text.chars().any(char::is_alphanumeric) => {
            return Err(format!(
                "unknown delimiter `{text}` (expected a character, several written together, tab, space, comma, \
                 semicolon, colon, pipe or u+XXXX)"
            ));
        }
        None => text.chars().collect(),
    };
    let mut out: Vec<char> = Vec::new();
    for c in chars {
        if matches!(c, '\r' | '\n') {
            return Err("a line break cannot be a delimiter".to_owned());
        }
        if out.contains(&c) {
            return Err(format!("delimiter `{}` given twice", c.escape_default()));
        }
        out.push(c);
    }
    Ok(out)
}

/// Parses fixed column widths (`--widths`, §5.7): `10;20;8` or `10,20,8`; every width at least 1.
pub fn parse_widths(text: &str) -> Result<Vec<u32>, String> {
    let sep = if text.contains(';') { ';' } else { ',' };
    text.split(sep)
        .map(|w| {
            let w = w.trim();
            w.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| {
                format!("`{w}` is not a column width (a whole number ≥ 1, as in 10;20;8)")
            })
        })
        .collect()
}

/// Parses a text qualifier (`--qualifier`, §5.7): one character (or a character word: `tab`, `space`, `u+XXXX`, …), or `none`.
pub fn parse_qualifier(text: &str) -> Result<Option<char>, String> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    match char_word(t) {
        Some(c) if !matches!(c, '\r' | '\n') => Ok(Some(c)),
        _ => Err(format!(
            "`{text}` is not a qualifier (expected one character, as in \", or none)"
        )),
    }
}

/// Parses a separator of the regional settings (`--decimal`, `--thousands`, `--date-sep`, §5.7): one character (or
/// a character word: `space`, `nbsp`, `nnbsp`, …); `empty` is the word for no separator (`none` for thousands, `any`
/// for dates), `None` when the word may not be empty.
pub fn parse_separator(text: &str, empty: Option<&str>) -> Result<Option<char>, String> {
    let t = text.trim_matches(|c: char| c == '\r' || c == '\n');
    if let Some(word) = empty
        && t.eq_ignore_ascii_case(word)
    {
        return Ok(None);
    }
    match char_word(t) {
        Some(c) if !c.is_ascii_digit() && !matches!(c, '\r' | '\n') => Ok(Some(c)),
        _ => Err(match empty {
            Some(word) => format!(
                "`{text}` is not a separator (expected one character, space, nbsp or {word})"
            ),
            None => format!("`{text}` is not a separator (expected one character)"),
        }),
    }
}

/// A table format (contract §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct TableFormat {
    /// Type.
    pub kind: TableKind,
    /// First line.
    pub first_line: FirstLine,
    /// Regional settings.
    pub regional: Regional,
    /// Encoding label (`None`: Auto — BOM, UTF-8, detection).
    pub encoding: Option<String>,
    /// A 0x1A ends the data.
    pub ctrl_z_eof: bool,
}

impl TableFormat {
    /// The preset for a file name: `*.csv` comma, `*.tsv`/`*.tab` tab, workbook extensions Spreadsheet, else
    /// Detect.
    pub fn from_name(name: &str) -> TableFormat {
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
        let kind = match ext {
            "csv" => TableKind::Delimited(DelimitedOptions::with(',')),
            "tsv" | "tab" => TableKind::Delimited(DelimitedOptions::with('\t')),
            "xlsx" | "xlsm" | "xlsb" | "xls" | "xla" | "xlam" | "ods" => TableKind::Spreadsheet,
            _ => TableKind::Detect,
        };
        TableFormat {
            kind,
            ..TableFormat::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_of_the_format() {
        assert_eq!(
            TableKindWord::parse("Fixed_Width"),
            Ok(TableKindWord::Fixed)
        );
        assert!(
            TableKindWord::parse("xml")
                .unwrap_err()
                .contains("spreadsheet")
        );
        assert_eq!(parse_delimiters(","), Ok(vec![',']));
        assert_eq!(parse_delimiters("tab"), Ok(vec!['\t']));
        assert_eq!(parse_delimiters("\\t"), Ok(vec!['\t']));
        assert_eq!(parse_delimiters(",;"), Ok(vec![',', ';']));
        assert_eq!(parse_delimiters("pipe"), Ok(vec!['|']));
        assert_eq!(parse_delimiters("u+1f"), Ok(vec!['\u{1f}']));
        assert!(parse_delimiters(",,").is_err());
        assert!(parse_delimiters("\n").is_err());
        assert!(parse_delimiters("words").is_err());
        assert!(parse_delimiters("").is_err());
        assert_eq!(parse_widths("10;20;8"), Ok(vec![10, 20, 8]));
        assert_eq!(parse_widths("3,4"), Ok(vec![3, 4]));
        assert!(parse_widths("3;0").is_err());
        assert!(parse_widths("a").is_err());
        assert_eq!(parse_qualifier("NONE"), Ok(None));
        assert_eq!(parse_qualifier("'"), Ok(Some('\'')));
        assert!(parse_qualifier("ab").is_err());
        assert_eq!(parse_separator("none", Some("none")), Ok(None));
        assert_eq!(parse_separator("nnbsp", Some("none")), Ok(Some('\u{202f}')));
        assert_eq!(parse_separator(",", None), Ok(Some(',')));
        assert!(parse_separator("any", None).is_err());
        assert!(parse_separator("1", Some("none")).is_err());
    }
}

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
                return Err(format!(
                    "`{}` cannot be a delimiter",
                    d.escape_default()
                ));
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

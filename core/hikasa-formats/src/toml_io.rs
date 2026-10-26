//! Reading and writing format files: [`FORMAT_SCHEMA_VERSION`], [`parse_format`], [`to_toml`] and
//! [`unknown_keys`] (FF-004, SES-052, D-31).

use hikasa_core::schema::check_schema_version;
use hikasa_core::warning::{Warning, WarningCode};
use toml::{Table, Value};

use crate::error::FormatError;
use crate::grammar::RAW_FIELDS;
use crate::model::FileFormat;

/// TOML schema version of format files.
pub const FORMAT_SCHEMA_VERSION: u32 = 1;

/// Parses one format file. Unknown keys become warnings (D-31); a newer schema version is an error.
///
/// `origin` names the file in messages (a path, or `built-in:<file>` for presets). The `schema_version` is
/// checked before the shape, so a file written by a newer Hikasa reports [`FormatError::Schema`] rather than a
/// type error. Only the syntax and the shape are checked here; [`crate::CompiledFormat::compile`] validates
/// ranges, patterns, masks and encodings.
pub fn parse_format(toml: &str, origin: &str) -> Result<(FileFormat, Vec<Warning>), FormatError> {
    let table: Table = toml::from_str(toml).map_err(|e| FormatError::Toml {
        origin: origin.to_owned(),
        message: one_line(&e.to_string()),
    })?;
    if let Some(Value::Integer(found)) = table.get("schema_version") {
        check_schema_integer(*found, FORMAT_SCHEMA_VERSION, origin)?;
    }
    let format: FileFormat = toml::from_str(toml).map_err(|e| FormatError::Toml {
        origin: origin.to_owned(),
        message: one_line(&e.to_string()),
    })?;
    let warnings = unknown_keys(&table)
        .into_iter()
        .map(|key| Warning {
            code: WarningCode::UnknownConfigKey,
            message: format!("{origin}: unknown key `{key}`"),
            spec: None,
        })
        .collect();
    Ok((format, warnings))
}

/// SES-052 for a `schema_version` integer read before the shape is checked: any value above `supported`,
/// including one that does not fit a `u32` (reported as `u32::MAX`), is [`FormatError::Schema`]. Negative
/// values are left to the shape check (a type error).
pub(crate) fn check_schema_integer(
    found: i64,
    supported: u32,
    origin: &str,
) -> Result<(), FormatError> {
    if found < 0 {
        return Ok(());
    }
    let found = u32::try_from(found).unwrap_or(u32::MAX);
    check_schema_version(found, supported)
        .map(|_| ())
        .map_err(|source| FormatError::Schema {
            origin: origin.to_owned(),
            source,
        })
}

/// Canonical TOML (stable key order; round-trips: INV-FMT-1).
///
/// Keys follow the declaration order of [`FileFormat`]; grammar items omit fields at their default value.
/// Comments of the source file (such as a preset's `# Sources:` line) are not kept.
pub fn to_toml(format: &FileFormat) -> String {
    match toml::to_string_pretty(format) {
        Ok(text) => text,
        // Every field of the model is representable in TOML (string keys, no `None` outside tables), so this
        // arm is unreachable; it is kept total instead of panicking (T9).
        Err(e) => format!("# cannot serialise format {}: {e}\n", format.id),
    }
}

/// Makes a multi-line parser message (with a source excerpt) fit in one line of an error report.
fn one_line(message: &str) -> String {
    message
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

const TOP_KEYS: &[&str] = &[
    "schema_version",
    "id",
    "name",
    "description",
    "kind",
    "mask",
    "conversion",
    "encoding",
    "text",
    "elements",
    "grammar",
    "line_weight",
    "table",
];
const TEXT_KEYS: &[&str] = &[
    "tab_size",
    "tab_as_spaces",
    "chars_per_line_limit",
    "lines_independent",
    "column_based",
    "ctrl_z_eof",
    "save_trim_trailing_ws",
    "save_leading_spaces_to_tabs",
    "word_chars",
];
const ELEMENT_KEYS: &[&str] = &["case_sensitive", "important"];
const WEIGHT_KEYS: &[&str] = &["pattern", "regex", "match_case", "priority"];
const TABLE_KEYS: &[&str] = &["delimiter", "qualifier", "consecutive_as_one"];
const EXTERNAL_KEYS: &[&str] = &[
    "method",
    "load",
    "save",
    "filename_encoding",
    "timeout_secs",
];

/// Dotted paths of the keys of a format file that the model does not know (`grammar[2].foo`, `text.x`), in
/// file order of the tables and sorted key order inside each table. Values of the wrong type are reported by
/// the deserialiser, not here.
pub fn unknown_keys(table: &Table) -> Vec<String> {
    let mut out = Vec::new();
    for (key, value) in table {
        if !TOP_KEYS.contains(&key.as_str()) {
            out.push(key.clone());
            continue;
        }
        match (key.as_str(), value) {
            ("text", Value::Table(t)) => scan(t, TEXT_KEYS, "text", &mut out),
            ("table", Value::Table(t)) => scan(t, TABLE_KEYS, "table", &mut out),
            ("encoding", Value::Table(t)) => {
                let allowed: &[&str] = match t.get("mode").and_then(Value::as_str) {
                    Some("fixed") => &["mode", "label"],
                    Some("auto") => &["mode", "fallback"],
                    _ => &["mode", "fallback", "label"],
                };
                scan(t, allowed, "encoding", &mut out);
            }
            ("conversion", Value::Table(t)) => {
                let allowed: &[&str] = match t.get("method").and_then(Value::as_str) {
                    Some("none" | "sorted") => &["method"],
                    _ => EXTERNAL_KEYS,
                };
                scan(t, allowed, "conversion", &mut out);
            }
            ("elements", Value::Table(t)) => {
                for (name, info) in t {
                    if let Value::Table(info) = info {
                        scan(info, ELEMENT_KEYS, &format!("elements.{name}"), &mut out);
                    }
                }
            }
            ("grammar", Value::Array(items)) => {
                for (k, item) in items.iter().enumerate() {
                    if let Value::Table(item) = item {
                        scan(item, RAW_FIELDS, &format!("grammar[{k}]"), &mut out);
                    }
                }
            }
            ("line_weight", Value::Array(items)) => {
                for (k, item) in items.iter().enumerate() {
                    if let Value::Table(item) = item {
                        scan(item, WEIGHT_KEYS, &format!("line_weight[{k}]"), &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn scan(table: &Table, allowed: &[&str], prefix: &str, out: &mut Vec<String>) {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            out.push(format!("{prefix}.{key}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::ItemKind;
    use crate::model::{Conversion, EncodingSetting, FormatKind};

    const MINIMAL: &str = r#"
schema_version = 1
id = "demo"
name = "Demo"
kind = "text"
"#;

    #[test]
    fn minimal_format_gets_defaults() {
        let (f, warnings) = parse_format(MINIMAL, "demo.toml").unwrap();
        assert!(warnings.is_empty());
        assert_eq!(f.id.as_str(), "demo");
        assert_eq!(f.kind, FormatKind::Text);
        assert_eq!(f.encoding, EncodingSetting::Auto { fallback: None });
        assert_eq!(f.conversion, Conversion::None);
        assert_eq!(f.text.tab_size, 4);
        assert!(f.grammar.is_empty());
        let again = parse_format(&to_toml(&f), "x").unwrap().0;
        assert_eq!(again, f);
    }

    #[test]
    fn newer_schema_is_an_error() {
        let text = MINIMAL.replace(
            "schema_version = 1",
            "schema_version = 2\nfuture = { x = 1 }",
        );
        let err = parse_format(&text, "f.toml").unwrap_err();
        assert!(matches!(err, FormatError::Schema { .. }), "{err}");
        assert_eq!(
            err.to_string(),
            "f.toml: schema_version 2 is newer than the supported version 1"
        );
    }

    /// A version that does not even fit a `u32` is still "newer" (SES-052), not a type error; a negative one is
    /// a type error (wave-2 review).
    #[test]
    fn huge_schema_version_is_a_schema_error() {
        let text = MINIMAL.replace("schema_version = 1", "schema_version = 4294967296");
        let err = parse_format(&text, "f.toml").unwrap_err();
        assert!(matches!(err, FormatError::Schema { .. }), "{err}");
        assert_eq!(
            err.to_string(),
            "f.toml: schema_version 4294967295 is newer than the supported version 1"
        );
        let text = MINIMAL.replace("schema_version = 1", "schema_version = -1");
        let err = parse_format(&text, "f.toml").unwrap_err();
        assert!(matches!(err, FormatError::Toml { .. }), "{err}");
        let err = crate::registry::FormatOrder::parse(
            "schema_version = 99999999999
",
            "o.toml",
        )
        .unwrap_err();
        assert!(matches!(err, FormatError::Schema { .. }), "{err}");
    }

    #[test]
    fn older_schema_is_accepted() {
        let text = MINIMAL.replace("schema_version = 1", "schema_version = 0");
        assert_eq!(parse_format(&text, "f").unwrap().0.schema_version, 0);
    }

    #[test]
    fn unknown_keys_are_warnings() {
        let text = format!(
            "{MINIMAL}colour = 1\n[text]\ntab_size = 2\nwrap = true\n[encoding]\nmode = \"fixed\"\nlabel = \
             \"utf-8\"\nfallback = \"x\"\n[elements]\nNumber = {{ case_sensitive = false, bold = true }}\n\
             [[grammar]]\nelement = \"Comment\"\nkind = \"delimited\"\nfrom = \"#\"\nflavour = 1\n\
             [[line_weight]]\npattern = \"x\"\npriority = 2\nweight = 3\n"
        );
        let (f, warnings) = parse_format(&text, "u.toml").unwrap();
        assert_eq!(f.text.tab_size, 2);
        let messages: Vec<_> = warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "u.toml: unknown key `colour`",
                "u.toml: unknown key `elements.Number.bold`",
                "u.toml: unknown key `encoding.fallback`",
                "u.toml: unknown key `grammar[0].flavour`",
                "u.toml: unknown key `line_weight[0].weight`",
                "u.toml: unknown key `text.wrap`",
            ]
        );
        assert!(
            warnings
                .iter()
                .all(|w| w.code == WarningCode::UnknownConfigKey)
        );
    }

    #[test]
    fn errors_name_the_origin_and_the_field() {
        let err = parse_format("id = ", "bad.toml").unwrap_err();
        assert!(matches!(err, FormatError::Toml { .. }));
        assert!(err.to_string().starts_with("bad.toml: "), "{err}");
        assert!(!err.to_string().contains('\n'));
        let text =
            format!("{MINIMAL}[[grammar]]\nelement = \"K\"\nkind = \"basic\"\ntokens = [\"a\"]\n");
        let err = parse_format(&text, "g.toml").unwrap_err().to_string();
        assert!(
            err.contains("field `tokens` is not allowed for kind `basic`"),
            "{err}"
        );
        let err = parse_format(&MINIMAL.replace("demo", "Demo!"), "i.toml")
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid format id"), "{err}");
    }

    #[test]
    fn canonical_toml_round_trips_every_shape() {
        let text = r#"
schema_version = 1
id = "all"
name = "All"
description = "every field"
kind = "table"
mask = "*.a;*.b"

[conversion]
method = "external"
load = 'conv "%s" %t'
save = "back %s %t"
filename_encoding = "ansi"
timeout_secs = 0

[encoding]
mode = "auto"
fallback = "windows-1252"

[text]
tab_size = 8
tab_as_spaces = true
chars_per_line_limit = 120
lines_independent = true
column_based = true
ctrl_z_eof = true
save_trim_trailing_ws = true
save_leading_spaces_to_tabs = true
word_chars = "-$"

[elements]
Zeta = { important = false }
Alpha = { case_sensitive = false }

[[grammar]]
element = "Zeta"
kind = "lines"
heading = "PAGE"
next_lines = 2
or_line_one = true

[[grammar]]
element = "Alpha"
kind = "list"
regex = true
match_case = false
tokens = ['\d+', "x"]

[[line_weight]]
pattern = '^\s*object'
regex = true
match_case = false
priority = 4

[table]
delimiter = "\t"
qualifier = '"'
consecutive_as_one = true
"#;
        let (f, warnings) = parse_format(text, "all.toml").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(matches!(
            f.grammar[1].kind,
            ItemKind::List {
                regex: true,
                match_case: false,
                ..
            }
        ));
        let canonical = to_toml(&f);
        let (again, warnings) = parse_format(&canonical, "canonical").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}\n{canonical}");
        assert_eq!(again, f, "{canonical}");
        assert_eq!(to_toml(&again), canonical);
        assert!(
            canonical.starts_with("schema_version = 1\nid = \"all\"\n"),
            "{canonical}"
        );
    }
}

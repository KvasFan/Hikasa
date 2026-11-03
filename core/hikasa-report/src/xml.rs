//! The bytes of the XML dumps (M2 W7, D-198): one document type, `hikasa-report` in the namespace
//! `urn:hikasa:report:1` (schema `docs/schemas/hikasa-report-1.xsd`). `<?xml version="1.0" encoding="UTF-8"?>` then
//! `\n`; UTF-8 without BOM; `\n` line ends; two spaces per depth; one element per line; booleans `true`, `false`.
//!
//! Escaping: `&`, `<`, `>` always, `"` in attributes; a CR is `&#13;`, a TAB and an LF in an attribute `&#9;`,
//! `&#10;` (parsers normalise them away otherwise); a character XML 1.0 cannot carry (C0 controls except TAB, LF and
//! CR; U+FFFE, U+FFFF) is the empty element `<c x="1b"/>` in text content and its escape `\u{1b}` in an attribute,
//! where elements cannot go, and there a backslash is `\\`, so every attribute reads back to one string (D-198, W7a
//! review). Names come escaped by `escape_name` (which doubles the backslash itself) through [`name_attr`] and carry
//! `raw-name`.

use std::fmt::Write as _;
use std::io::Write;

use hikasa_core::Warning;

use crate::meta::{ReportMeta, format_zoned};

/// The XML declaration line.
pub(crate) const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// The namespace of version 1.
pub(crate) const NAMESPACE: &str = "urn:hikasa:report:1";

/// True for a character XML 1.0 cannot carry, even as a character reference.
pub(crate) const fn forbidden(c: char) -> bool {
    matches!(c, '\0'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
}

/// Appends `s` as text content.
pub(crate) fn push_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c if forbidden(c) => {
                let _ = write!(out, "<c x=\"{:x}\"/>", u32::from(c));
            }
            c => out.push(c),
        }
    }
}

/// Appends `s` as an attribute value (without the quotes): a character XML cannot carry as `\u{…}` and a backslash
/// as `\\`, as `escape_name` writes them.
pub(crate) fn push_attr_value(out: &mut String, s: &str) {
    push_attr_chars(out, s, true);
}

/// Appends an attribute value; `double_backslash` false for a name `escape_name` already escaped.
fn push_attr_chars(out: &mut String, s: &str, double_backslash: bool) {
    for c in s.chars() {
        match c {
            '\\' if double_backslash => out.push_str("\\\\"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if forbidden(c) => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
}

/// Appends ` name="value"`.
pub(crate) fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_attr_value(out, value);
    out.push('"');
}

/// Appends ` name="value"` for a name or path `escape_name` already escaped (its backslashes doubled, its controls
/// `\u{…}`): the backslash is not doubled again.
pub(crate) fn name_attr(out: &mut String, name: &str, escaped: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_attr_chars(out, escaped, false);
    out.push('"');
}

/// `true`, `false`.
pub(crate) const fn boolean(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// Appends the indentation of `depth`.
pub(crate) fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Appends `<name` at `depth`.
pub(crate) fn open(out: &mut String, depth: usize, name: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(name);
}

/// Appends `<name>text</name>\n` at `depth`.
pub(crate) fn text_element(
    out: &mut String,
    depth: usize,
    name: &str,
    attrs: &[(&str, &str)],
    text: &str,
) {
    open(out, depth, name);
    for (n, v) in attrs {
        attr(out, n, v);
    }
    out.push('>');
    push_text(out, text);
    out.push_str("</");
    out.push_str(name);
    out.push_str(">\n");
}

/// Appends `<name a="v"/>\n` at `depth`.
pub(crate) fn empty_element(out: &mut String, depth: usize, name: &str, attrs: &[(&str, &str)]) {
    open(out, depth, name);
    for (n, v) in attrs {
        attr(out, n, v);
    }
    out.push_str("/>\n");
}

/// One `side` element of the header.
pub(crate) struct XmlSide<'a> {
    /// `left`, `right`.
    pub id: &'static str,
    /// The spec.
    pub spec: String,
    /// Text only: `encoding`, `bom`, `eol`, `binary`.
    pub text: Option<[(&'static str, &'a str); 4]>,
}

/// The header of a dump.
pub(crate) struct XmlHeader<'a> {
    /// `text`, `folder`.
    pub kind: &'static str,
    /// The default title.
    pub default_title: &'static str,
    /// The two sides.
    pub sides: [XmlSide<'a>; 2],
    /// The attributes of `mode`, in the schema's order.
    pub mode: Vec<(&'static str, String)>,
    /// Why the comparison is incomplete (`status`).
    pub status: Option<&'static str>,
    /// The comparison's and the rendering's warnings.
    pub warnings: &'a [Warning],
}

/// Writes the declaration, the root's start tag and the header.
pub(crate) fn write_start(
    w: &mut dyn Write,
    meta: &ReportMeta,
    h: &XmlHeader<'_>,
) -> std::io::Result<()> {
    let mut out = String::with_capacity(1024);
    out.push_str(DECLARATION);
    out.push_str("<hikasa-report");
    attr(&mut out, "xmlns", NAMESPACE);
    attr(&mut out, "version", "1");
    attr(&mut out, "type", h.kind);
    out.push_str(">\n");
    out.push_str("  <header>\n");
    text_element(
        &mut out,
        2,
        "title",
        &[],
        meta.title.as_deref().unwrap_or(h.default_title),
    );
    if let Some(at) = &meta.generated_at {
        empty_element(
            &mut out,
            2,
            "generated",
            &[("time", &format_zoned(at)), ("by", &meta.generator)],
        );
    }
    for side in &h.sides {
        open(&mut out, 2, "side");
        attr(&mut out, "id", side.id);
        attr(&mut out, "spec", &side.spec);
        for (n, v) in side.text.iter().flatten() {
            attr(&mut out, n, v);
        }
        out.push_str("/>\n");
    }
    open(&mut out, 2, "mode");
    for (n, v) in &h.mode {
        attr(&mut out, n, v);
    }
    out.push_str("/>\n");
    text_element(
        &mut out,
        2,
        "rules",
        &[],
        meta.filters.as_deref().unwrap_or(""),
    );
    for (id, error) in [("left", &meta.errors.left), ("right", &meta.errors.right)] {
        if let Some(error) = error {
            text_element(&mut out, 2, "error", &[("side", id)], error);
        }
    }
    if let Some(why) = h.status {
        text_element(&mut out, 2, "status", &[("complete", "false")], why);
    }
    for warning in h.warnings {
        text_element(
            &mut out,
            2,
            "warning",
            &[("code", warning.code.as_str())],
            &warning.message,
        );
    }
    out.push_str("  </header>\n");
    w.write_all(out.as_bytes())
}

/// Writes the summary and the end of the root.
pub(crate) fn write_end(
    w: &mut dyn Write,
    summary_attrs: &[(&str, &str)],
    counts: &[crate::summary_counts::SummaryCount],
) -> std::io::Result<()> {
    let mut out = String::with_capacity(64 * (counts.len() + 2));
    open(&mut out, 1, "summary");
    for (n, v) in summary_attrs {
        attr(&mut out, n, v);
    }
    if counts.is_empty() {
        out.push_str("/>\n");
    } else {
        out.push_str(">\n");
        for c in counts {
            empty_element(
                &mut out,
                2,
                "count",
                &[
                    ("group", c.group),
                    ("name", c.name),
                    ("value", &c.value.to_string()),
                ],
            );
        }
        out.push_str("  </summary>\n");
    }
    out.push_str("</hikasa-report>\n");
    w.write_all(out.as_bytes())
}

/// Lower-case hex of `bytes` (`raw-name`).
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        let mut out = String::new();
        push_text(
            &mut out,
            "a&<>\"'\r\t\n\u{0}\u{1b}\u{fffe}\u{ffff}\u{202e}\u{85}]]>",
        );
        assert_eq!(
            out,
            "a&amp;&lt;&gt;\"'&#13;\t\n<c x=\"0\"/><c x=\"1b\"/><c x=\"fffe\"/><c x=\"ffff\"/>\u{202e}\u{85}]]&gt;"
        );
        let mut out = String::new();
        attr(&mut out, "n", "a\"b\tc\nd\re\u{1}<&");
        assert_eq!(out, " n=\"a&quot;b&#9;c&#10;d&#13;e\\u{1}&lt;&amp;\"");
        // A backslash is doubled, so a control and the six characters of its escape differ.
        let mut out = String::new();
        attr(&mut out, "n", "\u{1}|\\u{1}|C:\\x");
        assert_eq!(out, " n=\"\\u{1}|\\\\u{1}|C:\\\\x\"");
        // A name comes escaped: its doubled backslash stays as it is.
        let mut out = String::new();
        name_attr(&mut out, "name", "n\\\\b\\u{1}");
        assert_eq!(out, " name=\"n\\\\b\\u{1}\"");
        assert_eq!(hex(&[0xff, 0x0a]), "ff0a");
        assert!(forbidden('\u{8}') && !forbidden('\t') && !forbidden('\u{7f}'));
    }
}

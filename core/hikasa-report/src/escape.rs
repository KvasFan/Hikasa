//! Escaping: HTML text and attributes, CSV fields (RFC 4180), control characters in text output, file names in
//! text output and in patch headers. Every function borrows its input when nothing needs escaping.

use std::borrow::Cow;
use std::fmt::Write as _;

/// True for the bidirectional formatting characters (Unicode `Bidi_Control`: U+061C, U+200E, U+200F,
/// U+202A–U+202E, U+2066–U+2069). Shown raw they reorder the text around them (a name `U+202E txt.exe` displays
/// as `exe.txt`), so reports show them as escapes, like control characters.
pub(crate) const fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// True for the characters that text and HTML output show as escapes: control characters (Unicode `Cc`:
/// U+0000–U+001F and U+007F–U+009F) and bidirectional formatting characters ([`is_bidi_control`]).
pub(crate) fn is_escaped(c: char) -> bool {
    c.is_control() || is_bidi_control(c)
}

/// The visible escape of a control or bidirectional formatting character (`\n`, `\t`, `\r`, otherwise `\u{…}` in
/// lower-case hex), or `None` for any other character ([`is_escaped`]).
pub(crate) fn control_escape(c: char) -> Option<Cow<'static, str>> {
    if !is_escaped(c) {
        return None;
    }
    Some(match c {
        '\n' => Cow::Borrowed("\\n"),
        '\t' => Cow::Borrowed("\\t"),
        '\r' => Cow::Borrowed("\\r"),
        other => Cow::Owned(format!("\\u{{{:x}}}", u32::from(other))),
    })
}

/// Escapes every control and bidirectional formatting character of `s` (`\n`, `\t`, `\r`, `\u{…}`): header values
/// and other text in text output. Public (additive, wave-4 session review): adapters escape terminal and list
/// output with it (`hikasa_report::escape_controls`).
pub fn escape_controls(s: &str) -> Cow<'_, str> {
    if !s.chars().any(is_escaped) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match control_escape(c) {
            Some(e) => out.push_str(&e),
            None => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// File content written to a terminal (ANSI patches): every control and bidirectional formatting character except
/// TAB becomes its escape, as in the side-by-side report, so file content cannot move the cursor, erase lines or
/// reorder the text around it. Public (additive, wave-4 session review).
pub fn escape_terminal(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c != '\t' && is_escaped(c)) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match control_escape(c) {
            Some(e) if c != '\t' => out.push_str(&e),
            _ => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Escapes a file name (or path) for text and HTML output (§4.3.3): like [`escape_controls`], and a backslash
/// becomes `\\`, so the escaping is unambiguous: every printed name maps back to exactly one name (a name holding
/// a line feed prints as `a\nb`, a name holding a backslash and an `n` as `a\\nb`). Public (additive, wave-4 session
/// review): `vfs ls`, `gui list` and other name lists use it.
pub fn escape_name(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c == '\\' || is_escaped(c)) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c => match control_escape(c) {
                Some(e) => out.push_str(&e),
                None => out.push(c),
            },
        }
    }
    Cow::Owned(out)
}

/// HTML escaping for text nodes and quoted attribute values: `&`, `<`, `>`, `"` and `'` become entities, and
/// control characters other than TAB, and bidirectional formatting characters, become their visible escapes
/// (`\u{1b}`, `\u{202e}`), which keeps the document valid and its text in order.
pub(crate) fn html(s: &str) -> Cow<'_, str> {
    if !s
        .chars()
        .any(|c| matches!(c, '&' | '<' | '>' | '"' | '\'') || (is_escaped(c) && c != '\t'))
    {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        push_html_char(&mut out, c);
    }
    Cow::Owned(out)
}

/// Appends one character, HTML-escaped as [`html`] does.
pub(crate) fn push_html_char(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        '\t' => out.push('\t'),
        c => match control_escape(c) {
            Some(e) => out.push_str(&e),
            None => out.push(c),
        },
    }
}

/// One CSV field (RFC 4180): quoted, with `"` doubled, when it contains `,`, `"`, CR or LF.
pub(crate) fn csv_field(s: &str) -> Cow<'_, str> {
    if !s.contains([',', '"', '\r', '\n']) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 4);
    out.push('"');
    for c in s.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    Cow::Owned(out)
}

/// A file name in a patch header (`--- name`): written as is unless it contains a byte that patch readers treat
/// specially (a control byte, DEL, `"` or `\`), in which case it is C-quoted the way git and GNU diff quote names:
/// `"…"` with `\a \b \t \n \v \f \r \" \\` and other control bytes as three octal digits. Bytes ≥ 0x80 stay
/// UTF-8.
pub(crate) fn patch_name(s: &str) -> Cow<'_, str> {
    let needs = |b: u8| b < 0x20 || b == 0x7f || b == b'"' || b == b'\\';
    if !s.bytes().any(needs) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    out.push('"');
    for c in s.chars() {
        match c {
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Cow::Owned(out)
}

/// The CSS of a custom stylesheet made safe to embed in `<style>`: every `</` becomes `<\/` (in CSS an escaped
/// `/` is the same character), so the text cannot close the element; CRLF and CR line ends become LF (reports use
/// `\n`), and any other control character except TAB becomes a CSS hex escape (`\1 `), so the document holds
/// no control characters.
pub(crate) fn css_text(s: &str) -> Cow<'_, str> {
    let special = |c: char| c.is_control() && !matches!(c, '\t' | '\n');
    if !s.contains("</") && !s.chars().any(special) {
        return Cow::Borrowed(s);
    }
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if special(c) {
            let _ = write!(out, "\\{:x} ", u32::from(c));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out.replace("</", "<\\/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_escapes() {
        assert_eq!(control_escape('a'), None);
        assert_eq!(control_escape('\n').unwrap(), "\\n");
        assert_eq!(control_escape('\t').unwrap(), "\\t");
        assert_eq!(control_escape('\r').unwrap(), "\\r");
        assert_eq!(control_escape('\u{1b}').unwrap(), "\\u{1b}");
        assert_eq!(control_escape('\u{0}').unwrap(), "\\u{0}");
        assert_eq!(control_escape('\u{7f}').unwrap(), "\\u{7f}");
        assert_eq!(control_escape('\u{85}').unwrap(), "\\u{85}");
        assert_eq!(control_escape('é'), None);
        assert!(matches!(escape_controls("plain"), Cow::Borrowed("plain")));
        assert_eq!(escape_controls("a\nb\tc\u{1}"), "a\\nb\\tc\\u{1}");
    }

    #[test]
    fn bidi_controls_are_escaped() {
        for c in [
            '\u{61c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
            '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert!(is_bidi_control(c), "{c:?}");
            assert_eq!(
                control_escape(c).unwrap(),
                format!("\\u{{{:x}}}", u32::from(c))
            );
        }
        // Neighbours and ordinary right-to-left letters stay.
        for c in ['\u{200d}', '\u{2029}', '\u{2065}', '\u{206a}', 'א', 'ع'] {
            assert!(!is_bidi_control(c), "{c:?}");
        }
        assert_eq!(escape_controls("\u{202e}txt.exe"), "\\u{202e}txt.exe");
        assert_eq!(html("<\u{2067}x\u{2069}>"), "&lt;\\u{2067}x\\u{2069}&gt;");
    }

    #[test]
    fn terminal_text_keeps_tabs_only() {
        assert!(matches!(escape_terminal("a\tb é"), Cow::Borrowed(_)));
        assert_eq!(
            escape_terminal("ok\u{1b}[1A\tx\u{202e}y\r"),
            "ok\\u{1b}[1A\tx\\u{202e}y\\r"
        );
    }

    #[test]
    fn name_escaping_is_unambiguous() {
        assert!(matches!(escape_name("plain.txt"), Cow::Borrowed(_)));
        // A line feed and a backslash followed by `n` print differently.
        assert_eq!(escape_name("a\nb.txt"), "a\\nb.txt");
        assert_eq!(escape_name("a\\nb.txt"), "a\\\\nb.txt");
        assert_ne!(escape_name("a\nb.txt"), escape_name("a\\nb.txt"));
        assert_eq!(escape_name("x\\"), "x\\\\");
        assert_eq!(escape_name("\u{202e}txt.exe"), "\\u{202e}txt.exe");
        assert_eq!(escape_name("tab\there"), "tab\\there");
    }

    #[test]
    fn html_escapes() {
        assert!(matches!(html("plain text"), Cow::Borrowed(_)));
        assert_eq!(
            html("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(html("tab\there"), "tab\there");
        assert_eq!(html("bell\u{7}"), "bell\\u{7}");
        assert_eq!(html("ünïcode ✓"), "ünïcode ✓");
    }

    #[test]
    fn csv_quoting() {
        assert!(matches!(csv_field("plain"), Cow::Borrowed(_)));
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
        assert_eq!(csv_field("cr\r"), "\"cr\r\"");
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn patch_names() {
        assert!(matches!(patch_name("/a/b c.txt"), Cow::Borrowed(_)));
        assert_eq!(patch_name("C:\\w\\x.c"), "\"C:\\\\w\\\\x.c\"");
        assert_eq!(patch_name("a\tb"), "\"a\\tb\"");
        assert_eq!(patch_name("q\"x"), "\"q\\\"x\"");
        assert_eq!(patch_name("x\u{1}y"), "\"x\\001y\"");
        assert_eq!(patch_name("del\u{7f}"), "\"del\\177\"");
        assert_eq!(patch_name("ü\\"), "\"ü\\\\\"");
    }

    #[test]
    fn css_is_neutralised() {
        assert!(matches!(css_text("a { color: red }"), Cow::Borrowed(_)));
        assert_eq!(css_text("</style><script>"), "<\\/style><script>");
        assert_eq!(css_text("a {}\r\nb {}\r"), "a {}\nb {}\n");
        assert_eq!(css_text("x\u{1}y\tz"), "x\\1 y\tz");
    }
}

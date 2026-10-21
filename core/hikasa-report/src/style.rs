//! Presentation constants: ANSI SGR sequences for coloured text output, the embedded stylesheets of the HTML
//! reports (colour and monochrome, RPT-002; RPT-010 colour semantics; FC-031 folder colours), and the document
//! skeleton every HTML report shares (self-contained: no scripts, no external resources, no generated ids).

use std::io::Write;

use crate::escape;

/// ANSI SGR sequences (text output with `ansi = true`).
pub(crate) struct Sgr;

impl Sgr {
    /// Reset.
    pub const RESET: &'static str = "\x1b[0m";
    /// Bold (titles, file headers).
    pub const BOLD: &'static str = "\x1b[1m";
    /// Red: important differences, newer/different items.
    pub const RED: &'static str = "\x1b[31m";
    /// Blue: unimportant differences.
    pub const BLUE: &'static str = "\x1b[34m";
    /// Magenta: orphans (purple, FC-031).
    pub const MAGENTA: &'static str = "\x1b[35m";
    /// Cyan: hunk headers; filtered items (teal, FC-031).
    pub const CYAN: &'static str = "\x1b[36m";
    /// Grey: older or unknown items; hidden-line rows.
    pub const GREY: &'static str = "\x1b[90m";
    /// Bold red: errors.
    pub const ERROR: &'static str = "\x1b[1;31m";
}

/// Layout rules shared by every HTML report.
const BASE: &str = "\
body { margin: 1em; font-family: sans-serif; font-size: 10pt; color: #000000; background: #ffffff; }
h1 { font-size: 14pt; margin: 0 0 0.5em 0; }
h2 { font-size: 12pt; margin: 1em 0 0.4em 0; }
table { border-collapse: collapse; }
table.meta { margin-bottom: 1em; }
table.meta th { text-align: left; vertical-align: top; padding: 0.1em 1em 0.1em 0; }
table.meta td { padding: 0.1em 0; white-space: pre-wrap; overflow-wrap: anywhere; }
table.legend { margin-bottom: 1em; }
table.legend td { border: 1px solid #999999; padding: 0.15em 0.6em; }
table.summary { margin-bottom: 1em; }
table.summary th, table.summary td { border: 1px solid #999999; padding: 0.15em 0.6em; }
table.summary th { text-align: left; }
table.summary td.num { text-align: right; }
";

/// Layout of the text side-by-side table.
const TEXT_LAYOUT: &str = "\
table.sbs { width: 100%; table-layout: fixed; font-family: monospace; }
table.sbs col.n { width: 5em; }
table.sbs col.m { width: 1.5em; }
table.sbs th { text-align: left; border-bottom: 1px solid #999999; padding: 0.1em 0.3em; overflow-wrap: anywhere; }
table.sbs td { vertical-align: top; padding: 0 0.3em; }
table.sbs td.n { text-align: right; color: #808080; }
table.sbs td.m { text-align: center; }
table.sbs td.t { white-space: pre-wrap; overflow-wrap: anywhere; }
table.sbs tr.gap td { text-align: center; font-style: italic; padding: 0.15em; }
";

/// RPT-010 colours: important red, unimportant blue, orphan, replacement, hidden lines.
const TEXT_COLOUR: &str = "\
tr.imp td.t { background: #ffe8e8; }
tr.unimp td.t { background: #e8eeff; }
tr.lo td.t.l, tr.ro td.t.r, td.lo, td.ro { background: #f6e8fa; }
tr.lo.imp td.t.l, tr.ro.imp td.t.r { color: #c00000; }
tr.lo.unimp td.t.l, tr.ro.unimp td.t.r { color: #0030c0; }
span.imp, td.imp { color: #c00000; background: #ffc8c8; }
span.unimp, td.unimp { color: #0030c0; background: #c8d4ff; }
span.repl, td.repl { color: #805000; background: #ffe8b0; }
tr.gap td, td.gap { background: #eeeeee; color: #555555; }
td.x { background: #f2f2f2; }
";

/// The monochrome variant (html-mono): emphasis instead of colour.
const TEXT_MONO: &str = "\
tr.imp td.m, tr.unimp td.m { font-weight: bold; }
tr.lo td.t.l, tr.ro td.t.r, td.lo, td.ro { border-left: 2px solid #000000; }
tr.lo.imp td.t.l, tr.ro.imp td.t.r { font-weight: bold; }
tr.lo.unimp td.t.l, tr.ro.unimp td.t.r { font-style: italic; }
span.imp, td.imp { font-weight: bold; text-decoration: underline; }
span.unimp, td.unimp { font-style: italic; text-decoration: underline dotted; }
span.repl, td.repl { text-decoration: underline wavy; }
tr.gap td, td.gap { font-style: italic; border-top: 1px dashed #000000; border-bottom: 1px dashed #000000; }
td.x { background: repeating-linear-gradient(135deg, #ffffff 0, #ffffff 3px, #cccccc 3px, #cccccc 4px); }
";

/// Layout of the folder side-by-side table.
const FOLDER_LAYOUT: &str = "\
table.fsbs { font-family: monospace; }
table.fsbs th { text-align: left; border-bottom: 1px solid #999999; padding: 0.1em 0.5em; }
table.fsbs td { padding: 0 0.5em; white-space: pre; vertical-align: top; }
table.fsbs td.num { text-align: right; }
table.fsbs td.m { text-align: center; }
";

/// FC-031 colours: black same, red newer/different, purple orphan, grey older/unknown, teal filtered.
const FOLDER_COLOUR: &str = "\
.c-same { color: #000000; }
.c-newer { color: #c00000; }
.c-unimp { color: #0030c0; }
.c-orphan { color: #800080; }
.c-older { color: #808080; }
.c-filtered { color: #008080; }
.c-error { color: #a04000; font-weight: bold; }
";

/// The monochrome folder variant.
const FOLDER_MONO: &str = "\
.c-newer { font-weight: bold; }
.c-unimp { font-style: italic; }
.c-orphan { font-style: italic; text-decoration: underline; }
.c-older { color: #808080; }
.c-filtered { text-decoration: line-through; }
.c-error { font-weight: bold; text-decoration: underline wavy; }
";

/// Which report a stylesheet serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sheet {
    /// Text reports.
    Text,
    /// Folder reports.
    Folder,
}

/// The built-in stylesheet.
pub(crate) fn stylesheet(sheet: Sheet, mono: bool) -> String {
    let (layout, palette) = match (sheet, mono) {
        (Sheet::Text, false) => (TEXT_LAYOUT, TEXT_COLOUR),
        (Sheet::Text, true) => (TEXT_LAYOUT, TEXT_MONO),
        (Sheet::Folder, false) => (FOLDER_LAYOUT, FOLDER_COLOUR),
        (Sheet::Folder, true) => (FOLDER_LAYOUT, FOLDER_MONO),
    };
    let mut css = String::with_capacity(BASE.len() + layout.len() + palette.len());
    css.push_str(BASE);
    css.push_str(layout);
    css.push_str(palette);
    css
}

/// Writes `<!DOCTYPE html>` … `<body>`: charset, generator, title, the built-in stylesheet and, after it, the
/// custom one (so that its rules win).
pub(crate) fn html_start(
    w: &mut dyn Write,
    title: &str,
    generator: &str,
    sheet: Sheet,
    mono: bool,
    custom_css: Option<&str>,
) -> std::io::Result<()> {
    writeln!(w, "<!DOCTYPE html>")?;
    writeln!(w, "<html lang=\"en\">")?;
    writeln!(w, "<head>")?;
    writeln!(w, "<meta charset=\"utf-8\">")?;
    if !generator.is_empty() {
        writeln!(
            w,
            "<meta name=\"generator\" content=\"{}\">",
            escape::html(generator)
        )?;
    }
    writeln!(w, "<title>{}</title>", escape::html(title))?;
    writeln!(w, "<style>")?;
    w.write_all(stylesheet(sheet, mono).as_bytes())?;
    if let Some(css) = custom_css {
        let css = escape::css_text(css);
        w.write_all(css.as_bytes())?;
        if !css.ends_with('\n') {
            writeln!(w)?;
        }
    }
    writeln!(w, "</style>")?;
    writeln!(w, "</head>")?;
    writeln!(w, "<body>")?;
    Ok(())
}

/// Writes `</body></html>`.
pub(crate) fn html_end(w: &mut dyn Write) -> std::io::Result<()> {
    writeln!(w, "</body>")?;
    writeln!(w, "</html>")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylesheets_carry_the_contract_classes() {
        let colour = stylesheet(Sheet::Text, false);
        for class in [".imp", ".unimp", ".lo", ".ro", ".gap", ".repl"] {
            assert!(colour.contains(class), "{class}");
        }
        assert!(colour.contains("#c00000") && colour.contains("#0030c0"));
        let mono = stylesheet(Sheet::Text, true);
        assert!(!mono.contains("#c00000"));
        assert!(stylesheet(Sheet::Folder, false).contains(".c-filtered"));
        assert!(!stylesheet(Sheet::Folder, true).contains("#800080"));
        for css in [colour, mono] {
            assert!(!css.contains("url("), "no external resources");
            assert!(!css.contains("@import"));
        }
    }

    #[test]
    fn document_skeleton() {
        let mut out = Vec::new();
        html_start(
            &mut out,
            "A <b>",
            "hikasa \"x\"",
            Sheet::Text,
            false,
            Some("p { color: red }</style>"),
        )
        .unwrap();
        html_end(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n"
        ));
        assert!(s.contains("<meta name=\"generator\" content=\"hikasa &quot;x&quot;\">"));
        assert!(s.contains("<title>A &lt;b&gt;</title>"));
        assert!(s.contains("p { color: red }<\\/style>\n</style>"));
        assert!(s.ends_with("</body>\n</html>\n"));
        assert!(!s.contains("<script"));
    }
}

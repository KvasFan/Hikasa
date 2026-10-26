//! [`ReportMeta`] and the RPT-004 header: title, generation time (injected by the caller, D-24), mode, rules and
//! filters in effect, the specs of both sides, per-side errors and warnings. The wording is fixed English
//! (deterministic, no locale, §3.4).

use std::io::Write;

use hikasa_core::{CaseSensitivity, ContentMethod, MaskOptions, NameFilter, Pair, Warning};
use hikasa_folder::{
    CaseMode, DateCutoff, FolderComparison, FolderDisplayFilter, FolderMode, FolderRules,
    FolderToggles,
};
use hikasa_text::{TextComparison, TextRules};

use crate::escape;
use crate::folder::FolderReportOptions;
use crate::options::DisplayMode;
use crate::style::Sgr;
use crate::text::{PatchStyle, TextLayout, TextReportOptions};

/// Report header data (RPT-004: title, produced timestamp, mode, filters in effect, specs, per-side errors).
/// `generated_at = None` omits the line (golden tests). The specs come from the result model itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMeta {
    /// Title (default "Text Compare" / "Folder Compare").
    pub title: Option<String>,
    /// Generation time and zone, injected by the caller (D-24).
    pub generated_at: Option<jiff::Zoned>,
    /// "hikasa 0.1.0".
    pub generator: String,
    /// One-line mode summary ("display: differences; ignore unimportant: no"); `None` omits the line.
    pub mode: Option<String>,
    /// One-line summary of the rules/filters in effect ("size, timestamps (2 s), content: rules;
    /// filters: *.c;-obj\"); `None` omits the line.
    pub filters: Option<String>,
    /// Per-side load errors (a side that failed to load in a GUI view); M1 CLI reports have none.
    pub errors: Pair<Option<String>>,
}

impl ReportMeta {
    /// Header for a text report: `mode` from `opts`, `filters` from the rules' importance and alignment
    /// summary (deterministic wording, §3.4).
    pub fn for_text(
        cmp: &TextComparison,
        opts: &TextReportOptions,
        generated_at: Option<jiff::Zoned>,
        generator: &str,
    ) -> ReportMeta {
        ReportMeta {
            title: None,
            generated_at,
            generator: generator.to_owned(),
            mode: Some(text_mode(opts)),
            filters: Some(text_rules_summary(cmp)),
            errors: Pair::new(None, None),
        }
    }

    /// The same header with `title` (the CLI's `--title`; additive, wave 4).
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> ReportMeta {
        self.title = Some(title.into());
        self
    }

    /// Header for a folder report: `mode` from `opts.display`, `filters` from criteria, name and other filters.
    pub fn for_folder(
        cmp: &FolderComparison,
        opts: &FolderReportOptions,
        generated_at: Option<jiff::Zoned>,
        generator: &str,
    ) -> ReportMeta {
        ReportMeta {
            title: None,
            generated_at,
            generator: generator.to_owned(),
            mode: Some(folder_mode(opts)),
            filters: Some(folder_rules_summary(cmp.rules())),
            errors: Pair::new(None, None),
        }
    }
}

/// `yes` / `no`.
fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// The `mode` line of a text report: the effective display filter of the layout and ignore-unimportant.
fn text_mode(opts: &TextReportOptions) -> String {
    let display = match opts.layout {
        TextLayout::AnnotatedUnified { lines }
        | TextLayout::Patch(PatchStyle::Unified { lines })
        | TextLayout::Patch(PatchStyle::Context { lines }) => {
            DisplayMode::Context { lines }.describe()
        }
        TextLayout::Patch(PatchStyle::Normal) => DisplayMode::Mismatches.describe(),
        TextLayout::SideBySide | TextLayout::Summary | TextLayout::Statistics => {
            opts.display.describe()
        }
    };
    format!(
        "display: {display}; ignore unimportant: {}",
        yes_no(opts.ignore_unimportant)
    )
}

/// The `filters` line of a text report: format, algorithm, importance and alignment rules.
pub(crate) fn text_rules_summary(cmp: &TextComparison) -> String {
    let rules: &TextRules = cmp.rules();
    let imp = &rules.importance;
    let align = &rules.alignment;
    let mut unimportant: Vec<String> = Vec::new();
    let ws = [
        (imp.leading_whitespace, "leading whitespace"),
        (imp.embedded_whitespace, "embedded whitespace"),
        (imp.trailing_whitespace, "trailing whitespace"),
    ];
    if ws.iter().all(|(important, _)| !important) {
        unimportant.push("whitespace".to_owned());
    } else {
        unimportant.extend(
            ws.iter()
                .filter(|(important, _)| !important)
                .map(|(_, name)| (*name).to_owned()),
        );
    }
    if !imp.everything_else {
        unimportant.push("everything else".to_owned());
    }
    if !imp.everything_else_case_sensitive {
        unimportant.push("case".to_owned());
    }
    for (name, important) in &imp.elements {
        if !important {
            unimportant.push(format!("element {name}"));
        }
    }
    for p in &imp.unimportant {
        let kind = if p.regex { "regex" } else { "text" };
        let case = if p.match_case { "" } else { " (any case)" };
        unimportant.push(format!("{kind} {:?}{case}", p.pattern));
    }
    let unimportant = if unimportant.is_empty() {
        "none".to_owned()
    } else {
        unimportant.join(", ")
    };
    let closeness = if align.closeness {
        format!("{}‰", align.closeness_threshold_permille)
    } else {
        "off".to_owned()
    };
    format!(
        "format: {}; algorithm: {} ({}); skew: {}; closeness: {closeness}; never align: {}; unimportant: {unimportant}; \
         line endings: {}; orphan lines: {}",
        cmp.format().name(),
        cmp.algorithm().as_str(),
        align.algorithm.as_str(),
        align.skew_tolerance,
        yes_no(align.never_align_differences),
        if imp.compare_line_endings {
            "compared"
        } else {
            "ignored"
        },
        if imp.orphans_always_important {
            "always important"
        } else {
            "important unless trivial"
        },
    )
}

/// Words of a folder display filter.
pub(crate) fn folder_filter_words(filter: FolderDisplayFilter) -> String {
    use FolderDisplayFilter as F;
    match filter {
        F::All => "all".to_owned(),
        F::Differences => "differences".to_owned(),
        F::Same => "same".to_owned(),
        F::NoOrphans => "no orphans".to_owned(),
        F::DifferencesNoOrphans => "differences without orphans".to_owned(),
        F::Orphans => "orphans".to_owned(),
        F::LeftNewer => "left newer".to_owned(),
        F::RightNewer => "right newer".to_owned(),
        F::LeftNewerAndLeftOrphans => "left newer and left orphans".to_owned(),
        F::RightNewerAndRightOrphans => "right newer and right orphans".to_owned(),
        F::LeftOrphans => "left orphans".to_owned(),
        F::RightOrphans => "right orphans".to_owned(),
        F::None => "none".to_owned(),
        F::Toggles { .. } => {
            let names = toggle_names(filter.toggles());
            if names.is_empty() {
                "toggles (nothing)".to_owned()
            } else {
                format!("toggles ({})", names.join(", ").replace('_', " "))
            }
        }
    }
}

/// The snake_case names of a toggle set in bit order (§4.1 rule 9).
pub(crate) fn toggle_names(toggles: FolderToggles) -> Vec<&'static str> {
    [
        (FolderToggles::SAME, "same"),
        (FolderToggles::LEFT_ORPHANS, "left_orphans"),
        (FolderToggles::RIGHT_ORPHANS, "right_orphans"),
        (FolderToggles::LEFT_NEWER, "left_newer"),
        (FolderToggles::RIGHT_NEWER, "right_newer"),
        (FolderToggles::DIFFERENCES, "differences"),
    ]
    .iter()
    .filter(|(t, _)| toggles.contains(*t))
    .map(|(_, n)| *n)
    .collect()
}

/// Words of a folder mode.
pub(crate) fn folder_mode_words(mode: FolderMode) -> &'static str {
    match mode {
        FolderMode::AlwaysShowFolders => "always show folders",
        FolderMode::CompareStructure => "compare structure",
        FolderMode::OnlyFiles => "only files",
    }
}

/// The `mode` line of a folder report.
fn folder_mode(opts: &FolderReportOptions) -> String {
    let d = &opts.display;
    format!(
        "display: {}; folders: {}; ignore unimportant: {}; suppress filters: {}",
        folder_filter_words(d.filter),
        folder_mode_words(d.folders),
        yes_no(d.ignore_unimportant),
        yes_no(d.suppress_filters)
    )
}

/// The `filters` line of a folder report: criteria, name filters, other filters, and non-default alignment and
/// handling options.
pub(crate) fn folder_rules_summary(rules: &FolderRules) -> String {
    let c = &rules.criteria;
    let mut criteria: Vec<String> = Vec::new();
    if c.size {
        criteria.push("size".to_owned());
    }
    let t = &c.timestamps;
    if t.enabled {
        let mut parts = vec![format!("{} s", t.tolerance_secs)];
        if t.ignore_dst {
            parts.push("ignore DST".to_owned());
        }
        if t.ignore_timezone {
            parts.push("ignore time zone".to_owned());
        }
        if let Some(off) = t.offset {
            parts.push(format!("{} {:+} h", off.side.as_str(), off.hours));
        }
        criteria.push(format!("timestamps ({})", parts.join(", ")));
    }
    if c.filename_case {
        criteria.push("name case".to_owned());
    }
    if !c.dos_attributes.is_empty() {
        criteria.push(format!("attributes ({})", c.dos_attributes.letters()));
    }
    for (on, name) in [
        (c.unix.permissions, "permissions"),
        (c.unix.owner, "owner"),
        (c.unix.group, "group"),
    ] {
        if on {
            criteria.push(name.to_owned());
        }
    }
    if c.content.method != ContentMethod::None {
        let mut extra = Vec::new();
        if c.content.skip_if_quick_same {
            extra.push("skip if quick tests same");
        }
        if !c.content.override_quick {
            extra.push("quick tests not overridden");
        }
        let extra = if extra.is_empty() {
            String::new()
        } else {
            format!(" ({})", extra.join(", "))
        };
        criteria.push(format!("content: {}{extra}", c.content.method.as_str()));
    }
    let mut parts = vec![if criteria.is_empty() {
        "no criteria".to_owned()
    } else {
        criteria.join(", ")
    }];
    let names = &rules.filters.names;
    let masks = NameFilter::from_lists(
        names,
        MaskOptions {
            case: CaseSensitivity::Sensitive,
        },
    )
    .map(|f| f.to_filter_string())
    .unwrap_or_else(|_| {
        let mut items: Vec<String> = Vec::new();
        items.extend(names.include_files.iter().cloned());
        items.extend(names.include_folders.iter().cloned());
        items.extend(names.exclude_files.iter().map(|m| format!("-{m}")));
        items.extend(names.exclude_folders.iter().map(|m| format!("-{m}")));
        items.join(";")
    });
    parts.push(format!(
        "filters: {}",
        if masks.is_empty() { "none" } else { &masks }
    ));
    let other = &rules.filters.other;
    if let Some(size) = other.size {
        parts.push(format!(
            "size {} {} bytes",
            cmp_symbol(size.cmp),
            size.bytes
        ));
    }
    if let Some(date) = other.date {
        let cutoff = match date.cutoff {
            DateCutoff::Absolute { at } => at.to_rfc3339(),
            DateCutoff::DaysAgo { days } => format!("{} ago", crate::options::plural(days, "day")),
        };
        parts.push(format!("modified {} {cutoff}", cmp_symbol(date.cmp)));
    }
    if !other.exclude_protected {
        parts.push("protected items included".to_owned());
    }
    match rules.alignment.case {
        CaseMode::Auto => {}
        CaseMode::Sensitive => parts.push("case-sensitive names".to_owned()),
        CaseMode::Insensitive => parts.push("case-insensitive names".to_owned()),
    }
    if rules.alignment.align_extensions {
        parts.push("align extensions".to_owned());
    }
    if rules.alignment.align_normalization {
        parts.push("align NFC/NFD names".to_owned());
    }
    let h = &rules.handling;
    if h.follow_symlinks {
        parts.push("follow symlinks".to_owned());
    }
    if h.flatten {
        parts.push("flatten".to_owned());
    }
    if !h.scan_orphan_folders {
        parts.push("orphan folders not scanned".to_owned());
    }
    parts.join("; ")
}

/// `<` or `>`.
fn cmp_symbol(cmp: hikasa_folder::Cmp) -> &'static str {
    match cmp {
        hikasa_folder::Cmp::Less => "<",
        hikasa_folder::Cmp::Greater => ">",
    }
}

/// RFC 3339 with the zone's offset ("2026-10-21T14:05:00+02:00"), the `Generated:` value.
pub(crate) fn format_zoned(z: &jiff::Zoned) -> String {
    z.timestamp().display_with_offset(z.offset()).to_string()
}

/// A rendered RPT-004 header: a title and labelled lines, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    /// Title.
    pub title: String,
    /// `(label, value)` lines.
    pub lines: Vec<(&'static str, String)>,
}

impl Header {
    /// The common header of a report: title (or `default_title`), `Generated`, `Left`/`Right` (the given side
    /// descriptions), `Mode`, `Rules`, per-side errors, then `extra` lines and one `Warning` line per warning.
    pub fn new(
        meta: &ReportMeta,
        default_title: &str,
        sides: Pair<String>,
        extra: Vec<(&'static str, String)>,
        warnings: &[Warning],
    ) -> Header {
        let mut lines = Vec::new();
        if let Some(at) = &meta.generated_at {
            let mut value = format_zoned(at);
            if !meta.generator.is_empty() {
                value.push_str(" by ");
                value.push_str(&meta.generator);
            }
            lines.push(("Generated", value));
        }
        lines.push(("Left", sides.left));
        lines.push(("Right", sides.right));
        if let Some(mode) = &meta.mode {
            lines.push(("Mode", mode.clone()));
        }
        if let Some(filters) = &meta.filters {
            lines.push(("Rules", filters.clone()));
        }
        if let Some(e) = &meta.errors.left {
            lines.push(("Left error", e.clone()));
        }
        if let Some(e) = &meta.errors.right {
            lines.push(("Right error", e.clone()));
        }
        lines.extend(extra);
        for w in warnings {
            lines.push(("Warning", format!("{}: {}", w.code.as_str(), w.message)));
        }
        Header {
            title: meta
                .title
                .clone()
                .unwrap_or_else(|| default_title.to_owned()),
            lines,
        }
    }

    /// Writes the header as text, every line prefixed by `prefix` (`"# "` in patches, so that no header line can
    /// be read as patch syntax), followed by one empty line (or `prefix` alone). Control characters are escaped.
    pub fn write_text(&self, w: &mut dyn Write, prefix: &str, ansi: bool) -> std::io::Result<()> {
        let (bold, reset) = if ansi {
            (Sgr::BOLD, Sgr::RESET)
        } else {
            ("", "")
        };
        writeln!(
            w,
            "{prefix}{bold}{}{reset}",
            escape::escape_controls(&self.title)
        )?;
        for (label, value) in &self.lines {
            writeln!(w, "{prefix}{label}: {}", escape::escape_controls(value))?;
        }
        writeln!(w, "{}", prefix.trim_end())?;
        Ok(())
    }

    /// Writes `<h1>` and the `meta` table.
    pub fn write_html(&self, w: &mut dyn Write) -> std::io::Result<()> {
        writeln!(w, "<h1>{}</h1>", escape::html(&self.title))?;
        writeln!(w, "<table class=\"meta\">")?;
        for (label, value) in &self.lines {
            let class = match *label {
                "Left error" | "Right error" => " class=\"error\"",
                "Warning" => " class=\"warning\"",
                _ => "",
            };
            writeln!(
                w,
                "<tr{class}><th>{}</th><td>{}</td></tr>",
                escape::html(label),
                escape::html(value)
            )?;
        }
        writeln!(w, "</table>")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::WarningCode;

    fn meta() -> ReportMeta {
        ReportMeta {
            title: None,
            generated_at: None,
            generator: "hikasa test".into(),
            mode: Some("display: all; ignore unimportant: no".into()),
            filters: Some("format: Everything Else".into()),
            errors: Pair::new(None, Some("right side: not found".into())),
        }
    }

    #[test]
    fn header_lines_in_order() {
        let mut m = meta();
        m.generated_at = Some(
            "2026-10-21T12:00:00Z"
                .parse::<jiff::Timestamp>()
                .unwrap()
                .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(2))),
        );
        let warnings = [Warning {
            code: WarningCode::LossyDecode,
            message: "3 malformed sequences".into(),
            spec: None,
        }];
        let h = Header::new(
            &m,
            "Text Compare",
            Pair::new("/l/a (UTF-8, lf)".into(), "/r/a (UTF-8, cr_lf)".into()),
            vec![("Status", "incomplete".into())],
            &warnings,
        );
        let mut out = Vec::new();
        h.write_text(&mut out, "", false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Text Compare\n\
             Generated: 2026-10-21T14:00:00+02:00 by hikasa test\n\
             Left: /l/a (UTF-8, lf)\n\
             Right: /r/a (UTF-8, cr_lf)\n\
             Mode: display: all; ignore unimportant: no\n\
             Rules: format: Everything Else\n\
             Right error: right side: not found\n\
             Status: incomplete\n\
             Warning: lossy_decode: 3 malformed sequences\n\
             \n"
        );
        let mut out = Vec::new();
        h.write_text(&mut out, "# ", false).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.lines().all(|l| l.starts_with('#')), "{text}");
        assert!(text.ends_with("\n#\n"));
    }

    #[test]
    fn header_omits_absent_lines_and_escapes() {
        let m = ReportMeta {
            title: Some("my\ntitle <x>".into()),
            generated_at: None,
            generator: String::new(),
            mode: None,
            filters: None,
            errors: Pair::new(None, None),
        };
        let h = Header::new(
            &m,
            "Text Compare",
            Pair::new("a".into(), "b".into()),
            vec![],
            &[],
        );
        let mut out = Vec::new();
        h.write_text(&mut out, "", false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "my\\ntitle <x>\nLeft: a\nRight: b\n\n"
        );
        let mut out = Vec::new();
        h.write_html(&mut out).unwrap();
        let html = String::from_utf8(out).unwrap();
        assert!(
            html.starts_with("<h1>my\\ntitle &lt;x&gt;</h1>\n"),
            "{html}"
        );
        assert!(html.contains("<tr><th>Left</th><td>a</td></tr>"));
    }

    #[test]
    fn generated_with_offset_and_generator() {
        let z = "2026-01-02T03:04:05.5Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC);
        assert_eq!(format_zoned(&z), "2026-01-02T03:04:05.5+00:00");
        let mut m = meta();
        m.generated_at = Some(z);
        m.generator.clear();
        let h = Header::new(
            &m,
            "T",
            Pair::new(String::new(), String::new()),
            vec![],
            &[],
        );
        assert_eq!(
            h.lines[0],
            ("Generated", "2026-01-02T03:04:05.5+00:00".into())
        );
    }

    #[test]
    fn folder_rule_wording() {
        let mut rules = FolderRules::default();
        assert_eq!(
            folder_rules_summary(&rules),
            "size, timestamps (2 s); filters: none"
        );
        rules.criteria.content.method = ContentMethod::Rules;
        rules.filters.names.include_files = vec!["*.c".into()];
        rules.filters.names.exclude_folders = vec!["obj".into()];
        assert_eq!(
            folder_rules_summary(&rules),
            "size, timestamps (2 s), content: rules; filters: *.c;-obj\\"
        );
        rules.criteria.size = false;
        rules.criteria.timestamps.enabled = false;
        rules.criteria.content.method = ContentMethod::None;
        rules.filters.names = Default::default();
        rules.filters.other.exclude_protected = false;
        rules.handling.flatten = true;
        rules.alignment.case = CaseMode::Insensitive;
        assert_eq!(
            folder_rules_summary(&rules),
            "no criteria; filters: none; protected items included; case-insensitive names; flatten"
        );
    }

    #[test]
    fn folder_filter_wording() {
        assert_eq!(folder_filter_words(FolderDisplayFilter::All), "all");
        assert_eq!(
            folder_filter_words(FolderDisplayFilter::DifferencesNoOrphans),
            "differences without orphans"
        );
        assert_eq!(
            folder_filter_words(FolderDisplayFilter::Toggles { bits: 1 | 32 }),
            "toggles (same, differences)"
        );
        assert_eq!(
            folder_filter_words(FolderDisplayFilter::Toggles { bits: 0 }),
            "toggles (nothing)"
        );
        assert_eq!(folder_mode_words(FolderMode::OnlyFiles), "only files");
    }

    #[test]
    fn text_mode_wording() {
        let mut opts = TextReportOptions::default();
        assert_eq!(
            text_mode(&opts),
            "display: context (3 lines); ignore unimportant: no"
        );
        opts.layout = TextLayout::SideBySide;
        opts.display = DisplayMode::Mismatches;
        opts.ignore_unimportant = true;
        assert_eq!(
            text_mode(&opts),
            "display: differences; ignore unimportant: yes"
        );
        opts.layout = TextLayout::Patch(PatchStyle::Normal);
        opts.display = DisplayMode::All;
        assert_eq!(
            text_mode(&opts),
            "display: differences; ignore unimportant: yes"
        );
    }
}

//! Folder reports: the status list (CLI default, one record per line, §4.3.3), side-by-side text and HTML with
//! [`FolderColumns`] (FC-040), the summary (counts per state, FC-064) and the `hikasa.folder_compare.v1` JSON
//! document (D-29).
//!
//! Locking (§3.4, §5.2): the visible rows are collected by one traversal under one read guard
//! ([`FolderComparison::read`]); the per-row formatting then reads the model in chunks of 4096 rows, each under
//! its own guard. The traversal is linear and a finished comparison has no writer waiting for the guard, so this
//! costs nothing in M1 (recorded in §1.7.3).

mod html;
mod list;
mod sbs;
mod summary;

use std::io::{BufWriter, Write};

use hikasa_core::{Pair, Side};
use hikasa_folder::{
    Attrs, Category, ColourClass, DisplayOptions, EntryView, FolderComparison, FolderDisplayFilter,
    FolderMode, NodeFlags, NodeId, NodeKind, NodeView, TreeRead, VisibleRow,
};

use crate::error::ReportError;
use crate::escape;
use crate::meta::{Header, ReportMeta};
use crate::options::{HtmlStyle, OutputFormat};
use crate::style::Sgr;

/// Default title of folder reports.
pub(crate) const FOLDER_TITLE: &str = "Folder Compare";

/// Rows read under one read guard (§3.4).
pub(crate) const CHUNK_ROWS: usize = 4096;

bitflags::bitflags! {
    /// Folder report columns (13-reports; defaults SIZE | MODIFIED).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FolderColumns: u16 {
        /// The comparison state, in words, next to the marker.
        const STATUS = 1;
        /// Size in bytes (a folder: the total of the files below it on that side, FC-040).
        const SIZE = 2;
        /// Modification time (RFC 3339 UTC).
        const MODIFIED = 4;
        /// DOS attribute letters or the Unix permission string.
        const ATTRIBUTES = 8;
        /// CRC32 (8 hex digits) when known.
        const CRC = 16;
        /// Unix owner (numeric uid in M1).
        const OWNER = 32;
        /// Unix group (numeric gid in M1).
        const GROUP = 64;
    }
}

impl Default for FolderColumns {
    /// `SIZE | MODIFIED` (13-reports).
    fn default() -> FolderColumns {
        FolderColumns::SIZE | FolderColumns::MODIFIED
    }
}

/// The per-side columns in display order, with their titles and alignment (`true` = right-aligned).
pub(crate) const SIDE_COLUMNS: [(FolderColumns, &str, bool); 6] = [
    (FolderColumns::SIZE, "Size", true),
    (FolderColumns::MODIFIED, "Modified", false),
    (FolderColumns::ATTRIBUTES, "Attributes", false),
    (FolderColumns::CRC, "CRC", false),
    (FolderColumns::OWNER, "Owner", true),
    (FolderColumns::GROUP, "Group", true),
];

impl FolderColumns {
    /// Parses a column list (additive): items separated by `,`, `;` or spaces, each `status`, `size`,
    /// `modified` (or `timestamp`, `mtime`), `attributes` (or `attrs`), `crc`, `owner`, `group`, or `none` (no
    /// column), also with the script prefix `column-` (SCR-001); ASCII case-insensitive. An empty list is the
    /// empty set. `vcs`, `revision` and `version` columns are not available in M1 (rejected).
    pub fn parse_list(list: &str) -> Result<FolderColumns, ReportError> {
        let mut cols = FolderColumns::empty();
        for item in list.split([',', ';', ' ', '\t']).filter(|s| !s.is_empty()) {
            let w = item.to_ascii_lowercase().replace('_', "-");
            let w = w.strip_prefix("column-").unwrap_or(&w);
            cols |= match w {
                "status" => FolderColumns::STATUS,
                "size" => FolderColumns::SIZE,
                "modified" | "timestamp" | "mtime" => FolderColumns::MODIFIED,
                "attributes" | "attrs" | "attribs" => FolderColumns::ATTRIBUTES,
                "crc" => FolderColumns::CRC,
                "owner" => FolderColumns::OWNER,
                "group" => FolderColumns::GROUP,
                "none" => FolderColumns::empty(),
                _ => {
                    return Err(ReportError::invalid(
                        "folder column",
                        item,
                        "status, size, modified, attributes, crc, owner, group, none",
                    ));
                }
            };
        }
        Ok(cols)
    }

    /// The set columns' names in bit order (`status`, `size`, `modified`, `attributes`, `crc`, `owner`, `group`).
    pub fn names(self) -> Vec<&'static str> {
        [
            (FolderColumns::STATUS, "status"),
            (FolderColumns::SIZE, "size"),
            (FolderColumns::MODIFIED, "modified"),
            (FolderColumns::ATTRIBUTES, "attributes"),
            (FolderColumns::CRC, "crc"),
            (FolderColumns::OWNER, "owner"),
            (FolderColumns::GROUP, "group"),
        ]
        .iter()
        .filter(|(c, _)| self.contains(*c))
        .map(|(_, n)| *n)
        .collect()
    }
}

/// Folder report layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FolderLayout {
    /// CLI default: `<marker> <path>` lines plus a `#` summary line (§4.3.3).
    List,
    /// Side-by-side table.
    SideBySide,
    /// Counts per state (13-reports).
    Summary,
    /// The `hikasa.folder_compare.v1` JSON document.
    Json,
}

impl FolderLayout {
    /// Parses `list`, `side-by-side`, `summary`, `json` (additive; ASCII case-insensitive, `_` equals `-`).
    pub fn parse(word: &str) -> Result<FolderLayout, ReportError> {
        Ok(
            match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "list" => FolderLayout::List,
                "side-by-side" | "sbs" => FolderLayout::SideBySide,
                "summary" => FolderLayout::Summary,
                "json" => FolderLayout::Json,
                _ => {
                    return Err(ReportError::invalid(
                        "folder report layout",
                        word,
                        "list, side-by-side, summary, json",
                    ));
                }
            },
        )
    }

    /// `list`, `side-by-side`, `summary`, `json`.
    pub const fn name(self) -> &'static str {
        match self {
            FolderLayout::List => "list",
            FolderLayout::SideBySide => "side-by-side",
            FolderLayout::Summary => "summary",
            FolderLayout::Json => "json",
        }
    }
}

/// Folder report options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderReportOptions {
    /// Layout.
    pub layout: FolderLayout,
    /// Display options (filter, folder mode, ignore-unimportant, suppress filters).
    pub display: DisplayOptions,
    /// Columns (SideBySide).
    pub columns: FolderColumns,
    /// Output format (HTML allowed for SideBySide and Summary).
    pub format: OutputFormat,
}

impl Default for FolderReportOptions {
    /// The list; display All with the folder structure, ignore-unimportant and suppress-filters off; columns
    /// SIZE | MODIFIED; plain text.
    fn default() -> FolderReportOptions {
        FolderReportOptions {
            layout: FolderLayout::List,
            display: DisplayOptions {
                filter: FolderDisplayFilter::All,
                folders: FolderMode::CompareStructure,
                ignore_unimportant: false,
                suppress_filters: false,
            },
            columns: FolderColumns::default(),
            format: OutputFormat::Text { ansi: false },
        }
    }
}

/// Renders a folder report into `out`.
pub fn render_folder_report(
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    out: &mut dyn Write,
) -> Result<(), ReportError> {
    render_folder_report_with(cmp, opts, meta, &HtmlStyle::default(), out)
}

/// [`render_folder_report`] with an [`HtmlStyle`] (custom CSS for the HTML layouts; additive).
pub fn render_folder_report_with(
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    style: &HtmlStyle,
    out: &mut dyn Write,
) -> Result<(), ReportError> {
    let mut w = BufWriter::with_capacity(64 * 1024, out);
    let css = style.custom_css.as_deref();
    match (opts.layout, opts.format) {
        (FolderLayout::List, OutputFormat::Text { ansi }) => list::write(&mut w, cmp, opts, ansi)?,
        (FolderLayout::SideBySide, OutputFormat::Text { ansi }) => {
            sbs::write(&mut w, cmp, opts, meta, ansi)?;
        }
        (FolderLayout::SideBySide, OutputFormat::Html { mono }) => {
            html::write(&mut w, cmp, opts, meta, mono, css)?;
        }
        (FolderLayout::Summary, OutputFormat::Text { ansi }) => {
            summary::write_text(&mut w, cmp, opts, meta, ansi)?;
        }
        (FolderLayout::Summary, OutputFormat::Html { mono }) => {
            summary::write_html(&mut w, cmp, opts, meta, mono, css)?;
        }
        (FolderLayout::Json, OutputFormat::Text { .. }) => {
            // The result document with its schema id first (§4.1 rule 10; wave-4 decision).
            let doc = crate::json::folder_compare_json(cmp, &opts.display);
            serde_json::to_writer_pretty(&mut w, &crate::json::FolderReportJson::new(&doc))
                .map_err(ReportError::from_json)?;
            w.write_all(b"\n")?;
        }
        (
            layout @ (FolderLayout::List | FolderLayout::Json),
            format @ OutputFormat::Html { .. },
        ) => {
            return Err(ReportError::Unsupported {
                layout: layout.name(),
                format: format.name(),
            });
        }
    }
    w.flush()?;
    Ok(())
}

/// The visible rows of a report (every visible folder expanded), collected under one read guard.
pub(crate) fn visible(cmp: &FolderComparison, display: &DisplayOptions) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    cmp.read().visible_rows(display, &|_| true, &mut rows);
    rows
}

/// The list marker of a node (§4.3.3): `=` same, `>` left newer, `<` right newer, `!` different, `~` different
/// only through unimportant content differences (same under ignore-unimportant), `-` left only, `+` right only,
/// `?` unknown, `E` error, `.` filtered.
pub(crate) fn marker(read: &TreeRead<'_>, id: NodeId, ignore_unimportant: bool) -> char {
    match read.category(id, ignore_unimportant) {
        Category::Same => '=',
        Category::LeftNewer => '>',
        Category::RightNewer => '<',
        Category::Different => {
            if !ignore_unimportant && read.category(id, true) == Category::Same {
                '~'
            } else {
                '!'
            }
        }
        Category::LeftOrphan => '-',
        Category::RightOrphan => '+',
        Category::Unknown => '?',
        Category::Error => 'E',
        Category::Filtered => '.',
    }
}

/// The status words of a marker.
pub(crate) fn status_words(marker: char) -> &'static str {
    match marker {
        '=' => "same",
        '>' => "left newer",
        '<' => "right newer",
        '!' => "different",
        '~' => "unimportant differences",
        '-' => "left only",
        '+' => "right only",
        '?' => "unknown",
        'E' => "error",
        _ => "filtered",
    }
}

/// The ANSI colour of a marker (FC-031: red newer/different, purple orphan, grey unknown, teal filtered).
pub(crate) fn marker_sgr(marker: char) -> &'static str {
    match marker {
        '>' | '<' | '!' => Sgr::RED,
        '~' => Sgr::BLUE,
        '-' | '+' => Sgr::MAGENTA,
        '?' => Sgr::GREY,
        'E' => Sgr::ERROR,
        '.' => Sgr::CYAN,
        _ => "",
    }
}

/// The ANSI colour of a side's colour class.
pub(crate) fn colour_sgr(colour: ColourClass) -> &'static str {
    match colour {
        ColourClass::Same | ColourClass::Absent => "",
        ColourClass::NewerOrDifferent => Sgr::RED,
        ColourClass::Orphan => Sgr::MAGENTA,
        ColourClass::OlderOrUnknown => Sgr::GREY,
        ColourClass::Filtered => Sgr::CYAN,
        ColourClass::Error => Sgr::ERROR,
    }
}

/// The CSS class of a side's colour class (HTML).
pub(crate) fn colour_class(colour: ColourClass) -> &'static str {
    match colour {
        ColourClass::Same => "c-same",
        ColourClass::NewerOrDifferent => "c-newer",
        ColourClass::Orphan => "c-orphan",
        ColourClass::OlderOrUnknown => "c-older",
        ColourClass::Filtered => "c-filtered",
        ColourClass::Error => "c-error",
        ColourClass::Absent => "c-absent",
    }
}

/// The CSS class of a marker (HTML).
pub(crate) fn marker_class(marker: char) -> &'static str {
    match marker {
        '>' | '<' | '!' => "c-newer",
        '~' => "c-unimp",
        '-' | '+' => "c-orphan",
        '?' => "c-older",
        'E' => "c-error",
        '.' => "c-filtered",
        _ => "c-same",
    }
}

/// The name cell of one side (without indentation): in flatten mode the containing folder first (FC-014 Path),
/// then the name escaped ([`escape::escape_name`]), `/` after a folder, ` -> target` after an unfollowed symlink
/// (the target with its control and bidirectional characters escaped).
pub(crate) fn name_text(node: &NodeView<'_>, entry: &EntryView<'_>) -> String {
    let mut s = String::new();
    if let Some(dir) = entry.dir.filter(|d| !d.is_empty()) {
        s.push_str(&escape::escape_name(dir));
        s.push('/');
    }
    s.push_str(&escape::escape_name(entry.name));
    if node.kind == NodeKind::Folder {
        s.push('/');
    }
    if let (Some(target), false) = (entry.link_target, entry.followed) {
        s.push_str(" -> ");
        s.push_str(&escape::escape_controls(target));
    }
    s
}

/// What a side-by-side row knows of one side's entry, beyond its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Known {
    /// The metadata (and, for a folder, the total of its subtree).
    All,
    /// The metadata, but not the folder's total: the folder has an error on that side (its listing failed), so
    /// the total of its subtree is unknown.
    NoTotal,
    /// Only the name: a name-only entry (D-45: an item hidden by a name mask, never stat'ed) or an entry whose
    /// metadata could not be read (the side's error flag with no time, no attributes and size 0: the scanner
    /// keeps only the name).
    NameOnly,
}

impl Known {
    /// What is known of `side`'s entry of `node`.
    pub(crate) fn of(node: &NodeView<'_>, entry: &EntryView<'_>, side: Side) -> Known {
        let error = node.flags.contains(NodeFlags::error(side));
        if entry.stat_skipped
            || (error
                && entry.mtime.is_none()
                && entry.size == 0
                && matches!(entry.attrs, Attrs::None))
        {
            Known::NameOnly
        } else if error && node.kind == NodeKind::Folder {
            Known::NoTotal
        } else {
            Known::All
        }
    }
}

/// The value of one per-side column for a present entry (`folder_bytes`: the side's subtree total); blank for an
/// entry of which only the name is known, and the Size of a folder whose total is unknown ([`Known`]).
pub(crate) fn column_value(
    column: FolderColumns,
    node: &NodeView<'_>,
    entry: &EntryView<'_>,
    folder_bytes: u64,
    known: Known,
) -> String {
    if known == Known::NameOnly || (known == Known::NoTotal && column == FolderColumns::SIZE) {
        return String::new();
    }
    let unix = match entry.attrs {
        Attrs::Unix { meta } => Some(meta),
        _ => None,
    };
    if column == FolderColumns::SIZE {
        if node.kind == NodeKind::Folder {
            folder_bytes.to_string()
        } else {
            entry.size.to_string()
        }
    } else if column == FolderColumns::MODIFIED {
        entry.mtime.map(|t| t.to_rfc3339()).unwrap_or_default()
    } else if column == FolderColumns::ATTRIBUTES {
        match entry.attrs {
            Attrs::Dos { attrs } => attrs.letters(),
            Attrs::Unix { meta } => meta.permissions_string(entry.kind),
            Attrs::None => String::new(),
        }
    } else if column == FolderColumns::CRC {
        entry.crc32.map(|c| format!("{c:08x}")).unwrap_or_default()
    } else if column == FolderColumns::OWNER {
        unix.map(|m| m.uid.to_string()).unwrap_or_default()
    } else if column == FolderColumns::GROUP {
        unix.map(|m| m.gid.to_string()).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Everything a side-by-side renderer shows of one visible row.
pub(crate) struct RowCells {
    /// Depth (root children = 0).
    pub depth: u32,
    /// Marker.
    pub marker: char,
    /// Name (without indentation) and the enabled per-side columns, per side; `None` for an absent side.
    pub sides: [Option<(String, Vec<String>)>; 2],
    /// Colour class per side.
    pub colours: [ColourClass; 2],
}

/// The cells of one visible row.
pub(crate) fn row_cells(
    read: &TreeRead<'_>,
    row: VisibleRow,
    columns: FolderColumns,
    ignore_unimportant: bool,
) -> Option<RowCells> {
    let node = read.node(row.node)?;
    let totals = read.totals(row.node);
    let mut sides: [Option<(String, Vec<String>)>; 2] = [None, None];
    for (i, side) in Side::INPUTS.iter().enumerate() {
        let Some(entry) = node.side(*side) else {
            continue;
        };
        let bytes = totals.get(*side).map_or(0, |t| t.bytes);
        let known = Known::of(&node, entry, *side);
        let values = SIDE_COLUMNS
            .iter()
            .filter(|(c, _, _)| columns.contains(*c))
            .map(|(c, _, _)| column_value(*c, &node, entry, bytes, known))
            .collect();
        sides[i] = Some((name_text(&node, entry), values));
    }
    Some(RowCells {
        depth: row.depth,
        marker: marker(read, row.node, ignore_unimportant),
        sides,
        colours: [
            read.colour(row.node, Side::Left, ignore_unimportant),
            read.colour(row.node, Side::Right, ignore_unimportant),
        ],
    })
}

/// The RPT-004 header of a folder report (with a `Status: incomplete` line for a cancelled scan).
pub(crate) fn header(cmp: &FolderComparison, meta: &ReportMeta) -> Header {
    let read = cmp.read();
    let specs = cmp.model().specs();
    let mut extra = Vec::new();
    if !read.is_complete() {
        extra.push((
            "Status",
            "incomplete (the comparison was cancelled)".to_owned(),
        ));
    }
    Header::new(
        meta,
        FOLDER_TITLE,
        Pair::new(specs.left.canonical(), specs.right.canonical()),
        extra,
        read.warnings(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_words() {
        assert_eq!(
            FolderColumns::default(),
            FolderColumns::SIZE | FolderColumns::MODIFIED
        );
        assert_eq!(
            FolderColumns::parse_list("size,modified,crc").unwrap(),
            FolderColumns::SIZE | FolderColumns::MODIFIED | FolderColumns::CRC
        );
        assert_eq!(
            FolderColumns::parse_list("column-size;column-timestamp column-attributes").unwrap(),
            FolderColumns::SIZE | FolderColumns::MODIFIED | FolderColumns::ATTRIBUTES
        );
        assert_eq!(
            FolderColumns::parse_list("STATUS,owner,group").unwrap(),
            FolderColumns::STATUS | FolderColumns::OWNER | FolderColumns::GROUP
        );
        assert_eq!(
            FolderColumns::parse_list("none").unwrap(),
            FolderColumns::empty()
        );
        assert_eq!(
            FolderColumns::parse_list("").unwrap(),
            FolderColumns::empty()
        );
        assert!(FolderColumns::parse_list("size,version").is_err());
        assert_eq!(
            FolderColumns::all().names(),
            [
                "status",
                "size",
                "modified",
                "attributes",
                "crc",
                "owner",
                "group"
            ]
        );
    }

    #[test]
    fn layout_words_and_markers() {
        assert_eq!(
            FolderLayout::parse("side_by_side").unwrap(),
            FolderLayout::SideBySide
        );
        assert_eq!(FolderLayout::parse("JSON").unwrap(), FolderLayout::Json);
        assert!(FolderLayout::parse("xml").is_err());
        assert_eq!(FolderLayout::Summary.name(), "summary");
        for (m, w) in [
            ('=', "same"),
            ('~', "unimportant differences"),
            ('.', "filtered"),
        ] {
            assert_eq!(status_words(m), w);
        }
        assert_eq!(marker_sgr('-'), Sgr::MAGENTA);
        assert_eq!(marker_class('E'), "c-error");
        assert_eq!(colour_class(ColourClass::Filtered), "c-filtered");
        assert_eq!(colour_sgr(ColourClass::Same), "");
        let d = FolderReportOptions::default();
        assert_eq!(d.layout, FolderLayout::List);
        assert_eq!(d.display.filter, FolderDisplayFilter::All);
    }
}

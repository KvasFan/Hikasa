//! The XML dump of a folder comparison (M2 W7, D-198): the header, a `tree` of `folder` and `entry` elements nested as
//! the comparison's tree (XPath and XSLT walk trees; the JSON report stays flat for `jq`), each with its `left` and
//! `right` (every column, as known) and, for a linked entry, its `file-report`; then the summary with
//! [`summary_counts`](crate::summary_counts). The rows are those of the display options and the scope.

use std::io::Write;

use hikasa_core::Side;
use hikasa_folder::select::Selection;
use hikasa_folder::{
    FolderComparison, FolderDisplayFilter, NodeFlags, NodeKind, TreeRead, VisibleRow,
};

use crate::error::ReportError;
use crate::escape::escape_name;
use crate::folder::{
    CHUNK_ROWS, FOLDER_TITLE, FolderColumns, FolderReportOptions, Known, column_value, marker,
    scoped,
};
use crate::links::{PairLink, PairLinks};
use crate::meta::ReportMeta;
use crate::options::folder_mode_word;
use crate::summary_counts::of_folder;
use crate::xml::{self, XmlHeader, XmlSide};

/// The `state` of a marker.
pub(crate) const fn state_word(marker: char) -> &'static str {
    match marker {
        '=' => "same",
        '>' => "left-newer",
        '<' => "right-newer",
        '!' => "different",
        '~' => "unimportant",
        '-' => "left-only",
        '+' => "right-only",
        '?' => "unknown",
        'E' => "error",
        _ => "filtered",
    }
}

/// The `display` word of a folder filter (the canonical word without `display-`).
pub(crate) const fn display_word(filter: FolderDisplayFilter) -> &'static str {
    use FolderDisplayFilter as F;
    match filter {
        F::All => "all",
        F::Differences => "mismatches",
        F::Same => "matches",
        F::NoOrphans => "no-orphans",
        F::DifferencesNoOrphans => "mismatches-no-orphans",
        F::Orphans => "orphans",
        F::LeftNewer => "left-newer",
        F::RightNewer => "right-newer",
        F::LeftNewerAndLeftOrphans => "left-newer-orphans",
        F::RightNewerAndRightOrphans => "right-newer-orphans",
        F::LeftOrphans => "left-orphans",
        F::RightOrphans => "right-orphans",
        F::None => "none",
        F::Toggles { .. } => "toggles",
    }
}

/// The columns of a side, in the schema's order, with their attribute names.
const COLUMNS: [(FolderColumns, &str); 6] = [
    (FolderColumns::SIZE, "size"),
    (FolderColumns::MODIFIED, "modified"),
    (FolderColumns::ATTRIBUTES, "attributes"),
    (FolderColumns::CRC, "crc"),
    (FolderColumns::OWNER, "owner"),
    (FolderColumns::GROUP, "group"),
];

/// Appends the element of one row (`folder` left open: its children follow).
fn push_row(
    out: &mut String,
    read: &TreeRead<'_>,
    row: VisibleRow,
    depth: usize,
    iu: bool,
    links: Option<&dyn PairLinks>,
) -> bool {
    let Some(node) = read.node(row.node) else {
        return false;
    };
    let folder = node.kind == NodeKind::Folder;
    let path = read.path(row.node);
    let first = node.side(Side::Left).or(node.side(Side::Right));
    let name = first
        .map(|e| escape_name(e.name).into_owned())
        .unwrap_or_default();
    let m = marker(read, row.node, iu);
    xml::open(out, depth, if folder { "folder" } else { "entry" });
    xml::attr(out, "name", &name);
    xml::attr(out, "path", &escape_name(path.as_str()));
    xml::attr(out, "state", state_word(m));
    let mut buf = [0u8; 4];
    xml::attr(out, "marker", m.encode_utf8(&mut buf));
    if !folder {
        let kind = match node.kind {
            NodeKind::Symlink => "link",
            NodeKind::Other => "other",
            NodeKind::File | NodeKind::Folder => "file",
        };
        xml::attr(out, "kind", kind);
    }
    if node.flags.contains(NodeFlags::IGNORED) {
        xml::attr(out, "ignored", "true");
    }
    out.push_str(">\n");
    let totals = read.totals(row.node);
    let errors = read.errors(row.node);
    for (side, element) in [(Side::Left, "left"), (Side::Right, "right")] {
        let Some(entry) = node.side(side) else {
            continue;
        };
        xml::open(out, depth + 1, element);
        let shown = escape_name(entry.name);
        xml::attr(out, "name", &shown);
        let raw = read.raw_name(row.node, side);
        let exact = raw
            .as_ref()
            .is_some_and(|raw| raw.as_encoded_bytes() == entry.name.as_bytes());
        if (entry.name_lossy || !exact || shown != entry.name)
            && let Some(raw) = &raw
        {
            xml::attr(out, "raw-name", &xml::hex(raw.as_encoded_bytes()));
        }
        let bytes = totals.get(side).map_or(0, |t| t.bytes);
        let known = Known::of(&node, entry, side);
        for (column, attr) in COLUMNS {
            let value = column_value(column, &node, entry, bytes, known);
            if !value.is_empty() {
                xml::attr(out, attr, &value);
            }
        }
        if let Some(target) = entry.link_target {
            xml::attr(out, "link-target", target);
        }
        let messages: Vec<&str> = errors
            .iter()
            .filter(|e| e.side == Some(side))
            .map(|e| e.error.message.as_str())
            .collect();
        if !messages.is_empty() {
            xml::attr(out, "error", &messages.join("; "));
        }
        out.push_str("/>\n");
    }
    if !folder && let Some(link) = links.and_then(|links| links.link(row.node)) {
        xml::open(out, depth + 1, "file-report");
        xml::attr(out, "status", link.status());
        match &link {
            PairLink::Href(href) => xml::attr(out, "href", href),
            PairLink::Skipped(why) | PairLink::Failed(why) => xml::attr(out, "reason", why),
        }
        out.push_str("/>\n");
    }
    if !folder {
        xml::indent(out, depth);
        out.push_str("</entry>\n");
    }
    folder
}

/// Writes the dump.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    scope: Option<&Selection>,
    links: Option<&dyn PairLinks>,
) -> Result<(), ReportError> {
    let d = &opts.display;
    let iu = d.ignore_unimportant;
    let mut mode = vec![
        ("display", display_word(d.filter).to_owned()),
        ("ignore-unimportant", xml::boolean(iu).to_owned()),
        ("folders", folder_mode_word(d.folders).to_owned()),
        (
            "suppress-filters",
            xml::boolean(d.suppress_filters).to_owned(),
        ),
    ];
    if let Some(selection) = scope {
        let n = selection.len();
        mode.push((
            "scope",
            format!("{n} selected {}", if n == 1 { "row" } else { "rows" }),
        ));
    }
    let specs = cmp.model().specs();
    let (warnings, complete) = {
        let read = cmp.read();
        (read.warnings().to_vec(), read.is_complete())
    };
    xml::write_start(
        w,
        meta,
        &XmlHeader {
            kind: "folder",
            default_title: FOLDER_TITLE,
            sides: [
                XmlSide {
                    id: "left",
                    spec: specs.left.canonical(),
                    text: None,
                },
                XmlSide {
                    id: "right",
                    spec: specs.right.canonical(),
                    text: None,
                },
            ],
            mode,
            status: (!complete).then_some("the comparison was cancelled"),
            warnings: &warnings,
        },
    )?;
    let rows = scoped(cmp, d, scope);
    let flat = cmp.rules().handling.flatten;
    let mut tree = String::from("  <tree");
    if flat {
        xml::attr(&mut tree, "flat", "true");
    }
    if rows.is_empty() {
        tree.push_str("/>\n");
        w.write_all(tree.as_bytes())?;
    } else {
        tree.push_str(">\n");
        w.write_all(tree.as_bytes())?;
        // The open folders: their paths, the innermost last.
        let mut open: Vec<String> = Vec::new();
        let mut out = String::new();
        for chunk in rows.chunks(CHUNK_ROWS) {
            let read = cmp.read();
            for row in chunk {
                out.clear();
                let path = read.path(row.node);
                let path = path.as_str();
                while let Some(top) = open.last() {
                    let inside = path
                        .strip_prefix(top.as_str())
                        .is_some_and(|rest| rest.starts_with('/'));
                    if inside {
                        break;
                    }
                    open.pop();
                    xml::indent(&mut out, 2 + open.len());
                    out.push_str("</folder>\n");
                }
                if push_row(&mut out, &read, *row, 2 + open.len(), iu, links) {
                    open.push(path.to_owned());
                }
                w.write_all(out.as_bytes())?;
            }
        }
        out.clear();
        while open.pop().is_some() {
            xml::indent(&mut out, 2 + open.len());
            out.push_str("</folder>\n");
        }
        out.push_str("  </tree>\n");
        w.write_all(out.as_bytes())?;
    }
    let counts = of_folder(cmp, iu);
    xml::write_end(
        w,
        &[
            ("verdict", cmp.verdict(iu).as_str()),
            ("complete", xml::boolean(complete)),
        ],
        &counts,
    )?;
    Ok(())
}

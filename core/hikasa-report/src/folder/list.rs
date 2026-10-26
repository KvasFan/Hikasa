//! The status list (§4.3.3, the CLI default): `<marker>  <path>[/]` per visible row, then
//! `# <n> same, <n> left newer, <n> right newer, <n> different, <n> left only, <n> right only, <n> unknown,
//! <n> errors` (file counts of the statistics), omitted when the comparison holds no item at all.

use std::io::Write;

use hikasa_folder::{FolderComparison, NodeKind};

use crate::error::ReportError;
use crate::escape;
use crate::folder::{CHUNK_ROWS, FolderReportOptions, marker, marker_sgr, visible};
use crate::style::Sgr;

/// Writes the list.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    ansi: bool,
) -> Result<(), ReportError> {
    let iu = opts.display.ignore_unimportant;
    let rows = visible(cmp, &opts.display);
    let mut line = String::new();
    for chunk in rows.chunks(CHUNK_ROWS) {
        let read = cmp.read();
        for row in chunk {
            let Some(node) = read.node(row.node) else {
                continue;
            };
            let m = marker(&read, row.node, iu);
            line.clear();
            let colour = if ansi { marker_sgr(m) } else { "" };
            line.push_str(colour);
            line.push(m);
            line.push_str("  ");
            line.push_str(&escape::escape_name(read.path(row.node).as_str()));
            if node.kind == NodeKind::Folder {
                line.push('/');
            }
            if !colour.is_empty() {
                line.push_str(Sgr::RESET);
            }
            line.push('\n');
            w.write_all(line.as_bytes())?;
        }
    }
    let stats = cmp.stats(iu);
    if stats.files.total() + stats.folders.total() > 0 {
        let f = &stats.files;
        writeln!(
            w,
            "# {} same, {} left newer, {} right newer, {} different, {} left only, {} right only, {} unknown, {} errors",
            f.same,
            f.left_newer,
            f.right_newer,
            f.different,
            f.left_orphan,
            f.right_orphan,
            f.unknown,
            f.error
        )?;
    }
    Ok(())
}

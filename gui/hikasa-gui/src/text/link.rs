//! The session link of a child text view (TC-052; M2 W11b, contract §1.11.20, D-311), toolkit-free: the next or
//! previous pair of difference files of the parent comparison (`parent_row`, `TreeRead::next_difference_file`), the
//! request that reopens this view with it, the parent link moved to it, and the text of *Copy File and Open Next*'s
//! steps as core plans them. The jobs (the parent compared again, the steps, their execution) and the prompts are the
//! window's (`app/link.rs`).

use hikasa_core::Side;
use hikasa_folder::NodeId;
use hikasa_folder::view::{Dir, NextFileOptions};
use hikasa_session::api::{
    CopyFileSteps, PreparedFolderCompare, TextCompareRequest, parent_link_of, parent_row,
    spec_os_string,
};
use hikasa_session::sessions::ParentLink;

/// The message when no pair is left (D-311).
pub const NO_MORE_FILES: &str = "No more difference files.";
/// The message when the parent no longer has the child's row.
pub const ROW_GONE: &str = "The pair is no longer in the parent comparison.";

/// The next pair of `link`'s row in `dir`: its row, the request that shows it (this view's rules, the pair's specs) and
/// the parent link moved to it. `Err` with the message panel's text when there is none.
pub fn next_pair(
    parent: &PreparedFolderCompare,
    link: &ParentLink,
    dir: Dir,
    opts: &NextFileOptions,
    current: &TextCompareRequest,
) -> Result<(NodeId, TextCompareRequest, ParentLink), &'static str> {
    let read = parent.model().read();
    let row = parent_row(&read, link).ok_or(ROW_GONE)?;
    let next = read
        .next_difference_file(row, dir, opts)
        .ok_or(NO_MORE_FILES)?;
    pair_of(parent, next, link, current).ok_or(NO_MORE_FILES)
}

/// The request and link of row `node` of the parent: this view's request with the pair's specs.
pub fn pair_of(
    parent: &PreparedFolderCompare,
    node: NodeId,
    link: &ParentLink,
    current: &TextCompareRequest,
) -> Option<(NodeId, TextCompareRequest, ParentLink)> {
    let read = parent.model().read();
    let left = read.spec(node, Side::Left)?;
    let right = read.spec(node, Side::Right)?;
    let mut request = current.clone();
    request.left = spec_os_string(&left);
    request.right = spec_os_string(&right);
    // The pair's own anchors, ignored lines, encodings and formats (picked for its files) do not carry over to
    // another pair (W11b review finding 7).
    request.anchors = Vec::new();
    request.ignored = Default::default();
    request.ignored_check = Default::default();
    request.encodings = Default::default();
    request.formats = Default::default();
    let moved = parent_link_of(&read, node, &link.session)?;
    Some((node, request, moved))
}

/// The confirmation's lines of a copy (TC-052, help `dialog-copy-file-next`), one per step as core plans them:
/// *Save the edits of <source>*, *Discard the edits of <target>*, *Copy <file> to the right*, *Open the next pair:
/// <names>*.
pub fn steps_text(
    steps: &CopyFileSteps,
    names: [&str; 2],
    from: Side,
    next: Option<String>,
) -> Vec<String> {
    let (source, target) = if from == Side::Right {
        (names[1], names[0])
    } else {
        (names[0], names[1])
    };
    let mut out = Vec::new();
    if steps.save_source.is_some() {
        out.push(format!("Save the edits of {source}"));
    }
    if steps.discard_target {
        out.push(format!("Discard the edits of {target}"));
    }
    let to = if from == Side::Right { "left" } else { "right" };
    let items = steps.copy.plan.items.len();
    if items <= 1 {
        out.push(format!("Copy {source} to the {to}"));
    } else {
        out.push(format!("Copy {source} to the {to} ({items} items)"));
    }
    match next {
        Some(names) => out.push(format!("Open the next pair: {names}")),
        None => out.push("Then no more difference files: the view stays".to_owned()),
    }
    out
}

/// The names of row `node` (`a.c ⇄ a.c`) for the confirmation.
pub fn pair_names(parent: &PreparedFolderCompare, node: NodeId) -> Option<String> {
    let read = parent.model().read();
    let name = |side| {
        read.spec(node, side)
            .map(|s| crate::shared::spec_name(&s.canonical()).to_owned())
    };
    Some(format!("{} ⇄ {}", name(Side::Left)?, name(Side::Right)?))
}

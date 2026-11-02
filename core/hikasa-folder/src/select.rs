//! Selection (FC-060, FC-061, FC-052, FC-062; D-175, D-176): the one owner of the script `select` vocabulary (T11), what a
//! selection means to an operation ([`resolve`]), Find Filename ([`Finder`]), the pair a command opens ([`open_pair`]) and
//! the masks of Exclude ([`exclude_masks`]). Everything here is pure: it reads the model and writes nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use hikasa_core::{ContentKind, Pair, Side, Spec, case};
use hikasa_formats::{Pattern, RegexLimits};
use serde::{Deserialize, Serialize};

use crate::display::{DisplayOptions, FolderDisplayFilter, FolderMode, VisibleRow, is_visible};
use crate::error::FolderError;
use crate::model::{NodeFlags, NodeId, NodeKind, NodeView, ScanStatus, TreeRead};
use crate::quick::DiffFlags;
use crate::state::{Category, CompareState};

bitflags::bitflags! {
    /// A set of sides.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct SideSet: u8 {
        /// The left side.
        const LEFT = 1;
        /// The right side.
        const RIGHT = 2;
    }
}

impl SideSet {
    /// The set of `side` (empty for `Center` and `Output`).
    pub const fn of(side: Side) -> SideSet {
        match side {
            Side::Left => SideSet::LEFT,
            Side::Right => SideSet::RIGHT,
            Side::Center | Side::Output => SideSet::empty(),
        }
    }

    /// The sides of the set, left first.
    pub fn sides(self) -> impl Iterator<Item = Side> {
        [Side::Left, Side::Right]
            .into_iter()
            .filter(move |side| self.contains(SideSet::of(*side)))
    }
}

/// The side slot of a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelSide {
    /// `left`, `lt`.
    Left,
    /// `right`, `rt`.
    Right,
    /// `all`: each side that matches.
    All,
}

/// The state slot of a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelState {
    /// Every row.
    All,
    /// Same, on both sides.
    Exact,
    /// Differences other than the timestamp.
    Diff,
    /// The side is the newer one.
    Newer,
    /// The other side is the newer one.
    Older,
    /// On that side alone.
    Orphan,
}

/// The kind slot of a mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelKind {
    /// Files and folders.
    All,
    /// Files, links and special files.
    Files,
    /// Folders.
    Folders,
}

/// One word of `select` (SCR-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectMask {
    /// `<side>.<state>.<kind>`.
    Result {
        /// The side.
        side: SelSide,
        /// The state.
        state: SelState,
        /// The kind.
        kind: SelKind,
    },
    /// `empty.folders`: a folder with no file below it, on each present side.
    EmptyFolders,
}

impl SelectMask {
    /// Parses one word: `.`-separated, case-insensitive, `lt` and `rt` for `left` and `right`. The three slots are in order
    /// and optional, and a word fills the first slot it fits, so a leading `all` is the side. Anything else is
    /// [`FolderError::InvalidRules`] naming the word.
    pub fn parse(word: &str) -> Result<SelectMask, FolderError> {
        let invalid = || FolderError::InvalidRules(format!("unknown select mask `{word}`"));
        let lower = word.trim().to_ascii_lowercase();
        if lower == "empty.folders" {
            return Ok(SelectMask::EmptyFolders);
        }
        let mut side = None;
        let mut state = None;
        let mut kind = None;
        for token in lower.split('.') {
            let as_side = match token {
                "left" | "lt" => Some(SelSide::Left),
                "right" | "rt" => Some(SelSide::Right),
                "all" => Some(SelSide::All),
                _ => None,
            };
            let as_state = match token {
                "all" => Some(SelState::All),
                "exact" => Some(SelState::Exact),
                "diff" => Some(SelState::Diff),
                "newer" => Some(SelState::Newer),
                "older" => Some(SelState::Older),
                "orphan" => Some(SelState::Orphan),
                _ => None,
            };
            let as_kind = match token {
                "all" => Some(SelKind::All),
                "files" => Some(SelKind::Files),
                "folders" => Some(SelKind::Folders),
                _ => None,
            };
            if side.is_none() && state.is_none() && kind.is_none() && as_side.is_some() {
                side = as_side;
            } else if state.is_none() && kind.is_none() && as_state.is_some() {
                state = as_state;
            } else if kind.is_none() && as_kind.is_some() {
                kind = as_kind;
            } else {
                return Err(invalid());
            }
        }
        Ok(SelectMask::Result {
            side: side.unwrap_or(SelSide::All),
            state: state.unwrap_or(SelState::All),
            kind: kind.unwrap_or(SelKind::All),
        })
    }
}

impl fmt::Display for SelectMask {
    /// The canonical word, all three slots.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let SelectMask::Result { side, state, kind } = self else {
            return f.write_str("empty.folders");
        };
        let side = match side {
            SelSide::Left => "left",
            SelSide::Right => "right",
            SelSide::All => "all",
        };
        let state = match state {
            SelState::All => "all",
            SelState::Exact => "exact",
            SelState::Diff => "diff",
            SelState::Newer => "newer",
            SelState::Older => "older",
            SelState::Orphan => "orphan",
        };
        let kind = match kind {
            SelKind::All => "all",
            SelKind::Files => "files",
            SelKind::Folders => "folders",
        };
        write!(f, "{side}.{state}.{kind}")
    }
}

/// What `select` sees. `expanded`: `None` = every folder is open (a headless `expand all`).
#[derive(Clone, Copy)]
pub struct SelectScope<'a> {
    /// The display options.
    pub display: &'a DisplayOptions,
    /// Whether a folder is open.
    pub expanded: Option<&'a dyn Fn(NodeId) -> bool>,
}

impl fmt::Debug for SelectScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectScope")
            .field("display", self.display)
            .field("all_expanded", &self.expanded.is_none())
            .finish()
    }
}

/// Selected rows, each with its sides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    rows: BTreeMap<NodeId, SideSet>,
}

impl Selection {
    /// Adds `sides` of `node`.
    pub fn insert(&mut self, node: NodeId, sides: SideSet) {
        if !sides.is_empty() {
            *self.rows.entry(node).or_default() |= sides;
        }
    }

    /// Removes `sides` of `node`.
    pub fn remove(&mut self, node: NodeId, sides: SideSet) {
        if let Some(set) = self.rows.get_mut(&node) {
            set.remove(sides);
            if set.is_empty() {
                self.rows.remove(&node);
            }
        }
    }

    /// The sides of `node` (empty when it is not selected).
    pub fn get(&self, node: NodeId) -> SideSet {
        self.rows.get(&node).copied().unwrap_or_default()
    }

    /// True when nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// The rows, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, SideSet)> + '_ {
        self.rows.iter().map(|(&node, &sides)| (node, sides))
    }

    /// Keeps `side` alone (Shift+Left, Shift+Right).
    pub fn restrict(&mut self, side: Side) {
        let keep = SideSet::of(side);
        self.rows.retain(|_, set| {
            *set &= keep;
            !set.is_empty()
        });
    }

    /// Inverts every shown row, on every side it has.
    pub fn invert(&mut self, read: &TreeRead<'_>, scope: &SelectScope<'_>) {
        for row in shown_rows(read, scope) {
            let present = present_sides(read.node(row.node).as_ref());
            let now = self.get(row.node);
            self.remove(row.node, now);
            self.insert(row.node, present.difference(now));
        }
    }
}

/// The sides a node has.
fn present_sides(view: Option<&NodeView<'_>>) -> SideSet {
    let mut sides = SideSet::empty();
    if let Some(view) = view {
        sides.set(SideSet::LEFT, view.left.is_some());
        sides.set(SideSet::RIGHT, view.right.is_some());
    }
    sides
}

/// The rows `scope` shows.
fn shown_rows(read: &TreeRead<'_>, scope: &SelectScope<'_>) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    let all = |_: NodeId| true;
    read.visible_rows(scope.display, scope.expanded.unwrap_or(&all), &mut rows);
    rows
}

/// True for a content result that differs.
fn content_differs(kind: Option<ContentKind>) -> bool {
    matches!(
        kind,
        Some(
            ContentKind::UnimportantOnly
                | ContentKind::ImportantDifferences
                | ContentKind::BinaryDifferent
        )
    )
}

/// The file-like row `view` against `state`, on `side`; `category` is its category (§1.6.9).
fn file_matches(view: &NodeView<'_>, category: Category, state: SelState, side: Side) -> bool {
    if view.side(side).is_none() {
        return false;
    }
    let both = view.left.is_some() && view.right.is_some();
    let certain = !matches!(
        category,
        Category::Unknown | Category::Error | Category::Filtered
    );
    match state {
        SelState::All => true,
        _ if !certain => false,
        SelState::Exact => category == Category::Same && both,
        SelState::Diff => {
            both && category != Category::Same
                && (!view.diffs.difference(DiffFlags::TIME).is_empty()
                    || content_differs(view.content.map(|c| c.kind)))
        }
        SelState::Newer => matches!(
            (category, side),
            (Category::LeftNewer, Side::Left) | (Category::RightNewer, Side::Right)
        ),
        SelState::Older => matches!(
            (category, side),
            (Category::LeftNewer, Side::Right) | (Category::RightNewer, Side::Left)
        ),
        SelState::Orphan => matches!(
            (category, side),
            (Category::LeftOrphan, Side::Left) | (Category::RightOrphan, Side::Right)
        ),
    }
}

/// The verdict of a folder with no shown row on `side`: only `all`, `orphan` when it is one, `exact` when it is on both.
fn empty_folder_matches(view: &NodeView<'_>, state: SelState, side: Side) -> bool {
    let both = view.left.is_some() && view.right.is_some();
    match state {
        SelState::All => true,
        SelState::Exact => both,
        SelState::Orphan => !both && view.side(side).is_some(),
        SelState::Diff | SelState::Newer | SelState::Older => false,
    }
}

/// The sides on which each node matches `state`, by id, in one bottom-up pass: O(nodes).
fn result_matches(
    read: &TreeRead<'_>,
    display: &DisplayOptions,
    state: SelState,
    iu: bool,
) -> Vec<SideSet> {
    let store = read.store();
    let mut found = vec![SideSet::empty(); read.len() as usize];
    for id in store.post_order() {
        let node_id = NodeId(id);
        let Some(view) = read.node(node_id) else {
            continue;
        };
        let mut sides = SideSet::empty();
        for side in [Side::Left, Side::Right] {
            let hit = if !view.is_folder() {
                file_matches(&view, read.category(node_id, iu), state, side)
            } else if view.side(side).is_none() || view.scan != ScanStatus::Scanned {
                false
            } else {
                let mut any_row = false;
                let mut all_match = true;
                for child in read.children(node_id) {
                    let shown = store.node(child).is_some_and(|node| is_visible(store, node, display));
                    let here = read.node(child).is_some_and(|v| v.side(side).is_some());
                    if shown && here {
                        any_row = true;
                        let matched = found
                            .get(child.0 as usize)
                            .is_some_and(|set| set.contains(SideSet::of(side)));
                        all_match &= matched;
                    }
                }
                if any_row {
                    all_match
                } else {
                    empty_folder_matches(&view, state, side)
                }
            };
            sides.set(SideSet::of(side), hit);
        }
        if let Some(slot) = found.get_mut(id as usize) {
            *slot = sides;
        }
    }
    found
}

/// The sides on which each folder has no file below it, by id.
fn empty_folders(read: &TreeRead<'_>) -> Vec<SideSet> {
    let store = read.store();
    let mut found = vec![SideSet::empty(); read.len() as usize];
    let mut files_below = vec![SideSet::empty(); read.len() as usize];
    for id in store.post_order() {
        let node_id = NodeId(id);
        let Some(view) = read.node(node_id) else {
            continue;
        };
        let present = present_sides(Some(&view));
        let below = if view.is_folder() {
            read.children(node_id).fold(SideSet::empty(), |all, child| {
                all | files_below.get(child.0 as usize).copied().unwrap_or_default()
            })
        } else {
            present
        };
        if let Some(slot) = files_below.get_mut(id as usize) {
            *slot = below;
        }
        if view.is_folder()
            && view.scan == ScanStatus::Scanned
            && let Some(slot) = found.get_mut(id as usize)
        {
            *slot = present.difference(below);
        }
    }
    found
}

/// Adds what `masks` select to `into`, which starts empty for script `select`. The masks are OR-ed (SCR-002); rows are what
/// `scope` shows, and a collapsed folder is one row, whose match still reads what is below it. One bottom-up pass per mask.
pub fn select(
    read: &TreeRead<'_>,
    scope: &SelectScope<'_>,
    masks: &[SelectMask],
    iu: bool,
    into: &mut Selection,
) {
    let rows = shown_rows(read, scope);
    for mask in masks {
        let (found, side, kind) = match *mask {
            SelectMask::EmptyFolders => (empty_folders(read), SelSide::All, SelKind::Folders),
            SelectMask::Result { side, state, kind } => (
                result_matches(read, scope.display, state, iu),
                side,
                kind,
            ),
        };
        let sides = match side {
            SelSide::Left => SideSet::LEFT,
            SelSide::Right => SideSet::RIGHT,
            SelSide::All => SideSet::all(),
        };
        for row in &rows {
            let Some(view) = read.node(row.node) else {
                continue;
            };
            let fits = match kind {
                SelKind::All => true,
                SelKind::Files => !view.is_folder(),
                SelKind::Folders => view.is_folder(),
            };
            if fits {
                let hit = found.get(row.node.0 as usize).copied().unwrap_or_default();
                into.insert(row.node, hit & sides);
            }
        }
    }
}

/// Adds every shown file-like row, on every side it has.
pub fn select_all_files(read: &TreeRead<'_>, scope: &SelectScope<'_>, into: &mut Selection) {
    for row in shown_rows(read, scope) {
        if let Some(view) = read.node(row.node)
            && !view.is_folder()
        {
            into.insert(row.node, present_sides(Some(&view)));
        }
    }
}

/// The items of a selection in tree order: `(row, side)`, left first on one row.
fn items_in_order(read: &TreeRead<'_>, sel: &Selection) -> Vec<(NodeId, Side)> {
    let mut found = Vec::new();
    let mut left = sel.len();
    for (id, _) in read.preorder() {
        if left == 0 {
            break;
        }
        let sides = sel.get(id);
        if !sides.is_empty() {
            left -= 1;
            found.extend(sides.sides().map(|side| (id, side)));
        }
    }
    found
}

/// FC-062: what `Open`, `Compare To` compare. Two selected items anywhere → those two, in tree order; one item with both
/// sides → its pair; one item and a Space `mark` → those two. Otherwise `None`.
pub fn open_pair(
    read: &TreeRead<'_>,
    sel: &Selection,
    mark: Option<(NodeId, Side)>,
) -> Option<Pair<Spec>> {
    let items = items_in_order(read, sel);
    let spec = |(id, side): (NodeId, Side)| read.spec(id, side);
    match items.as_slice() {
        [first, second] => Some(Pair::new(spec(*first)?, spec(*second)?)),
        [one] => {
            let mark = mark.filter(|mark| mark != one)?;
            let mut marked = Selection::default();
            marked.insert(one.0, SideSet::of(one.1));
            marked.insert(mark.0, SideSet::of(mark.1));
            match items_in_order(read, &marked).as_slice() {
                [first, second] => Some(Pair::new(spec(*first)?, spec(*second)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// What a selection takes in: the rows of a folder an operation sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Include {
    /// Everything: filtered rows, rows the display hides, name-only items (D-45).
    pub hidden: bool,
    /// What is shown.
    pub display: DisplayOptions,
}

impl Include {
    /// Everything a file filter leaves: display `all`, folders always shown.
    pub const fn visible() -> Include {
        Include {
            hidden: false,
            display: DisplayOptions {
                filter: FolderDisplayFilter::All,
                folders: FolderMode::AlwaysShowFolders,
                ignore_unimportant: false,
                suppress_filters: false,
            },
        }
    }
}

/// One row of a selection as an operation sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolved {
    /// The row.
    pub node: NodeId,
    /// The sides it counts on.
    pub sides: SideSet,
    /// The sides on which its whole subtree is included: only then may a plan treat it as one item.
    pub whole: SideSet,
    /// Its level (the base's children are 0).
    pub depth: u32,
}

/// The sides on which something is selected below a row, by row.
fn selected_below(read: &TreeRead<'_>, sel: &Selection) -> BTreeMap<NodeId, SideSet> {
    let mut below: BTreeMap<NodeId, SideSet> = BTreeMap::new();
    for (node, sides) in sel.iter() {
        for ancestor in read.ancestors(node) {
            let known = below.entry(ancestor).or_default();
            if known.contains(sides) {
                break;
            }
            *known |= sides;
        }
    }
    below
}

/// True when a row itself leaves something out on `side`: it failed or is unknown (D-19).
fn row_failed(read: &TreeRead<'_>, id: NodeId, view: &NodeView<'_>, side: Side) -> bool {
    if view.flags.contains(NodeFlags::error(side)) {
        return true;
    }
    if view.is_folder() {
        return view.scan == ScanStatus::Failed;
    }
    matches!(view.state, CompareState::Unknown | CompareState::Error)
        || matches!(read.category(id, false), Category::Unknown | Category::Error)
}

/// A step of the walk of [`resolve`].
enum Visit {
    /// Enter a row, with the sides an ancestor covers.
    Enter {
        id: NodeId,
        covered: SideSet,
        depth: u32,
    },
    /// All of a folder's children were seen: its `whole` is known.
    Leave { id: NodeId, row: Option<usize> },
}

/// The sides on which a counted row is whole, given what is left out below it.
fn whole_sides(
    read: &TreeRead<'_>,
    id: NodeId,
    view: &NodeView<'_>,
    counted: SideSet,
    missing: SideSet,
) -> SideSet {
    let mut whole = SideSet::empty();
    for side in counted.sides() {
        let bit = SideSet::of(side);
        let listed = !view.is_folder() || view.scan == ScanStatus::Scanned;
        whole.set(
            bit,
            listed && !missing.contains(bit) && !row_failed(read, id, view, side),
        );
    }
    whole
}

/// **Visible contents (D-176):** what a selection means to an operation, the one definition. A folder selected on a side
/// contributes the rows `include.display` shows, without items hidden by file filters; with `include.hidden` everything. A
/// folder and any row below it selected on one side: only the rows below count there. A row selected by itself counts,
/// whatever hides it: what its operation does with a filtered row is the plan's (`skip filtered`). The list is in
/// pre-order and has no root. O(rows reached + depth × selected).
pub fn resolve(read: &TreeRead<'_>, sel: &Selection, include: &Include) -> Vec<Resolved> {
    let store = read.store();
    let below = selected_below(read, sel);
    let mut out: Vec<Resolved> = Vec::new();
    // Per folder on the walk: the sides on which something below it is left out.
    let mut left_out: BTreeMap<NodeId, SideSet> = BTreeMap::new();
    let lose = |left_out: &mut BTreeMap<NodeId, SideSet>, view: &NodeView<'_>, whole| {
        if let Some(parent) = view.parent {
            *left_out.entry(parent).or_default() |= present_sides(Some(view)).difference(whole);
        }
    };
    let mut stack = vec![Visit::Enter {
        id: NodeId::ROOT,
        covered: SideSet::empty(),
        depth: 0,
    }];
    while let Some(visit) = stack.pop() {
        let (id, covered, depth) = match visit {
            Visit::Enter { id, covered, depth } => (id, covered, depth),
            Visit::Leave { id, row } => {
                let Some(view) = read.node(id) else { continue };
                let missing = left_out.remove(&id).unwrap_or_default();
                let counted = row
                    .and_then(|row| out.get(row))
                    .map_or(SideSet::empty(), |row| row.sides);
                let whole = whole_sides(read, id, &view, counted, missing);
                if let Some(slot) = row.and_then(|row| out.get_mut(row)) {
                    slot.whole = whole;
                }
                lose(&mut left_out, &view, whole);
                continue;
            }
        };
        let Some(view) = read.node(id) else { continue };
        let explicit = sel.get(id);
        let selected_here = below.get(&id).copied().unwrap_or_default();
        let is_root = id == NodeId::ROOT;
        let mut counted = SideSet::empty();
        if !is_root {
            let shown = store
                .node(id)
                .is_some_and(|node| include.hidden || is_visible(store, node, &include.display));
            for side in present_sides(Some(&view)).sides() {
                let bit = SideSet::of(side);
                let filtered = view.side(side).is_some_and(|entry| entry.filtered);
                let reached = covered.contains(bit) && shown && (include.hidden || !filtered);
                counted.set(
                    bit,
                    (reached || explicit.contains(bit)) && !selected_here.contains(bit),
                );
            }
        }
        let row = (!counted.is_empty()).then(|| {
            out.push(Resolved {
                node: id,
                sides: counted,
                whole: SideSet::empty(),
                depth,
            });
            out.len() - 1
        });
        let down = if is_root {
            explicit.difference(selected_here)
        } else {
            counted
        };
        if !view.is_folder() || (down.is_empty() && selected_here.is_empty()) {
            // A folder here counts on no side: nothing of it is selected.
            let whole = whole_sides(read, id, &view, counted, SideSet::empty());
            if let Some(slot) = row.and_then(|row| out.get_mut(row)) {
                slot.whole = whole;
            }
            lose(&mut left_out, &view, whole);
            continue;
        }
        stack.push(Visit::Leave { id, row });
        let mut children: Vec<NodeId> = read.children(id).collect();
        children.reverse();
        let level = if is_root { 0 } else { depth + 1 };
        stack.extend(children.into_iter().map(|child| Visit::Enter {
            id: child,
            covered: down,
            depth: level,
        }));
    }
    out
}

/// What Find Filename looks for (FC-061).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindQuery {
    /// The text or the regular expression.
    pub text: String,
    /// Match case.
    pub case: bool,
    /// Whole words: the neighbours of a match are no letter, digit or `_`.
    pub whole_word: bool,
    /// `text` is a regular expression.
    pub regex: bool,
    /// Wrap around at the end.
    pub wrap: bool,
    /// Search upwards.
    pub up: bool,
    /// The sides whose names are looked at.
    pub sides: SideSet,
}

/// How a [`Finder`] matches.
#[derive(Debug)]
enum Needle {
    /// A substring, folded unless the case matters.
    Text { text: String, folded: bool },
    /// A compiled pattern.
    Regex(Box<Pattern>),
}

/// A compiled [`FindQuery`].
#[derive(Debug)]
pub struct Finder {
    needle: Needle,
    whole_word: bool,
    wrap: bool,
    up: bool,
    sides: SideSet,
}

/// True when `c` is part of a word.
fn word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// True when the match `start..end` of `text` stands alone.
fn stands_alone(text: &str, start: usize, end: usize) -> bool {
    let before = text.get(..start).and_then(|head| head.chars().next_back());
    let after = text.get(end..).and_then(|tail| tail.chars().next());
    !word_char(before) && !word_char(after)
}

impl Finder {
    /// Compiles `query`. A bad regular expression is `invalid_input`.
    pub fn new(query: &FindQuery) -> Result<Finder, FolderError> {
        let needle = if query.regex {
            let pattern = Pattern::compile(&query.text, !query.case, &RegexLimits::default())
                .map_err(|e| FolderError::InvalidRules(e.to_string()))?;
            Needle::Regex(Box::new(pattern))
        } else if query.case {
            Needle::Text {
                text: query.text.clone(),
                folded: false,
            }
        } else {
            Needle::Text {
                text: case::fold(&query.text).into_owned(),
                folded: true,
            }
        };
        Ok(Finder {
            needle,
            whole_word: query.whole_word,
            wrap: query.wrap,
            up: query.up,
            sides: query.sides,
        })
    }

    /// True when `name` holds a match.
    fn hits(&self, name: &str) -> bool {
        match &self.needle {
            Needle::Text { text, folded } => {
                let folded_name = if *folded {
                    case::fold(name)
                } else {
                    std::borrow::Cow::Borrowed(name)
                };
                let hay = folded_name.as_ref();
                if text.is_empty() {
                    return false;
                }
                hay.match_indices(text.as_str()).any(|(start, found)| {
                    !self.whole_word || stands_alone(hay, start, start + found.len())
                })
            }
            Needle::Regex(pattern) => {
                let mut from = 0;
                // A run-time limit of the engine ends this name: it is no match.
                while let Ok(Some(found)) = pattern.find_at(name, from) {
                    if !self.whole_word || stands_alone(name, found.start, found.end) {
                        return true;
                    }
                    let Some(step) = name.get(found.start..).and_then(|tail| tail.chars().next())
                    else {
                        return false;
                    };
                    from = found.start + step.len_utf8();
                }
                false
            }
        }
    }

    /// True when the name of `row` matches on a side of the query.
    fn row_hits(&self, read: &TreeRead<'_>, row: &VisibleRow) -> bool {
        read.node(row.node).is_some_and(|view| {
            self.sides
                .sides()
                .any(|side| view.side(side).is_some_and(|entry| self.hits(entry.name)))
        })
    }

    /// The next row after `from` in the query's direction, wrapping with `wrap`; matches the name on the query's sides.
    /// With no `from` the search starts at the first row, or the last one going up.
    pub fn next(
        &self,
        read: &TreeRead<'_>,
        rows: &[VisibleRow],
        from: Option<usize>,
    ) -> Option<usize> {
        let len = rows.len();
        if len == 0 {
            return None;
        }
        let hits = |index: &usize| rows.get(*index).is_some_and(|row| self.row_hits(read, row));
        let (head, tail): (Vec<usize>, Vec<usize>) = match (from, self.up) {
            (Some(from), false) => ((from + 1..len).collect(), (0..=from.min(len - 1)).collect()),
            (None, false) => ((0..len).collect(), Vec::new()),
            (Some(from), true) => (
                (0..from.min(len)).rev().collect(),
                (from.min(len - 1)..len).rev().collect(),
            ),
            (None, true) => ((0..len).rev().collect(), Vec::new()),
        };
        head.iter()
            .find(|index| hits(index))
            .or_else(|| {
                if self.wrap {
                    tail.iter().find(|index| hits(index))
                } else {
                    None
                }
            })
            .copied()
    }
}

/// What Exclude takes out of a comparison (FC-052).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludeWhat {
    /// The selected items, by their paths.
    Items,
    /// Every item of the selected files' extensions.
    Type,
}

/// One character as a set of its own.
fn push_set(out: &mut String, c: char) {
    out.push('[');
    out.push(c);
    out.push(']');
}

/// A name as a mask that matches that name and no other: what a mask reads as syntax becomes a one-character set (`[[]`,
/// `[?]`, `[ ]`). A `!` needs none: it has no meaning in a mask, and `[!]` is no set (§1.6.25).
pub fn escape_mask(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    let last = name.char_indices().next_back().map(|(at, _)| at);
    for (at, c) in name.char_indices() {
        let blank = c == ' ' || c == '\t';
        let edge = at == 0 || Some(at) == last;
        let syntax = matches!(c, '[' | '?' | '*' | ';' | '\\' | '/')
            || (blank && edge)
            || (c == '-' && at == 0)
            || (c == '.' && Some(at) == last);
        if syntax {
            push_set(&mut out, c);
        } else {
            out.push(c);
        }
    }
    out
}

/// The extension of a name by D-07: a `.` that is neither first nor last; the last one counts.
fn extension(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then_some(ext)
}

/// The anchored mask of `id` on `side`, by that side's own names: `a\b.txt`, and `.\b.txt` in the base.
fn anchored_mask(read: &TreeRead<'_>, id: NodeId, side: Side) -> Option<String> {
    let mut names = Vec::new();
    let mut at = Some(id);
    while let Some(node) = at.filter(|node| *node != NodeId::ROOT) {
        let view = read.node(node)?;
        names.push(escape_mask(view.side(side)?.name));
        at = view.parent;
    }
    names.reverse();
    let mut mask = names.join("\\");
    if names.len() == 1 {
        mask.insert_str(0, ".\\");
    }
    Some(mask)
}

/// FC-052: masks for the exclude lists, in `NameFilterLists` form: (`exclude_files`, `exclude_folders`), each entry once.
/// `Items`: a file by its anchored relative mask, a folder by `a\b\`. `Type`: `*.<ext>` per distinct extension of the
/// selected files; a file without one goes as an item, and a selected folder is no type.
pub fn exclude_masks(
    read: &TreeRead<'_>,
    sel: &Selection,
    what: ExcludeWhat,
) -> (Vec<String>, Vec<String>) {
    let mut files = BTreeSet::new();
    let mut folders = BTreeSet::new();
    for (id, side) in items_in_order(read, sel) {
        let Some(view) = read.node(id) else { continue };
        let Some(entry) = view.side(side) else {
            continue;
        };
        if view.kind == NodeKind::Folder {
            if what == ExcludeWhat::Items
                && let Some(mask) = anchored_mask(read, id, side)
            {
                folders.insert(format!("{mask}\\"));
            }
            continue;
        }
        match (what, extension(entry.name)) {
            (ExcludeWhat::Type, Some(ext)) => {
                files.insert(format!("*.{}", escape_mask(ext)));
            }
            _ => {
                files.extend(anchored_mask(read, id, side));
            }
        }
    }
    (files.into_iter().collect(), folders.into_iter().collect())
}

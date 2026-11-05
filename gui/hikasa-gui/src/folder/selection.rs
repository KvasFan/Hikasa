//! The selection of a folder view (M2 W9a, FC-060, FC-064; D-238): the core's `Selection` (node → sides, what every
//! plan gets) with the current row, the anchor, the focus sides and the Space mark, and the pointer and keyboard
//! tables as pure calls. Toolkit-free: `ui/folder.rs` turns clicks, drags and keys into these calls.

use hikasa_core::Side;
use hikasa_folder::NodeId;
use hikasa_folder::VisibleRow;
use hikasa_folder::select::{Selection, SideSet};

/// Where a pointer is on a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A side's cells.
    Side(Side),
    /// The centre column.
    Centre,
}

impl Hit {
    /// The sides a click on this place selects.
    pub fn sides(self) -> SideSet {
        match self {
            Hit::Side(side) => SideSet::of(side),
            Hit::Centre => SideSet::all(),
        }
    }
}

/// The modifiers of a click or key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Ctrl (Cmd on macOS).
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
}

/// What a press on a row starts (D-238).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// A press on a row that is not selected: a drag of ≥ 4 points selects a range (a swipe).
    Swipe,
    /// A press on a selected row: a drag moves the selection (D-251).
    Drag,
}

/// The points a press must move vertically before it is a swipe.
pub const SWIPE_THRESHOLD: f32 = 4.0;

/// A swipe in progress: the pressed row and side, and whether the pointer has crossed to the centre or the other
/// side (then both sides are selected, FC-060, and stay so for the rest of the swipe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swipe {
    /// The pressed row (an index in the visible rows).
    pub from: usize,
    /// The pressed side (the centre: both).
    pub hit: Hit,
    /// The pointer left the pressed side.
    pub crossed: bool,
}

impl Swipe {
    /// A swipe from `from` on `hit`.
    pub fn new(from: usize, hit: Hit) -> Swipe {
        Swipe {
            from,
            hit,
            crossed: hit == Hit::Centre,
        }
    }

    /// The pointer is over `hit`: crossing the centre or entering the other side selects both sides.
    pub fn moved_over(&mut self, hit: Hit) {
        if hit != self.hit {
            self.crossed = true;
        }
    }

    /// The sides the swipe selects.
    pub fn sides(&self) -> SideSet {
        if self.crossed {
            SideSet::all()
        } else {
            self.hit.sides()
        }
    }
}

/// The selection of a view (D-238).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewSelection {
    /// Node → sides: the data every plan gets.
    pub items: Selection,
    /// The current row (keyboard focus).
    pub current: Option<NodeId>,
    /// Where a Shift range starts.
    pub anchor: Option<NodeId>,
    /// The focus sides (left, right or both).
    pub focus: SideSet,
    /// The Space mark (FC-060), for Open and Compare To.
    pub mark: Option<(NodeId, Side)>,
    /// Bumped by every change the user makes (W6d's `selection_moves`).
    pub moves: u64,
}

/// The index of `node` in `rows`.
fn index_of(rows: &[VisibleRow], node: NodeId) -> Option<usize> {
    rows.iter().position(|r| r.node == node)
}

impl ViewSelection {
    /// A fresh selection with the focus on both sides.
    pub fn new() -> ViewSelection {
        ViewSelection {
            focus: SideSet::all(),
            ..ViewSelection::default()
        }
    }

    /// The current row's index in `rows`.
    pub fn current_index(&self, rows: &[VisibleRow]) -> Option<usize> {
        index_of(rows, self.current?)
    }

    /// Bumps `moves` (a change the user made; also an Edit command that changed the items in place).
    pub fn changed(&mut self) {
        self.moves += 1;
    }

    /// Replaces the selection with `rows[from..=to]` (either order) on `sides`, each row on the sides it has.
    fn select_range(
        &mut self,
        rows: &[VisibleRow],
        present: &dyn Fn(NodeId) -> SideSet,
        from: usize,
        to: usize,
        sides: SideSet,
        add: bool,
    ) {
        if !add {
            self.items = Selection::default();
        }
        let (lo, hi) = (from.min(to), from.max(to));
        for row in rows.iter().take(hi + 1).skip(lo) {
            self.items.insert(row.node, present(row.node) & sides);
        }
    }

    /// A click on row `index` at `hit` with `mods` (D-238's pointer table): plain selects that row (on that side, or
    /// every side it is on for the centre), Ctrl toggles it, Shift selects the range from the anchor on the anchor's
    /// focus sides (Ctrl+Shift adds it).
    pub fn click(
        &mut self,
        rows: &[VisibleRow],
        present: &dyn Fn(NodeId) -> SideSet,
        index: usize,
        hit: Hit,
        mods: Mods,
    ) {
        let Some(row) = rows.get(index) else {
            return;
        };
        let node = row.node;
        let sides = present(node) & hit.sides();
        if mods.shift {
            let anchor = self.anchor.and_then(|a| index_of(rows, a)).unwrap_or(index);
            let focus = self.focus;
            self.select_range(rows, present, anchor, index, focus, mods.ctrl);
            self.current = Some(node);
        } else if mods.ctrl {
            let now = self.items.get(node);
            let toggle = if hit == Hit::Centre {
                present(node)
            } else {
                sides
            };
            if now.contains(toggle) && !toggle.is_empty() {
                self.items.remove(node, toggle);
            } else {
                self.items.insert(node, toggle);
            }
            self.current = Some(node);
            self.anchor = Some(node);
            self.focus = hit.sides();
        } else {
            self.items = Selection::default();
            self.items.insert(node, sides);
            self.current = Some(node);
            self.anchor = Some(node);
            self.focus = hit.sides();
        }
        self.changed();
    }

    /// What a press on row `index` starts: a drag when the row is selected on the pressed side, else a swipe.
    pub fn press(&self, rows: &[VisibleRow], index: usize, hit: Hit) -> Press {
        match rows.get(index) {
            Some(row) if self.items.get(row.node).intersects(hit.sides()) => Press::Drag,
            _ => Press::Swipe,
        }
    }

    /// A swipe reached row `to`: the range from its pressed row on its sides replaces the selection.
    pub fn swipe(
        &mut self,
        rows: &[VisibleRow],
        present: &dyn Fn(NodeId) -> SideSet,
        swipe: &Swipe,
        to: usize,
    ) {
        let to = to.min(rows.len().saturating_sub(1));
        let sides = swipe.sides();
        self.select_range(rows, present, swipe.from, to, sides, false);
        self.current = rows.get(to).map(|r| r.node);
        self.anchor = rows.get(swipe.from).map(|r| r.node);
        self.focus = sides;
        self.changed();
    }

    /// A click below the last row clears the selection.
    pub fn clear(&mut self) {
        if !self.items.is_empty() {
            self.items = Selection::default();
            self.changed();
        }
    }

    /// The keyboard moves `current` to `to` (Up, Down, PgUp, PgDn, Home, End): plain selects it on the focus sides,
    /// Shift extends from the anchor, Ctrl moves `current` only.
    pub fn key_move(
        &mut self,
        rows: &[VisibleRow],
        present: &dyn Fn(NodeId) -> SideSet,
        to: usize,
        mods: Mods,
    ) {
        let Some(row) = rows.get(to) else {
            return;
        };
        let node = row.node;
        if mods.ctrl && !mods.shift {
            self.current = Some(node);
        } else if mods.shift {
            let anchor = self
                .anchor
                .and_then(|a| index_of(rows, a))
                .or_else(|| self.current_index(rows))
                .unwrap_or(to);
            if self.anchor.is_none() {
                self.anchor = rows.get(anchor).map(|r| r.node);
            }
            let focus = self.focus;
            self.select_range(rows, present, anchor, to, focus, false);
            self.current = Some(node);
        } else {
            self.items = Selection::default();
            self.items.insert(node, present(node) & self.focus);
            self.current = Some(node);
            self.anchor = Some(node);
        }
        self.changed();
    }

    /// Ctrl+Space: toggles `current` on the focus sides.
    pub fn toggle_current(&mut self, present: &dyn Fn(NodeId) -> SideSet) {
        let Some(node) = self.current else {
            return;
        };
        let sides = present(node) & self.focus;
        if self.items.get(node).contains(sides) && !sides.is_empty() {
            self.items.remove(node, sides);
        } else {
            self.items.insert(node, sides);
        }
        self.changed();
    }

    /// Shift+Left, Shift+Right: keeps `side` alone and focuses it.
    pub fn restrict(&mut self, side: Side) {
        self.items.restrict(side);
        self.focus = SideSet::of(side);
        self.changed();
    }

    /// Space: marks `current` on its first focused side it is present on; Space on the marked item unmarks.
    pub fn space(&mut self, present: &dyn Fn(NodeId) -> SideSet) {
        let Some(node) = self.current else {
            return;
        };
        let have = present(node) & self.focus;
        let Some(side) = have.sides().next() else {
            return;
        };
        self.mark = if self.mark == Some((node, side)) {
            None
        } else {
            Some((node, side))
        };
        self.changed();
    }

    /// Esc: no mark.
    pub fn unmark(&mut self) {
        if self.mark.take().is_some() {
            self.changed();
        }
    }

    /// Only shown rows are selected: rows no longer in `rows` (collapsed, hidden by the display, gone) and sides they
    /// no longer have leave the selection, the current row, the anchor and the mark.
    pub fn retain_shown(&mut self, rows: &[VisibleRow], present: &dyn Fn(NodeId) -> SideSet) {
        let shown: std::collections::HashSet<NodeId> = rows.iter().map(|r| r.node).collect();
        let mut kept = Selection::default();
        for (node, sides) in self.items.iter() {
            if shown.contains(&node) {
                kept.insert(node, sides & present(node));
            }
        }
        self.items = kept;
        let keep = |id: Option<NodeId>| id.filter(|n| shown.contains(n));
        self.current = keep(self.current);
        self.anchor = keep(self.anchor);
        self.mark = self
            .mark
            .filter(|(n, side)| shown.contains(n) && present(*n).contains(SideSet::of(*side)));
    }

    /// Moves every id by a refresh's `renumbered` (`follow`).
    pub fn renumber(&mut self, follow: &dyn Fn(NodeId) -> NodeId) {
        let mut moved = Selection::default();
        for (node, sides) in self.items.iter() {
            moved.insert(follow(node), sides);
        }
        self.items = moved;
        self.current = self.current.map(follow);
        self.anchor = self.anchor.map(follow);
        self.mark = self.mark.map(|(n, s)| (follow(n), s));
    }

    /// Selects one row on every side it has (Find, next difference): current and anchor move there.
    pub fn select_one(&mut self, node: NodeId, sides: SideSet) {
        self.items = Selection::default();
        self.items.insert(node, sides);
        self.current = Some(node);
        self.anchor = Some(node);
        self.changed();
    }

    /// Replaces the items (an Edit command's result); current and anchor stay where they are.
    pub fn set_items(&mut self, items: Selection) {
        self.items = items;
        self.changed();
    }
}

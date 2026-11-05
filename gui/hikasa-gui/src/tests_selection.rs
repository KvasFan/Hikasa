//! M2 W9a acceptance A9.4 (D-238), the selection model: the pointer and keyboard tables on a fixture of rows, a swipe
//! crossing the centre, a press on a selected row, Shift+Left/Right, the Space mark, rows that stop being shown, and a
//! seeded property: random gesture sequences keep every selected node shown and present on each side it is selected on.

use hikasa_core::Side;
use hikasa_folder::select::SideSet;
use hikasa_folder::{NodeId, VisibleRow};

use crate::folder::selection::{Hit, Mods, Press, Swipe, ViewSelection};

/// Six rows: 1 both, 2 left only, 3 both, 4 right only, 5 both, 6 both.
fn rows() -> Vec<VisibleRow> {
    (1..=6)
        .map(|n| VisibleRow {
            node: NodeId(n),
            depth: 0,
        })
        .collect()
}

fn present(node: NodeId) -> SideSet {
    match node.0 {
        2 => SideSet::LEFT,
        4 => SideSet::RIGHT,
        _ => SideSet::all(),
    }
}

fn picked(sel: &ViewSelection) -> Vec<(u32, SideSet)> {
    sel.items.iter().map(|(n, s)| (n.0, s)).collect()
}

const L: SideSet = SideSet::LEFT;
const R: SideSet = SideSet::RIGHT;
const B: SideSet = SideSet::all();
const NONE: Mods = Mods {
    ctrl: false,
    shift: false,
};
const CTRL: Mods = Mods {
    ctrl: true,
    shift: false,
};
const SHIFT: Mods = Mods {
    ctrl: false,
    shift: true,
};

#[test]
fn the_pointer_table() {
    let rows = rows();
    let mut sel = ViewSelection::new();
    // A click on a side cell: that row on that side; focus that side.
    sel.click(&rows, &present, 0, Hit::Side(Side::Left), NONE);
    assert_eq!(picked(&sel), [(1, L)]);
    assert_eq!(sel.focus, L);
    assert_eq!(sel.current, Some(NodeId(1)));
    // A side the row lacks selects nothing of it.
    sel.click(&rows, &present, 3, Hit::Side(Side::Left), NONE);
    assert!(picked(&sel).is_empty());
    // The centre: every side it is on; focus both.
    sel.click(&rows, &present, 1, Hit::Centre, NONE);
    assert_eq!(picked(&sel), [(2, L)]);
    assert_eq!(sel.focus, B);
    // Ctrl toggles on that side.
    sel.click(&rows, &present, 2, Hit::Side(Side::Right), CTRL);
    assert_eq!(picked(&sel), [(2, L), (3, R)]);
    sel.click(&rows, &present, 2, Hit::Side(Side::Right), CTRL);
    assert_eq!(picked(&sel), [(2, L)]);
    // Shift: the range from the anchor on the anchor's focus sides replaces the selection.
    sel.click(&rows, &present, 0, Hit::Side(Side::Left), NONE);
    sel.click(&rows, &present, 3, Hit::Side(Side::Left), SHIFT);
    assert_eq!(picked(&sel), [(1, L), (2, L), (3, L)]);
    // Ctrl+Shift adds the range.
    sel.click(&rows, &present, 5, Hit::Side(Side::Right), CTRL);
    sel.click(
        &rows,
        &present,
        4,
        Hit::Side(Side::Right),
        Mods {
            ctrl: true,
            shift: true,
        },
    );
    assert_eq!(picked(&sel), [(1, L), (2, L), (3, L), (5, R), (6, R)]);
    // A press on a selected row is a drag; on another a swipe.
    assert_eq!(sel.press(&rows, 0, Hit::Side(Side::Left)), Press::Drag);
    assert_eq!(sel.press(&rows, 0, Hit::Side(Side::Right)), Press::Swipe);
    assert_eq!(sel.press(&rows, 3, Hit::Side(Side::Left)), Press::Swipe);
    // Below the last row: nothing.
    sel.clear();
    assert!(sel.items.is_empty());
}

#[test]
fn a_swipe_crossing_the_centre_selects_both_sides() {
    let rows = rows();
    let mut sel = ViewSelection::new();
    let mut swipe = Swipe::new(0, Hit::Side(Side::Left));
    sel.swipe(&rows, &present, &swipe, 2);
    assert_eq!(picked(&sel), [(1, L), (2, L), (3, L)]);
    swipe.moved_over(Hit::Centre);
    sel.swipe(&rows, &present, &swipe, 3);
    assert_eq!(picked(&sel), [(1, B), (2, L), (3, B), (4, R)]);
    // Back on the pressed side: still both (it crossed once).
    swipe.moved_over(Hit::Side(Side::Left));
    sel.swipe(&rows, &present, &swipe, 1);
    assert_eq!(picked(&sel), [(1, B), (2, L)]);
    // Upwards from a lower row.
    let up = Swipe::new(5, Hit::Side(Side::Right));
    sel.swipe(&rows, &present, &up, 3);
    assert_eq!(picked(&sel), [(4, R), (5, R), (6, R)]);
}

#[test]
fn the_keyboard_table_restrict_and_the_mark() {
    let rows = rows();
    let mut sel = ViewSelection::new();
    sel.key_move(&rows, &present, 0, NONE);
    assert_eq!(picked(&sel), [(1, B)]);
    sel.key_move(&rows, &present, 2, SHIFT);
    assert_eq!(picked(&sel), [(1, B), (2, L), (3, B)]);
    // Ctrl moves the current row only; Ctrl+Space toggles it.
    sel.key_move(&rows, &present, 4, CTRL);
    assert_eq!(sel.current, Some(NodeId(5)));
    assert_eq!(picked(&sel).len(), 3);
    sel.toggle_current(&present);
    assert_eq!(picked(&sel), [(1, B), (2, L), (3, B), (5, B)]);
    sel.toggle_current(&present);
    assert_eq!(picked(&sel).len(), 3);
    // Shift+Right keeps the right side and focuses it.
    sel.restrict(Side::Right);
    assert_eq!(picked(&sel), [(1, R), (3, R)]);
    assert_eq!(sel.focus, R);
    // Space marks the current row on its first focused side; again unmarks; Esc unmarks.
    sel.key_move(&rows, &present, 3, NONE);
    sel.space(&present);
    assert_eq!(sel.mark, Some((NodeId(4), Side::Right)));
    sel.space(&present);
    assert_eq!(sel.mark, None);
    sel.space(&present);
    sel.unmark();
    assert_eq!(sel.mark, None);
}

#[test]
fn rows_no_longer_shown_leave_the_selection() {
    let rows = rows();
    let mut sel = ViewSelection::new();
    sel.click(&rows, &present, 0, Hit::Centre, NONE);
    sel.click(&rows, &present, 5, Hit::Centre, SHIFT);
    sel.key_move(&rows, &present, 2, CTRL);
    sel.space(&present);
    // Rows 3 and 4 collapse away; 5 loses its right side.
    let kept: Vec<VisibleRow> = rows
        .iter()
        .copied()
        .filter(|r| r.node.0 != 3 && r.node.0 != 4)
        .collect();
    let now = |n: NodeId| if n.0 == 5 { L } else { present(n) };
    sel.retain_shown(&kept, &now);
    assert_eq!(picked(&sel), [(1, B), (2, L), (5, L), (6, B)]);
    assert_eq!(sel.current, None);
    assert_eq!(sel.mark, None);
    // A refresh's renumbering moves the ids.
    sel.renumber(&|n: NodeId| NodeId(n.0 + 10));
    assert_eq!(picked(&sel), [(11, B), (12, L), (15, L), (16, B)]);
}

/// A tiny deterministic generator (xorshift64*): the GUI crate has no proptest, so the property draws its own cases.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn hit_of(k: usize) -> Hit {
    match k % 3 {
        0 => Hit::Side(Side::Left),
        1 => Hit::Side(Side::Right),
        _ => Hit::Centre,
    }
}

/// Property (A9.4): after any sequence of gestures, keys and display changes, every selected node is a shown row and is
/// present on each side it is selected on; the mark too. 2,000 seeded sequences of 40 steps.
#[test]
fn p_random_gestures_keep_the_selection_shown_and_present() {
    for seed in 1..=2000u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let total = 1 + rng.below(12);
        let sides: Vec<SideSet> = (0..=total)
            .map(|_| match rng.below(3) {
                0 => L,
                1 => R,
                _ => B,
            })
            .collect();
        let mut shown: Vec<VisibleRow> = (1..=total as u32)
            .map(|n| VisibleRow {
                node: NodeId(n),
                depth: 0,
            })
            .collect();
        let present = |n: NodeId| sides.get(n.0 as usize).copied().unwrap_or(B);
        let mut sel = ViewSelection::new();
        let mut swipe: Option<Swipe> = None;
        for _ in 0..40 {
            if shown.is_empty() {
                break;
            }
            let at = rng.below(shown.len());
            let mods = Mods {
                ctrl: rng.below(3) == 0,
                shift: rng.below(3) == 0,
            };
            match rng.below(10) {
                0 | 1 => sel.click(&shown, &present, at, hit_of(rng.below(3)), mods),
                2 => {
                    let s = swipe.get_or_insert_with(|| Swipe::new(at, hit_of(rng.below(3))));
                    s.moved_over(hit_of(rng.below(3)));
                    let s = *s;
                    sel.swipe(&shown, &present, &s, at);
                }
                3 => sel.key_move(&shown, &present, at, mods),
                4 => sel.toggle_current(&present),
                5 => sel.restrict(if rng.below(2) == 0 {
                    Side::Left
                } else {
                    Side::Right
                }),
                6 => sel.space(&present),
                7 => {
                    // A collapse or a display change: some rows go; the model drops them.
                    let gone = rng.below(shown.len());
                    shown.remove(gone);
                    swipe = None;
                    sel.retain_shown(&shown, &present);
                }
                8 => sel.clear(),
                _ => swipe = None,
            }
            for (node, picked) in sel.items.iter() {
                assert!(
                    shown.iter().any(|r| r.node == node),
                    "seed {seed}: {node:?} selected but not shown"
                );
                assert!(
                    present(node).contains(picked) && !picked.is_empty(),
                    "seed {seed}: {node:?} selected on {picked:?}, present {:?}",
                    present(node)
                );
            }
            if let Some((node, side)) = sel.mark {
                assert!(shown.iter().any(|r| r.node == node), "seed {seed}: mark");
                assert!(
                    present(node).contains(SideSet::of(side)),
                    "seed {seed}: mark side"
                );
            }
        }
    }
}

/// Mutation E: the Space mark leaves with its side: a shown row that no longer has the marked side drops the mark,
/// one that still has it keeps it.
#[test]
fn the_mark_leaves_with_its_side() {
    let rows = vec![VisibleRow {
        node: NodeId(1),
        depth: 0,
    }];
    let mut sel = ViewSelection::new();
    sel.mark = Some((NodeId(1), Side::Left));
    sel.current = Some(NodeId(1));
    sel.retain_shown(&rows, &|_| SideSet::RIGHT);
    assert_eq!(sel.mark, None, "the left item is gone");
    assert_eq!(sel.current, Some(NodeId(1)), "the row stays");
    sel.mark = Some((NodeId(1), Side::Right));
    sel.retain_shown(&rows, &|_| SideSet::RIGHT);
    assert_eq!(sel.mark, Some((NodeId(1), Side::Right)));
}

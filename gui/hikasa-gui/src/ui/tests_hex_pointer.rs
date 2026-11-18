//! A13.3 (M2 W13a, W13a review F12, F13): the Hex Compare view's pointer and keys through the real drawing code
//! (`ui/hex.rs`) — a click reaches the cell under it with the panes scrolled sideways, a drag selects and scrolls at the
//! pane's edge, PgDn keeps the caret on the screen, F3 runs Find Next, Enter in a header's path reopens that side, Enter
//! in the Find panel searches, the header's Format button opens its menu on a left click (and leaves no menu behind),
//! Ctrl+C in the value panel copies its lines, Shift+F10 opens the pane's menu at the caret, the splitter drags.

use egui::{Event, FullOutput, Key, Modifiers, PointerButton, Pos2, Rect, Shape, pos2, vec2};
use hikasa_core::Side;
use hikasa_session::ipc::ViewId;

use super::HikasaApp;
use super::tests::{frame, new_app, pointer_button, raw};
use crate::hex::HexViewModel;
use crate::hex::pane::{CaretKey, RowGeometry};
use crate::tests::World;
use crate::tests_hex_open::{HEX_FORMAT, bytes};
use crate::window::ViewModel;

/// One text on screen.
#[derive(Debug, Clone)]
struct Drawn {
    text: String,
    rect: Rect,
}

fn collect(shape: &Shape, out: &mut Vec<Drawn>) {
    match shape {
        Shape::Text(t) => out.push(Drawn {
            text: t.galley.text().to_owned(),
            rect: t.galley.rect.translate(t.pos.to_vec2()),
        }),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

/// A window with one hex view, one frame at a time.
struct Screen {
    world: World,
    ctx: egui::Context,
    app: HikasaApp,
    time: f64,
    view: ViewId,
    clip: std::sync::Arc<hikasa_session::clipboard::FixedClipboard>,
}

impl Screen {
    /// The pair `a`, `b` (`.bin`, a Hex format) open in a window of 1200 × 800.
    fn open(a: &[u8], b: &[u8]) -> Screen {
        let world = World::with_files(&[("formats/firmware.toml", HEX_FORMAT)]);
        let ctx = egui::Context::default();
        let mut app = new_app(&world, &ctx);
        let clip = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::text("clip"));
        app.core
            .set_script_clipboard(std::sync::Arc::clone(&clip) as _);
        let (pa, pb) = crate::tests_hex_open::write_pair(&world, a, b);
        let view = world.open(&mut app.core, &pa, &pb);
        let mut s = Screen {
            world,
            ctx,
            app,
            time: 0.0,
            view,
            clip,
        };
        s.settle();
        s.model().set_filter(hikasa_hex::DisplayFilter::All);
        s.settle();
        s
    }

    fn model(&mut self) -> &mut HexViewModel {
        match self.app.core.tab_mut(self.view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Hex(m)) => m,
            _ => panic!("the hex view"),
        }
    }

    fn input(&mut self, events: Vec<Event>) -> FullOutput {
        self.time += 0.1;
        let mut input = raw(self.time);
        input.events = events;
        frame(&mut self.app, &self.ctx, input)
    }

    fn settle(&mut self) -> FullOutput {
        self.world.settle(&mut self.app.core);
        for _ in 0..3 {
            self.input(Vec::new());
            self.world.settle(&mut self.app.core);
        }
        self.input(Vec::new())
    }

    fn drawn(&mut self) -> Vec<Drawn> {
        let out = self.settle();
        let mut drawn = Vec::new();
        for clipped in &out.shapes {
            collect(&clipped.shape, &mut drawn);
        }
        drawn
    }

    fn texts(&mut self) -> Vec<String> {
        self.drawn().into_iter().map(|d| d.text).collect()
    }

    /// The leftmost drawn text equal to `text` (the left pane's).
    fn find(&mut self, text: &str) -> Rect {
        let drawn = self.drawn();
        drawn
            .iter()
            .filter(|d| d.text == text)
            .min_by(|a, b| a.rect.left().total_cmp(&b.rect.left()))
            .map(|d| d.rect)
            .unwrap_or_else(|| {
                let all: Vec<&str> = drawn.iter().map(|d| d.text.as_str()).collect();
                panic!("{text:?} is not on screen: {all:?}")
            })
    }

    fn press_release(&mut self, pos: Pos2, button: PointerButton) {
        self.input(vec![Event::PointerMoved(pos)]);
        self.input(vec![Event::PointerMoved(pos)]);
        self.input(vec![pointer_button(pos, button, true)]);
        self.input(vec![pointer_button(pos, button, false)]);
        self.input(vec![Event::PointerGone]);
    }

    fn click_at(&mut self, pos: Pos2) {
        self.press_release(pos, PointerButton::Primary);
        self.settle();
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.input(vec![
            Event::ModifiersChanged(modifiers),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ]);
        self.input(vec![
            Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
            Event::ModifiersChanged(Modifiers::NONE),
        ]);
        self.settle();
    }

    /// The row geometry of the view and the drawn hex text of filtered row `k` of the left pane: (geometry, the
    /// text's rect, the advance of a digit).
    fn hex_row(&mut self, k: u64) -> (RowGeometry, Rect, f32) {
        let m = self.model();
        let d = *m.display();
        let width = match d.layout.address {
            hikasa_hex::Radix::Hex => {
                hikasa_hex::nav::address_text(Side::Left, 0, d.layout.address, m.address_width())
                    .len() as u32
            }
            hikasa_hex::Radix::Decimal => u32::from(m.address_width()),
        };
        let geometry = RowGeometry::new(
            d.addresses.then_some(width),
            d.layout.bytes_per_line,
            d.layout.group,
            d.layout.chars != hikasa_hex::CharSet::None,
        );
        let g = m.generation;
        let row = m.screen.cached(g, k).cloned().expect("the row is drawn");
        let mut text = String::new();
        let mut last = geometry.address;
        for (i, cell) in row.cells.iter().enumerate() {
            let x = geometry.hex_x(i as u32);
            text.push_str(&" ".repeat((x - last) as usize));
            last = x + 2;
            match cell.left {
                Some(b) => text.push_str(&format!("{b:02x}")),
                None => text.push_str("  "),
            }
        }
        let rect = self.find(&text);
        let cw = rect.width() / text.chars().count() as f32;
        (geometry, rect, cw)
    }

    /// The centre of byte `i` of the left pane's filtered row `k` on screen.
    fn cell(&mut self, k: u64, i: u32) -> Pos2 {
        let (geometry, rect, cw) = self.hex_row(k);
        let x = rect.left() + (geometry.hex_x(i) - geometry.address) as f32 * cw + cw;
        pos2(x, rect.center().y)
    }

    /// The left pane's body (from the hex view's drop rects: header and rows).
    fn pane(&mut self, side: Side) -> Rect {
        self.settle();
        self.app.hexes[&self.view]
            .drop_rects
            .iter()
            .find(|(s, _)| *s == side)
            .map(|(_, r)| *r)
            .expect("the pane")
    }
}

/// F12: a click reaches the cell under the pointer with the panes scrolled sideways (`hit_at`'s `scroll_x - PAD`).
#[test]
fn a13_3_a_click_reaches_the_cell_with_the_panes_scrolled_sideways() {
    let a = bytes(201, 64 * 200);
    let mut s = Screen::open(&a, &a);
    let m = s.model();
    let layout = hikasa_hex::HexLayout {
        bytes_per_line: 64,
        ..m.display().layout
    };
    m.set_layout(layout, true, crate::hex::PaneLayout::SideBySide, 0);
    s.settle();
    s.model().set_scroll_x(150.0);
    let at = s.cell(2, 20);
    s.click_at(at);
    let m = s.model();
    assert_eq!(m.focus(), Side::Left);
    assert_eq!(m.pane(Side::Left).caret, 2 * 64 + 20);
}

/// F12: press and drag selects from the pressed cell; held below the pane, the view scrolls (both panes together).
#[test]
fn a13_3_a_drag_selects_and_scrolls_at_the_edge() {
    let a = bytes(202, 16 * 2000);
    let mut s = Screen::open(&a, &a);
    let from = s.cell(1, 3);
    let body = s.pane(Side::Left);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    let below = pos2(from.x, body.bottom() + 20.0);
    for _ in 0..12 {
        s.input(vec![Event::PointerMoved(below)]);
    }
    s.input(vec![pointer_button(below, PointerButton::Primary, false)]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    let m = s.model();
    assert!(
        m.screen.top >= 5,
        "scrolled at the edge: top {}",
        m.screen.top
    );
    let sel = m.selection();
    assert_eq!(sel.start, 16 + 3, "from the pressed cell");
    assert!(
        sel.end > (u64::from(m.screen.rows_visible) + 1) * 16,
        "past the first screen: {sel:?}"
    );
}

/// F3, F12: PgDn as the window runs it pages the screen once and keeps the caret on it, at the same place.
#[test]
fn a13_3_page_down_keeps_the_caret_on_the_screen() {
    let a = bytes(203, 16 * 3000);
    let mut s = Screen::open(&a, &a);
    let at = s.cell(4, 2);
    s.click_at(at);
    assert_eq!(s.model().pane(Side::Left).caret, 4 * 16 + 2);
    for key in [Key::PageDown, Key::PageDown, Key::PageUp] {
        let before = {
            let m = s.model();
            (
                m.screen.top,
                m.row_of_column(m.pane(Side::Left).caret).expect("row") - m.screen.top,
            )
        };
        s.key(key, Modifiers::NONE);
        let m = s.model();
        let rows = u64::from(m.screen.rows_visible);
        let row = m.row_of_column(m.pane(Side::Left).caret).expect("row");
        assert_eq!(m.screen.top.abs_diff(before.0), rows, "{key:?}: one screen");
        assert_eq!(row - m.screen.top, before.1, "{key:?}: the same place");
    }
    let _ = CaretKey::PageDown;
}

/// F12: F3 composes to Find Next in the panes (the catalogue's key through `caret_keys`).
#[test]
fn a13_3_f3_runs_find_next() {
    let mut a = bytes(204, 5000);
    a[3000..3004].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let mut s = Screen::open(&a, &a);
    let at = s.cell(0, 0);
    s.click_at(at);
    let m = s.model();
    m.find.pattern = "de ad be ef".into();
    m.find.hex = true;
    m.validate_find();
    s.key(Key::F3, Modifiers::NONE);
    let m = s.model();
    assert_eq!(m.find_hit().map(|(_, r)| r.clone()), Some(3000..3004));
}

/// F12: Enter in a header's path reopens that side with the typed path.
#[test]
fn a13_3_enter_in_a_header_reopens_that_side() {
    let a = bytes(205, 3000);
    let mut s = Screen::open(&a, &a);
    let other = s.world.path("work/h/c.bin");
    std::fs::write(&other, bytes(206, 3001)).expect("write");
    let shown = s.model().inputs()[0].clone();
    let field = s.find(&shown);
    s.click_at(field.center());
    s.key(Key::A, Modifiers::COMMAND);
    s.input(vec![Event::Text(other.display().to_string())]);
    s.key(Key::Enter, Modifiers::NONE);
    s.settle();
    assert_eq!(s.model().inputs()[0], other.display().to_string());
    assert_eq!(s.model().inputs()[1], shown.replace("a.bin", "b.bin"));
}

/// F12: Enter in the Find panel's pattern searches (Find Next).
#[test]
fn a13_3_enter_in_the_find_panel_searches() {
    let mut a = bytes(207, 5000);
    a[4000..4006].copy_from_slice(b"NEEDLE");
    let mut s = Screen::open(&a, &a);
    s.model().open_find();
    s.model().find.hex = false;
    s.model().find.pattern.clear();
    s.settle();
    let field = s.find("text");
    s.click_at(field.center());
    s.input(vec![Event::Text("NEEDLE".into())]);
    s.key(Key::Enter, Modifiers::NONE);
    s.settle();
    assert_eq!(
        s.model().find_hit().map(|(_, r)| r.clone()),
        Some(4000..4006)
    );
}

/// F13: a left click on a header's Format button opens the header's menu; a right-click below the rows of a pane then
/// shows the pane's menu (no menu of the header is left over).
#[test]
fn a13_3_the_format_button_opens_its_menu_and_leaves_none_behind() {
    let a = bytes(208, 64);
    let mut s = Screen::open(&a, &a);
    assert!(!s.texts().iter().any(|t| t == "Open Clipboard"));
    let button = s.find("Firmware");
    s.click_at(button.center());
    let texts = s.texts();
    assert!(
        texts.iter().any(|t| t == "Open Clipboard"),
        "the header's menu: {texts:?}"
    );
    s.key(Key::Escape, Modifiers::NONE);
    assert!(!s.texts().iter().any(|t| t == "Open Clipboard"), "closed");
    let body = s.pane(Side::Right);
    let below = pos2(body.center().x, body.bottom() - 10.0);
    s.press_release(below, PointerButton::Secondary);
    let texts = s.texts();
    assert!(
        texts.iter().any(|t| t == "Copy as Text"),
        "the pane's menu: {texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "Open Clipboard"), "{texts:?}");
}

/// F13: Ctrl+C in the value panel copies its lines (not the bytes).
#[test]
fn a13_3_ctrl_c_in_the_value_panel_copies_its_lines() {
    let a = bytes(209, 3000);
    let mut s = Screen::open(&a, &a);
    s.model().set_values(hikasa_session::api::HexValues::Little);
    s.model().set_caret(5, false);
    s.settle();
    let lines = s
        .model()
        .value_lines()
        .map(|(t, _)| t.clone())
        .expect("lines");
    let panel = s
        .drawn()
        .into_iter()
        .find(|d| d.text.starts_with("Little Endian Values"))
        .map(|d| d.rect)
        .expect("the panel");
    s.click_at(panel.center());
    s.key(Key::C, Modifiers::COMMAND);
    s.settle();
    assert_eq!(
        s.clip.payloads().last().map(|p| p.text.clone()),
        Some(lines)
    );
}

/// §1.13.8: Shift+F10 opens the focus pane's menu at the caret.
#[test]
fn a13_3_shift_f10_opens_the_pane_menu_at_the_caret() {
    let a = bytes(210, 16 * 100);
    let mut s = Screen::open(&a, &a);
    let at = s.cell(6, 9);
    s.click_at(at);
    assert!(!s.texts().iter().any(|t| t == "Copy as Text"));
    s.key(Key::F10, Modifiers::SHIFT);
    let entry = s
        .drawn()
        .into_iter()
        .find(|d| d.text == "Copy as Text")
        .map(|d| d.rect)
        .expect("the pane's menu");
    assert!(
        (entry.left() - at.x).abs() < 80.0 && entry.top() > at.y && entry.top() - at.y < 120.0,
        "the menu at the caret {at:?}: {entry:?}"
    );
}

/// §1.13.4: a drag of the splitter moves it; the share is the view type's layout key.
#[test]
fn a13_3_the_splitter_drags_and_is_kept() {
    let a = bytes(211, 3000);
    let mut s = Screen::open(&a, &a);
    let left = s.pane(Side::Left);
    let from = pos2(left.right() + 0.5, left.center().y);
    let by = vec2(120.0, 0.0);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![Event::PointerMoved(from)]);
    s.input(vec![pointer_button(from, PointerButton::Primary, true)]);
    for step in 1..=4 {
        s.input(vec![Event::PointerMoved(from + by * (step as f32 / 4.0))]);
    }
    s.input(vec![pointer_button(
        from + by,
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![Event::PointerGone]);
    s.settle();
    let split = s.model().display().split;
    assert!(split > 0.55, "the splitter moved: {split}");
    let wider = s.pane(Side::Left);
    assert!(wider.width() > left.width() + 100.0);
    let keys = s.model().layout_keys();
    assert!(
        keys.iter().any(|(k, v)| *k == "splitter"
            && matches!(v, hikasa_session::settings::state::LayoutValue::Float(f) if (*f - f64::from(split)).abs() < 0.01)),
        "{keys:?}"
    );
}

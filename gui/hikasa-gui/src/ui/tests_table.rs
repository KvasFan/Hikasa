//! The Table Compare grid over a headless `egui::Context` (no window opens): what only the drawing layer decides — it
//! lays out the cells on screen only (ARCH-GUI-002), a click on a cell or a sheet tab reaches the model, the keys
//! scroll and move the current cell, a drag on a head's edge resizes its column, both layouts draw.

use egui::{Event, FullOutput, Key, Modifiers, PointerButton, Pos2, Rect, Shape};
use hikasa_core::Side;

use super::HikasaApp;
use super::tests::{frame, new_app, pointer_button, raw};
use crate::commands::CommandId;
use crate::table_view::TableViewModel;
use crate::tests::World;
use crate::window::ViewModel;

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

struct Screen {
    world: World,
    ctx: egui::Context,
    app: HikasaApp,
    time: f64,
    view: hikasa_session::ipc::ViewId,
}

impl Screen {
    /// A window showing a table view of `left` and `right` (relative to the world).
    fn open(world: World, left: &str, l: &str, right: &str, r: &str) -> Screen {
        let ctx = egui::Context::default();
        let mut app = new_app(&world, &ctx);
        let a = world.write(left, l);
        let b = world.write(right, r);
        let view = world.open(&mut app.core, &a, &b);
        let mut s = Screen {
            world,
            ctx,
            app,
            time: 0.0,
            view,
        };
        s.settle();
        s
    }

    fn input(&mut self, events: Vec<Event>) -> FullOutput {
        self.time += 0.05;
        let mut input = raw(self.time);
        input.events = events;
        frame(&mut self.app, &self.ctx, input)
    }

    fn settle(&mut self) -> FullOutput {
        self.world.settle(&mut self.app.core);
        for _ in 0..3 {
            self.input(Vec::new());
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

    fn find(&mut self, text: &str) -> Rect {
        let drawn = self.drawn();
        drawn
            .iter()
            .rev()
            .find(|d| d.text == text)
            .map(|d| d.rect)
            .unwrap_or_else(|| {
                let all: Vec<&str> = drawn.iter().map(|d| d.text.as_str()).take(80).collect();
                panic!("{text:?} is not on screen: {all:?}")
            })
    }

    fn click_at(&mut self, pos: Pos2) {
        self.input(vec![Event::PointerMoved(pos)]);
        self.input(vec![Event::PointerMoved(pos)]);
        self.input(vec![pointer_button(pos, PointerButton::Primary, true)]);
        self.input(vec![pointer_button(pos, PointerButton::Primary, false)]);
        self.input(vec![Event::PointerGone]);
        self.settle();
    }

    fn key(&mut self, k: Key) {
        self.input(vec![Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.input(Vec::new());
    }

    fn model(&mut self) -> &mut TableViewModel {
        match self.app.core.tab_mut(self.view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Table(m)) => m,
            _ => panic!("not a table view"),
        }
    }

    fn top(&self) -> f64 {
        self.app.tables.get(&self.view).map_or(0.0, |s| s.top)
    }
}

fn big(rows: usize, change_every: usize) -> (String, String) {
    let mut l = String::from("id,a,b,c\n");
    let mut r = l.clone();
    for i in 0..rows {
        l.push_str(&format!("{i},a{i},b{i},c{i}\n"));
        if i % change_every == 7 {
            r.push_str(&format!("{i},A{i},b{i},c{i}\n"));
        } else {
            r.push_str(&format!("{i},a{i},b{i},c{i}\n"));
        }
    }
    (l, r)
}

/// ARCH-GUI-002: 20,000 rows, a few hundred texts on screen; PgDn scrolls a page, End reaches the last row and the
/// cells of the last rows are drawn.
#[test]
fn the_grid_lays_out_the_screen_only() {
    let (l, r) = big(20_000, 1000);
    let mut s = Screen::open(World::new(), "work/l.csv", &l, "work/r.csv", &r);
    assert_eq!(s.model().len(), 20_000);
    let drawn = s.drawn();
    assert!(
        (20..2_000).contains(&drawn.len()),
        "{} texts on screen",
        drawn.len()
    );
    assert!(drawn.iter().any(|d| d.text == "a0"), "the first row");
    let before = s.top();
    s.key(Key::PageDown);
    assert!(s.top() > before, "PgDn scrolls: {before} → {}", s.top());
    s.key(Key::End);
    s.settle();
    let drawn = s.drawn();
    assert!(
        drawn.iter().any(|d| d.text == "c19999"),
        "the last row is drawn"
    );
}

/// A click on a cell makes it the current cell on its side; the arrow keys move it; a click on a sheet tab shows the
/// sheet.
#[test]
fn clicks_and_keys_reach_the_model() {
    let mut s = Screen::open(
        World::new(),
        "work/a.csv",
        "id,name\n1,apple\n2,pear\n",
        "work/b.csv",
        "id,name\n1,apple\n2,Pear\n",
    );
    let at = s.find("Pear").center();
    s.click_at(at);
    let cursor = s.model().cursor().expect("a current cell");
    assert_eq!((cursor.row, cursor.col, cursor.side), (1, 1, Side::Right));
    s.key(Key::ArrowUp);
    let cursor = s.model().cursor().expect("a current cell");
    assert_eq!((cursor.row, cursor.col), (0, 1));
    s.key(Key::ArrowLeft);
    assert_eq!(s.model().cursor().map(|c| c.col), Some(0));
    // The status bar names the cell.
    let root = s.app.core.root();
    let status = s.app.core.status(root);
    assert!(status.left.contains("A2 (right)"), "{}", status.left);
}

/// A drag on the right edge of a column head widens the column; both layouts draw every side's cells.
#[test]
fn resize_and_layouts() {
    let mut s = Screen::open(
        World::new(),
        "work/a.csv",
        "id,name\n1,apple\n",
        "work/b.csv",
        "id,name\n1,apple\n",
    );
    let before = s.model().column_head(0).expect("id").width;
    // The edge of the first column: the left of the second head minus a pixel.
    let edge = s.find("name").left_center() - egui::vec2(5.0, 0.0);
    s.input(vec![Event::PointerMoved(edge)]);
    s.input(vec![Event::PointerMoved(edge)]);
    s.input(vec![pointer_button(edge, PointerButton::Primary, true)]);
    for k in 1..=4 {
        s.input(vec![Event::PointerMoved(
            edge + egui::vec2(15.0 * k as f32, 0.0),
        )]);
    }
    s.input(vec![pointer_button(
        edge + egui::vec2(60.0, 0.0),
        PointerButton::Primary,
        false,
    )]);
    s.input(vec![Event::PointerGone]);
    let after = s.model().column_head(0).expect("id").width;
    assert!(after > before + 2.0, "{before} → {after}");
    let root = s.app.core.root();
    s.app.core.execute(root, CommandId::OverUnder);
    let drawn = s.drawn();
    let apples: Vec<&Drawn> = drawn.iter().filter(|d| d.text == "apple").collect();
    assert_eq!(apples.len(), 2, "both sides");
    assert!(
        (apples[0].rect.center().y - apples[1].rect.center().y).abs() > 50.0,
        "one above the other"
    );
}

/// The sheet tabs of two workbooks: a click on one shows its rows.
#[test]
fn a_sheet_tab_click_shows_the_sheet() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/table");
    let copy = |name: &str| {
        let to = world.path(&format!("work/{name}"));
        std::fs::copy(dir.join(name), &to).expect("copy");
        to
    };
    let (l, r) = (copy("books-left.xlsx"), copy("books-right.xlsx"));
    let view = world.open(&mut app.core, &l, &r);
    let mut s = Screen {
        world,
        ctx,
        app,
        time: 0.0,
        view,
    };
    s.settle();
    assert_eq!(s.model().active_sheet(), 0);
    let at = s.find("Prices").center();
    s.click_at(at);
    assert_eq!(s.model().active_sheet(), 1);
    assert!(
        s.drawn().iter().any(|d| d.text == "plum"),
        "the Prices rows"
    );
}

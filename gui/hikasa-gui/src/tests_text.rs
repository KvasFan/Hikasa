//! The fixture of the W11a text editor tests (contract §6.11, *Through the models*): an `AppCore` over a temporary
//! configuration with a `FixedClipboard` (primary selection included), a Text Compare view of generated files, and
//! explicit frame times (`now`) for the debounce, the blink and the primary selection's settle. Seeded xorshift
//! (§6.1) for the generated pairs and the random scripts.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::Side;
use hikasa_session::clipboard::FixedClipboard;
use hikasa_session::ipc::ViewId;
use hikasa_text::edit::{EditableDocument, TextPos, doc_end};

use crate::app::UiMsg;
use crate::commands::CommandId;
use crate::tests::{TestApp, World, text_model};
use crate::text::TextViewModel;
use crate::text::input::{self, Key, Press};
use crate::text::wrap::FixedMetrics;

/// Seeded xorshift (§6.1).
pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub(crate) fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

/// A generated pair of `lines` lines of C-like text: the right side changes about one line in eight.
pub(crate) fn generated_pair(lines: usize, seed: u64) -> (String, String) {
    let mut rng = Rng(seed | 1);
    let mut left = String::new();
    let mut right = String::new();
    for i in 0..lines {
        let line = format!("int v{i} = f({}, {});\n", rng.below(100), i % 7);
        left.push_str(&line);
        match rng.below(16) {
            0 => right.push_str(&format!("int v{i} = g({});\n", rng.below(100))),
            1 => {}
            2 => {
                right.push_str(&line);
                right.push_str("// added\n");
            }
            _ => right.push_str(&line),
        }
    }
    (left, right)
}

/// A Text Compare view of two files under test.
pub(crate) struct Fixture {
    pub(crate) world: World,
    pub(crate) app: TestApp,
    pub(crate) view: ViewId,
    pub(crate) clip: Arc<FixedClipboard>,
    pub(crate) paths: [PathBuf; 2],
    /// The frame time the tests advance.
    pub(crate) now: f64,
}

impl Fixture {
    /// Writes `left` and `right` (names `left.c`, `right.c`) and opens them; settles the open.
    pub(crate) fn new(left: &str, right: &str) -> Fixture {
        Fixture::named(left, right, "left.c", "right.c")
    }

    /// [`Fixture::new`] with the files' names.
    pub(crate) fn named(left: &str, right: &str, lname: &str, rname: &str) -> Fixture {
        let world = World::new();
        let l = world.write(&format!("work/{lname}"), left);
        let r = world.write(&format!("work/{rname}"), right);
        Fixture::open_paths(world, l, r)
    }

    /// Opens two existing files of `world`.
    pub(crate) fn open_paths(world: World, l: PathBuf, r: PathBuf) -> Fixture {
        let mut app = world.app(None);
        let clip = Arc::new(FixedClipboard::text("clip"));
        app.0.set_script_clipboard(Arc::clone(&clip) as _);
        let view = world.open(&mut app, &l, &r);
        let mut f = Fixture {
            world,
            app,
            view,
            clip,
            paths: [l, r],
            now: 1.0,
        };
        f.settle();
        f
    }

    /// The view's model.
    pub(crate) fn m(&mut self) -> &mut TextViewModel {
        text_model(&mut self.app, self.view)
    }

    /// Advances the frame clock by `dt` and runs one frame's polling.
    pub(crate) fn frame(&mut self, dt: f64) {
        self.now += dt;
        let now = self.now;
        self.m().set_now(now);
        crate::text::clipboard::tick(self.m());
        for msg in self.world.mailbox.drain() {
            self.app.handle(msg);
        }
        self.app.handle(UiMsg::Tick);
    }

    /// Frames until nothing runs and nothing waits (the debounce passed, every recompare installed, no job).
    pub(crate) fn settle(&mut self) {
        let began = Instant::now();
        loop {
            self.frame(0.05);
            let idle = {
                let m = self.m();
                m.recompare_state().idle() && !m.busy()
            };
            if idle && !self.app.busy() && self.world.mailbox.is_empty() {
                return;
            }
            assert!(
                began.elapsed() < Duration::from_secs(60),
                "the view did not settle"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A key on the focus pane (FixedMetrics, a page of 10 visual lines).
    pub(crate) fn key(&mut self, press: Press) -> bool {
        input::key(self.m(), press, &FixedMetrics::default(), 10)
    }

    /// A plain key.
    pub(crate) fn plain(&mut self, key: Key) -> bool {
        self.key(Press::plain(key))
    }

    /// Typed text on the focus pane at the current frame time.
    pub(crate) fn type_text(&mut self, text: &str) {
        input::text(self.m(), text);
    }

    /// Runs a command of the window.
    pub(crate) fn run(&mut self, id: CommandId) {
        self.app.execute(id);
    }

    /// The whole text of a pane.
    pub(crate) fn text(&mut self, side: Side) -> String {
        let doc = self.doc(side);
        doc.text(TextPos::ZERO..doc_end(&doc))
    }

    /// A clone of a pane's document.
    pub(crate) fn doc(&mut self, side: Side) -> EditableDocument {
        self.m().doc(side).expect("a document").clone()
    }

    /// The caret of a pane.
    pub(crate) fn caret(&mut self, side: Side) -> TextPos {
        self.m().pane(side).caret
    }

    /// Puts the caret of the focus pane at `pos` (no selection).
    pub(crate) fn place(&mut self, side: Side, line: u32, byte: u32) {
        let m = self.m();
        m.set_focus(side);
        m.pane_mut(side).set_caret(TextPos::new(line, byte), false);
    }

    /// The comparison of the view as `hikasa.text_compare.v1` without the files' specs and times.
    pub(crate) fn json(&mut self) -> serde_json::Value {
        let response = self.m().response().expect("a response").clone();
        let cmp = std::sync::Arc::clone(self.m().comparison().expect("a comparison"));
        let r = hikasa_session::api::TextCompareResponse {
            comparison: cmp,
            ..response
        };
        strip(serde_json::to_value(r.json(&hikasa_text::ViewOptions::default())).expect("json"))
    }
}

/// A JSON document without the parts that name or time the files.
pub(crate) fn strip(mut v: serde_json::Value) -> serde_json::Value {
    fn walk(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                for key in [
                    "left",
                    "right",
                    "spec",
                    "specs",
                    "path",
                    "size",
                    "mtime",
                    "modified",
                    "elapsed_ms",
                    "times",
                    "verdict",
                    "exit_code",
                    "identical",
                    "encoding_source",
                    "encoding",
                ] {
                    if let Some(x) = map.get_mut(key)
                        && !x.is_array()
                        && !x.is_object()
                    {
                        *x = serde_json::Value::Null;
                    }
                }
                for (_, x) in map.iter_mut() {
                    walk(x);
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut v);
    v
}

/// A fresh comparison of two texts written to files (what `hikasa text compare --json` reads).
pub(crate) fn fresh_json(world: &World, left: &str, right: &str) -> serde_json::Value {
    let l = world.write("work/fresh/left.c", left);
    let r = world.write("work/fresh/right.c", right);
    let req = hikasa_session::api::TextCompareRequest {
        left: l.into_os_string(),
        right: r.into_os_string(),
        ..hikasa_session::api::TextCompareRequest::default()
    };
    let job = world.service.job(
        hikasa_core::JobKind::TextCompare,
        Arc::new(hikasa_core::NullProgress),
    );
    let response = world
        .service
        .text_compare(&req, &job, None)
        .expect("compare");
    strip(serde_json::to_value(response.json(&hikasa_text::ViewOptions::default())).expect("json"))
}

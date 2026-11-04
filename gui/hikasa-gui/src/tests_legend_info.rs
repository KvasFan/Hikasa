//! M2 W9a acceptance A9.8: the Legend lists every row of the glyph table and every colour class once, painted in the
//! palette's colours, toggles and is remembered in `[layout] legend`; Info's figures equal `hikasa folder compare
//! --json`'s statistics for the same tree, are partial while scanning, and Copy places the text on the clipboard.

use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_folder::Category;
use hikasa_session::api::FolderCompareRequest;
use hikasa_session::clipboard::FixedClipboard;
use hikasa_session::settings::state::LayoutValue;

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::folder::glyphs::GLYPH_TABLE;
use crate::info_dialog::{COPY, InfoDialog};
use crate::legend::{COLOUR_CLASSES, Swatch, sections};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{gate, open, poll_until, press, put, set_gate, top_title};

#[test]
fn a9_8_the_legend_lists_every_glyph_and_colour_class_once() {
    let all = sections();
    let glyphs: Vec<_> = all
        .iter()
        .flat_map(|s| &s.entries)
        .filter_map(|e| match e.swatch {
            Swatch::Glyph(g) => Some((g, e.text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        glyphs,
        GLYPH_TABLE.to_vec(),
        "every glyph row once, in order"
    );
    let classes: Vec<_> = all
        .iter()
        .flat_map(|s| &s.entries)
        .filter_map(|e| match e.swatch {
            Swatch::Class(c, k) => Some((c, k)),
            _ => None,
        })
        .collect();
    assert_eq!(
        classes.len(),
        8,
        "same, newer, older, orphan, different, unknown, error, filtered out"
    );
    let expected: Vec<_> = COLOUR_CLASSES.iter().map(|(c, k, _)| (*c, *k)).collect();
    assert_eq!(classes, expected);
    // Each class has a colour in both built-in palettes (classes may share one: newer and different, by default).
    for scheme in ["light", "dark"] {
        let pal = crate::ui::theme::Palette::from_scheme(
            &hikasa_session::settings::Scheme::resolve(scheme, None),
        );
        assert!(
            classes.iter().all(|(c, k)| pal.folder(*c, *k).is_some()),
            "{scheme}"
        );
    }
    // The centre column: both arrows, every other category's mark, every content result, the badge and the ring.
    let marks = all
        .iter()
        .flat_map(|s| &s.entries)
        .filter(|e| matches!(e.swatch, Swatch::Mark(_)))
        .count();
    assert_eq!(
        marks,
        2 + (Category::ALL.len() - 2) + hikasa_core::ContentKind::ALL.len()
    );
    for swatch in [Swatch::Ignored, Swatch::Marked] {
        assert_eq!(
            all.iter()
                .flat_map(|s| &s.entries)
                .filter(|e| e.swatch == swatch)
                .count(),
            1
        );
    }
    legend_text_lines(&crate::legend::text());
}

/// One line per entry and per section title.
fn legend_text_lines(text: &str) {
    assert!(text.starts_with("Colours\n"), "{text}");
    assert_eq!(
        text.lines().count(),
        3 + 8 + GLYPH_TABLE.len() + 17,
        "{text}"
    );
}

#[test]
fn a9_8_the_legend_paints_with_the_palette() {
    let ctx = egui::Context::default();
    let pal = crate::ui::theme::light();
    let all = sections();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 1600.0),
        )),
        time: Some(0.0),
        ..egui::RawInput::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        egui::Panel::right("legend")
            .exact_size(320.0)
            .show(ui, |ui| crate::ui::legend::show(ui, &all, &pal));
    });
    out.textures_delta.clear();
    let mut fills = Vec::new();
    let mut strokes = Vec::new();
    for clipped in &out.shapes {
        collect(&clipped.shape, &mut fills, &mut strokes);
    }
    for (class, category, text) in COLOUR_CLASSES {
        let colour = pal.folder(*class, *category).expect("a colour");
        assert!(fills.contains(&colour), "{text}: {colour:?} not painted");
    }
    for unusual in [pal.c_unknown, pal.c_unopened, pal.error] {
        assert!(
            fills.contains(&unusual) || strokes.contains(&unusual),
            "{unusual:?} not painted"
        );
    }
    assert!(strokes.contains(&pal.accent), "the Space ring");
}

fn collect(shape: &egui::Shape, fills: &mut Vec<egui::Color32>, strokes: &mut Vec<egui::Color32>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, fills, strokes)),
        egui::Shape::Circle(c) => {
            fills.push(c.fill);
            strokes.push(c.stroke.color);
        }
        egui::Shape::Rect(r) => {
            fills.push(r.fill);
            strokes.push(r.stroke.color);
        }
        egui::Shape::LineSegment { stroke, .. } => strokes.push(stroke.color),
        egui::Shape::Path(p) => {
            fills.push(p.fill);
            if let egui::epaint::ColorMode::Solid(c) = p.stroke.color {
                strokes.push(c);
            }
        }
        egui::Shape::Mesh(m) => fills.extend(m.vertices.iter().map(|v| v.color)),
        egui::Shape::Text(t) => fills.push(t.fallback_color),
        _ => {}
    }
}

#[test]
fn a9_8_legend_toggles_and_is_remembered() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    assert_eq!(app.checked(CommandId::Legend), Some(false));
    app.execute(CommandId::Legend);
    assert_eq!(app.checked(CommandId::Legend), Some(true));
    let keys = folder_model(&mut app, view).layout_keys();
    assert!(
        keys.contains(&("legend", LayoutValue::Bool(true))),
        "{keys:?}"
    );
    // A new view reads it back from the layout.
    let mut state = hikasa_session::settings::State::default();
    state.set_layout_value("folder-compare", "legend", LayoutValue::Bool(true));
    let m = folder_model(&mut app, view);
    m.execute(&world.service, CommandId::Legend);
    assert!(!m.show_legend());
    m.apply_layout(&state);
    assert!(m.show_legend());
    app.execute(CommandId::Legend);
    assert_eq!(app.checked(CommandId::Legend), Some(false));
}

fn info_dialog(app: &mut TestApp) -> &mut InfoDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<InfoDialog>())
        .expect("the Info dialog")
}

#[test]
fn a9_8_info_equals_the_cli_statistics_and_copies_its_text() {
    let world = World::new();
    put(&world, "l", "same.txt", "s\n", 1_000);
    put(&world, "r", "same.txt", "s\n", 1_000);
    put(&world, "l", "new.txt", "new left\n", 3_000);
    put(&world, "r", "new.txt", "old\n", 1_000);
    put(&world, "l", "lonly.txt", "l\n", 1_000);
    put(&world, "r", "d/ronly.txt", "rr\n", 1_000);
    let clipboard = Arc::new(FixedClipboard::empty());
    let mut app = world.app(None);
    app.0.set_script_clipboard(Arc::clone(&clipboard) as _);
    let view = open(&world, &mut app);
    world.settle(&mut app);
    app.execute(CommandId::Info);
    assert_eq!(top_title(&app).as_deref(), Some("Folder Compare Info"));
    let shown = *info_dialog(&mut app).stats();
    assert!(!info_dialog(&mut app).partial());
    // `hikasa folder compare --json` of the same folders.
    let request = FolderCompareRequest {
        left: world.path("work/l").into_os_string(),
        right: world.path("work/r").into_os_string(),
        ..FolderCompareRequest::default()
    };
    let response = world
        .service
        .spawn(JobKind::FolderCompare, "cli".into(), move |svc, job| {
            svc.folder_compare(&request, job, &hikasa_folder::NullObserver)
        })
        .wait()
        .expect("compare");
    let display = *folder_model(&mut app, view).display_options();
    let json = response.json(&display);
    let cli = serde_json::to_value(json.stats).expect("json");
    let gui = serde_json::to_value(shown).expect("json");
    for key in ["files", "folders", "left", "right", "content"] {
        assert_eq!(gui[key], cli[key], "{key}");
    }
    assert_eq!(shown.left.files, 3);
    assert_eq!(shown.right.files, 3);
    // Copy places the table as text; the dialog stays.
    press(&world, &mut app, COPY);
    let payloads = clipboard.payloads();
    assert_eq!(payloads.len(), 1);
    let text = &payloads[0].text;
    assert_eq!(text, &info_dialog(&mut app).text());
    assert!(text.starts_with("Folder Compare Info\n\nPairs"), "{text}");
    assert!(text.contains("Bytes"), "{text}");
    assert_eq!(
        info_dialog(&mut app).message.as_deref(),
        Some("Copied to the clipboard.")
    );
    press(&world, &mut app, ButtonRole::Close);
    assert!(top_title(&app).is_none());
}

#[test]
fn a9_8_info_while_scanning_says_partial() {
    let world = World::new();
    for i in 0..5 {
        put(&world, "l", &format!("f{i}.txt"), "x\n", 1_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let g = gate(false);
    folder_model(&mut app, view).scan_gate = Some(Arc::clone(&g));
    app.execute(CommandId::FullRefresh);
    poll_until(&world, &mut app, |a| {
        let m = folder_model(a, view);
        m.scanning() && m.prepared().is_some_and(|p| p.model().read().len() > 2)
    });
    app.execute(CommandId::Info);
    let d = info_dialog(&mut app);
    assert!(d.partial());
    assert!(d.text().contains("partial"), "{}", d.text());
    assert_eq!(d.stats().left.files, 5);
    set_gate(&g, true);
    world.settle(&mut app);
}

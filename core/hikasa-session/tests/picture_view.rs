//! M2 W13b in the Service (§1.13.20, §6.13 A13.11, A13.14, A13.21, A13.22): the typed forms the Picture Compare view
//! uses — `picture_response` equal to `picture_compare`'s verdict, warnings and statistics; `picture_write_with` with
//! the window's colours; `PictureColours::to_render`; the new `[view]` keys (`zoom`, `center`, `difference_pane`)
//! read and checked by `session edit --set`; the Picture row of `infer_view_kind` and `is_served`.

mod common;

use common::{job, os, service, write};
use hikasa_core::{ErrorKind, JobKind};
use hikasa_picture::{DiffMode, Pane, PictureColors, PngLevel, Rgba8, RgbaImage, Zoom, encode_png};
use hikasa_session::api::{
    PictureCompareRequest, PictureGeometryWords, PictureRuleWords, PictureViewState, ReportTarget,
    infer_view_kind, picture_view_keys_of,
};
use hikasa_session::ipc::ViewKind;
use hikasa_session::sessions::{Session, SessionRef, SessionType};

fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&f(x, y));
        }
    }
    encode_png(
        &RgbaImage {
            width: w,
            height: h,
            rgba,
        },
        PngLevel::Fast,
    )
    .unwrap()
}

fn gradient(x: u32, y: u32) -> [u8; 4] {
    [(x * 20) as u8, (y * 30) as u8, 77, 255]
}

fn req(l: &std::path::Path, r: &std::path::Path) -> PictureCompareRequest {
    PictureCompareRequest {
        left: os(l),
        right: os(r),
        ..PictureCompareRequest::default()
    }
}

/// `picture_response` of what the view holds equals `picture_compare`'s answer of the same words: the verdict and exit
/// code of every class (different, similar with and without Ignore Unimportant, binary same, rules same), the
/// warnings, the statistics and `human_lines`.
#[test]
fn picture_response_equals_picture_compare() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.png", &png(6, 5, gradient));
    let same = write(&w, "same.png", &png(6, 5, gradient));
    // The same pixels, another encoding: rules same.
    let other = write(&w, "other.png", &{
        let image = RgbaImage {
            width: 6,
            height: 5,
            rgba: (0..30).flat_map(|k| gradient(k % 6, k / 6)).collect(),
        };
        encode_png(&image, PngLevel::Best).unwrap()
    });
    let near = write(
        &w,
        "near.png",
        &png(6, 5, |x, y| {
            let mut p = gradient(x, y);
            if x == 2 && y == 1 {
                p[0] = p[0].wrapping_add(4);
            }
            p
        }),
    );
    let wide = write(&w, "wide.png", &png(9, 3, gradient));
    let cases: &[(&std::path::Path, PictureRuleWords, PictureGeometryWords)] = &[
        (&same, PictureRuleWords::default(), Default::default()),
        (&other, PictureRuleWords::default(), Default::default()),
        (&near, PictureRuleWords::default(), Default::default()),
        (
            &near,
            PictureRuleWords {
                tolerance: Some("10".into()),
                ..PictureRuleWords::default()
            },
            Default::default(),
        ),
        (
            &near,
            PictureRuleWords {
                tolerance: Some("10".into()),
                ignore_unimportant: true,
                mode: Some("blend".into()),
                blend: Some("30".into()),
                ..PictureRuleWords::default()
            },
            Default::default(),
        ),
        (
            &wide,
            PictureRuleWords {
                auto_scale: true,
                ..PictureRuleWords::default()
            },
            PictureGeometryWords {
                left_orientation: Some("cw".into()),
                offset: Some("1,-2".into()),
                ..PictureGeometryWords::default()
            },
        ),
    ];
    for (right, rules, geometry) in cases {
        let request = PictureCompareRequest {
            rules: rules.clone(),
            geometry: geometry.clone(),
            ..req(&a, right)
        };
        let run = job(&svc, JobKind::PictureCompare);
        let direct = svc.picture_compare(&request, &run).unwrap();
        let settings = svc.check_picture_words(&request).unwrap();
        let loaded = svc.picture_load(&request, &run).unwrap();
        let comparison = svc.picture_diff(&loaded, &settings.options, &run).unwrap();
        let identical = svc.picture_bytes_identical(&loaded, &run).unwrap();
        let view = svc.picture_response(
            &loaded,
            &comparison,
            &PictureViewState {
                mode: settings.mode,
                ignore_unimportant: settings.ignore_unimportant,
                bytes_identical: identical,
                colors: svc.picture_colors(),
                verbose: false,
            },
        );
        let what = right.display();
        assert_eq!(view.kind, direct.kind, "{what}");
        assert_eq!(view.exit_code, direct.exit_code, "{what}");
        assert_eq!(view.warnings, direct.warnings, "{what}");
        assert_eq!(view.comparison.stats(), direct.comparison.stats(), "{what}");
        assert_eq!(view.human_lines(), direct.human_lines(), "{what}");
        assert_eq!(
            serde_json::to_value(view.json()).unwrap(),
            serde_json::to_value(direct.json()).unwrap(),
            "{what}"
        );
    }
}

/// `picture_write_with` renders with the colours given (the window's); `picture_write` is it with the theme's; the
/// checkerboard flag reaches the image; printer and browser are refused.
#[test]
fn picture_write_with_takes_the_window_colours() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.png", &png(5, 4, gradient));
    let b = write(
        &w,
        "b.png",
        &png(7, 4, |x, y| {
            if x == 1 {
                [9, 9, 9, 255]
            } else if x == 6 {
                [0, 0, 0, 0]
            } else {
                gradient(x, y)
            }
        }),
    );
    let run = job(&svc, JobKind::PictureCompare);
    let request = req(&a, &b);
    let settings = svc.check_picture_words(&request).unwrap();
    let loaded = svc.picture_load(&request, &run).unwrap();
    let comparison = svc.picture_diff(&loaded, &settings.options, &run).unwrap();
    let dark = PictureColors {
        same: Rgba8::rgb(0x101010),
        similar: Rgba8::rgb(0x202020),
        different: Rgba8::rgb(0xff00ff),
        background: Rgba8::rgb(0),
        checker_light: Rgba8::rgb(0x404040),
        checker_dark: Rgba8::rgb(0x303030),
    };
    for pane in [Pane::Left, Pane::Right, Pane::Difference] {
        for mode in [
            DiffMode::Tolerance,
            DiffMode::Range,
            DiffMode::Blend { percent: 40 },
        ] {
            for checkerboard in [false, true] {
                let image = svc
                    .picture_write_with(
                        &comparison,
                        pane,
                        mode,
                        checkerboard,
                        &dark,
                        &ReportTarget::Clipboard,
                        &run,
                    )
                    .unwrap()
                    .clipboard
                    .unwrap();
                let want = comparison
                    .render(pane, mode, &dark, checkerboard, &run.ctl())
                    .unwrap();
                assert_eq!((image.width, image.height), (want.width, want.height));
                assert_eq!(image.rgba, want.rgba, "{pane:?} {mode:?} {checkerboard}");
            }
        }
    }
    let theme = svc
        .picture_write(
            &comparison,
            Pane::Difference,
            DiffMode::Tolerance,
            false,
            &ReportTarget::Clipboard,
            &run,
        )
        .unwrap();
    let with = svc
        .picture_write_with(
            &comparison,
            Pane::Difference,
            DiffMode::Tolerance,
            false,
            &svc.picture_colors(),
            &ReportTarget::Clipboard,
            &run,
        )
        .unwrap();
    assert_eq!(theme, with);
    for target in [ReportTarget::Printer, ReportTarget::Browser] {
        let err = svc
            .picture_write_with(
                &comparison,
                Pane::Left,
                DiffMode::Tolerance,
                false,
                &dark,
                &target,
                &run,
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }
}

/// `PictureColours::to_render` of the built-in light scheme is the renderer's default; an alpha written by a scheme
/// is kept, an absent one is 255; the Service's colours are the selected scheme's through it.
#[test]
fn picture_colours_to_render() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let light = hikasa_session::settings::theme::Scheme::resolve("light", None);
    assert_eq!(light.picture.to_render(), PictureColors::default());
    assert_eq!(svc.picture_colors(), PictureColors::default());
    let mut colours = light.picture;
    colours.similar.alpha = Some(80);
    let render = colours.to_render();
    assert_eq!(render.similar.a, 80);
    assert_eq!(render.same.a, 255);
    let dark = hikasa_session::settings::theme::Scheme::resolve("dark", None);
    assert_ne!(dark.picture.to_render(), PictureColors::default());
}

/// D-370: the new `[view]` keys — defaults, values, refusals — and `session edit --set` refusing what the reader refuses.
#[test]
fn view_keys_zoom_center_difference_pane() {
    let parse = |text: &str| picture_view_keys_of(&toml::from_str::<toml::Table>(text).unwrap());
    let empty = parse("").unwrap();
    assert_eq!(empty.zoom, Zoom::Fit);
    assert_eq!(empty.center, None);
    assert!(empty.difference_pane);
    let set = parse("zoom = \"150%\"\ncenter = [10, 2.5]\ndifference_pane = false\n").unwrap();
    assert_eq!(set.zoom, Zoom::Scale(1.5));
    assert_eq!(set.center, Some((10.0, 2.5)));
    assert!(!set.difference_pane);
    assert_eq!(parse("zoom = \"actual\"").unwrap().zoom, Zoom::Actual);
    for bad in [
        "zoom = \"9000%\"",
        "zoom = 150",
        "center = [1]",
        "center = [\"a\", 1]",
        "center = 3",
        "difference_pane = \"yes\"",
    ] {
        assert_eq!(
            parse(bad).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
    // `session edit --set` checks the keys through the same reader.
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.png", &png(4, 4, gradient));
    let b = write(&w, "b.png", &png(4, 4, gradient));
    let mut session = Session::builtin(SessionType::PictureCompare);
    session.specs.left = a.display().to_string();
    session.specs.right = b.display().to_string();
    let reference = SessionRef::parse("pics").unwrap();
    svc.session_save(&reference, &session, false).unwrap();
    let edit = |key: &str, value: &str| {
        svc.session_edit(
            &hikasa_session::api::SessionTarget::Named(reference.clone()),
            &hikasa_session::api::SessionEdit {
                set: vec![(key.to_owned(), value.to_owned())],
                ..hikasa_session::api::SessionEdit::default()
            },
            false,
        )
    };
    assert!(edit("view.zoom", "\"200%\"").is_ok());
    assert!(edit("view.center", "[3, 4]").is_ok());
    assert!(edit("view.difference_pane", "false").is_ok());
    for (key, value) in [
        ("view.zoom", "\"huge\""),
        ("view.center", "[1, 2, 3]"),
        ("view.difference_pane", "1"),
    ] {
        assert_eq!(
            edit(key, value).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{key}"
        );
    }
    // The CLI's `session run` ignores the view-only keys.
    assert!(svc.session_run_request(&reference).is_ok());
    // `picture_view_table` writes what the readers read back.
    let geometry = hikasa_picture::Geometry {
        orientation: hikasa_core::Pair::new(
            hikasa_picture::Orientation::Cw90,
            hikasa_picture::Orientation::FlipH,
        ),
        auto_scale: false,
        offset: (-3, 7),
    };
    for keys in [
        hikasa_session::api::PictureViewKeys {
            zoom: Zoom::Scale(2.0 / 3.0),
            center: Some((10.5, -2.0)),
            difference_pane: false,
        },
        hikasa_session::api::PictureViewKeys {
            zoom: Zoom::Fit,
            center: None,
            difference_pane: true,
        },
    ] {
        let table = hikasa_session::api::picture_view_table(&geometry, &keys, false, true);
        let back = picture_view_keys_of(&table).unwrap();
        assert_eq!(back.center, keys.center);
        assert_eq!(back.difference_pane, keys.difference_pane);
        assert_eq!(back.zoom.word(), keys.zoom.word());
        let words = hikasa_session::api::picture::picture_geometry_words_of(&table);
        assert_eq!(words.left_orientation.as_deref(), Some("cw"));
        assert_eq!(words.right_orientation.as_deref(), Some("flip-h"));
        assert_eq!(words.offset.as_deref(), Some("-3,7"));
        assert_eq!(table["layout"].as_str(), Some("stacked"));
        assert_eq!(table["checkerboard"].as_bool(), Some(false));
    }
}

/// A13.21 (the Picture row): two files of a Picture format open in Picture Compare by name (nothing is read); a
/// picture against a text name follows the left name (D-15); `--fv` wins; the kind is served.
#[test]
fn picture_pairs_open_in_picture_compare() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.png", b"not even a picture");
    let b = write(&w, "b.jpg", b"nor this");
    let t = write(&w, "t.txt", b"text\n");
    let spec = |p: &std::path::Path| svc.parse_spec(p.as_os_str()).unwrap();
    for (left, right, want) in [
        (&a, &b, ViewKind::PictureCompare),
        (&a, &t, ViewKind::PictureCompare),
        (&t, &a, ViewKind::TextCompare),
    ] {
        let got = infer_view_kind(&svc, &[spec(left), spec(right)], None).unwrap();
        assert_eq!(got, want, "{} {}", left.display(), right.display());
    }
    let forced = infer_view_kind(&svc, &[spec(&a), spec(&b)], Some(ViewKind::TextCompare));
    assert_eq!(forced.unwrap(), ViewKind::TextCompare);
    assert!(ViewKind::PictureCompare.is_served());
    // A folder view's Open (FC-062): by name under the session's FF-002 overrides.
    let none = std::collections::BTreeMap::new();
    assert!(svc.is_picture_pair_with(a.as_os_str(), b.as_os_str(), &none));
    assert!(!svc.is_picture_pair_with(t.as_os_str(), a.as_os_str(), &none));
    let off = std::collections::BTreeMap::from([("pictures".to_owned(), false)]);
    assert!(!svc.is_picture_pair_with(a.as_os_str(), b.as_os_str(), &off));
}

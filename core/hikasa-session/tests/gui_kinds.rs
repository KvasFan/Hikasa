//! W13a in the Service (§6.13 A13.21, A13.4, A13.6, A13.7, A13.8, A13.22): the Hex rows of the view kinds
//! (`infer_view_kind`, `view_kind_of_pair` with a folder session's FF-002 overrides, `is_served`), and the typed calls
//! of the Hex Compare view — `hex_address` (every spelling of D-333), `hex_finder` (equal to `hex find`'s pattern
//! parse), `hex_response` (equal to `hex_compare`'s verdict and statistics, with the raw comparison of a converted
//! pair), and the `[view]` state keys checked by `session edit --set`.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{job, os, service_with, write};
use hikasa_core::{ContentKind, JobKind, Side, Spec};
use hikasa_hex::nav::AddressQuery;
use hikasa_hex::{Direction, DisplayFilter, HexLayout, Radix};
use hikasa_session::api::{
    HexCompareRequest, HexFindQuery, HexValues, HexViewState, hex_view_state_of, infer_view_kind,
};
use hikasa_session::ipc::ViewKind;

const HEX_UPPER: &str = r#"schema_version = 1
id = "hexup"
name = "Hex Upper"
kind = "hex"
mask = "*.up;*.bin"

[conversion]
method = "external"
load = "conv %s %t"
"#;

const HEX_RAW: &str = r#"schema_version = 1
id = "firmware"
name = "Firmware"
kind = "hex"
mask = "*.fw"
"#;

fn setup() -> (tempfile::TempDir, hikasa_session::Service) {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/hexup.toml",
        HEX_UPPER.as_bytes(),
    );
    write(
        &tmp.path().join("config"),
        "formats/firmware.toml",
        HEX_RAW.as_bytes(),
    );
    let svc = service_with(tmp.path(), None, Arc::new(common::UpperRunner::default()));
    (tmp, svc)
}

/// A13.21: `infer_view_kind` of two files: a Hex format → Hex Compare, a table → Table Compare, text → Text Compare,
/// a binary pair without a Hex format → Text Compare (no sniffing); `is_served` for Hex Compare.
#[test]
fn a13_21_infer_view_kind_hex_rows() {
    let (tmp, svc) = setup();
    let w = tmp.path().join("work");
    let fw = (write(&w, "a.fw", &[0, 1, 2]), write(&w, "b.fw", &[0, 1, 3]));
    let csv = (write(&w, "a.csv", b"a\n1\n"), write(&w, "b.csv", b"a\n2\n"));
    let txt = (write(&w, "a.txt", b"x\n"), write(&w, "b.txt", b"y\n"));
    let dat = (
        write(&w, "a.dat", &[0, 0xFF, 7]),
        write(&w, "b.dat", &[0, 0xFE, 7]),
    );
    let spec =
        |p: &std::path::Path| Spec::Local(hikasa_core::LocalPath::new(p.to_path_buf()).unwrap());
    let kind = |(l, r): &(std::path::PathBuf, std::path::PathBuf)| {
        infer_view_kind(&svc, &[spec(l), spec(r)], None).unwrap()
    };
    assert_eq!(kind(&fw), ViewKind::HexCompare);
    assert_eq!(kind(&csv), ViewKind::TableCompare);
    assert_eq!(kind(&txt), ViewKind::TextCompare);
    assert_eq!(kind(&dat), ViewKind::TextCompare, "no content sniffing");
    // The left name decides (D-15), else the right one.
    assert_eq!(kind(&(fw.0.clone(), txt.1.clone())), ViewKind::HexCompare);
    assert!(ViewKind::HexCompare.is_served());
    assert_eq!(ViewKind::parse("Hex Compare"), Some(ViewKind::HexCompare));
}

/// A13.21: `view_kind_of_pair` by names, under a folder session's FF-002 overrides (a Hex format turned off opens
/// the pair in Text Compare). Mutation: the overrides ignored.
#[test]
fn a13_21_view_kind_of_pair_with_overrides() {
    let (_tmp, svc) = setup();
    let os = std::ffi::OsStr::new;
    let none = BTreeMap::new();
    assert_eq!(
        svc.view_kind_of_pair(os("/w/a.fw"), os("/w/b.fw"), &none),
        ViewKind::HexCompare
    );
    assert_eq!(
        svc.view_kind_of_pair(os("/w/a.csv"), os("/w/b.csv"), &none),
        ViewKind::TableCompare
    );
    assert_eq!(
        svc.view_kind_of_pair(os("/w/a.txt"), os("/w/b.fw"), &none),
        ViewKind::TextCompare,
        "the left name first"
    );
    assert_eq!(
        svc.view_kind_of_pair(os("/w/a.dat"), os("/w/b.dat"), &none),
        ViewKind::TextCompare
    );
    let mut off = BTreeMap::new();
    off.insert("firmware".to_owned(), false);
    assert_eq!(
        svc.view_kind_of_pair(os("/w/a.fw"), os("/w/b.fw"), &off),
        ViewKind::TextCompare
    );
    // `is_table_pair` is the Table case of the same rule.
    assert_eq!(
        svc.is_table_pair(os("/w/a.csv"), os("/w/b.txt")),
        svc.view_kind_of_pair(os("/w/a.csv"), os("/w/b.txt"), &none) == ViewKind::TableCompare
    );
}

/// A13.6: `hex_address` takes every spelling of D-333 with the dialog's radix, and refuses with the core's message.
#[test]
fn a13_6_hex_address_spellings() {
    let (_tmp, svc) = setup();
    for (text, radix, want) in [
        ("0x1F", Radix::Hex, AddressQuery::Absolute(31)),
        ("1Fh", Radix::Decimal, AddressQuery::Absolute(31)),
        ("$1f", Radix::Decimal, AddressQuery::Absolute(31)),
        ("#31", Radix::Hex, AddressQuery::Absolute(31)),
        ("31d", Radix::Hex, AddressQuery::Absolute(31)),
        ("31", Radix::Hex, AddressQuery::Absolute(0x31)),
        ("31", Radix::Decimal, AddressQuery::Absolute(31)),
        ("+10", Radix::Decimal, AddressQuery::Relative(10)),
        ("-0x10", Radix::Decimal, AddressQuery::Relative(-16)),
        ("end", Radix::Hex, AddressQuery::End),
        ("0x0000_1F40", Radix::Hex, AddressQuery::Absolute(0x1F40)),
    ] {
        assert_eq!(svc.hex_address(text, radix).unwrap(), want, "{text}");
        assert_eq!(
            Ok(want),
            hikasa_hex::nav::parse_address(text, radix),
            "the core's parser"
        );
    }
    for bad in ["", "xyz", "0x", "1.5"] {
        let e = svc.hex_address(bad, Radix::Hex).unwrap_err();
        assert!(e.to_string().contains("address"), "{e}");
    }
}

/// A13.7: `hex_finder` builds the finder `hex find` builds (text in each encoding, case, hex with `??`), and refuses
/// empty, oversize, all-wildcard and malformed patterns with the core's message.
#[test]
fn a13_7_hex_finder_queries() {
    let (_tmp, svc) = setup();
    let hay: Vec<u8> = [
        &b"..Hello.."[..],
        &[0xDE, 0xAD, 0x00, 0xEF],
        "h\0e\0l\0l\0o\0".as_bytes(),
        &[0, b'H', 0, b'I'],
    ]
    .concat();
    let doc = hikasa_hex::HexDocument::from_bytes("h", Arc::from(&hay[..]));
    let find = |q: HexFindQuery| {
        let f = svc.hex_finder(&q).unwrap();
        f.next(
            &doc,
            0,
            Direction::Forward,
            false,
            &Default::default(),
            &hikasa_core::Ctl::detached(),
        )
        .unwrap()
    };
    let text = |p: &str, enc: Option<&str>, case: bool| HexFindQuery {
        pattern: p.into(),
        hex: false,
        encoding: enc.map(Into::into),
        match_case: case,
    };
    assert_eq!(find(text("hello", None, false)), Some(2..7));
    assert_eq!(find(text("hello", None, true)), None);
    assert_eq!(find(text("HELLO", Some("utf-16le"), false)), Some(13..23));
    assert_eq!(find(text("HI", Some("utf-16be"), true)), Some(23..27));
    assert_eq!(find(text("Hello", Some("latin1"), true)), Some(2..7));
    let hex = |p: &str| HexFindQuery {
        pattern: p.into(),
        hex: true,
        // Ignored for bytes (the panel greys them).
        encoding: Some("utf-16le".into()),
        match_case: true,
    };
    assert_eq!(find(hex("de ad ?? ef")), Some(9..13));
    for bad in [
        hex(""),
        hex("?? ??"),
        hex("dea"),
        hex("zz"),
        text("", None, false),
    ] {
        assert!(svc.hex_finder(&bad).is_err(), "{bad:?}");
    }
    let long = hex(&"00 ".repeat(4097));
    assert!(
        svc.hex_finder(&long)
            .unwrap_err()
            .to_string()
            .contains("4096")
    );
    assert!(svc.hex_finder(&text("x", Some("ebcdic"), false)).is_err());
}

/// A13.8: `hex_response` of a landed alignment equals `hex_compare`'s verdict, exit code and statistics, and
/// `human_lines`' summary; a converted pair takes the raw comparison (`hex_raw_same`) for D-331's rows 3–5.
#[test]
fn a13_8_hex_response_equals_hex_compare() {
    let (tmp, svc) = setup();
    let w = tmp.path().join("work");
    let mut data: Vec<u8> = (0..40_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let a = write(&w, "a.fw", &data);
    data[12_345] ^= 0x55;
    data.splice(30_000..30_000, [1, 2, 3]);
    let b = write(&w, "b.fw", &data);
    let up = |name: &str, text: &[u8]| write(&w, name, text);
    let (u1, u2, u3, u4) = (
        up("one.up", b"abc"),
        up("two.up", b"abc"),
        up("three.up", b"ABC"),
        up("four.up", b"abd"),
    );
    for (l, r, kind) in [
        (&a, &b, ContentKind::BinaryDifferent),
        (&a, &a, ContentKind::BinarySame),
        (&u1, &u2, ContentKind::BinarySame),
        (&u1, &u3, ContentKind::RulesSame),
        (&u1, &u4, ContentKind::ImportantDifferences),
    ] {
        let req = HexCompareRequest {
            left: os(l),
            right: os(r),
            ..HexCompareRequest::default()
        };
        let j = job(&svc, JobKind::HexCompare);
        let cli = svc.hex_compare(&req, &j).unwrap();
        let opened = svc.hex_open(&req, &j).unwrap();
        let settings = svc.hex_settings(&req).unwrap();
        let al = svc.hex_align(&opened, &settings.align, &j).unwrap();
        let raw = svc.hex_raw_same(&opened, &j).unwrap();
        assert_eq!(raw.is_some(), opened.converted());
        let gui = svc
            .hex_response(
                &opened,
                &al,
                HexLayout::default(),
                DisplayFilter::Differences,
            )
            .with_raw_same(raw);
        assert_eq!((gui.kind, gui.exit_code), (cli.kind, cli.exit_code));
        assert_eq!(gui.kind, kind);
        assert_eq!(gui.alignment.stats, cli.alignment.stats);
        assert_eq!(gui.alignment.sections, cli.alignment.sections);
        assert_eq!(
            gui.human_lines().unwrap().last(),
            cli.human_lines().unwrap().last()
        );
        assert!(svc.hex_changed(&opened).is_empty());
    }
    // A changed input is `input_changed`.
    let req = HexCompareRequest {
        left: os(&a),
        right: os(&b),
        ..HexCompareRequest::default()
    };
    let opened = svc.hex_open(&req, &job(&svc, JobKind::HexCompare)).unwrap();
    std::fs::write(&a, b"shorter").unwrap();
    let changed = svc.hex_changed(&opened);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].code.as_str(), "input_changed");
    let _ = Side::Left;
}

/// A13.22: the `[view]` state keys of a hex session (`addresses`, `thumbnail`, `values`, `values_radix`): defaults
/// for a pre-W13 table, words parsed, refusals; `session edit --set` checks them.
#[test]
fn a13_22_hex_view_state_keys() {
    let empty = toml::Table::new();
    assert_eq!(hex_view_state_of(&empty).unwrap(), HexViewState::default());
    let view: toml::Table = toml::from_str(
        "addresses = false\nthumbnail = false\nvalues = \"big\"\nvalues_radix = \"hex\"\nbytes_per_line = 8",
    )
    .unwrap();
    assert_eq!(
        hex_view_state_of(&view).unwrap(),
        HexViewState {
            addresses: false,
            thumbnail: false,
            values: HexValues::Big,
            values_radix: Radix::Hex,
        }
    );
    for bad in [
        "addresses = 1",
        "values = \"middle\"",
        "values_radix = \"octal\"",
    ] {
        let t: toml::Table = toml::from_str(bad).unwrap();
        assert!(hex_view_state_of(&t).is_err(), "{bad}");
    }
    assert_eq!(HexValues::None.toggled(true), HexValues::Little);
    assert_eq!(HexValues::Little.toggled(false), HexValues::Both);
    assert_eq!(HexValues::Both.toggled(true), HexValues::Big);
    // `session edit --set` checks them through the same parser.
    let (tmp, svc) = setup();
    let w = tmp.path().join("work");
    let (a, b) = (write(&w, "a.fw", b"a"), write(&w, "b.fw", b"b"));
    let mut session = hikasa_session::sessions::Session::builtin(
        hikasa_session::sessions::SessionType::HexCompare,
    );
    session.specs.left = a.display().to_string();
    session.specs.right = b.display().to_string();
    let r = hikasa_session::sessions::SessionRef::parse("hx").unwrap();
    svc.session_save(&r, &session, false).unwrap();
    let target = hikasa_session::api::SessionTarget::Named(r);
    let edit = |k: &str, v: &str| hikasa_session::api::SessionEdit {
        set: vec![(k.into(), v.into())],
        ..Default::default()
    };
    assert!(
        svc.session_edit(&target, &edit("view.values", "both"), false)
            .is_ok()
    );
    assert!(
        svc.session_edit(&target, &edit("view.values", "middle"), false)
            .is_err()
    );
    assert!(
        svc.session_edit(&target, &edit("view.addresses", "false"), false)
            .is_ok()
    );
}

// ---- M2 W13c: the Quick Compare dialog's calls (D-105, D-368) -------------------------------------------------------

/// `Service::spec_facts`: a file's size and modification time; a missing file is `LoadFailed` naming the side asked
/// for (W13c review finding 9).
#[test]
fn a13_20_spec_facts_names_its_side() {
    let (tmp, svc) = setup();
    let w = tmp.path().join("work");
    let there = write(&w, "there.txt", b"12345\n");
    let (size, time) = svc.spec_facts(Side::Left, &os(&there)).unwrap();
    assert_eq!(size, 6);
    assert!(time.is_some_and(|t| !t.is_empty()));
    let missing = w.join("missing.txt");
    for side in [Side::Left, Side::Right] {
        match svc.spec_facts(side, &os(&missing)) {
            Err(hikasa_session::ApiError::LoadFailed { side: s, .. }) => assert_eq!(s, side),
            other => panic!("{side:?}: {other:?}"),
        }
    }
}

/// `resolve_open` marks an inferred kind (`gui open a b`), not a forced one (`--fv`).
#[test]
fn a13_20_resolve_open_marks_an_inferred_kind() {
    let (tmp, svc) = setup();
    let w = tmp.path().join("work");
    let (a, b) = (write(&w, "a.txt", b"x\n"), write(&w, "b.txt", b"y\n"));
    let request = hikasa_session::api::GuiOpenRequest {
        specs: vec![os(&a), os(&b)],
        ..hikasa_session::api::GuiOpenRequest::default()
    };
    let inferred = svc.resolve_open(&request).unwrap();
    assert!(inferred.options.inferred);
    assert!(inferred.options.quick_eligible());
    let forced = svc
        .resolve_open(&hikasa_session::api::GuiOpenRequest {
            kind: Some(ViewKind::TextCompare),
            ..request
        })
        .unwrap();
    assert!(!forced.options.inferred);
    assert!(!forced.options.quick_eligible());
}

/// `OpenOptions::quick_eligible`: inferred and no view word; each view word alone turns it off.
#[test]
fn a13_20_quick_eligible_needs_no_view_word() {
    use hikasa_session::ipc::OpenOptions;
    let base = OpenOptions {
        inferred: true,
        ..OpenOptions::default()
    };
    assert!(base.quick_eligible());
    assert!(!OpenOptions::default().quick_eligible(), "not inferred");
    type Word = (&'static str, Box<dyn Fn(&mut OpenOptions)>);
    let words: Vec<Word> = vec![
        ("display", Box::new(|o| o.display = Some("all".into()))),
        ("context", Box::new(|o| o.context_lines = Some(2))),
        ("folders", Box::new(|o| o.folders = Some("all".into()))),
        ("suppress", Box::new(|o| o.suppress_filters = true)),
        ("expandall", Box::new(|o| o.expand_all = true)),
        ("format", Box::new(|o| o.format = Some("C/C++".into()))),
        (
            "left format",
            Box::new(|o| o.formats[0] = Some("C/C++".into())),
        ),
        (
            "right format",
            Box::new(|o| o.formats[1] = Some("C/C++".into())),
        ),
        ("anchors", Box::new(|o| o.anchors = vec!["1:2".into()])),
        ("ignored", Box::new(|o| o.ignored = vec!["left:1".into()])),
        (
            "savetarget",
            Box::new(|o| o.save_target = Some("/x".into())),
        ),
        ("sync", Box::new(|o| o.sync = Some(Default::default()))),
        ("table", Box::new(|o| o.table = Some(Default::default()))),
        ("hex", Box::new(|o| o.hex = Some(Default::default()))),
        (
            "picture",
            Box::new(|o| o.picture = Some(Default::default())),
        ),
    ];
    for (name, set) in words {
        let mut o = base.clone();
        set(&mut o);
        assert!(!o.quick_eligible(), "{name}");
    }
}

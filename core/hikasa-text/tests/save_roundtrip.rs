//! M2 W10b acceptance A10.13 (D-272): the save round trip and P-SAVE.
//!
//! - Every encoding/BOM fixture of `hikasa-formats` (`tests/fixtures/encoding`) and generated files in every legacy
//!   encoding (windows-1252, ISO-8859-2, KOI8-R, Shift_JIS, EUC-JP, GB18030, Big5, EUC-KR, IBM866, ISO-2022-JP) and
//!   every UTF (8 with and without BOM, 16 and 32 LE/BE with and without BOM) × LF, CRLF, CR, mixed, no final
//!   terminator × malformed bytes × a Ctrl-Z tail × CPL-wrapped lines: load → save without edits = the source bytes.
//! - P-SAVE: random edits (replace, insert, delete lines) against an independent model of the physical lines → every
//!   unchanged line is its source bytes at its place, every edited line its encoded text and terminator, the BOM the
//!   source's, the tail kept; an edited ISO-2022-JP document and an encoding change are whole re-encodes with
//!   `save_reencoded`.
//! - The format's save options touch edited lines only and form one undo step.
//!
//! 64 P-SAVE cases unless `PROPTEST_CASES` says otherwise.

mod common;

use std::fs;

use common::save::{forced, open, render, save, spec, text_format};
use hikasa_core::{CaseSensitivity, WarningCode};
use hikasa_formats::{DecodeOptions, Encoder, EncodingSetting, TextEncoding};
use hikasa_text::edit::{EditRequest, EditableDocument, GroupHint, TextPos};
use hikasa_text::save::{
    ContentOptions, ReencodeReason, SaveIssue, SaveOptions, SaveRequest, SourceMap,
    apply_save_options, save_content,
};
use hikasa_text::{LineEnding, LineIdx, LineMark};
use hikasa_vfs::{MemMeta, MemoryVfs};
use proptest::prelude::*;
use proptest::test_runner::Config;
use serde::Deserialize;

fn memory() -> MemoryVfs {
    MemoryVfs::new(CaseSensitivity::Sensitive)
}

fn put(vfs: &MemoryVfs, path: &str, bytes: &[u8]) {
    vfs.insert_file(
        &common::host(path),
        bytes.to_vec(),
        MemMeta {
            mtime: hikasa_core::FileTime::from_unix(1_700_000_000, 0),
            ..MemMeta::default()
        },
    )
    .unwrap();
}

fn read(vfs: &MemoryVfs, path: &str) -> Vec<u8> {
    vfs.read_back(&common::host(path)).unwrap().to_vec()
}

fn label(l: &str) -> TextEncoding {
    TextEncoding::for_label(l).unwrap_or_else(|| panic!("label {l}"))
}

/// Saves `doc` in place without an edit (every line spliced) and returns the bytes written.
fn resave(vfs: &MemoryVfs, path: &str, doc: &mut EditableDocument) -> Vec<u8> {
    let planned = common::save::plan(vfs, doc, &SaveRequest::default());
    let item = &planned.items[0];
    assert_eq!(item.content.encoded_lines, 0, "{path}: {:?}", item.issues);
    assert!(planned.confirmed, "{:?}", item.issues);
    let report = common::save::execute(vfs, planned, doc);
    assert!(report.ok(), "{:?}", report.op.results);
    read(vfs, path)
}

// ---- the fixtures of hikasa-formats --------------------------------------------------------------------------

#[derive(Deserialize)]
struct Case {
    #[serde(default)]
    setting: Option<EncodingSetting>,
    #[serde(default)]
    options: Options,
}

#[derive(Deserialize, Default)]
struct Options {
    #[serde(rename = "override")]
    override_label: Option<String>,
    locale: Option<String>,
}

/// Every encoding fixture loads and saves back to its bytes, as its own `[setting]` and `[options]` detect it.
#[test]
fn a10_13_every_encoding_fixture_saves_back_to_its_bytes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../hikasa-formats/tests/fixtures/encoding");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    names.sort();
    assert!(names.len() >= 25, "{names:?}");
    for name in names {
        let stem = name.trim_end_matches(".bin");
        let bytes = fs::read(dir.join(&name)).unwrap();
        let toml_text = fs::read_to_string(dir.join(format!("{stem}.expected.toml"))).unwrap();
        let case: Case = toml::from_str(&toml_text).unwrap_or_else(|e| panic!("{stem}: {e}"));
        let setting = case
            .setting
            .unwrap_or(EncodingSetting::Auto { fallback: None });
        let decode = DecodeOptions {
            override_encoding: case.options.override_label.as_deref().map(label),
            locale_fallback: case
                .options
                .locale
                .as_deref()
                .map_or(TextEncoding::UTF_8, label),
            ..DecodeOptions::default()
        };
        for (ctrl_z, cpl) in [(false, None), (true, Some(3))] {
            let vfs = memory();
            put(&vfs, "/f.txt", &bytes);
            let format = text_format(setting.clone(), ctrl_z);
            let mut doc = open(&vfs, &spec("/f.txt"), &format, decode, cpl);
            let saved = resave(&vfs, "/f.txt", &mut doc);
            assert_eq!(saved, bytes, "{stem} (ctrl-z {ctrl_z}, cpl {cpl:?})");
            // And again from the file just written.
            assert_eq!(
                resave(&vfs, "/f.txt", &mut doc),
                bytes,
                "{stem}, second save"
            );
        }
    }
}

// ---- generated files -----------------------------------------------------------------------------------------

/// The encodings of A10.13, with a BOM where the encoding has one and the case asks.
fn encodings() -> Vec<(TextEncoding, bool)> {
    let mut all = Vec::new();
    for l in [
        "windows-1252",
        "ISO-8859-2",
        "KOI8-R",
        "Shift_JIS",
        "EUC-JP",
        "gb18030",
        "Big5",
        "EUC-KR",
        "IBM866",
        "ISO-2022-JP",
    ] {
        all.push((label(l), false));
    }
    for e in [
        TextEncoding::UTF_8,
        TextEncoding::UTF_16LE,
        TextEncoding::UTF_16BE,
        TextEncoding::Utf32Le,
        TextEncoding::Utf32Be,
    ] {
        all.push((e, false));
        all.push((e, true));
    }
    all
}

/// Characters each encoding writes and reads back unchanged.
fn alphabet(enc: TextEncoding) -> Vec<char> {
    let own: &str = match enc.name() {
        "windows-1252" => "\u{e9}\u{e0}\u{fc}\u{df}\u{20ac}\u{153}",
        "ISO-8859-2" => "\u{17e}\u{165}\u{10d}\u{fd}\u{105}\u{142}",
        "KOI8-R" | "IBM866" => "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}",
        "Shift_JIS" | "EUC-JP" | "ISO-2022-JP" => "\u{65e5}\u{672c}\u{8a9e}\u{30ab}\u{30ca}",
        "gb18030" => "\u{4e2d}\u{6587}\u{1F600}\u{e9}",
        "Big5" => "\u{4e2d}\u{6587}\u{5b57}",
        "EUC-KR" => "\u{d55c}\u{ad6d}\u{c5b4}",
        _ => "\u{e9}\u{4e2d}\u{1F600}\u{43f}",
    };
    let mut chars: Vec<char> = "ab z\t".chars().collect();
    chars.extend(own.chars());
    chars
}

/// Bytes a reader of `enc` decodes to U+FFFD and that hold no CR or LF.
fn malformed(enc: TextEncoding) -> Option<&'static [u8]> {
    match enc.name() {
        "UTF-8" => Some(b"\xFF\xC3"),
        "Shift_JIS" => Some(b"\xFD"),
        "EUC-KR" | "EUC-JP" | "Big5" => Some(b"\xFF"),
        "UTF-16LE" => Some(b"\x00\xD8"),
        "UTF-16BE" => Some(b"\xD8\x00"),
        "UTF-32LE" => Some(b"\x00\x00\x11\x00"),
        "UTF-32BE" => Some(b"\x00\x11\x00\x00"),
        _ => None,
    }
}

/// One generated line: text pieces (raw malformed bytes among them) and a terminator.
#[derive(Debug, Clone)]
struct GenLine {
    text: String,
    bad: bool,
    eol: LineEnding,
}

/// The bytes of `lines` in `enc` (BOM when asked, malformed bytes in the lines that ask, `tail` after a Ctrl-Z).
fn file_bytes(enc: TextEncoding, bom: bool, lines: &[GenLine], tail: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(enc.bom());
    }
    for line in lines {
        let mut chunk = Vec::new();
        Encoder::new(enc).encode(&line.text, true, &mut chunk);
        out.extend_from_slice(&chunk);
        if line.bad
            && let Some(bad) = malformed(enc)
        {
            out.extend_from_slice(bad);
        }
        let mut eol = Vec::new();
        Encoder::new(enc).encode(line.eol.as_text(), true, &mut eol);
        out.extend_from_slice(&eol);
    }
    if let Some(tail) = tail.filter(|_| enc.is_ascii_compatible()) {
        out.push(0x1A);
        out.extend_from_slice(tail);
    }
    out
}

/// The terminator cases of A10.13.
fn terminators(case: usize, n: usize) -> Vec<LineEnding> {
    (0..n)
        .map(|k| {
            let last = k + 1 == n;
            match case {
                0 => LineEnding::Lf,
                1 => LineEnding::CrLf,
                2 => LineEnding::Cr,
                3 => [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr][k % 3],
                _ if last => LineEnding::None,
                _ => LineEnding::Lf,
            }
        })
        .collect()
}

/// Every encoding × terminator case × malformed bytes × Ctrl-Z tail × CPL: a save without an edit is the source.
#[test]
fn a10_13_generated_files_save_back_to_their_bytes() {
    let mut cases = 0;
    for (enc, bom) in encodings() {
        let chars = alphabet(enc);
        for case in 0..5 {
            for (bad, tail, cpl) in [
                (false, false, None),
                (true, false, None),
                (false, true, None),
                (true, true, Some(4)),
            ] {
                let eols = terminators(case, 7);
                let lines: Vec<GenLine> = eols
                    .iter()
                    .enumerate()
                    .map(|(k, &eol)| GenLine {
                        text: (0..k * 3).map(|i| chars[(i + k) % chars.len()]).collect(),
                        bad: bad && k % 2 == 1,
                        eol,
                    })
                    .collect();
                let tail = tail.then_some(&b"tail \x00\xFF\n bytes"[..]);
                let bytes = file_bytes(enc, bom, &lines, tail);
                let vfs = memory();
                put(&vfs, "/g.txt", &bytes);
                let format = text_format(EncodingSetting::Auto { fallback: None }, tail.is_some());
                let mut doc = open(&vfs, &spec("/g.txt"), &format, forced(enc), cpl);
                assert_eq!(doc.encoding(), (enc, bom), "{enc} bom {bom}");
                let saved = resave(&vfs, "/g.txt", &mut doc);
                assert_eq!(
                    saved, bytes,
                    "{enc} bom {bom} case {case} bad {bad} tail {tail:?} cpl {cpl:?}"
                );
                cases += 1;
            }
        }
    }
    assert!(cases >= 300, "{cases}");
}

// ---- P-SAVE ---------------------------------------------------------------------------------------------------

/// The model of one physical line: the source line it still is (or none), its text and terminator.
#[derive(Debug, Clone)]
struct ModelLine {
    orig: Option<usize>,
    text: String,
    eol: LineEnding,
}

/// An edit of the property.
#[derive(Debug, Clone)]
enum Op {
    /// Replaces the content of physical line `at % n` (non-empty text).
    Replace(usize, String),
    /// Inserts a line before physical line `at % n`.
    Insert(usize, String),
    /// Deletes physical line `at % n` (never the last).
    Delete(usize),
}

/// The table lines of each physical line of `doc`: `[first, end)`.
fn physical(doc: &EditableDocument) -> Vec<(u32, u32)> {
    let n = doc.line_count();
    let mut out = Vec::new();
    let mut p = 0;
    while p < n {
        let mut end = p;
        while end < n && doc.line_ending(LineIdx(end)) == LineEnding::Artificial {
            end += 1;
        }
        out.push((p, end + 1));
        p = end + 1;
    }
    out
}

/// A CR line followed by an empty LF line reads as one CRLF line (as a load reads those bytes, §1.10.29 W10a n3).
fn join_cr_lf(model: &mut Vec<ModelLine>) {
    let mut k = 0;
    while k + 1 < model.len() {
        if model[k].eol == LineEnding::Cr
            && model[k + 1].text.is_empty()
            && model[k + 1].eol == LineEnding::Lf
        {
            model[k].eol = LineEnding::CrLf;
            model[k].orig = None;
            model.remove(k + 1);
        } else {
            k += 1;
        }
    }
}

/// Applies `op` to the document and to the model.
fn apply(doc: &mut EditableDocument, model: &mut Vec<ModelLine>, op: &Op) {
    let phys = physical(doc);
    let n = phys.len();
    if n == 0 {
        return;
    }
    let last_pos = |(first, end): (u32, u32)| {
        let l = end - 1;
        let _ = first;
        TextPos::new(l, doc.line(LineIdx(l)).len() as u32)
    };
    match op {
        Op::Replace(at, text) => {
            let k = at % n;
            let range = TextPos::new(phys[k].0, 0)..last_pos(phys[k]);
            doc.apply(
                &EditRequest::Replace {
                    range,
                    text: text.clone(),
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
            model[k].text = text.clone();
            model[k].orig = None;
        }
        Op::Insert(at, text) => {
            let k = at % n;
            let eol = doc.insert_eol();
            let at = TextPos::new(phys[k].0, 0);
            doc.apply(
                &EditRequest::Replace {
                    range: at..at,
                    text: format!("{text}\n"),
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
            model.insert(
                k,
                ModelLine {
                    orig: None,
                    text: text.clone(),
                    eol,
                },
            );
        }
        Op::Delete(at) => {
            if n < 2 {
                return;
            }
            let k = at % (n - 1);
            let range = TextPos::new(phys[k].0, 0)..TextPos::new(phys[k + 1].0, 0);
            doc.apply(
                &EditRequest::Replace {
                    range,
                    text: String::new(),
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
            model.remove(k);
        }
    }
    join_cr_lf(model);
}

fn op_strategy(chars: Vec<char>) -> impl Strategy<Value = Op> {
    let text = proptest::collection::vec(proptest::sample::select(chars), 1..6)
        .prop_map(|c| c.into_iter().collect::<String>());
    prop_oneof![
        (any::<usize>(), text.clone()).prop_map(|(a, t)| Op::Replace(a, t)),
        (any::<usize>(), text).prop_map(|(a, t)| Op::Insert(a, t)),
        any::<usize>().prop_map(Op::Delete),
    ]
}

/// One P-SAVE case: the document, its edits, the encoding.
#[derive(Debug, Clone)]
struct SaveCase {
    enc: TextEncoding,
    bom: bool,
    lines: Vec<GenLine>,
    tail: bool,
    cpl: Option<u32>,
    ops: Vec<Op>,
}

fn case_strategy() -> impl Strategy<Value = SaveCase> {
    (
        proptest::sample::select(encodings()),
        0usize..5,
        any::<bool>(),
        any::<bool>(),
        proptest::option::of(3u32..8),
    )
        .prop_flat_map(|((enc, bom), eol_case, bad, tail, cpl)| {
            let chars = alphabet(enc);
            let line = proptest::collection::vec(proptest::sample::select(chars.clone()), 0..12)
                .prop_map(|c| c.into_iter().collect::<String>());
            (
                proptest::collection::vec((line, any::<bool>()), 1..14),
                proptest::collection::vec(op_strategy(chars), 0..6),
            )
                .prop_map(move |(texts, ops)| {
                    let eols = terminators(eol_case, texts.len());
                    let lines = texts
                        .into_iter()
                        .zip(eols)
                        .map(|((text, b), eol)| GenLine {
                            text,
                            bad: bad && b,
                            eol,
                        })
                        .collect();
                    SaveCase {
                        enc,
                        bom,
                        lines,
                        tail,
                        cpl,
                        ops,
                    }
                })
        })
}

/// The bytes the model says a save writes: each unchanged line its source bytes, each other its encoded text and
/// terminator (ISO-2022-JP with an edit: the whole text through one encoder), the source's BOM, the tail.
fn expected(case: &SaveCase, model: &[ModelLine], source: &[u8], map: &SourceMap) -> Vec<u8> {
    let mut out = Vec::new();
    if case.bom {
        out.extend_from_slice(case.enc.bom());
    }
    let stateful = case.enc.name() == "ISO-2022-JP";
    let edited =
        model.iter().any(|line| line.orig.is_none()) || model.len() != map.lines() as usize;
    if stateful && edited {
        let text: String = model
            .iter()
            .map(|l| format!("{}{}", l.text, l.eol.as_text()))
            .collect();
        Encoder::new(case.enc).encode(&text, true, &mut out);
    } else {
        for line in model {
            match line.orig {
                Some(k) => {
                    let r = map.line_bytes(k as u32).unwrap();
                    out.extend_from_slice(&source[r.start as usize..r.end as usize]);
                }
                None => {
                    let text = format!("{}{}", line.text, line.eol.as_text());
                    Encoder::new(case.enc).encode(&text, true, &mut out);
                }
            }
        }
    }
    let tail = map.tail();
    out.extend_from_slice(&source[tail.start as usize..tail.end as usize]);
    out
}

fn config() -> Config {
    let mut config = common::config("tests/proptest-regressions/save_roundtrip.txt");
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 64;
    }
    config
}

proptest! {
    #![proptest_config(config())]

    /// P-SAVE (D-272).
    #[test]
    fn p_save(case in case_strategy()) {
        let tail = case.tail.then_some(&b"\x00tail\xFE\n"[..]);
        let source = file_bytes(case.enc, case.bom, &case.lines, tail);
        let vfs = memory();
        put(&vfs, "/p.txt", &source);
        let format = text_format(EncodingSetting::Auto { fallback: None }, case.tail);
        let mut doc = open(&vfs, &spec("/p.txt"), &format, forced(case.enc), case.cpl);
        // The model: the source's physical lines as the document reads them.
        let mut model: Vec<ModelLine> = physical(&doc)
            .iter()
            .enumerate()
            .map(|(k, &(first, end))| ModelLine {
                orig: Some(k),
                text: (first..end).map(|l| doc.line(LineIdx(l)).into_owned()).collect(),
                eol: doc.line_ending(LineIdx(end - 1)),
            })
            .collect();
        let loaded = model.len();
        for op in &case.ops {
            apply(&mut doc, &mut model, op);
        }
        let map = SourceMap::scan(
            &mut std::io::Cursor::new(&source),
            case.enc,
            if case.bom { case.enc.bom().len() as u8 } else { 0 },
            case.tail,
            &hikasa_core::Ctl::detached(),
        )
        .unwrap()
        .unwrap();
        prop_assert_eq!(map.lines() as usize, loaded, "the source's lines");
        let content = save_content(&doc, Some(&map), &ContentOptions::default());
        let want = expected(&case, &model, &source, &map);
        let got = render(&content, &source);
        prop_assert_eq!(&got, &want, "{:?}", content.reencoded());
        // Whether a line is encoded is the document's call: an edit that leaves the text a source line had (type "本"
        // into a line, then delete the equal line before it) may keep that line's `orig` where the model counts it
        // edited; the bytes are the same either way (checked above). Found at 20,000 cases (W10b note 66).
        let stateful_edit = case.enc.name() == "ISO-2022-JP"
            && (0..doc.line_count()).any(|l| doc.marks(LineIdx(l)).orig.is_none());
        if stateful_edit {
            prop_assert_eq!(content.reencoded(), Some(ReencodeReason::Stateful));
        } else {
            prop_assert_eq!(content.reencoded(), None, "a splice");
            // Nothing but the edited lines is encoded: without an edit nothing is.
            if case.ops.is_empty() {
                prop_assert_eq!(content.encoded_lines, 0, "lines encoded without an edit");
            }
        }
        // Through the plan and the file: the same bytes, and the next save of the saved file splices it whole.
        let report = save(&vfs, &mut doc, &SaveRequest::default());
        prop_assert!(report.ok(), "{:?}", report.op.results);
        prop_assert_eq!(read(&vfs, "/p.txt"), want.clone(), "the file of the first save");
        prop_assert!(!doc.is_modified());
        let report = save(&vfs, &mut doc, &SaveRequest::default());
        prop_assert!(report.ok());
        prop_assert_eq!(read(&vfs, "/p.txt"), want, "the file of the second save");
    }
}

// ---- whole re-encodes ------------------------------------------------------------------------------------------

/// An edited ISO-2022-JP document and an encoding change encode the whole text, with `save_reencoded`.
#[test]
fn a10_13_stateful_and_encoding_changes_are_whole_re_encodes() {
    let enc = label("ISO-2022-JP");
    let lines: Vec<GenLine> = (0..4)
        .map(|k| GenLine {
            text: format!("{k}\u{65e5}\u{672c}"),
            bad: false,
            eol: LineEnding::Lf,
        })
        .collect();
    let bytes = file_bytes(enc, false, &lines, None);
    let vfs = memory();
    put(&vfs, "/j.txt", &bytes);
    let format = text_format(EncodingSetting::Auto { fallback: None }, false);
    let mut doc = open(&vfs, &spec("/j.txt"), &format, forced(enc), None);
    assert_eq!(
        resave(&vfs, "/j.txt", &mut doc),
        bytes,
        "without an edit: one unchanged run"
    );
    let end = TextPos::new(1, doc.line(LineIdx(1)).len() as u32);
    doc.apply(
        &EditRequest::Replace {
            range: end..end,
            text: "\u{8a9e}".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    let planned = common::save::plan(&vfs, &doc, &SaveRequest::default());
    assert!(planned.items[0].issues.contains(&SaveIssue::Reencoded {
        reason: ReencodeReason::Stateful
    }));
    assert!(
        planned
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::SaveReencoded)
    );
    let report = common::save::execute(&vfs, common::save::confirm_all(planned), &mut doc);
    assert!(report.ok());
    let mut want = Vec::new();
    Encoder::new(enc).encode(
        "0\u{65e5}\u{672c}\n1\u{65e5}\u{672c}\u{8a9e}\n2\u{65e5}\u{672c}\n3\u{65e5}\u{672c}\n",
        true,
        &mut want,
    );
    assert_eq!(read(&vfs, "/j.txt"), want);

    // An encoding change: windows-1252 to UTF-8 with a BOM.
    let vfs = memory();
    put(&vfs, "/e.txt", b"caf\xE9\r\nna\xEFve\r\n");
    let mut doc = open(
        &vfs,
        &spec("/e.txt"),
        &format,
        forced(label("windows-1252")),
        None,
    );
    doc.apply(
        &EditRequest::SetEncoding {
            encoding: TextEncoding::UTF_8,
            bom: true,
        },
        GroupHint::None,
    )
    .unwrap();
    let planned = common::save::plan(&vfs, &doc, &SaveRequest::default());
    assert!(planned.items[0].issues.contains(&SaveIssue::Reencoded {
        reason: ReencodeReason::EncodingChanged
    }));
    assert!(
        planned
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::SaveReencoded)
    );
    let report = common::save::execute(&vfs, common::save::confirm_all(planned), &mut doc);
    assert!(report.ok());
    assert_eq!(
        read(&vfs, "/e.txt"),
        "\u{feff}caf\u{e9}\r\nna\u{ef}ve\r\n".as_bytes()
    );
    // The saved file is the new splice source, in its new encoding: the next save splices it.
    let content = save_content(&doc, None, &ContentOptions::default());
    assert_eq!(content.encoding, TextEncoding::UTF_8);
    let again = common::save::plan(&vfs, &doc, &SaveRequest::default());
    assert!(
        again.items[0].issues.is_empty(),
        "{:?}",
        again.items[0].issues
    );
    assert_eq!(again.items[0].content.encoded_lines, 0);
}

// ---- save options ----------------------------------------------------------------------------------------------

/// The format's save options change the edited and new lines only, as one undo step; the unchanged lines' trailing
/// blanks stay in the file.
#[test]
fn a10_13_save_options_touch_edited_lines_only_in_one_step() {
    let vfs = memory();
    let source = "keep  \n    lead\nedit\n        b\nlast\t\n";
    put(&vfs, "/o.txt", source.as_bytes());
    let format = text_format(EncodingSetting::Auto { fallback: None }, false);
    let mut doc = open(
        &vfs,
        &spec("/o.txt"),
        &format,
        DecodeOptions::default(),
        None,
    );
    let put_at = |doc: &mut EditableDocument, line: u32, byte: u32, text: &str| {
        let at = TextPos::new(line, byte);
        doc.apply(
            &EditRequest::Replace {
                range: at..at,
                text: text.into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    };
    put_at(&mut doc, 2, 4, "ed   ");
    put_at(&mut doc, 3, 0, "  ");
    let steps = doc.undo_steps();
    let opts = SaveOptions {
        trim_trailing_ws: true,
        leading_spaces_to_tabs: true,
    };
    let pre = apply_save_options(&mut doc, opts).unwrap().unwrap();
    assert_eq!(pre.lines, 2);
    assert_eq!(doc.undo_steps(), steps + 1, "one undo step");
    assert_eq!(
        doc.line(LineIdx(0)),
        "keep  ",
        "an unchanged line keeps its blanks"
    );
    assert_eq!(doc.line(LineIdx(1)), "    lead");
    assert_eq!(doc.line(LineIdx(2)), "edited");
    assert_eq!(doc.line(LineIdx(3)), "\t\t  b");
    assert_eq!(doc.mark(LineIdx(0)), LineMark::Original);
    let planned = common::save::plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(planned.items[0].content.pre_save, 2);
    let report = common::save::execute(&vfs, planned, &mut doc);
    assert!(report.ok());
    assert_eq!(
        String::from_utf8(read(&vfs, "/o.txt")).unwrap(),
        "keep  \n    lead\nedited\n\t\t  b\nlast\t\n"
    );
    // Nothing edited since the save: nothing to do.
    assert!(apply_save_options(&mut doc, opts).unwrap().is_none());
    // One undo takes the options back.
    let mut fresh = open(
        &vfs,
        &spec("/o.txt"),
        &format,
        DecodeOptions::default(),
        None,
    );
    put_at(&mut fresh, 0, 0, "x ");
    put_at(&mut fresh, 4, 5, " ");
    apply_save_options(&mut fresh, opts).unwrap().unwrap();
    assert_eq!(fresh.line(LineIdx(4)), "last");
    fresh.undo().unwrap();
    assert_eq!(fresh.line(LineIdx(4)), "last\t ");
    assert_eq!(fresh.line(LineIdx(0)), "x keep  ");
}

/// A wrapped line an edit keeps is spliced: the pieces of a physical line keep their marks together, so a delete of
/// the line before it (whose first piece is equal) leaves its malformed bytes as they were (a P-SAVE case).
#[test]
fn a10_13_a_wrapped_line_kept_by_an_edit_is_spliced() {
    let enc = TextEncoding::UTF_16BE;
    let lines: Vec<GenLine> = [
        ("zz a", false, LineEnding::Lf),
        ("zz ", true, LineEnding::CrLf),
        ("", false, LineEnding::Cr),
    ]
    .iter()
    .map(|&(text, bad, eol)| GenLine {
        text: text.into(),
        bad,
        eol,
    })
    .collect();
    let bytes = file_bytes(enc, false, &lines, None);
    let vfs = memory();
    put(&vfs, "/w.txt", &bytes);
    let format = text_format(EncodingSetting::Auto { fallback: None }, false);
    let mut doc = open(&vfs, &spec("/w.txt"), &format, forced(enc), Some(3));
    assert_eq!(doc.line_count(), 5, "two wrapped lines");
    doc.apply(
        &EditRequest::Replace {
            range: TextPos::new(0, 0)..TextPos::new(2, 0),
            text: String::new(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    assert_eq!(doc.marks(LineIdx(0)).orig, Some(LineIdx(2)));
    assert_eq!(doc.marks(LineIdx(1)).orig, Some(LineIdx(3)));
    let planned = common::save::plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(planned.items[0].content.encoded_lines, 0);
    let report = common::save::execute(&vfs, planned, &mut doc);
    assert!(report.ok());
    let first = file_bytes(enc, false, &lines[..1], None).len();
    assert_eq!(read(&vfs, "/w.txt"), bytes[first..].to_vec());
}

/// A BOM taken away or added (`SetEncoding` with the same encoding) splices every line after the source's BOM.
#[test]
fn a10_13_the_bom_alone_changes() {
    let vfs = memory();
    put(&vfs, "/b.txt", b"\xEF\xBB\xBFone\ntwo\n");
    let format = text_format(EncodingSetting::Auto { fallback: None }, false);
    let mut doc = open(
        &vfs,
        &spec("/b.txt"),
        &format,
        DecodeOptions::default(),
        None,
    );
    assert_eq!(doc.encoding(), (TextEncoding::UTF_8, true));
    for (bom, want) in [
        (false, &b"one\ntwo\n"[..]),
        (true, &b"\xEF\xBB\xBFone\ntwo\n"[..]),
    ] {
        doc.apply(
            &EditRequest::SetEncoding {
                encoding: TextEncoding::UTF_8,
                bom,
            },
            GroupHint::None,
        )
        .unwrap();
        let planned = common::save::plan(&vfs, &doc, &SaveRequest::default());
        assert_eq!(planned.items[0].content.encoded_lines, 0, "bom {bom}");
        assert!(common::save::execute(&vfs, planned, &mut doc).ok());
        assert_eq!(read(&vfs, "/b.txt"), want, "bom {bom}");
    }
}

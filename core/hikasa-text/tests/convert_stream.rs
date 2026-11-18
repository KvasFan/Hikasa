//! M2 W10b review fix F4 (W10b note 60): `hikasa text convert` streams its conversions through the save instead of
//! editing the document. P-CONVERT-STREAM: for random files (every terminator, lone CRs before blank lines, malformed
//! bytes, a Ctrl-Z tail, BOMs, legacy, UTF-16 and stateful encodings), random conversions in random order over random
//! line ranges, and an encoding or BOM change, the streamed content is byte for byte what the editor writes after the
//! same `transform` steps and `SetEncoding` (`save_content`), with the same counts, issues and splice, and the bytes a
//! save plan streams onto the file are those.
//!
//! 256 cases unless `PROPTEST_CASES` says otherwise.

mod common;

use common::save::{forced, open, render, save, spec, text_format};
use hikasa_core::{CaseSensitivity, Ctl};
use hikasa_formats::{Encoder, EncodingSetting, TextEncoding};
use hikasa_text::edit::{EditRequest, EditableDocument, GroupHint};
use hikasa_text::save::{
    ContentOptions, ConvertSpec, SaveRequest, SourceMap, convert_counts, converted_content,
    save_content,
};
use hikasa_text::transform::{Transform, transform};
use hikasa_text::{LineEnding, LineIdx, LineMark};
use hikasa_vfs::{MemMeta, MemoryVfs};
use proptest::prelude::*;

/// Seeded xorshift64* (§6.1).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn pick<T: Copy>(&mut self, of: &[T]) -> T {
        of[self.below(of.len() as u64) as usize]
    }
}

fn label(l: &str) -> TextEncoding {
    TextEncoding::for_label(l).unwrap()
}

/// Line contents: blanks only (a trim empties them), leading blanks, trailing blanks, tabs, wide and accented
/// characters, half-width katakana (unmappable in ISO-2022-JP), a literal U+FFFD.
const TEXTS: [&str; 16] = [
    "",
    " ",
    "  ",
    "\t",
    " \t ",
    "    x",
    "\tab\t",
    "a  ",
    "  \tb  c\t",
    "\u{a0}",
    "\u{65e5} \t",
    "e\u{301}\t.",
    "        y",
    "\u{ff76}k",
    "\u{fffd}",
    "z",
];

fn bytes_of(enc: TextEncoding, text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    Encoder::new(enc).encode(text, true, &mut out);
    out
}

/// A random file: its bytes, its encoding, its BOM and whether it has a Ctrl-Z tail.
fn file(rng: &mut Rng) -> (Vec<u8>, TextEncoding, bool, bool) {
    let enc = rng.pick(&[
        TextEncoding::UTF_8,
        TextEncoding::UTF_8,
        TextEncoding::UTF_16LE,
        label("windows-1252"),
        label("Shift_JIS"),
        label("ISO-2022-JP"),
    ]);
    let bom = !enc.bom().is_empty() && rng.below(2) == 0;
    let tail = enc.is_ascii_compatible() && rng.below(5) == 0;
    let mut out = Vec::new();
    if bom {
        out.extend_from_slice(enc.bom());
    }
    let lines = rng.below(14);
    for k in 0..lines {
        let mut text = rng.pick(&TEXTS).to_owned();
        if rng.below(4) == 0 {
            text.push_str(rng.pick(&TEXTS));
        }
        // Only characters the encoding writes.
        if Encoder::new(enc).encode(&text, true, &mut Vec::new()) > 0 {
            text = "w ".to_owned();
        }
        out.extend_from_slice(&bytes_of(enc, &text));
        if enc == TextEncoding::UTF_8 && rng.below(6) == 0 {
            out.extend_from_slice(b"\xFF");
        }
        let eol = if k + 1 == lines && rng.below(3) == 0 {
            ""
        } else {
            rng.pick(&["\n", "\r\n", "\r", "\r", "\n"])
        };
        out.extend_from_slice(&bytes_of(enc, eol));
    }
    if tail {
        out.extend_from_slice(b"\x1Atail\xFE\n");
    }
    (out, enc, bom, tail)
}

fn steps(rng: &mut Rng) -> Vec<Transform> {
    let all = [
        Transform::TrimTrailing,
        Transform::LeadingSpacesToTabs,
        Transform::TabsToSpaces,
        Transform::LineEndings(LineEnding::Lf),
        Transform::LineEndings(LineEnding::CrLf),
        Transform::LineEndings(LineEnding::Cr),
    ];
    (0..rng.below(5)).map(|_| rng.pick(&all)).collect()
}

/// The lines whose mark is *Edited* (no CPL: table lines are physical lines).
fn edited(doc: &EditableDocument) -> u64 {
    (0..doc.line_count())
        .filter(|&l| doc.mark(LineIdx(l)) == LineMark::Edited)
        .count() as u64
}

fn p_convert_stream(seed: u64) {
    let mut rng = Rng(seed | 1);
    let (source, enc, bom, tail) = file(&mut rng);
    let spec_of = ConvertSpec {
        steps: steps(&mut rng),
        lines: match rng.below(3) {
            0 => {
                let a = rng.below(12) as u32;
                Some(a..a + rng.below(8) as u32)
            }
            _ => None,
        },
        tab_size: rng.pick(&[1, 2, 4, 8]),
    };
    let to = match rng.below(4) {
        0 => rng.pick(&[
            TextEncoding::UTF_8,
            TextEncoding::UTF_16BE,
            label("windows-1252"),
            label("ISO-2022-JP"),
        ]),
        _ => enc,
    };
    let to_bom = !to.bom().is_empty() && rng.below(2) == 0;
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_file(
        &common::host("/c.txt"),
        source.clone(),
        MemMeta {
            mtime: hikasa_core::FileTime::from_unix(1_700_000_000, 0),
            ..MemMeta::default()
        },
    )
    .unwrap();
    let format = text_format(EncodingSetting::Auto { fallback: None }, tail);
    let load = || {
        let mut doc = open(&vfs, &spec("/c.txt"), &format, forced(enc), None);
        if doc.encoding() != (to, to_bom) {
            doc.apply(
                &EditRequest::SetEncoding {
                    encoding: to,
                    bom: to_bom,
                },
                GroupHint::None,
            )
            .unwrap();
        }
        doc
    };
    let ctx = format!(
        "seed {seed}: {:?} {} bom {bom} tail {tail} → {} bom {to_bom}, {:?}",
        String::from_utf8_lossy(&source),
        enc.name(),
        to.name(),
        spec_of
    );
    // The editor: each step one request, then the save's content.
    let mut editor = load();
    let mut counts = Vec::new();
    for &step in &spec_of.steps {
        let out = transform(&editor, step, spec_of.lines.clone(), spec_of.tab_size);
        counts.push(out.lines);
        if let Some(request) = &out.request {
            editor.apply(request, GroupHint::None).unwrap();
        }
    }
    let map = SourceMap::scan(
        &mut std::io::Cursor::new(&source),
        enc,
        if bom { enc.bom().len() as u8 } else { 0 },
        tail,
        &Ctl::detached(),
    )
    .unwrap()
    .unwrap();
    let want = save_content(&editor, Some(&map), &ContentOptions::default());
    let want_bytes = render(&want, &source);
    // The stream.
    let mut streamed = load();
    let got_counts = convert_counts(&streamed, &spec_of, &Ctl::detached()).unwrap();
    assert_eq!(got_counts.steps, counts, "step counts, {ctx}");
    assert_eq!(got_counts.changed, edited(&editor), "changed lines, {ctx}");
    assert_eq!(
        got_counts.lines,
        u64::from(editor.line_count()),
        "lines, {ctx}"
    );
    assert_eq!(&got_counts.eols, editor.document().eol(), "eols, {ctx}");
    let got = converted_content(
        &streamed,
        Some(map.clone()),
        &spec_of,
        None,
        &Ctl::detached(),
    )
    .unwrap();
    let got_bytes = render(&got, &source);
    assert_eq!(
        String::from_utf8_lossy(&got_bytes),
        String::from_utf8_lossy(&want_bytes),
        "{ctx}"
    );
    assert_eq!(got_bytes, want_bytes, "{ctx}");
    assert_eq!(got.bytes, want.bytes, "bytes, {ctx}");
    assert_eq!(
        got.bytes,
        got_bytes.len() as u64,
        "the counted bytes, {ctx}"
    );
    assert_eq!(got.spliced_bytes, want.spliced_bytes, "spliced, {ctx}");
    assert_eq!(
        got.encoded_lines, want.encoded_lines,
        "encoded lines, {ctx}"
    );
    assert_eq!(got.issues, want.issues, "issues, {ctx}");
    assert_eq!(got.eols, want.eols, "eols, {ctx}");
    // Through a plan: the file gets those bytes.
    let req = SaveRequest {
        transform: Some(spec_of.clone()),
        ..SaveRequest::default()
    };
    let report = save(&vfs, &mut streamed, &req);
    assert!(report.ok(), "{:?}, {ctx}", report.op.results);
    assert_eq!(
        vfs.read_back(&common::host("/c.txt")).unwrap().to_vec(),
        want_bytes,
        "the file, {ctx}"
    );
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/convert_stream.txt"))]

    /// P-CONVERT-STREAM (W10b note 60).
    #[test]
    fn p_convert_stream_equals_the_editor_and_its_save(seed in any::<u64>()) {
        p_convert_stream(seed);
    }
}

/// The cases the stream must emulate: a trim that empties the line after a lone CR (one CRLF line, so later steps
/// count one line less), and a range conversion of line endings with its neighbours.
#[test]
fn the_editor_s_line_joins_and_neighbours() {
    for (text, steps, lines) in [
        (
            "a\r  \nb \n",
            vec![
                Transform::TrimTrailing,
                Transform::LineEndings(LineEnding::Lf),
            ],
            None,
        ),
        (
            "a\r \n x\n y\n",
            vec![Transform::TrimTrailing, Transform::TabsToSpaces],
            Some(1..2),
        ),
        (
            "a\r\r\r\nb\n\n",
            vec![Transform::LineEndings(LineEnding::Lf)],
            Some(2..3),
        ),
        (
            "a\nb\n\n\nc\n",
            vec![Transform::LineEndings(LineEnding::Cr)],
            Some(1..2),
        ),
    ] {
        let source = text.as_bytes();
        let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
        vfs.insert_file(&common::host("/j.txt"), source.to_vec(), MemMeta::default())
            .unwrap();
        let format = text_format(EncodingSetting::Auto { fallback: None }, false);
        let spec_of = ConvertSpec {
            steps: steps.clone(),
            lines: lines.clone(),
            tab_size: 4,
        };
        let mut editor = open(
            &vfs,
            &spec("/j.txt"),
            &format,
            forced(TextEncoding::UTF_8),
            None,
        );
        for &step in &steps {
            if let Some(request) = transform(&editor, step, lines.clone(), 4).request {
                editor.apply(&request, GroupHint::None).unwrap();
            }
        }
        let streamed = open(
            &vfs,
            &spec("/j.txt"),
            &format,
            forced(TextEncoding::UTF_8),
            None,
        );
        let got = converted_content(&streamed, None, &spec_of, None, &Ctl::detached()).unwrap();
        assert_eq!(
            String::from_utf8(render(&got, source)).unwrap(),
            editor.document().rope().to_string(),
            "{text:?} {steps:?} {lines:?}"
        );
    }
}

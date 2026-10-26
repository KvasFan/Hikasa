//! Patch application (wave-4 decisions 1 and 2): every patch style (unified, annotated unified, context, normal),
//! under the default rules (line endings not compared) and with line endings compared, turns the left file into
//! the right one byte for byte, for LF, CRLF, CR-only (written as LF with `patch_lossy_eol`: checked on the files
//! with their lone CRs turned into LF), mixed endings, missing final newlines and UTF-8 BOMs. Checked with the
//! position-exact appliers of `common::apply` on every platform, and on Unix with GNU `patch` (every style, no
//! fuzz) and `git apply` (unified) when they are on PATH. A missing tool is reported on stderr and skipped; with
//! `HIKASA_REQUIRE_PATCH_TOOLS=1` it is a failure. Also: the `patch_encoding` warning (decision 2) and the
//! escaping of file content in ANSI patches (decision 5).

mod common;

use std::path::Path;
use std::process::Command;

use common::{apply, compare_bytes};
use hikasa_core::{Warning, WarningCode};
use hikasa_report::{
    HtmlStyle, OutputFormat, PatchStyle, ReportMeta, TextLayout, TextReportOptions,
    render_text_report_with,
};
use hikasa_text::{TextComparison, TextRules};

/// `(name, left, right)`: the inputs of the application tests.
const CORPUS: &[(&str, &[u8], &[u8])] = &[
    ("identical", b"a\nb\n", b"a\nb\n"),
    ("both empty", b"", b""),
    ("empty left", b"", b"a\nb\n"),
    ("empty right", b"a\nb\n", b""),
    ("lf change", b"a\nb\nc\n", b"a\nB\nc\n"),
    ("left without final newline", b"a\nb", b"a\nb\n"),
    ("right without final newline", b"a\nb\n", b"a\nb"),
    ("both without final newline", b"a\nb", b"a\nc"),
    ("no final newline, then lines added", b"a\nb", b"a\nb\nc\n"),
    ("no final newline, deep", b"x\ny\nb", b"x\ny\nb\nc"),
    ("final newline and a change", b"x\na\nb", b"y\na\nb\n"),
    ("crlf change", b"a\r\nb\r\nc\r\n", b"a\r\nB\r\nc\r\n"),
    ("crlf to lf", b"a\r\nb\r\n", b"a\nb\n"),
    ("crlf to lf with a change", b"a\r\nb\r\nc\r\n", b"a\nB\nc\n"),
    (
        "crlf to lf, insertion",
        b"one\r\ntwo\r\nthree\r\n",
        b"one\ntwo\nthree\nfour\n",
    ),
    ("mixed endings", b"a\r\nb\nc\r\nd\n", b"a\nb\r\nc\r\nD\n"),
    ("mixed, final newline added", b"a\r\nb", b"a\r\nb\r\n"),
    ("cr only", b"a\rb\rc\r", b"a\rB\rc\r"),
    ("cr to lf", b"a\rb\r", b"a\nb\n"),
    (
        "bom, first line changed",
        b"\xEF\xBB\xBFa\nb\n",
        b"\xEF\xBB\xBFA\nb\n",
    ),
    ("bom removed", b"\xEF\xBB\xBFa\nb\n", b"a\nb\n"),
    ("bom added", b"a\nb\n", b"\xEF\xBB\xBFa\nb\n"),
    (
        "bom, later change",
        b"\xEF\xBB\xBFa\nb\nc\nd\ne\n",
        b"\xEF\xBB\xBFa\nb\nc\nd\nE\n",
    ),
    (
        "patch syntax in content",
        b"--- a\n+++ b\n@@ -1 +1 @@\n\\ x\n*** y\n# z\n-- q\n++ r\n",
        b"-- q2\n++ r\n--- a\n+++ b\n@@ -1 +1 @@\n\\ x\n*** y\n# z\n",
    ),
    ("delete all but one", b"a\nb\nc\nd\n", b"c\n"),
    (
        "duplicate lines",
        b"*** y\nB\n*** y\nc\na",
        b"a\n*** y\nB\nc\n",
    ),
    (
        "duplicate lines 2",
        b"a\n*** y\na  b\n*** y\n",
        b"*** y\na  b\n",
    ),
];

/// The layouts checked, with their applier and the GNU `patch` format flag.
fn layouts() -> Vec<(TextLayout, apply::Applier, &'static str)> {
    let mut out: Vec<(TextLayout, apply::Applier, &'static str)> = Vec::new();
    for n in [0, 1, 3] {
        out.push((
            TextLayout::Patch(PatchStyle::Unified { lines: n }),
            apply::unified,
            "-u",
        ));
        out.push((
            TextLayout::AnnotatedUnified { lines: n },
            apply::unified,
            "-u",
        ));
        out.push((
            TextLayout::Patch(PatchStyle::Context { lines: n }),
            apply::context,
            "-c",
        ));
    }
    out.push((TextLayout::Patch(PatchStyle::Normal), apply::normal, "-n"));
    out
}

/// A generated pair of longer files: sparse changes, endings and a missing final newline.
fn long_pair() -> (Vec<u8>, Vec<u8>) {
    let (mut l, mut r) = (String::new(), String::new());
    for k in 0..60 {
        let end_l = if k % 7 == 3 { "\r\n" } else { "\n" };
        let end_r = if k % 11 == 5 { "\r\n" } else { "\n" };
        l.push_str(&format!("line {}{end_l}", k % 9));
        if k % 13 != 4 {
            let text = if k % 17 == 8 {
                format!("LINE {k}")
            } else {
                format!("line {}", k % 9)
            };
            r.push_str(&format!("{text}{end_r}"));
        }
        if k % 19 == 10 {
            r.push_str("inserted\n");
        }
    }
    r.pop();
    (l.into_bytes(), r.into_bytes())
}

/// Renders `layout` and returns the patch and the warnings of the rendering.
fn render(cmp: &TextComparison, layout: TextLayout) -> (Vec<u8>, Vec<Warning>) {
    let opts = TextReportOptions {
        layout,
        ..TextReportOptions::default()
    };
    let meta = ReportMeta::for_text(cmp, &opts, None, "hikasa test");
    let mut out = Vec::new();
    let warnings =
        render_text_report_with(cmp, &opts, &meta, &HtmlStyle::default(), &mut out).unwrap();
    (out, warnings)
}

/// True when `bytes` holds a CR not followed by LF.
fn has_lone_cr(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .enumerate()
        .any(|(k, &b)| b == b'\r' && bytes.get(k + 1) != Some(&b'\n'))
}

/// The rule sets: the defaults (line endings not compared, D-04) and line endings compared.
fn rule_sets() -> [(&'static str, TextRules); 2] {
    let mut compared = TextRules::default();
    compared.importance.compare_line_endings = true;
    [
        ("default rules", TextRules::default()),
        ("line endings compared", compared),
    ]
}

/// Every input pair, rule set and layout: `(label, left as patched, right as patched, patch, warnings, gnu flag,
/// applier, unified?)`.
struct Rendered {
    label: String,
    left: Vec<u8>,
    right: Vec<u8>,
    patch: Vec<u8>,
    warnings: Vec<Warning>,
    gnu_flag: &'static str,
    applier: apply::Applier,
    unified: bool,
}

fn rendered() -> Vec<Rendered> {
    let mut inputs: Vec<(String, Vec<u8>, Vec<u8>)> = CORPUS
        .iter()
        .map(|(n, l, r)| ((*n).to_owned(), l.to_vec(), r.to_vec()))
        .collect();
    let (l, r) = long_pair();
    inputs.push(("long".to_owned(), l, r));
    let mut out = Vec::new();
    for (name, left, right) in &inputs {
        for (rules_name, rules) in rule_sets() {
            let cmp = compare_bytes(("f.txt", left), ("f.txt", right), &rules);
            let lossy = has_lone_cr(left) || has_lone_cr(right);
            let (l, r) = if lossy {
                (apply::lone_cr_to_lf(left), apply::lone_cr_to_lf(right))
            } else {
                (left.clone(), right.clone())
            };
            for (layout, applier, gnu_flag) in layouts() {
                let (patch, warnings) = render(&cmp, layout);
                out.push(Rendered {
                    label: format!("{name} / {rules_name} / {layout:?}"),
                    left: l.clone(),
                    right: r.clone(),
                    patch,
                    warnings,
                    gnu_flag,
                    applier,
                    unified: gnu_flag == "-u",
                });
            }
        }
    }
    out
}

#[test]
fn every_style_reproduces_the_right_file() {
    for case in rendered() {
        let shown = String::from_utf8_lossy(&case.patch);
        let got = (case.applier)(&case.left, &case.patch)
            .unwrap_or_else(|e| panic!("{}: {e}\n{shown}", case.label));
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&case.right),
            "{}\n{shown}",
            case.label
        );
        apply::no_newline_markers_are_last(&case.patch)
            .unwrap_or_else(|e| panic!("{}: {e}\n{shown}", case.label));
        if case.left == case.right && !case.label.starts_with("cr") {
            assert!(
                !shown.lines().any(|l| !l.starts_with('#')),
                "{}: identical files give the header only\n{shown}",
                case.label
            );
        }
    }
}

#[test]
fn lone_cr_endings_are_written_as_lf_and_warned() {
    for case in rendered() {
        let cr = case.label.starts_with("cr ");
        let warned = case
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::PatchLossyEol);
        assert_eq!(warned, cr, "{}: {:?}", case.label, case.warnings);
        // The warning is in the header too.
        if cr {
            assert!(
                String::from_utf8_lossy(&case.patch).contains("# Warning: patch_lossy_eol: "),
                "{}",
                case.label
            );
        }
    }
}

/// Whether an external tool runs; `None` (with a note on stderr) when it is not on PATH, a failure when
/// `HIKASA_REQUIRE_PATCH_TOOLS=1`.
fn tool(program: &str, version_arg: &str) -> Option<()> {
    let found = Command::new(program)
        .arg(version_arg)
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        let required = std::env::var_os("HIKASA_REQUIRE_PATCH_TOOLS").is_some_and(|v| v == "1");
        assert!(
            !required,
            "`{program}` is required (HIKASA_REQUIRE_PATCH_TOOLS=1) but not on PATH"
        );
        eprintln!("note: SKIPPED the `{program}` checks: `{program}` is not on PATH");
        return None;
    }
    Some(())
}

/// Runs `cmd` in `dir` and returns whether it succeeded, with its output.
fn run(cmd: &mut Command, dir: &Path) -> (bool, String) {
    let out = cmd.current_dir(dir).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
#[cfg_attr(not(unix), ignore = "the patch headers hold Unix host paths")]
fn gnu_patch_applies_every_style_without_fuzz() {
    if tool("patch", "--version").is_none() {
        return;
    }
    // The options below are GNU patch's; another patch(1) (macOS ships a BSD one) is not what this checks.
    let version = Command::new("patch").arg("--version").output().unwrap();
    if !String::from_utf8_lossy(&version.stdout).contains("GNU patch") {
        eprintln!("note: SKIPPED the GNU patch checks: `patch` is not GNU patch");
        return;
    }
    for case in rendered() {
        if case.left == case.right
            && !String::from_utf8_lossy(&case.patch)
                .lines()
                .any(|l| !l.starts_with('#'))
        {
            continue; // a header-only patch: patch(1) would report "Only garbage was found"
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), &case.left).unwrap();
        std::fs::write(dir.path().join("p.diff"), &case.patch).unwrap();
        let (ok, text) = run(
            Command::new("patch").args([
                "--batch",
                "--force",
                "--no-backup-if-mismatch",
                "-F0",
                "-r",
                "-",
                case.gnu_flag,
                "f.txt",
                "-i",
                "p.diff",
            ]),
            dir.path(),
        );
        let got = std::fs::read(dir.path().join("f.txt")).unwrap();
        assert!(
            ok && got == case.right,
            "{}: GNU patch {}: {text}\ngot {:?}\nwant {:?}\n{}",
            case.label,
            if ok { "applied" } else { "failed" },
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&case.right),
            String::from_utf8_lossy(&case.patch)
        );
    }
}

#[test]
#[cfg_attr(not(unix), ignore = "the patch headers hold Unix host paths")]
fn git_apply_applies_unified_patches() {
    if tool("git", "--version").is_none() {
        return;
    }
    for case in rendered().into_iter().filter(|c| c.unified) {
        if !String::from_utf8_lossy(&case.patch)
            .lines()
            .any(|l| !l.starts_with('#'))
        {
            continue; // a header-only patch: git apply reports "No valid patches in input"
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), &case.left).unwrap();
        std::fs::write(dir.path().join("p.diff"), &case.patch).unwrap();
        let ceiling = dir.path().parent().unwrap_or(dir.path());
        let (ok, text) = run(
            Command::new("git")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CEILING_DIRECTORIES", ceiling)
                .args([
                    "-c",
                    "core.autocrlf=false",
                    "-c",
                    "core.whitespace=-trailing-space,-space-before-tab,-indent-with-non-tab,-blank-at-eof,-cr-at-eol",
                    "apply",
                    "-p2",
                    "--unidiff-zero",
                    "--whitespace=nowarn",
                    "p.diff",
                ]),
            dir.path(),
        );
        let got = std::fs::read(dir.path().join("f.txt")).unwrap();
        assert!(
            ok && got == case.right,
            "{}: git apply {}: {text}\ngot {:?}\nwant {:?}\n{}",
            case.label,
            if ok { "applied" } else { "failed" },
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&case.right),
            String::from_utf8_lossy(&case.patch)
        );
    }
}

/// The `patch_encoding` warnings of the default annotated diff.
fn encoding_warnings(left: &[u8], right: &[u8]) -> Vec<Warning> {
    let cmp = compare_bytes(("f.txt", left), ("f.txt", right), &TextRules::default());
    render(&cmp, TextLayout::AnnotatedUnified { lines: 3 })
        .1
        .into_iter()
        .filter(|w| w.code == WarningCode::PatchEncoding)
        .collect()
}

#[test]
fn patches_of_files_in_other_encodings_warn_once_per_side() {
    // windows-1252 on both sides: both are written in UTF-8.
    let w = encoding_warnings(b"caf\xE9 a\nb\nc\n", b"caf\xE9 a\nB\nc\n");
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w[0].message
            .starts_with("the left file is stored in windows-1252;"),
        "{w:?}"
    );
    assert!(
        w[1].message
            .starts_with("the right file is stored in windows-1252;"),
        "{w:?}"
    );
    assert_eq!(
        w[0].spec.as_deref(),
        Some(common::host("/l/f.txt").as_str())
    );
    // UTF-16 against UTF-8: only the UTF-16 side.
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("a\nb\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let w = encoding_warnings(&utf16, b"a\nB\n");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].message
            .starts_with("the left file is stored in UTF-16LE;"),
        "{w:?}"
    );
    // Lossy UTF-8 (enough valid multi-byte text for UTF-8 to be detected, one invalid byte).
    let lossy = |last: &str| {
        let mut b = "é ok é é é é é é é é x".as_bytes().to_vec();
        b.extend_from_slice(b"\xFFy\n");
        b.extend_from_slice(last.as_bytes());
        b
    };
    let w = encoding_warnings(&lossy("b\n"), &lossy("B\n"));
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(w[0].message.contains("is not valid UTF-8"), "{w:?}");
    // Files that differ only in their encoding give no hunk, but the patch says it cannot reproduce the bytes.
    let w = encoding_warnings(b"caf\xE9\n", "café\n".as_bytes());
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].message
            .starts_with("the left file is stored in windows-1252;"),
        "{w:?}"
    );
    // Identical files in another encoding: nothing to warn about.
    assert!(encoding_warnings(b"caf\xE9\n", b"caf\xE9\n").is_empty());
    // UTF-8, with or without BOM: no warning (the BOM is written back).
    assert!(encoding_warnings(b"\xEF\xBB\xBFa\n", b"a\n").is_empty());
    // A file holding only a BOM has no line to write it with.
    let w = encoding_warnings(b"\xEF\xBB\xBF", b"");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].message.contains("holds only a byte-order mark"),
        "{w:?}"
    );
}

#[test]
fn the_bom_is_written_before_line_one() {
    let cmp = compare_bytes(
        ("f.txt", b"\xEF\xBB\xBFa\nb\n"),
        ("f.txt", b"\xEF\xBB\xBFA\nb\n"),
        &TextRules::default(),
    );
    let (patch, _) = render(&cmp, TextLayout::Patch(PatchStyle::Unified { lines: 1 }));
    let text = String::from_utf8(patch).unwrap();
    assert!(
        text.ends_with("@@ -1,2 +1,2 @@\n-\u{feff}a\n+\u{feff}A\n b\n"),
        "{text}"
    );
    // Line 1 as context carries it too.
    let cmp = compare_bytes(
        ("f.txt", b"\xEF\xBB\xBFa\nb\n"),
        ("f.txt", b"\xEF\xBB\xBFa\nB\n"),
        &TextRules::default(),
    );
    let (patch, _) = render(&cmp, TextLayout::Patch(PatchStyle::Context { lines: 1 }));
    let text = String::from_utf8(patch).unwrap();
    assert!(text.contains("\n  \u{feff}a\n"), "{text}");
}

#[test]
fn ansi_patches_escape_control_characters_of_the_content() {
    let cmp = compare_bytes(
        ("f.txt", b"keep\nold\n"),
        ("f.txt", b"keep\nok\x1b[1A\x1b[2K\xE2\x80\xAEevil\tx\n"),
        &TextRules::default(),
    );
    for layout in [
        TextLayout::AnnotatedUnified { lines: 3 },
        TextLayout::Patch(PatchStyle::Context { lines: 1 }),
        TextLayout::Patch(PatchStyle::Normal),
    ] {
        let opts = TextReportOptions {
            layout,
            format: OutputFormat::Text { ansi: true },
            ..TextReportOptions::default()
        };
        let meta = ReportMeta::for_text(&cmp, &opts, None, "hikasa test");
        let mut out = Vec::new();
        render_text_report_with(&cmp, &opts, &meta, &HtmlStyle::default(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains("\x1b[1A") && !text.contains("\x1b[2K"),
            "{layout:?}: {text:?}"
        );
        assert!(!text.contains('\u{202e}'), "{layout:?}: {text:?}");
        assert!(
            text.contains("ok\\u{1b}[1A\\u{1b}[2K\\u{202e}evil\tx"),
            "{layout:?}: {text:?}"
        );
        // The plain patch keeps the bytes (it must apply).
        let (plain, _) = render(&cmp, layout);
        assert!(
            String::from_utf8(plain)
                .unwrap()
                .contains("ok\x1b[1A\x1b[2K\u{202e}evil\tx")
        );
    }
}

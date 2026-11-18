//! W12c review fixes through the binary (`docs/design/m2-contracts.md` §1.12.33, *W12c review fixes*): External load
//! conversions of a Picture format (F1) and of a Hex format (D-331's rows, open item 4) run by `/bin/sh` programs;
//! `--dry-run` with `--diff-output` (F4); the words of `--diff-output` (F7); outputs onto inputs (F8); `hex dump
//! --offset` (F9); the defaults session of the subcommands (F10); the refusals and texts of F11; the tests of the
//! review's surviving mutations that need the binary.

mod common;

use common::{Sandbox, code, json, stderr, stdout};

/// CRC-32 of `data`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// An RGBA PNG of `w` × `h` with the pixel function `f` (zlib stored blocks).
fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend_from_slice(&f(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push(u8::from(i + 1 == chunks.len()));
        let len = c.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn gradient(x: u32, y: u32) -> [u8; 4] {
    [(x * 20) as u8, (y * 30) as u8, 77, 255]
}

/// `gradient` with the blue channel of every pixel `by` higher.
fn shifted(by: u8) -> impl Fn(u32, u32) -> [u8; 4] {
    move |x, y| {
        let mut p = gradient(x, y);
        p[2] += by;
        p
    }
}

fn run(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd().args(args).output().unwrap()
}

/// The pictures: `a.png`, its copy re-encoded with a text chunk (`same.png`: other bytes, the same pixels), `c.png`
/// (every pixel 3 apart), `d.png` (different).
fn pictures(s: &Sandbox) {
    s.file("a.png", png(8, 6, gradient));
    s.file("c.png", png(8, 6, shifted(3)));
    s.file("d.png", png(8, 6, shifted(60)));
}

/// The hex inputs.
fn hex_files(s: &Sandbox) {
    let base: Vec<u8> = (0..3000u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut b = base.clone();
    for x in &mut b[100..120] {
        *x ^= 0xFF;
    }
    b[2500] ^= 0xFF;
    s.file("a.bin", &base);
    s.file("b.bin", &b);
}

/// A `/bin/sh` program in the sandbox (executable).
#[cfg(unix)]
fn program(s: &Sandbox, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = s.file(name, format!("#!/bin/sh\n{body}\n"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("\"{}\" %s %t", path.display())
}

/// F1 and open item 4 through the binary: a Picture format whose External program strips a 4-byte wrapper is
/// converted in `picture compare`, `quick --rules` and the folder content test alike (T11); a Hex format whose
/// program upper-cases its input gives D-331's rows 1, 2 and 13.
#[cfg(unix)]
#[test]
fn f1_and_d331_conversions_through_the_binary() {
    let s = Sandbox::new();
    let unwrap = program(&s, "unwrap.sh", "exec tail -c +5 \"$1\" > \"$2\"");
    s.file(
        "cfg/formats/wrapped.toml",
        format!(
            "schema_version = 1\nid = \"wrapped\"\nname = \"Wrapped\"\nkind = \"picture\"\nmask = \"*.wrap\"\n\n\
             [conversion]\nmethod = \"external\"\nload = '{unwrap}'\n"
        ),
    );
    let wrap = |tag: &[u8], body: Vec<u8>| [tag.to_vec(), body].concat();
    s.file("l/same.wrap", wrap(b"WRAP", png(8, 6, gradient)));
    s.file("r/same.wrap", wrap(b"WRAX", png(8, 6, gradient)));
    s.file("l/diff.wrap", wrap(b"WRAP", png(8, 6, gradient)));
    s.file("r/diff.wrap", wrap(b"WRAP", png(8, 6, shifted(60))));
    for (name, exit, result) in [
        ("same.wrap", 2, "rules_same"),
        ("diff.wrap", 13, "important_differences"),
    ] {
        let (l, r) = (format!("l/{name}"), format!("r/{name}"));
        let out = run(&s, &["picture", "compare", &l, &r, "--json"]);
        assert_eq!(code(&out), exit, "{name}: {}", stderr(&out));
        let doc = json(&out);
        assert_eq!(doc["result"]["verdict"]["result"], result, "{doc}");
        assert_eq!(doc["result"]["left"]["converted"], true);
        assert_eq!(
            code(&run(&s, &["quick", &l, &r, "--rules"])),
            exit,
            "quick {name}"
        );
    }
    let out = run(
        &s,
        &[
            "folder",
            "compare",
            "l",
            "r",
            "--criteria",
            "rules-based",
            "--display",
            "all",
            "--json",
        ],
    );
    let doc = json(&out);
    for (name, result) in [
        ("same.wrap", "rules_same"),
        ("diff.wrap", "important_differences"),
    ] {
        let item = doc["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["path"] == name)
            .unwrap_or_else(|| panic!("{doc}"));
        assert_eq!(item["content"]["result"]["result"], result, "{name}");
    }
    // D-331 through the binary: identical inputs 1; other inputs, equal outputs 2; other outputs 13.
    let upper = program(&s, "upper.sh", "exec tr a-z A-Z < \"$1\" > \"$2\"");
    s.file(
        "cfg/formats/upper.toml",
        format!(
            "schema_version = 1\nid = \"upper\"\nname = \"Upper\"\nkind = \"hex\"\nmask = \"*.up\"\n\n\
             [conversion]\nmethod = \"external\"\nload = '{upper}'\n"
        ),
    );
    s.file("a.up", "abc");
    s.file("a2.up", "abc");
    s.file("b.up", "ABC");
    s.file("c.up", "abd");
    for (l, r, exit) in [
        ("a.up", "a2.up", 1),
        ("a.up", "b.up", 2),
        ("a.up", "c.up", 13),
    ] {
        let out = run(&s, &["hex", "compare", l, r]);
        assert_eq!(code(&out), exit, "{l} {r}: {}", stderr(&out));
    }
}

/// F4: `--dry-run` writes neither the report nor the image and lists both.
#[test]
fn f4_dry_run_lists_the_image_and_writes_nothing() {
    let s = Sandbox::new();
    pictures(&s);
    std::fs::create_dir_all(s.join("out")).unwrap();
    let out = run(
        &s,
        &[
            "picture",
            "compare",
            "a.png",
            "d.png",
            "--diff-output",
            "out/dry.png",
            "--dry-run",
        ],
    );
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(!s.join("out/dry.png").exists());
    let text = stdout(&out);
    assert!(
        text.contains("write ") && text.contains("dry.png"),
        "{text}"
    );
    assert!(text.contains("would write 1 file"), "{text}");
    let out = run(
        &s,
        &[
            "picture",
            "compare",
            "a.png",
            "d.png",
            "--diff-output",
            "out/dry.png",
            "--report",
            "summary",
            "--output",
            "out/dry.html",
            "--dry-run",
        ],
    );
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(!s.join("out/dry.png").exists() && !s.join("out/dry.html").exists());
    assert!(
        stdout(&out).contains("would write 2 files"),
        "{}",
        stdout(&out)
    );
    // `-` with a dry run writes nothing to stdout but the plan.
    let out = run(
        &s,
        &[
            "picture",
            "compare",
            "a.png",
            "d.png",
            "--diff-output",
            "-",
            "--dry-run",
        ],
    );
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("write the image to standard output"),
        "{}",
        stdout(&out)
    );
}

/// F7: only `-` and `clipboard://` are special to `--diff-output`; `clipboard`, `printer` and `browser` are files.
#[test]
fn f7_diff_output_words_are_files() {
    let s = Sandbox::new();
    pictures(&s);
    for word in ["clipboard", "printer", "browser"] {
        let out = run(
            &s,
            &[
                "picture",
                "compare",
                "a.png",
                "d.png",
                "--diff-output",
                word,
            ],
        );
        assert_eq!(code(&out), 13, "{word}: {}", stderr(&out));
        let written = std::fs::read(s.join(word)).unwrap();
        assert!(written.starts_with(b"\x89PNG\r\n\x1a\n"), "{word}");
    }
}

/// F8: an output never replaces an input (`--diff-output`, `--output`; the same path, a link, an archive member).
#[test]
fn f8_outputs_onto_inputs_are_refused() {
    let s = Sandbox::new();
    pictures(&s);
    hex_files(&s);
    let before = std::fs::read(s.join("a.png")).unwrap();
    let refused = |args: &[&str], what: &str| {
        let out = run(&s, args);
        assert_eq!(code(&out), 100, "{args:?}");
        assert!(stderr(&out).contains(what), "{args:?}: {}", stderr(&out));
    };
    refused(
        &[
            "picture",
            "compare",
            "a.png",
            "d.png",
            "--diff-output",
            "a.png",
        ],
        "left input",
    );
    refused(
        &[
            "picture",
            "compare",
            "a.png",
            "d.png",
            "--diff-output",
            "./d.png",
        ],
        "right input",
    );
    refused(
        &[
            "picture", "compare", "a.png", "d.png", "--report", "summary", "--output", "d.png",
        ],
        "right input",
    );
    refused(
        &[
            "hex", "compare", "a.bin", "b.bin", "--report", "summary", "--output", "a.bin",
        ],
        "left input",
    );
    assert_eq!(std::fs::read(s.join("a.png")).unwrap(), before);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(s.join("a.png"), s.join("link.png")).unwrap();
        refused(
            &[
                "picture",
                "compare",
                "a.png",
                "d.png",
                "--diff-output",
                "link.png",
            ],
            "left input",
        );
        assert_eq!(std::fs::read(s.join("a.png")).unwrap(), before);
    }
}

/// F9: `hex dump --offset` past the end or negative is refused (exit 100); the last byte and `end` are dumped.
#[test]
fn f9_hex_dump_offsets() {
    let s = Sandbox::new();
    s.file("r1.bin", vec![7u8; 1000]);
    for bad in ["1000", "0x30D40", "18446744073709551615", "-1", "-5"] {
        let out = run(
            &s,
            &["hex", "dump", "r1.bin", "--offset", bad, "--length", "4"],
        );
        assert_eq!(code(&out), 100, "{bad}: {}", stdout(&out));
    }
    let out = run(&s, &["hex", "dump", "r1.bin", "--offset", "1000"]);
    assert!(stderr(&out).contains("past the end"), "{}", stderr(&out));
    for good in ["999", "end", "0x3E7"] {
        let out = run(&s, &["hex", "dump", "r1.bin", "--offset", good, "--json"]);
        assert_eq!(code(&out), 0, "{good}: {}", stderr(&out));
        assert_eq!(json(&out)["result"]["offset"], 999, "{good}");
    }
}

/// F10 (D-374): `picture compare` and `hex compare` start from their type's defaults session, as `quick` and the
/// folder content test do; a flag typed wins.
#[test]
fn f10_subcommands_start_from_the_defaults_session() {
    let s = Sandbox::new();
    pictures(&s);
    hex_files(&s);
    assert_eq!(
        code(&run(&s, &["picture", "compare", "a.png", "c.png"])),
        13
    );
    let out = run(
        &s,
        &[
            "session",
            "edit",
            "--defaults",
            "picture-compare",
            "--set",
            "rules.picture.tolerance=4",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        code(&run(&s, &["picture", "compare", "a.png", "c.png"])),
        12
    );
    assert_eq!(code(&run(&s, &["quick", "a.png", "c.png", "--rules"])), 12);
    assert_eq!(
        code(&run(
            &s,
            &["picture", "compare", "a.png", "c.png", "--tolerance", "0"]
        )),
        13
    );
    let out = run(
        &s,
        &[
            "session",
            "edit",
            "--defaults",
            "hex-compare",
            "--set",
            "rules.hex.alignment=\"none\"",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = run(&s, &["hex", "compare", "a.bin", "b.bin", "--json"]);
    assert_eq!(json(&out)["result"]["alignment"]["mode"], "none");
    let out = run(
        &s,
        &[
            "hex",
            "compare",
            "a.bin",
            "b.bin",
            "--alignment",
            "complete",
            "--json",
        ],
    );
    assert_eq!(json(&out)["result"]["alignment"]["mode"], "complete");
}

/// F11: the refusals of flags that would be ignored, the texts, `session run`'s checkerboard; and the review's
/// survivors N15, N18, N21, N22, N24–N26, N28–N31, N33, N37, N44, N47.
#[test]
fn f11_refusals_texts_and_survivors() {
    let s = Sandbox::new();
    pictures(&s);
    hex_files(&s);
    let hex = |args: &[&str]| {
        let mut all = vec!["hex", "compare", "a.bin", "b.bin"];
        all.extend_from_slice(args);
        run(&s, &all)
    };
    // Layout flags with the sections of `--display differences` (or none): refused; with rows or a report: taken.
    for flags in [
        &["--context", "2"][..],
        &["--bytes-per-line", "8"],
        &["--display", "none", "--group", "2"],
    ] {
        let out = hex(flags);
        assert_eq!(code(&out), 100, "{flags:?}");
        assert!(stderr(&out).contains("have no rows"), "{}", stderr(&out));
    }
    assert_eq!(code(&hex(&["--display", "all", "--context", "2"])), 11);
    assert_eq!(
        code(&hex(&["--display", "same", "--bytes-per-line", "8"])),
        11
    );
    // N15: `--context` with `--json` alone is refused.
    assert_eq!(code(&hex(&["--context", "2", "--json"])), 100);
    // `--block` names the word typed.
    let out = hex(&["--block", "3k"]);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("\"3k\""), "{}", stderr(&out));
    // N24: `--gui` refuses `--preview`.
    let out = hex(&["--gui", "--no-launch", "--preview", "4"]);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("--preview"), "{}", stderr(&out));
    // A dump has no left side.
    let out = run(&s, &["hex", "dump", "nonexist.bin"]);
    assert_eq!(code(&out), 100);
    assert!(!stderr(&out).contains("left input"), "{}", stderr(&out));
    // N21: `--address` with `--json` is refused.
    assert_eq!(
        code(&run(
            &s,
            &["hex", "dump", "a.bin", "--address", "decimal", "--json"]
        )),
        100
    );
    // N18: `--count --json` keeps the counts, not every offset.
    let out = run(
        &s,
        &["hex", "find", "--hex", "00", "a.bin", "--count", "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let file = &json(&out)["result"]["files"][0];
    assert!(file["hit_count"].as_u64().unwrap() > 0, "{file}");
    assert_eq!(file["hits"], serde_json::json!([]), "{file}");
    // N22: a file that cannot be read is named on stderr.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = s.file("find/locked.bin", "needle");
        s.file("find/ok.bin", "needle");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&locked).is_err() {
            let out = run(&s, &["hex", "find", "needle", "find"]);
            assert_eq!(code(&out), 0);
            assert!(stderr(&out).contains("skipped: "), "{}", stderr(&out));
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    // N25, N26: `--diff-output -` refuses a report on stdout; alone, stdout holds the PNG and nothing else.
    assert_eq!(
        code(&run(
            &s,
            &[
                "picture",
                "compare",
                "a.png",
                "d.png",
                "--diff-output",
                "-",
                "--report",
                "summary"
            ]
        )),
        100
    );
    let out = run(
        &s,
        &["picture", "compare", "a.png", "d.png", "--diff-output", "-"],
    );
    assert_eq!(code(&out), 13);
    assert!(out.stdout.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(
        out.stdout
            .ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]),
        "{} bytes",
        out.stdout.len()
    );
    // N33: `mode-blend` keeps the comparison's percentage.
    let out = run(
        &s,
        &[
            "picture",
            "compare",
            "a.png",
            "d.png",
            "--mode",
            "blend",
            "--blend",
            "30",
            "--report",
            "summary",
            "--options",
            "mode-blend",
        ],
    );
    assert!(stdout(&out).contains("blend 30 %"), "{}", stdout(&out));
    // N31, N44: the summaries' values are escaped.
    s.file("x&y.png", png(8, 6, gradient));
    s.file("x&y.bin", b"abc");
    for (args, raw) in [
        (["picture", "compare", "x&y.png", "d.png"], "x&y.png"),
        (["hex", "compare", "x&y.bin", "a.bin"], "x&y.bin"),
    ] {
        let mut all = args.to_vec();
        all.extend(["--report", "summary", "--html"]);
        let out = run(&s, &all);
        let html = stdout(&out);
        assert!(html.contains("x&amp;y"), "{html}");
        assert!(!html.contains(raw), "{raw} unescaped");
    }
    // N37: the JSON names the tone map an HDR side was decoded with.
    let hdr = |rgbe: [u8; 4]| {
        let mut f = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
        f.extend(rgbe);
        f.extend(rgbe);
        f
    };
    s.file("l.hdr", hdr([128, 64, 32, 129]));
    s.file("r.hdr", hdr([128, 64, 33, 129]));
    let out = run(
        &s,
        &[
            "picture",
            "compare",
            "l.hdr",
            "r.hdr",
            "--tone-map",
            "reinhard",
            "--json",
        ],
    );
    assert_eq!(
        json(&out)["result"]["left"]["tone_map"],
        "reinhard",
        "{}",
        stderr(&out)
    );
    // Sessions.
    for (name, kind, l, r) in [
        ("h", "hex-compare", "a.bin", "b.bin"),
        ("p", "picture-compare", "a.png", "c.png"),
    ] {
        let out = run(
            &s,
            &[
                "session", "new", name, "--type", kind, "--left", l, "--right", r,
            ],
        );
        assert_eq!(code(&out), 0, "{}", stderr(&out));
    }
    // `--iu` means nothing to a hex session.
    let out = run(&s, &["session", "run", "h", "--iu"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("--ignore-unimportant"),
        "{}",
        stderr(&out)
    );
    // `--context` only with rows or a report (N47: it reaches the report).
    assert_eq!(
        code(&run(&s, &["session", "run", "h", "--context", "1"])),
        100
    );
    let report = |context: &str| {
        let out = run(
            &s,
            &[
                "session",
                "run",
                "h",
                "--report",
                "side-by-side",
                "--display",
                "differences",
                "--context",
                context,
            ],
        );
        assert_eq!(code(&out), 11, "{}", stderr(&out));
        stdout(&out).lines().count()
    };
    assert!(report("2") > report("0"));
    // N30: `--preview` reaches a hex session's run.
    let preview = |n: &str| stdout(&run(&s, &["session", "run", "h", "--preview", n]));
    assert_ne!(preview("2"), preview("8"));
    // N29: `--probe` reaches a picture session's run.
    let out = run(&s, &["session", "run", "p", "--probe", "2,3"]);
    assert!(stdout(&out).contains("probe 2,3:"), "{}", stdout(&out));
    // N28: the session's ignore_unimportant stays without `--iu`.
    let out = run(
        &s,
        &[
            "session",
            "edit",
            "p",
            "--set",
            "rules.picture.tolerance=4",
            "--set",
            "rules.picture.ignore_unimportant=true",
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(code(&run(&s, &["session", "run", "p"])), 2);
    // The session's `[view] checkerboard` (on by default) is the image's.
    let out = run(
        &s,
        &["session", "run", "p", "--diff-output", "s.png", "--json"],
    );
    assert_eq!(
        json(&out)["result"]["diff_output"]["checkerboard"],
        true,
        "{}",
        stderr(&out)
    );
    let out = run(
        &s,
        &["session", "edit", "p", "--set", "view.checkerboard=false"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = run(
        &s,
        &["session", "run", "p", "--diff-output", "s.png", "--json"],
    );
    assert_eq!(json(&out)["result"]["diff_output"]["checkerboard"], false);
    let out = run(
        &s,
        &[
            "session",
            "run",
            "p",
            "--diff-output",
            "s.png",
            "--checkerboard",
            "--json",
        ],
    );
    assert_eq!(json(&out)["result"]["diff_output"]["checkerboard"], true);
    // Help texts.
    let help = stdout(&run(&s, &["session", "run", "--help"]));
    assert!(help.contains("Hex Compare or Picture Compare"), "{help}");
    let help = stdout(&run(&s, &["picture", "compare", "--help"]));
    assert!(
        !help.contains("--line-numbers") && !help.contains("interleaved"),
        "{help}"
    );
}

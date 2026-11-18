//! `hikasa hex compare|dump|find` and `hikasa picture compare` through the binary (§6.12 A12.19–A12.22, A12.24,
//! A12.26): human and JSON goldens (`tests/goldens/hex/`, `tests/goldens/picture/`; the JSON goldens are validated
//! against `docs/schemas/` by `hikasa-report`'s `hex_picture_json_schema.rs`), the exit codes, the refusals, stdin,
//! archive members, `--diff-output` to a file, an archive member, stdout and the clipboard, HX-009 against `quick
//! --binary` and the folder binary test (A12.20), `session run` and the script commands against the subcommands (T11,
//! RPT-011), `--gui` (D-350).

mod common;

use std::sync::Arc;

use common::{CloseMode, FakeGui, Sandbox, assert_golden, code, json, serve, stderr, stdout};
use hikasa_core::ExitCode;

/// Seeded xorshift bytes.
fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// CRC-32 of `data`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// An RGBA PNG of `w` × `h` with the pixel function `f` (zlib stored blocks: no crate, nothing committed).
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

/// A zip of stored entries.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let header = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0x6000u16.to_le_bytes());
            out.extend_from_slice(&0x5cefu16.to_le_bytes());
            out.extend_from_slice(&crc32(data).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04");
        header(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&(0x0300u16 | 20).to_le_bytes());
        header(&mut central);
        central.extend_from_slice(&[0u8; 6]);
        central.extend_from_slice(&(0o100_644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0u8; 2]);
    out
}

fn gradient(x: u32, y: u32) -> [u8; 4] {
    [(x * 20) as u8, (y * 30) as u8, 77, 255]
}

fn run(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd().args(args).output().unwrap()
}

/// The hex inputs: a 3 KiB file, a copy with two changed runs and an insertion, a shorter one.
fn hex_files(s: &Sandbox) {
    let base = bytes(11, 3000);
    s.file("a.bin", &base);
    let mut b = base.clone();
    b[100..104].copy_from_slice(b"WXYZ");
    b.splice(2000..2000, b"inserted bytes".iter().copied());
    b[2500] ^= 1;
    s.file("b.bin", &b);
    s.file("same.bin", &base);
    s.file("short.bin", &base[..2990]);
}

/// A12.19: human output goldens (sections, `--max-sections`, `--preview`, `--display`), the JSON golden, every exit
/// code of the binary, the refusals.
#[test]
fn a12_19_hex_compare_outputs_and_exit_codes() {
    let s = Sandbox::new();
    hex_files(&s);
    let hex = |args: &[&str]| {
        let mut all = vec!["hex", "compare", "a.bin", "b.bin"];
        all.extend_from_slice(args);
        run(&s, &all)
    };
    let out = hex(&[]);
    assert_eq!(code(&out), 11, "{}", stderr(&out));
    assert_golden("hex/compare.txt", &s.normalise(&stdout(&out)));
    let out = hex(&[
        "--max-sections",
        "1",
        "--preview",
        "4",
        "--alignment",
        "complete",
    ]);
    assert_golden("hex/compare_max_sections.txt", &s.normalise(&stdout(&out)));
    let out = hex(&["--alignment", "none", "--display", "none"]);
    assert_golden("hex/compare_none.txt", &s.normalise(&stdout(&out)));
    let out = hex(&[
        "--display",
        "all",
        "--bytes-per-line",
        "8",
        "--group",
        "2",
        "--chars",
        "none",
        "--address",
        "decimal",
    ]);
    assert_eq!(code(&out), 11);
    let text = stdout(&out);
    assert!(
        text.lines().count() > 300,
        "every row: {}",
        text.lines().count()
    );
    let out = hex(&["--json"]);
    assert_eq!(code(&out), 11);
    assert_golden("hex/compare.json", &s.normalise(&stdout(&out)));
    // The exit codes: identical, different, a prefix, every alignment.
    for mode in ["none", "fast", "complete"] {
        let c = |l: &str, r: &str| code(&run(&s, &["hex", "compare", l, r, "--alignment", mode]));
        assert_eq!(c("a.bin", "same.bin"), 1, "{mode}");
        assert_eq!(c("a.bin", "b.bin"), 11, "{mode}");
        assert_eq!(c("a.bin", "short.bin"), 11, "{mode}");
    }
    // 100: a missing file, a bad word (nothing read), a refusal.
    let out = hex(&["--alignment", "slow"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("invalid alignment"),
        "{}",
        stderr(&out)
    );
    let out = run(&s, &["hex", "compare", "a.bin", "missing.bin", "--json"]);
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "not_found");
    let out = hex(&["--json", "--bytes-per-line", "8"]);
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "invalid_input");
    let out = hex(&["--preview", "5000"]);
    assert_eq!(code(&out), 100);
    // Standard input.
    let out = s
        .cmd()
        .args(["hex", "compare", "-", "a.bin"])
        .write_stdin(bytes(11, 3000))
        .output()
        .unwrap();
    assert_eq!(code(&out), 1, "{}", stderr(&out));
}

/// A12.19: an archive member as an input; A12.31: a non-UTF-8 name (Linux).
#[test]
fn a12_19_archive_members_and_names() {
    let s = Sandbox::new();
    hex_files(&s);
    let b = std::fs::read(s.join("b.bin")).unwrap();
    s.file("a.zip", zip_of(&[("in/b.bin", &b)]));
    let out = run(&s, &["hex", "compare", "b.bin", "a.zip/in/b.bin"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::OsStr::from_bytes(b"caf\xe9.bin");
        std::fs::copy(s.join("a.bin"), s.path().join(name)).unwrap();
        let out = s
            .cmd()
            .args(["hex", "compare"])
            .arg(name)
            .arg("a.bin")
            .output()
            .unwrap();
        assert_eq!(code(&out), 1, "{}", stderr(&out));
        let out = s
            .cmd()
            .args(["hex", "find", "--hex", "00"])
            .arg(name)
            .output()
            .unwrap();
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let png = png(2, 2, gradient);
        let pname = std::ffi::OsStr::from_bytes(b"\xffpic.png");
        std::fs::write(s.path().join(pname), &png).unwrap();
        s.file("pic.png", &png);
        let out = s
            .cmd()
            .args(["picture", "compare"])
            .arg(pname)
            .arg("pic.png")
            .output()
            .unwrap();
        assert_eq!(code(&out), 1, "{}", stderr(&out));
    }
}

/// A12.21: `hex dump` and `hex find` goldens (human and JSON); exit 0 with and without hits; `--archives opened`.
#[test]
fn a12_21_dump_and_find_goldens() {
    let s = Sandbox::new();
    s.file("d.bin", (0u8..=255).cycle().take(300).collect::<Vec<_>>());
    let out = run(
        &s,
        &["hex", "dump", "d.bin", "--offset", "0x20", "--length", "40"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("hex/dump.txt", &s.normalise(&stdout(&out)));
    let out = run(
        &s,
        &[
            "hex", "dump", "d.bin", "--offset", "296", "--values", "--json",
        ],
    );
    assert_eq!(code(&out), 0);
    assert_golden("hex/dump.json", &s.normalise(&stdout(&out)));
    let out = run(
        &s,
        &[
            "hex", "dump", "d.bin", "--offset", "end", "--length", "4", "--values",
        ],
    );
    assert_golden("hex/dump_values.txt", &s.normalise(&stdout(&out)));
    // Find over a tree with masks; a zip opened.
    s.file("t/a.bin", b"..MAGIC..MAGIC");
    s.file("t/sub/b.dat", b"magic");
    s.file("t/skip.txt", b"MAGIC");
    s.file("t/.hidden.bin", b"MAGIC");
    s.file("t/z.zip", zip_of(&[("m.bin", b"xMAGIC")]));
    let find = |args: &[&str]| {
        let mut all = vec!["hex", "find"];
        all.extend_from_slice(args);
        run(&s, &all)
    };
    let out = find(&["MAGIC", "t", "-r", "--mask", "*.bin;*.dat"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_golden("hex/find.txt", &s.normalise(&stdout(&out)));
    let out = find(&["MAGIC", "t", "-r", "--count", "--case"]);
    assert_golden("hex/find_count.txt", &s.normalise(&stdout(&out)));
    let out = find(&[
        "4d 41 47 49 43",
        "t",
        "-r",
        "--hex",
        "-l",
        "--archives",
        "opened",
    ]);
    assert_golden("hex/find_files.txt", &s.normalise(&stdout(&out)));
    let out = find(&["MAGIC", "t", "-r", "--max-matches", "2", "--json"]);
    assert_eq!(code(&out), 0);
    assert_golden("hex/find.json", &s.normalise(&stdout(&out)));
    let out = find(&["NOTHING", "t", "-r"]);
    assert_eq!((code(&out), stdout(&out)), (0, String::new()));
    let out = find(&["zz", "t", "--hex"]);
    assert_eq!(code(&out), 100);
}

/// A12.20, HX-009: for 50 generated pairs (equal, a prefix, one byte at chunk-relevant offsets, lengths around the
/// 8 MiB chunk), `hex compare` in every mode exits with `quick --binary`'s code, and the folder binary content test of
/// a tree holding them agrees row by row.
#[test]
fn a12_20_hex_verdicts_equal_quick_binary() {
    let s = Sandbox::new();
    let chunk = 8usize << 20;
    let mut names = Vec::new();
    for k in 0..50u64 {
        let (len, edit): (usize, Option<usize>) = match k {
            0 => (0, None),
            1 => (1, None),
            2 => (chunk + 1, Some(chunk)),
            3 => (chunk - 1, None),
            // 16 MiB + 1: `quick --binary` maps the files and visits two chunk pairs; the edit is the second chunk's
            // first byte.
            4 => (2 * chunk + 1, Some(chunk)),
            _ => (
                (bytes(k, 2).iter().map(|&b| usize::from(b)).sum::<usize>() * 37) % 6000 + 1,
                None,
            ),
        };
        let a = bytes(k + 100, len);
        let mut b = a.clone();
        match (k % 4, edit) {
            (_, Some(at)) => b[at] ^= 0x55,
            (0, None) => {}
            (1, None) if !b.is_empty() => {
                b.pop();
            }
            (2, None) if !b.is_empty() => {
                let at = (k as usize * 131) % b.len();
                b[at] = b[at].wrapping_add(1);
            }
            _ => b.extend_from_slice(b"tail"),
        }
        let name = format!("p{k:02}.bin");
        s.file(&format!("l/{name}"), &a);
        s.file(&format!("r/{name}"), &b);
        names.push(name);
    }
    let mut quick_codes = Vec::new();
    for name in &names {
        let (l, r) = (format!("l/{name}"), format!("r/{name}"));
        let quick = code(&run(&s, &["quick", &l, &r, "--binary"]));
        quick_codes.push(quick);
        for mode in ["none", "fast", "complete"] {
            let c = code(&run(&s, &["hex", "compare", &l, &r, "--alignment", mode]));
            assert_eq!(c, quick, "{name} {mode}");
        }
    }
    let out = run(
        &s,
        &[
            "folder",
            "compare",
            "l",
            "r",
            "--criteria",
            "binary",
            "--display",
            "all",
            "--json",
        ],
    );
    let doc = json(&out);
    for (name, quick) in names.iter().zip(&quick_codes) {
        let item = doc["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["path"] == name.as_str())
            .unwrap_or_else(|| panic!("{name}"));
        let result = item["content"]["result"]["result"].as_str().unwrap();
        let want = if *quick == 1 {
            "binary_same"
        } else {
            "binary_different"
        };
        assert_eq!(result, want, "{name}");
    }
}

/// The picture inputs.
fn pictures(s: &Sandbox) {
    s.file("a.png", png(8, 6, gradient));
    s.file("same.png", png(8, 6, gradient));
    s.file(
        "b.png",
        png(8, 6, |x, y| {
            let mut p = gradient(x, y);
            if x == 2 {
                p[1] = p[1].wrapping_add(4);
            }
            if (x, y) == (5, 5) {
                p[0] = 255;
            }
            p
        }),
    );
    s.file("wide.png", png(10, 6, gradient));
}

/// A12.22: human and JSON goldens, every exit code of D-339, the outputs of `--diff-output`, every refusal.
#[test]
fn a12_22_picture_compare() {
    let s = Sandbox::new();
    pictures(&s);
    let pic = |args: &[&str]| {
        let mut all = vec!["picture", "compare"];
        all.extend_from_slice(args);
        run(&s, &all)
    };
    let out = pic(&[
        "a.png", "b.png", "--probe", "2,0", "--probe", "5,5", "--probe", "50,50",
    ]);
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert_golden("picture/compare.txt", &s.normalise(&stdout(&out)));
    let out = pic(&[
        "a.png",
        "wide.png",
        "--auto-scale",
        "--left-orientation",
        "flip-h,flip-h",
        "--offset",
        "1,-1",
        "--tolerance",
        "4",
        "--mode",
        "blend",
        "--blend",
        "25",
        "--probe",
        "0,0",
        "--json",
    ]);
    assert_golden("picture/compare.json", &s.normalise(&stdout(&out)));
    // Exit codes.
    let c = |args: &[&str]| code(&pic(args));
    assert_eq!(c(&["a.png", "same.png"]), 1);
    let s2 = |f: &str, bytes: Vec<u8>| s.file(f, bytes);
    s2("a.bmp", bmp(8, 6));
    assert_eq!(c(&["a.png", "a.bmp"]), 2);
    s2(
        "c.png",
        png(8, 6, |x, y| {
            let mut p = gradient(x, y);
            p[2] += 2;
            p
        }),
    );
    assert_eq!(c(&["a.png", "c.png", "--tolerance", "2"]), 12);
    assert_eq!(c(&["a.png", "c.png", "--tolerance", "2", "--iu"]), 2);
    assert_eq!(c(&["a.png", "c.png"]), 13);
    assert_eq!(c(&["a.png", "nope.png"]), 100);
    // --diff-output: a file (the engine's render), an archive member, stdout, the clipboard.
    let out = pic(&[
        "a.png",
        "b.png",
        "--diff-output",
        "d.png",
        "--checkerboard",
        "--diff-pane",
        "left",
    ]);
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let written = std::fs::read(s.join("d.png")).unwrap();
    assert!(written.starts_with(b"\x89PNG"));
    let out = pic(&["a.png", "b.png", "--diff-output", "-", "--mode", "range"]);
    assert_eq!(code(&out), 13);
    assert!(out.stdout.starts_with(b"\x89PNG"), "only the PNG on stdout");
    s.file("box.zip", empty_zip());
    let out = pic(&["a.png", "b.png", "--diff-output", "box.zip/diff.png"]);
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    let out = run(&s, &["vfs", "cat", "box.zip/diff.png"]);
    assert!(out.stdout.starts_with(b"\x89PNG"), "{}", stderr(&out));
    let clip = s.file("clip.bin", b"");
    let out = s
        .cmd()
        .env("HIKASA_CLIPBOARD_FILE", &clip)
        .args([
            "picture",
            "compare",
            "a.png",
            "b.png",
            "--diff-output",
            "clipboard://",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert_eq!(
        json(&out)["result"]["diff_output"]["target"],
        "clipboard://"
    );
    assert!(std::fs::read(&clip).unwrap().starts_with(b"\x89PNG"));
    // The refusals of §1.12.23: exit 100 before anything is read.
    for (args, kind) in [
        (vec!["nope.png", "nope.png", "--metadata"], "unsupported"),
        (
            vec!["nope.png", "nope.png", "--diff-output", "-", "--json"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--checkerboard"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--diff-pane", "left"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--blend", "20"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--report", "side-by-side"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--tolerance", "x"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--gui", "--probe", "1,1"],
            "invalid_input",
        ),
        (
            vec!["nope.png", "nope.png", "--gui", "--offset", "a"],
            "invalid_input",
        ),
    ] {
        let mut args = args;
        if !args.contains(&"--json") {
            args.push("--json");
        }
        let out = pic(&args);
        assert_eq!(code(&out), 100, "{args:?}");
        let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(doc["error"]["kind"], kind, "{args:?}");
    }
}

/// A 24-bit BMP of the gradient.
fn bmp(w: u32, h: u32) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + row * h).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let mut line = Vec::new();
        for x in 0..w {
            let [r, g, b, _] = gradient(x, y);
            line.extend_from_slice(&[b, g, r]);
        }
        line.resize(row as usize, 0);
        out.extend_from_slice(&line);
    }
    out
}

fn empty_zip() -> Vec<u8> {
    zip_of(&[("keep.txt", b"x")])
}

/// A12.23 (RPT-011) and A12.24 (T11): the script commands write the bytes of the `--report` shortcuts; `session run`
/// gives the subcommand's JSON and exit code for both types.
#[test]
fn a12_23_scripts_and_sessions_equal_the_subcommands() {
    let s = Sandbox::new();
    hex_files(&s);
    pictures(&s);
    let fixed = |cmd: &mut assert_cmd::Command| {
        cmd.env("SOURCE_DATE_EPOCH", "1700000000");
    };
    for (cli, script) in [
        (
            vec![
                "hex",
                "compare",
                "a.bin",
                "b.bin",
                "--report",
                "interleaved",
                "--output",
                "cli.txt",
            ],
            "hex-report layout:interleaved a.bin b.bin output-to:script.txt",
        ),
        (
            vec![
                "hex", "compare", "a.bin", "b.bin", "--report", "summary", "--html", "--output",
                "cli.txt",
            ],
            "hex-report layout:summary output-options:html-color a.bin b.bin output-to:script.txt",
        ),
        (
            vec![
                "picture", "compare", "a.png", "b.png", "--report", "summary", "--output",
                "cli.txt",
            ],
            "picture-report a.png b.png output-to:script.txt",
        ),
        (
            vec![
                "picture",
                "compare",
                "a.png",
                "b.png",
                "--report",
                "side-by-side",
                "--html",
                "--options",
                "full-size,mode-range",
                "--output",
                "cli.txt",
            ],
            "picture-report layout:side-by-side options:full-size,mode-range output-options:html-color a.png b.png output-to:script.txt",
        ),
    ] {
        let mut cmd = s.cmd();
        fixed(&mut cmd);
        let out = cmd.args(&cli).output().unwrap();
        assert!(matches!(code(&out), 11 | 13), "{cli:?}: {}", stderr(&out));
        s.file("s.txt", script);
        let mut cmd = s.cmd();
        fixed(&mut cmd);
        let out = cmd.args(["script", "run", "s.txt"]).output().unwrap();
        assert_eq!(code(&out), 0, "{script}: {}", stderr(&out));
        let a = std::fs::read(s.join("cli.txt")).unwrap();
        let b = std::fs::read(s.join("script.txt")).unwrap();
        assert_eq!(a, b, "{script}");
    }
    // `media-report` stays refused.
    s.file("m.txt", "media-report\n");
    let out = run(&s, &["script", "run", "m.txt"]);
    assert_eq!(code(&out), 100);
    // session run = the subcommand (JSON and exit code).
    for (kind, l, r, sub) in [
        (
            "hex-compare",
            "a.bin",
            "b.bin",
            vec!["hex", "compare", "a.bin", "b.bin", "--json"],
        ),
        (
            "picture-compare",
            "a.png",
            "b.png",
            vec!["picture", "compare", "a.png", "b.png", "--json"],
        ),
    ] {
        let out = run(
            &s,
            &[
                "session", "new", kind, "--type", kind, "--left", l, "--right", r,
            ],
        );
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let a = run(&s, &["session", "run", kind, "--json"]);
        let b = run(&s, &sub);
        assert_eq!(code(&a), code(&b), "{kind}");
        assert_eq!(json(&a)["result"], json(&b)["result"], "{kind}");
    }
}

/// A12.26, A13.21: `--gui` checks every word first (a bad word exits 100 without contacting a GUI); since W13a a
/// served GUI opens `hex compare --gui` (its view's exit code), and answers `unsupported_view` for the picture kind
/// (exit 100) until W13b.
#[test]
fn a12_26_gui() {
    let s = Sandbox::new();
    hex_files(&s);
    pictures(&s);
    let out = run(
        &s,
        &[
            "hex",
            "compare",
            "a.bin",
            "b.bin",
            "--gui",
            "--alignment",
            "x",
            "--json",
        ],
    );
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "invalid_input");
    let out = run(
        &s,
        &[
            "hex",
            "compare",
            "a.bin",
            "b.bin",
            "--gui",
            "--max-sections",
            "3",
        ],
    );
    assert_eq!(code(&out), 100);
    let gui = FakeGui::new(CloseMode::Immediately, ExitCode::BinarySame);
    let _server = serve(&s, Arc::clone(&gui));
    let out = run(
        &s,
        &[
            "hex",
            "compare",
            "a.bin",
            "b.bin",
            "--gui",
            "--no-launch",
            "--wait",
        ],
    );
    assert_eq!(code(&out), 1, "served: the view's code: {}", stderr(&out));
    let out = run(
        &s,
        &[
            "picture",
            "compare",
            "a.png",
            "b.png",
            "--gui",
            "--no-launch",
        ],
    );
    assert_eq!(code(&out), 100, "{}", stderr(&out));
    assert!(stderr(&out).contains("not supported"), "{}", stderr(&out));
}

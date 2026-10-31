//! Archives through the binary (§1.5.13, A5.13): every W5a row of the command table, text and `--json`, exit codes,
//! `--archives`, `--open-archive`, settings. Fixtures are generated here by small writers: a stored zip, a tar and a
//! stored gzip stream need no crate, and nothing binary is committed.

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};

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

/// One zip entry: `(name, data, flags, method, unix mode)`.
struct ZipEntry<'a>(&'a str, &'a [u8], u16, u16, u32);

/// A zip of stored entries with a 0x5455 time of [`common::MTIME`].
fn zip_of(entries: &[ZipEntry<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut extra = vec![0x55, 0x54, 5, 0, 1];
    extra.extend_from_slice(&u32::try_from(common::MTIME).unwrap().to_le_bytes());
    for ZipEntry(name, data, flags, method, mode) in entries {
        let offset = u32::try_from(out.len()).unwrap();
        let common_part = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&0x6000u16.to_le_bytes());
            out.extend_from_slice(&0x5cefu16.to_le_bytes());
            out.extend_from_slice(&crc32(data).to_le_bytes());
            let len = u32::try_from(data.len()).unwrap();
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&u16::try_from(name.len()).unwrap().to_le_bytes());
            out.extend_from_slice(&u16::try_from(extra.len()).unwrap().to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04");
        common_part(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&extra);
        out.extend_from_slice(data);

        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&(0x0300u16 | 20).to_le_bytes());
        common_part(&mut central);
        central.extend_from_slice(&[0u8; 6]);
        central.extend_from_slice(&(mode << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&extra);
    }
    let start = u32::try_from(out.len()).unwrap();
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    let count = u16::try_from(entries.len()).unwrap();
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&u32::try_from(central.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0u8; 2]);
    out
}

/// A tar of `(name, type flag, link target, data)`.
fn tar_of(records: &[(&str, u8, &str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, flag, link, data) in records {
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[100..108].copy_from_slice(b"0000644\0");
        block[108..116].copy_from_slice(b"0000000\0");
        block[116..124].copy_from_slice(b"0000000\0");
        block[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        block[136..148].copy_from_slice(format!("{:011o}\0", common::MTIME).as_bytes());
        block[156] = *flag;
        block[157..157 + link.len()].copy_from_slice(link.as_bytes());
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        block[148..156].fill(b' ');
        let sum: u32 = block.iter().map(|&b| u32::from(b)).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&block);
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(512) {
            out.push(0);
        }
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// A gzip stream with stored blocks.
fn gz_of(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0];
    out.extend_from_slice(&u32::try_from(common::MTIME).unwrap().to_le_bytes());
    out.extend_from_slice(&[0, 0xff]);
    let mut chunks = data.chunks(0xffff).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = u16::try_from(chunk.len()).unwrap();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

const MODE: u32 = 0o100_644;
const LINK: u32 = 0o120_777;

/// `a.zip`: `x`, `inner/y`, a link and hostile and encrypted members; `b.tar.gz` and `plain/` hold the same files.
fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.file(
        "a.zip",
        zip_of(&[
            ZipEntry("x", b"alpha\n", 0, 0, MODE),
            ZipEntry("inner/y", b"beta\n", 0, 0, MODE),
        ]),
    );
    let tar = tar_of(&[
        ("x", b'0', "", b"alpha\n"),
        ("inner/y", b'0', "", b"beta\n"),
    ]);
    s.file("b.tar.gz", gz_of(&tar));
    s.file(
        "c.tar",
        tar_of(&[
            ("x", b'0', "", b"alpha\n"),
            ("inner/y", b'0', "", b"gamma\n"),
        ]),
    );
    s.file_at("plain/x", "alpha\n", common::MTIME);
    s.file_at("plain/inner/y", "beta\n", common::MTIME);
    s.file("other/x", "alpha\n");
    s.file("note.txt.gz", gz_of(b"one stream\n"));
    s.file(
        "odd.zip",
        zip_of(&[
            ZipEntry("ok", b"fine\n", 0, 0, MODE),
            ZipEntry("../evil", b"no\n", 0, 0, MODE),
            ZipEntry("lnk", b"/etc/passwd", 0, 0, LINK),
            ZipEntry("crypt", b"secret", 1, 0, MODE),
        ]),
    );
    s.file(
        "odd.tar",
        tar_of(&[("dev", b'3', "", b""), ("lnk", b'2', "/etc/passwd", b"")]),
    );
    s.file("fake.zip", "this is text\n");
    s.settle("plain");
    s
}

fn run(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd().args(args).output().unwrap()
}

#[test]
fn vfs_ls_lists_members_in_every_spelling() {
    let s = sandbox();
    let root = run(&s, &["vfs", "ls", "a.zip"]);
    assert_eq!(code(&root), 0, "{}", stderr(&root));
    assert_golden("archive/ls.txt", &s.normalise(&stdout(&root)));
    let slash = run(&s, &["vfs", "ls", "a.zip/"]);
    assert_eq!(stdout(&slash), stdout(&root));
    let inner = run(&s, &["vfs", "ls", "a.zip/inner"]);
    assert_golden("archive/ls_inner.txt", &s.normalise(&stdout(&inner)));
    let doc = run(&s, &["vfs", "ls", "a.zip", "--json"]);
    assert_eq!(json(&doc)["schema"], "hikasa.vfs_list.v1");
    assert_golden("archive/ls.json", &s.normalise(&stdout(&doc)));
}

#[test]
fn refused_rows_are_entry_errors() {
    let s = sandbox();
    let out = run(&s, &["vfs", "ls", "odd.zip"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let errors = stderr(&out);
    assert!(errors.contains("<refused-1>"), "{errors}");
    assert!(errors.contains("traversal"), "{errors}");
    let doc = run(&s, &["vfs", "ls", "odd.zip", "--json"]);
    let doc = json(&doc);
    assert_eq!(doc["result"]["errors"][0]["name"], "<refused-1>");
    let codes: Vec<&str> = doc["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        ["archive_member_refused", "archive_encrypted"],
        "{doc}"
    );
    assert_golden("archive/ls_refused.json", &s.normalise(&doc.to_string()));
}

#[test]
fn vfs_stat_of_the_file_the_root_and_a_member() {
    let s = sandbox();
    let file = json(&run(&s, &["vfs", "stat", "a.zip", "--json"]));
    assert_eq!(file["result"]["entry"]["kind"], "file");
    let root = run(&s, &["vfs", "stat", "a.zip/", "--json"]);
    let doc = json(&root);
    assert_eq!(doc["result"]["entry"]["kind"], "dir");
    let caps = doc["result"]["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "crc_provided"), "{caps:?}");
    assert!(!caps.iter().any(|c| c == "write"), "{caps:?}");
    let member = run(&s, &["vfs", "stat", "a.zip/x", "--json"]);
    let doc = json(&member);
    assert_eq!(
        doc["result"]["entry"]["crc32"],
        format!("{:08x}", crc32(b"alpha\n"))
    );
    assert_golden("archive/stat_member.json", &s.normalise(&stdout(&member)));
    let text = run(&s, &["vfs", "stat", "a.zip/x"]);
    assert_golden("archive/stat_member.txt", &s.normalise(&stdout(&text)));
}

#[test]
fn vfs_cat_of_members() {
    let s = sandbox();
    for (spec, want) in [
        ("a.zip/x", "alpha\n"),
        ("a.zip/inner/y", "beta\n"),
        ("b.tar.gz/inner/y", "beta\n"),
        ("note.txt.gz/note.txt", "one stream\n"),
    ] {
        let out = run(&s, &["vfs", "cat", spec]);
        assert_eq!(code(&out), 0, "{spec}: {}", stderr(&out));
        assert_eq!(stdout(&out), want, "{spec}");
    }
    for (spec, kind) in [
        ("odd.zip/lnk", "unsupported"),
        ("odd.tar/dev", "unsupported"),
        ("odd.tar/lnk", "unsupported"),
        ("odd.zip/crypt", "unsupported"),
        ("a.zip/missing", "not_found"),
        ("a.zip/INNER/y", "not_found"),
        ("a.zip/x/z", "not_a_directory"),
        ("fake.zip/x", "invalid_data"),
    ] {
        let out = run(&s, &["vfs", "cat", spec, "--json"]);
        assert_eq!(code(&out), 100, "{spec}");
        let doc = json(&out);
        assert_eq!(doc["error"]["kind"], kind, "{spec}: {doc}");
    }
}

#[test]
fn nested_specs() {
    let s = Sandbox::new();
    let tar = tar_of(&[("c.txt", b'0', "", b"deep\n")]);
    s.file(
        "n.zip",
        zip_of(&[ZipEntry("d/b.tar.gz", &gz_of(&tar), 0, 0, MODE)]),
    );
    let out = run(&s, &["vfs", "cat", "n.zip/d/b.tar.gz/c.txt"]);
    assert_eq!(stdout(&out), "deep\n", "{}", stderr(&out));
    let list = run(&s, &["vfs", "ls", "n.zip/d/b.tar.gz"]);
    assert!(stdout(&list).contains("c.txt"), "{}", stdout(&list));
    let file = json(&run(&s, &["vfs", "stat", "n.zip/d/b.tar.gz", "--json"]));
    assert_eq!(file["result"]["entry"]["kind"], "file");
}

#[test]
fn a_mask_match_with_text() {
    let s = sandbox();
    let out = run(&s, &["vfs", "ls", "fake.zip", "--json"]);
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "invalid_data");
    // The file stays a normal file.
    let cat = run(&s, &["vfs", "cat", "fake.zip"]);
    assert_eq!(stdout(&cat), "this is text\n");
    let quick = run(&s, &["quick", "fake.zip", "fake.zip"]);
    assert_eq!(code(&quick), 1, "{}", stderr(&quick));
    s.file("t1/fake.zip", "this is text\n");
    s.file("t2/fake.zip", "this is text\n");
    let scan = run(
        &s,
        &[
            "folder",
            "compare",
            "t1",
            "t2",
            "--archives",
            "folders",
            "--json",
        ],
    );
    assert_eq!(code(&scan), 100, "{}", stderr(&scan));
    let stats = json(&scan);
    assert!(stats.to_string().contains("not a valid archive"), "{stats}");
}

#[test]
fn folder_compare_of_archive_sides() {
    let s = sandbox();
    // An archive against a folder, a member folder, two archives of the same tree.
    let same = run(&s, &["folder", "compare", "a.zip", "plain"]);
    assert_eq!(code(&same), 2, "{}{}", stdout(&same), stderr(&same));
    let inner = run(&s, &["folder", "compare", "a.zip/inner", "plain/inner"]);
    assert_eq!(code(&inner), 2, "{}", stderr(&inner));
    let both = run(
        &s,
        &["folder", "compare", "a.zip", "b.tar.gz", "--content", "crc"],
    );
    assert_eq!(code(&both), 2, "{}", stderr(&both));
    let doc = run(
        &s,
        &[
            "folder",
            "compare",
            "a.zip",
            "b.tar.gz",
            "--content",
            "crc",
            "--json",
        ],
    );
    assert_golden("archive/folder_compare.json", &s.normalise(&stdout(&doc)));
    let differ = run(
        &s,
        &["folder", "compare", "a.zip", "c.tar", "--content", "binary"],
    );
    assert_eq!(code(&differ), 13, "{}", stderr(&differ));
    let human = s.normalise(&stdout(&differ));
    assert_golden("archive/folder_compare.txt", &human);
    // A base that cannot be loaded.
    let bad = run(&s, &["folder", "compare", "fake.zip", "plain"]);
    assert_eq!(code(&bad), 107, "{}", stderr(&bad));
}

#[test]
fn archives_inside_trees() {
    let s = sandbox();
    let zip = std::fs::read(s.join("a.zip")).unwrap();
    s.file_at("t1/a.zip", &zip, common::MTIME);
    s.file_at("t2/a.zip", &zip, common::MTIME);
    let t2 = zip_of(&[ZipEntry("x", b"other\n", 0, 0, MODE)]);
    s.file_at("t3/a.zip", &t2, common::MTIME);

    // The paths of the list report, without its mark column and summary.
    let nodes = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["folder", "compare", "--display", "all"];
        all.extend_from_slice(args);
        let out = run(&s, &all);
        let mut names: Vec<String> = stdout(&out)
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.get(3..).map(str::to_owned))
            .collect();
        names.sort();
        names
    };
    // The default leaves an archive a file; `--archives folders` or `--open-archive` opens it.
    assert_eq!(nodes(&["t1", "t2"]), ["a.zip"]);
    assert_eq!(nodes(&["t1", "t2", "--archives", "files"]), ["a.zip"]);
    let opened = ["a.zip/", "a.zip/inner/", "a.zip/inner/y", "a.zip/x"];
    assert_eq!(nodes(&["t1", "t2", "--archives", "folders"]), opened);
    assert_eq!(nodes(&["t1", "t2", "--open-archive", "a.zip"]), opened);

    let differ = run(
        &s,
        &["folder", "compare", "t1", "t3", "--content", "binary"],
    );
    assert_eq!(code(&differ), 13);
    let opened = run(
        &s,
        &[
            "folder",
            "compare",
            "t1",
            "t3",
            "--archives",
            "folders",
            "--content",
            "binary",
        ],
    );
    assert_eq!(code(&opened), 13);

    // `--open-archive` with `files`, an unknown word.
    let ignored = run(
        &s,
        &[
            "folder",
            "compare",
            "t1",
            "t2",
            "--archives",
            "files",
            "--open-archive",
            "a.zip",
            "--json",
        ],
    );
    assert_eq!(code(&ignored), 2, "{}", stderr(&ignored));
    assert_eq!(json(&ignored)["warnings"][0]["code"], "switch_ignored");
    let word = run(
        &s,
        &["folder", "compare", "t1", "t2", "--archives", "maybe"],
    );
    assert_eq!(code(&word), 100);
    assert!(
        stderr(&word).contains("files, opened, folders"),
        "{}",
        stderr(&word)
    );
}

#[test]
fn quick_and_text_compare_members() {
    let s = sandbox();
    let same = run(&s, &["quick", "a.zip/x", "plain/x"]);
    assert_eq!(code(&same), 1, "{}", stderr(&same));
    let differ = run(&s, &["quick", "a.zip/inner/y", "c.tar/inner/y"]);
    assert_eq!(code(&differ), 13, "{}", stderr(&differ));
    let text = run(
        &s,
        &["text", "compare", "a.zip/inner/y", "b.tar.gz/inner/y"],
    );
    assert_eq!(code(&text), 1, "{}", stderr(&text));
    let bad = run(&s, &["quick", "odd.zip/crypt", "plain/x"]);
    assert_eq!(code(&bad), 100);

    // `quick a.zip b.zip` compares the two files, by design.
    s.file("copy.zip", std::fs::read(s.join("a.zip")).unwrap());
    let files = run(&s, &["quick", "a.zip", "copy.zip"]);
    assert_eq!(code(&files), 1, "{}", stderr(&files));
    s.file(
        "other.zip",
        zip_of(&[ZipEntry("x", b"alpha\n", 0, 0, 0o100_600)]),
    );
    let other = run(&s, &["quick", "a.zip", "other.zip"]);
    assert_eq!(code(&other), 11, "{}", stderr(&other));
}

#[test]
fn copies_out_of_an_archive() {
    let s = sandbox();
    s.dir("out");
    let one = run(&s, &["vfs", "cp", "a.zip/x", "out/"]);
    assert_eq!(code(&one), 0, "{}", stderr(&one));
    assert_eq!(std::fs::read(s.join("out/x")).unwrap(), b"alpha\n");

    let tree = run(&s, &["vfs", "cp", "-r", "a.zip/", "out/"]);
    assert_eq!(code(&tree), 0, "{}", stderr(&tree));
    assert_eq!(
        std::fs::read(s.join("out/a.zip/inner/y")).unwrap(),
        b"beta\n"
    );

    // A hostile archive: link members skipped with one warning, refused rows fail, the tempdir stays clean.
    let odd = run(&s, &["vfs", "cp", "-r", "odd.zip/", "out/", "--json"]);
    assert_eq!(code(&odd), 100, "{}", stderr(&odd));
    let doc = json(&odd);
    let warnings = doc["warnings"].to_string();
    assert!(warnings.contains("archive_symlink_skipped"), "{warnings}");
    assert!(std::fs::read(s.join("out/odd.zip/ok")).is_ok());
    assert!(std::fs::symlink_metadata(s.join("out/odd.zip/lnk")).is_err());
    assert!(!s.join("evil").exists() && !s.join("out/evil").exists());
    // On Windows `<refused-1>` is no file name, so the plan's row reads differently (rule 17).
    if cfg!(unix) {
        assert_golden("archive/cp_refused.json", &s.normalise(&stdout(&odd)));
    }

    // Writes into a member are W5b: an item error, nothing changes.
    let before = std::fs::read(s.join("a.zip")).unwrap();
    let write = run(&s, &["vfs", "cp", "plain/x", "a.zip/new"]);
    assert_eq!(code(&write), 100, "{}", stderr(&write));
    assert_eq!(std::fs::read(s.join("a.zip")).unwrap(), before);
    let root = run(&s, &["vfs", "rm", "--yes", "a.zip/"]);
    assert_eq!(code(&root), 100, "{}", stdout(&root));
}

#[test]
fn no_cache_folder_is_left_behind() {
    let s = sandbox();
    let big = vec![7u8; (1 << 20) + 1];
    s.file("big.zip", zip_of(&[ZipEntry("m", &big, 0, 0, MODE)]));
    let out = run(&s, &["vfs", "cat", "big.zip/m"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out).len(),
        big.len(),
        "the member went through a file"
    );
    let cache = s.config().join("cache");
    assert!(!cache.exists(), "a spool leaves no folder, empty or not");
    let list = run(&s, &["vfs", "ls", "big.zip"]);
    assert_eq!(code(&list), 0);
    assert!(!cache.exists(), "a listing never creates one");
}

/// A5.5 with real processes: sessions that start and stop on one config folder each get their own bytes.
#[test]
fn concurrent_processes_read_their_own_members() {
    const WORKERS: u8 = 8;
    const ROUNDS: usize = 12;
    let s = sandbox();
    let members: Vec<Vec<u8>> = (0..WORKERS)
        .map(|worker| vec![b'A' + worker; (1 << 20) + 1])
        .collect();
    for (worker, data) in members.iter().enumerate() {
        s.file(
            &format!("a{worker}.zip"),
            zip_of(&[ZipEntry("big.bin", data, 0, 0, MODE)]),
        );
    }
    std::thread::scope(|scope| {
        for (worker, data) in members.iter().enumerate() {
            let s = &s;
            scope.spawn(move || {
                let member = format!("a{worker}.zip/big.bin");
                for round in 0..ROUNDS {
                    let out = run(s, &["vfs", "cat", &member]);
                    assert_eq!(code(&out), 0, "{}", stderr(&out));
                    assert!(out.stdout == *data, "worker {worker}, round {round}");
                }
            });
        }
    });
    assert!(!s.config().join("cache").join("archives").exists());
}

#[test]
fn settings_of_the_limits() {
    let s = sandbox();
    let get = run(&s, &["settings", "get", "archives.limits.entries_max"]);
    assert_eq!(stdout(&get).trim(), "1000000", "{}", stderr(&get));
    let set = run(&s, &["settings", "set", "archives.limits.nesting_max", "2"]);
    assert_eq!(code(&set), 0, "{}", stderr(&set));
    let get = run(&s, &["settings", "get", "archives.limits.nesting_max"]);
    assert_eq!(stdout(&get).trim(), "2");
    for (key, value) in [
        ("archives.limits.nesting_max", "33"),
        ("archives.limits.entries_max", "999"),
        ("archives.limits.stream_max_gib", "0"),
        ("archives.limits.cache_max_gib", "2000"),
        ("archives.limits.ratio_max", "-1"),
        ("archives.limits.ratio_floor", "1"),
    ] {
        let bad = run(&s, &["settings", "set", key, value]);
        assert_eq!(code(&bad), 100, "{key}: {}", stdout(&bad));
    }
    // A new mask applies to the next command, and limits reach the back-end.
    let set = run(&s, &["settings", "set", "archives.types.zip", "*.mine"]);
    assert_eq!(code(&set), 0, "{}", stderr(&set));
    s.file("z.mine", std::fs::read(s.join("a.zip")).unwrap());
    let out = run(&s, &["vfs", "cat", "z.mine/x"]);
    assert_eq!(stdout(&out), "alpha\n", "{}", stderr(&out));
    let tiny = zip_of(&[ZipEntry(
        "z.mine",
        &std::fs::read(s.join("a.zip")).unwrap(),
        0,
        0,
        MODE,
    )]);
    let mut doubled = tiny;
    for _ in 0..2 {
        doubled = zip_of(&[ZipEntry("z.mine", &doubled, 0, 0, MODE)]);
    }
    s.file("n.mine", doubled);
    let out = run(
        &s,
        &["vfs", "cat", "n.mine/z.mine/z.mine/z.mine/x", "--json"],
    );
    assert_eq!(code(&out), 100, "{}", stdout(&out));
    assert_eq!(json(&out)["error"]["kind"], "too_large");
}

#[test]
fn a_broken_archives_toml_is_one_warning() {
    let s = sandbox();
    std::fs::create_dir_all(s.config()).unwrap();
    std::fs::write(s.config().join("archives.toml"), "this is = = not toml").unwrap();
    let out = run(&s, &["quick", "plain/x", "other/x", "--json"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let doc = json(&out);
    let warnings = doc["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{doc}");
    assert_eq!(warnings[0]["code"], "invalid_config_value");
    // The defaults stand in.
    let cat = run(&s, &["vfs", "cat", "a.zip/x"]);
    assert_eq!(stdout(&cat), "alpha\n");
}

/// §1.5.13: the rule keys of `session new|edit`.
#[test]
fn session_rule_keys() {
    let s = sandbox();
    let new = run(&s, &["session", "new", "arc", "--type", "folder-compare"]);
    assert_eq!(code(&new), 0, "{}", stderr(&new));
    let edit = run(
        &s,
        &[
            "session",
            "edit",
            "arc",
            "--set",
            "rules.handling.archives=always",
            "--set",
            "rules.open_archives=[\"a.zip\"]",
        ],
    );
    assert_eq!(code(&edit), 0, "{}", stderr(&edit));
    let show = run(&s, &["session", "show", "arc", "--json"]);
    let doc = json(&show);
    let text = doc.to_string();
    assert!(text.contains("\"archives\":\"always\""), "{text}");
    assert!(text.contains("\"open_archives\":[\"a.zip\"]"), "{text}");
    let bad = run(
        &s,
        &[
            "session",
            "edit",
            "arc",
            "--set",
            "rules.handling.archives=maybe",
        ],
    );
    assert_eq!(code(&bad), 100, "{}", stdout(&bad));
}

#[test]
fn help_has_a_topic() {
    let s = Sandbox::new();
    let out = run(&s, &["help", "archives"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    for word in ["zip", "tar.zst", "--archives", "limits", "cache"] {
        assert!(text.contains(word), "{word}");
    }
    let index = stdout(&run(&s, &["help"]));
    assert!(index.contains("Archives"), "{index}");
}

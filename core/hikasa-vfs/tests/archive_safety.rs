//! A5.6–A5.9: names, kinds, limits and encrypted members (§1.5.7). Hostile members are raw bytes: tar headers written
//! by hand, zip records written field by field.

mod common;

use std::io::Read;

use common::archives::{
    OnMemory, RawEntry, RawTar, compress, on_disk, on_memory, raw_tar, raw_zip, tar_bytes,
    zip_bytes,
};
use hikasa_core::{CancelToken, Classify, Ctl, ErrorKind, Spec, WarningCode};
use hikasa_vfs::ops::{CopyOptions, ItemStatus, execute, plan_copy};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, EntryKind, FollowLinks, ListOptions, Listing, LocalVfs, Vfs,
    VfsError, VfsRouter,
};

fn never() -> CancelToken {
    CancelToken::never()
}

fn listing(vfs: &dyn Vfs, dir: &Spec) -> Listing {
    let mut listing = vfs.list(dir, &ListOptions::default(), &never()).unwrap();
    listing.sort_by_name();
    listing
}

fn names(listing: &Listing) -> Vec<&str> {
    listing.entries.iter().map(|e| e.name.as_str()).collect()
}

/// Every path of the tree below `dir`.
fn paths(vfs: &dyn Vfs, dir: &Spec, prefix: &str, out: &mut Vec<String>) {
    for entry in listing(vfs, dir).entries {
        let path = format!("{prefix}{}", entry.name);
        out.push(path.clone());
        if entry.kind == EntryKind::Dir {
            let child = vfs.child(dir, &entry.name).unwrap();
            paths(vfs, &child, &format!("{path}/"), out);
        }
    }
}

fn tree_of(vfs: &dyn Vfs, root: &Spec) -> Vec<String> {
    let mut out = Vec::new();
    paths(vfs, root, "", &mut out);
    out.sort();
    out
}

/// An archive of one body per name, in both formats.
fn both(names: &[&[u8]]) -> [(&'static str, Vec<u8>); 2] {
    let zip: Vec<RawEntry> = names.iter().map(|n| RawEntry::new(n, b"body")).collect();
    let tar: Vec<RawTar> = names.iter().map(|n| RawTar::file(n, b"body")).collect();
    [("a.zip", raw_zip(&zip)), ("a.tar", raw_tar(&tar))]
}

/// A5.6: rules 1–4 as tables, for zip and tar each. `Some` is the member it becomes; `None` a refusal.
#[test]
fn name_rows() {
    let long = vec![b'x'; 4097];
    type Row<'a> = (&'a [u8], Option<&'a str>, Option<&'a str>, &'a str);
    let rows: &[Row<'_>] = &[
        (b"../x", None, None, "traversal"),
        (b"a/../../x", None, None, "traversal"),
        (b"/etc/passwd", None, None, "absolute"),
        (br"\x", None, None, "absolute"),
        (br"\\srv\x", None, None, "absolute"),
        (br"C:\x", None, None, "drive"),
        (b"C:/x", None, None, "drive"),
        (b"c:x", None, None, "drive"),
        (br"..\..\x", None, None, "traversal"),
        (br"a\b", Some("a/b"), None, "backslash"),
        (b"a\0b", None, None, "nul"),
        (&long, None, None, "too_long"),
        (b"./a//b", Some("a/b"), Some("a/b"), ""),
        (b"good.txt", Some("good.txt"), Some("good.txt"), ""),
    ];
    for (raw, zip_path, tar_path, reason) in rows {
        // A tar header holds 100 bytes of name: longer ones and NUL arrive through a PAX `path`.
        let mut tar = common::archives::pax(&[("path", raw)]);
        tar.extend(raw_tar(&[RawTar::file(b"x", b"body")]));
        let cases = [
            ("a.zip", raw_zip(&[RawEntry::new(raw, b"body")]), *zip_path),
            ("a.tar", tar, *tar_path),
        ];
        for (name, bytes, want) in cases {
            let memory = on_memory(ArchiveLimits::default());
            let root = memory.root(name, bytes);
            let vfs = &*memory.vfs;
            let list = listing(vfs, &root);
            let shown = String::from_utf8_lossy(&raw[..raw.len().min(24)]).into_owned();
            match want {
                Some(path) => {
                    assert_eq!(tree_of(vfs, &root).last().unwrap(), path, "{name} {shown}");
                    assert!(list.errors.is_empty(), "{name} {shown}: {:?}", list.errors);
                    assert!(vfs.take_warnings().is_empty());
                }
                None => {
                    assert!(
                        list.entries.is_empty(),
                        "{name} {shown}: {:?}",
                        list.entries
                    );
                    assert_eq!(list.errors.len(), 1, "{name} {shown}");
                    let row = &list.errors[0];
                    assert_eq!(row.name.as_str(), "<refused-1>");
                    assert_eq!(row.error.kind, ErrorKind::InvalidData);
                    assert!(
                        row.error.message.starts_with("refused member \""),
                        "{}",
                        row.error.message
                    );
                    assert!(
                        row.error.message.ends_with(&format!(
                            ": {}",
                            if name == "a.tar" && raw.contains(&b'\\') {
                                "backslash"
                            } else {
                                reason
                            }
                        )),
                        "{name} {shown}: {}",
                        row.error.message
                    );
                    let warnings = vfs.take_warnings();
                    assert_eq!(warnings.len(), 1);
                    assert_eq!(warnings[0].code, WarningCode::ArchiveMemberRefused);
                }
            }
        }
    }
}

/// `.` and `./` give the root metadata, and nothing else.
#[test]
fn a_name_that_reduces_to_nothing_is_the_root() {
    let mut tar = RawTar::typed(b"./", b'5');
    tar.mtime = 5;
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.tar", raw_tar(&[tar, RawTar::file(b"./f", b"x")]));
    assert_eq!(names(&listing(&*memory.vfs, &root)), ["f"]);
    let entry = memory.vfs.stat(&root, FollowLinks::No).unwrap();
    assert!(entry.attrs.unix().is_some(), "the `./` entry's attributes");
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root(
        "a.zip",
        raw_zip(&[RawEntry::new(b".", b""), RawEntry::new(b"f", b"x")]),
    );
    let list = listing(&*memory.vfs, &root);
    assert_eq!(names(&list), ["f"]);
    assert!(list.errors.is_empty());
}

#[test]
fn invalid_utf8_and_cp437() {
    // UTF-8 flag and invalid bytes: lossy. No flag: as the crate decodes it (CP437; 0x81 is ü).
    let mut flagged = RawEntry::new(b"a\xff", b"1");
    flagged.flags = 1 << 11;
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.zip", raw_zip(&[flagged, RawEntry::new(b"b\x81", b"2")]));
    assert_eq!(names(&listing(&*memory.vfs, &root)), ["a\u{FFFD}", "bü"]);
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.tar", raw_tar(&[RawTar::file(b"c\xff", b"3")]));
    assert_eq!(names(&listing(&*memory.vfs, &root)), ["c\u{FFFD}"]);
    let member = memory
        .vfs
        .child(&root, &listing(&*memory.vfs, &root).entries[0].name)
        .unwrap();
    let mut body = String::new();
    memory
        .vfs
        .open_read(&member)
        .unwrap()
        .read_to_string(&mut body)
        .unwrap();
    assert_eq!(body, "3");
}

#[test]
fn duplicates_in_both_orders() {
    for (name, bytes) in [
        (
            "a.tar",
            raw_tar(&[RawTar::file(b"d", b"first"), RawTar::file(b"d", b"last")]),
        ),
        (
            "a.zip",
            raw_zip(&[RawEntry::new(b"d", b"first"), RawEntry::new(b"d", b"last")]),
        ),
        (
            "b.zip",
            raw_zip(&[
                RawEntry::new(b"./d", b"first"),
                RawEntry::new(b"d", b"last"),
            ]),
        ),
    ] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes);
        let root = Spec::archive_member(file.clone(), "");
        assert_eq!(names(&listing(&*memory.vfs, &root)), ["d"], "{name}");
        let mut body = String::new();
        let member = Spec::archive_member(file, "d");
        memory
            .vfs
            .open_read(&member)
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "last", "{name}");
        let warnings = memory.vfs.take_warnings();
        assert_eq!(warnings.len(), 1, "{name}");
        assert_eq!(warnings[0].code, WarningCode::ArchiveDuplicateMember);
    }
}

#[test]
fn a_file_and_its_parent_in_both_orders() {
    for order in [&[&b"a"[..], b"a/b"][..], &[b"a/b", b"a"]] {
        for (name, bytes) in both(order) {
            let memory = on_memory(ArchiveLimits::default());
            let root = memory.root(name, bytes);
            assert_eq!(tree_of(&*memory.vfs, &root), ["a", "a/b"], "{name}");
            let list = listing(&*memory.vfs, &root);
            assert_eq!(list.entries[0].kind, EntryKind::Dir);
            assert_eq!(list.errors.len(), 1, "{name}");
            assert!(list.errors[0].error.message.ends_with(": conflict"));
        }
    }
}

#[test]
fn case_twins_are_two_members() {
    for (name, bytes) in both(&[b"A.txt", b"a.txt"]) {
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name, bytes);
        assert_eq!(tree_of(&*memory.vfs, &root), ["A.txt", "a.txt"]);
    }
}

#[test]
fn the_rows_stop_at_1000_and_the_warning_has_the_total() {
    let names: Vec<String> = (0..1500).map(|i| format!("../{i}")).collect();
    let raw: Vec<&[u8]> = names.iter().map(|n| n.as_bytes()).collect();
    for (name, bytes) in both(&raw) {
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name, bytes);
        let list = listing(&*memory.vfs, &root);
        assert_eq!(list.errors.len(), 1000, "{name}");
        assert!(
            list.errors
                .iter()
                .any(|e| e.name.as_str() == "<refused-1000>")
        );
        // In archive order: row 1 is the first name.
        let first = list
            .errors
            .iter()
            .find(|e| e.name.as_str() == "<refused-1>")
            .unwrap();
        assert!(
            first.error.message.contains("\"../0\""),
            "{}",
            first.error.message
        );
        let warnings = memory.vfs.take_warnings();
        assert!(
            warnings[0].message.contains("1500"),
            "{}",
            warnings[0].message
        );
    }
}

/// A5.6: each hostile archive extracted: the tempdir holds only what lies below `out/`, and the cache numeric names.
#[test]
fn extraction_stays_below_the_target() {
    let hostile: &[&[u8]] = &[
        b"../up",
        b"a/../../up",
        b"/abs",
        br"\back",
        br"C:\drive",
        b"C:/drive",
        br"..\..\up",
        br"win\sep",
        b"good/ok.txt",
    ];
    let mut big = RawEntry::new(b"big", &vec![7u8; (1 << 20) + 5]);
    big.flags = 0;
    let zip_entries: Vec<RawEntry> = hostile
        .iter()
        .map(|n| RawEntry::new(n, b"payload"))
        .chain([big])
        .collect();
    let tar_records: Vec<RawTar> = hostile
        .iter()
        .map(|n| RawTar::file(n, b"payload"))
        .chain([
            RawTar::link(b"lnk", b'2', b"/etc/passwd"),
            RawTar::link(b"lnk/below", b'0', b""),
        ])
        .collect();
    for (name, bytes) in [
        ("h.zip", raw_zip(&zip_entries)),
        ("h.tar", raw_tar(&tar_records)),
    ] {
        let disk = on_disk(ArchiveLimits::default());
        let file = disk.put(name, &bytes);
        let out = disk.dir.join("out");
        std::fs::create_dir(&out).unwrap();
        let router = VfsRouter::new(LocalVfs::default()).with_archives(disk.vfs.clone());
        let pair = (
            Spec::archive_member(file, ""),
            common::archives::local(&out),
        );
        let options = CopyOptions {
            recursive: true,
            ..CopyOptions::default()
        };
        let plan = plan_copy(&router, &router, &[pair], &options, &Ctl::detached()).unwrap();
        let report = execute(&router, &router, plan, &Ctl::detached());
        assert!(report.results.iter().any(|i| i.status == ItemStatus::Done));

        // Only `out/` and the container in the work folder.
        let mut top: Vec<String> = std::fs::read_dir(&disk.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        top.sort();
        assert_eq!(top, [name.to_owned(), "out".to_owned()]);
        let parent = disk.dir.parent().unwrap();
        for item in std::fs::read_dir(parent).unwrap() {
            let item = item.unwrap().file_name().into_string().unwrap();
            assert!(item == "work" || item == "cache", "{item}");
        }
        let extracted = &out;
        assert!(extracted.join("good/ok.txt").is_file(), "{name}");
        assert!(!std::fs::symlink_metadata(extracted.join("lnk")).is_ok_and(|m| m.is_symlink()));
        if let Some(dir) = disk.vfs.cache_dir() {
            for item in std::fs::read_dir(dir).unwrap() {
                let item = item.unwrap().file_name().into_string().unwrap();
                assert!(item[1..].bytes().all(|b| b.is_ascii_digit()), "{item}");
            }
        }
    }
}

// ---- kinds ---------------------------------------------------------------------------------------------------

#[test]
fn links_are_listed_and_never_followed() {
    let targets: [&[u8]; 3] = [b"/etc/passwd", b"../..", b"real"];
    for target in targets {
        let zip = raw_zip(&[
            RawEntry::new(b"lnk", target).unix(0o120_777),
            RawEntry::new(b"real", b"body"),
        ]);
        let tar = raw_tar(&[
            RawTar::link(b"lnk", b'2', target),
            RawTar::file(b"real", b"body"),
        ]);
        for (name, bytes) in [("a.zip", zip), ("a.tar", tar)] {
            let memory = on_memory(ArchiveLimits::default());
            let file = memory.put(name, bytes);
            let vfs = &*memory.vfs;
            let link = Spec::archive_member(file.clone(), "lnk");
            let entry = vfs.stat(&link, FollowLinks::Yes).unwrap();
            assert_eq!(entry.kind, EntryKind::Symlink);
            assert_eq!(
                entry.link_target.as_deref().map(str::as_bytes),
                Some(target)
            );
            assert!(matches!(
                vfs.open_read(&link),
                Err(VfsError::OperationUnsupported { .. })
            ));
            let below = Spec::archive_member(file, "lnk/x");
            assert!(matches!(
                vfs.stat(&below, FollowLinks::No),
                Err(VfsError::NotADirectory { .. })
            ));
            assert!(matches!(
                vfs.open_read(&below),
                Err(VfsError::NotADirectory { .. })
            ));
            assert!(matches!(
                vfs.list(&link, &ListOptions::default(), &never()),
                Err(VfsError::NotADirectory { .. })
            ));
        }
    }
}

#[test]
fn a_member_below_a_link_is_refused() {
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root(
        "a.tar",
        raw_tar(&[
            RawTar::link(b"lnk", b'2', b"/tmp"),
            RawTar::file(b"lnk/x", b"evil"),
        ]),
    );
    let list = listing(&*memory.vfs, &root);
    assert_eq!(names(&list), ["lnk"]);
    assert_eq!(list.errors.len(), 1);
    assert!(list.errors[0].error.message.ends_with(": conflict"));
}

/// A copy out skips links with one warning and creates no link anywhere.
#[test]
fn a_copy_out_skips_links() {
    let disk = on_disk(ArchiveLimits::default());
    let tar = raw_tar(&[
        RawTar::file(b"f", b"body"),
        RawTar::link(b"l1", b'2', b"/etc/passwd"),
        RawTar::link(b"l2", b'2', b"f"),
        RawTar::typed(b"dev", b'3'),
        RawTar::typed(b"fifo", b'6'),
    ]);
    let file = disk.put("a.tar", &tar);
    let out = disk.dir.join("out");
    let router = VfsRouter::new(LocalVfs::default()).with_archives(disk.vfs.clone());
    let pair = (
        Spec::archive_member(file, ""),
        common::archives::local(&out),
    );
    let options = CopyOptions {
        recursive: true,
        ..CopyOptions::default()
    };
    let plan = plan_copy(&router, &router, &[pair], &options, &Ctl::detached()).unwrap();
    let warnings: Vec<_> = plan
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::ArchiveSymlinkSkipped)
        .collect();
    assert_eq!(warnings.len(), 1);
    let report = execute(&router, &router, plan, &Ctl::detached());
    assert!(
        report
            .results
            .iter()
            .all(|i| i.status != ItemStatus::Failed),
        "{:?}",
        report.results
    );
    let mut made: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    made.sort();
    assert_eq!(made, ["f"]);
}

#[test]
fn hard_links() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.tar",
        raw_tar(&[
            RawTar::file(b"real", b"body"),
            RawTar::link(b"good", b'1', b"real"),
            RawTar::link(b"chain", b'1', b"good"),
            RawTar::link(b"later", b'1', b"after"),
            RawTar::file(b"after", b"x"),
            RawTar::link(b"bad", b'1', b"../real"),
            RawTar::link(b"self", b'1', b"self"),
            RawTar::typed(b"dir/", b'5'),
            RawTar::link(b"todir", b'1', b"dir"),
        ]),
    );
    let root = Spec::archive_member(file.clone(), "");
    let vfs = &*memory.vfs;
    let list = listing(vfs, &root);
    assert_eq!(names(&list), ["after", "chain", "dir", "good", "real"]);
    for name in ["good", "chain"] {
        let mut body = String::new();
        vfs.open_read(&Spec::archive_member(file.clone(), name))
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "body", "{name}");
        let entry = vfs
            .stat(&Spec::archive_member(file.clone(), name), FollowLinks::No)
            .unwrap();
        assert_eq!((entry.kind, entry.size), (EntryKind::File, 4));
        assert_eq!(entry.crc32, Some(crc32fast::hash(b"body")));
    }
    assert_eq!(list.errors.len(), 4);
    assert!(
        list.errors
            .iter()
            .all(|e| e.error.message.ends_with(": hard_link"))
    );
}

#[test]
fn devices_are_other_and_sparse_is_listed() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.tar",
        raw_tar(&[
            RawTar::typed(b"chr", b'3'),
            RawTar::typed(b"blk", b'4'),
            RawTar::typed(b"fifo", b'6'),
        ]),
    );
    for entry in listing(&*memory.vfs, &Spec::archive_member(file, "")).entries {
        assert_eq!(entry.kind, EntryKind::Other, "{}", entry.name);
    }
}

// ---- limits --------------------------------------------------------------------------------------------------

fn limited(limits: ArchiveLimits) -> OnMemory {
    on_memory(limits)
}

fn expect_limit<T: std::fmt::Debug>(result: Result<T, VfsError>, limit: &str) {
    match result {
        Err(VfsError::ArchiveLimit { limit: got, .. }) if got == limit => {}
        other => panic!("expected the {limit} limit, got {other:?}"),
    }
}

fn expect_corrupt<T: std::fmt::Debug>(result: Result<T, VfsError>, reason: &str) {
    match result {
        Err(VfsError::ArchiveCorrupt { reason: got, .. }) if got == reason => {}
        other => panic!("expected {reason:?}, got {other:?}"),
    }
}

/// Rule 10, and an in-limit neighbour still reads.
#[test]
fn declared_and_streamed_sizes_must_agree() {
    let mut small = RawEntry::new(b"small", &[9u8; 100]);
    small.size = Some(10);
    let mut large = RawEntry::new(b"large", &[9u8; 100]);
    large.size = Some(1000);
    let mut crc = RawEntry::new(b"crc", &[9u8; 100]);
    crc.crc32 = Some(1);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.zip",
        raw_zip(&[small, large, crc, RawEntry::new(b"ok", b"fine")]),
    );
    let open = |name: &str| {
        memory
            .vfs
            .open_read(&Spec::archive_member(file.clone(), name))
            .map(drop)
    };
    expect_corrupt(open("small"), "size mismatch");
    expect_corrupt(open("large"), "size mismatch");
    expect_corrupt(open("crc"), "CRC mismatch");
    open("ok").unwrap();
    assert_eq!(memory.vfs.cache_used(), 0);

    // The same over 1 MiB: the partial cache file is removed.
    let mut large = RawEntry::new(b"m", &vec![9u8; 2 << 20]);
    large.size = Some(3 << 20);
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &raw_zip(&[large]));
    expect_corrupt(
        disk.vfs
            .open_read(&Spec::archive_member(file, "m"))
            .map(drop),
        "size mismatch",
    );
    assert_eq!(disk.vfs.cache_used(), 0);
    let left = std::fs::read_dir(disk.vfs.cache_dir().unwrap())
        .unwrap()
        .count();
    assert_eq!(left, 0, "no partial cache file");
}

/// Rule 11, per stream.
#[test]
fn ratio_of_each_stream() {
    let zeros = vec![0u8; 8 << 20];
    let limits = ArchiveLimits {
        ratio_max: 10,
        ratio_floor: 1 << 20,
        ..ArchiveLimits::default()
    };
    for format in [
        ArchiveFormat::Gz,
        ArchiveFormat::Bz2,
        ArchiveFormat::Xz,
        ArchiveFormat::Zst,
    ] {
        let bytes = compress(format, &zeros);
        let memory = limited(limits);
        let root = memory.root("x.bin.gz", Vec::new());
        let _ = root;
        let name = common::archives::name_of(format);
        let root = memory.root(name, bytes.clone());
        expect_limit(
            memory
                .vfs
                .list(&root, &ListOptions::default(), &never())
                .map(drop),
            "ratio",
        );
        // `ratio_max = 0` turns it off.
        let off = limited(ArchiveLimits {
            ratio_max: 0,
            ..limits
        });
        let root = off.root(name, bytes);
        let list = listing(&*off.vfs, &root);
        assert_eq!(list.entries[0].size, 8 << 20, "{format:?}");
    }
    // Under the floor it is not looked at.
    let bytes = compress(ArchiveFormat::Zst, &vec![0u8; 512 << 10]);
    let memory = limited(limits);
    let root = memory.root("x.bin.zst", bytes);
    assert_eq!(listing(&*memory.vfs, &root).entries.len(), 1);
}

#[test]
fn ratio_of_a_tar_stream_and_of_a_whole_zip() {
    let limits = ArchiveLimits {
        ratio_max: 10,
        ratio_floor: 1 << 20,
        ..ArchiveLimits::default()
    };
    let tar = tar_bytes(&vec![("zeros".to_owned(), vec![0u8; 8 << 20])]);
    let memory = limited(limits);
    let root = memory.root("a.tar.zst", compress(ArchiveFormat::TarZst, &tar));
    expect_limit(
        memory
            .vfs
            .list(&root, &ListOptions::default(), &never())
            .map(drop),
        "ratio",
    );

    // Declared total / container length, before decoding, and the listing still works.
    let data = vec![
        ("zeros".to_owned(), vec![0u8; 8 << 20]),
        ("small".to_owned(), b"x".to_vec()),
    ];
    let memory = limited(limits);
    let file = memory.put("a.zip", zip_bytes(&data));
    assert_eq!(
        listing(&*memory.vfs, &Spec::archive_member(file.clone(), ""))
            .entries
            .len(),
        2
    );
    expect_limit(
        memory
            .vfs
            .open_read(&Spec::archive_member(file, "small"))
            .map(drop),
        "ratio",
    );
    assert_eq!(memory.vfs.counters().member_decodes, 0);
}

/// Rule 12: overlapping zip entries are stopped by the total.
#[test]
fn overlapping_zip_entries_stop_at_the_total() {
    let body = vec![3u8; 1 << 20];
    let mut entries = vec![RawEntry::new(b"m0", &body)];
    for i in 1..8 {
        let mut entry = RawEntry::new(format!("m{i}").as_bytes(), &body);
        entry.at = Some(0);
        entries.push(entry);
    }
    let limits = ArchiveLimits {
        stream_max: 3 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = limited(limits);
    let file = memory.put("a.zip", raw_zip(&entries));
    let mut failed = None;
    for i in 0..8 {
        if let Err(error) = memory
            .vfs
            .open_read(&Spec::archive_member(file.clone(), format!("m{i}")))
        {
            failed = Some((i, error));
            break;
        }
    }
    let (at, error) = failed.expect("the total stops the reads");
    assert!(at >= 2, "{at}");
    expect_limit(Err::<(), _>(error), "stream");
}

#[test]
fn stream_max_during_an_index_pass() {
    let limits = ArchiveLimits {
        stream_max: 4 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let tar = tar_bytes(&vec![("zeros".to_owned(), vec![0u8; 8 << 20])]);
    for format in [
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarZst,
    ] {
        let memory = limited(limits);
        let root = memory.root(common::archives::name_of(format), compress(format, &tar));
        expect_limit(
            memory
                .vfs
                .list(&root, &ListOptions::default(), &never())
                .map(drop),
            "stream",
        );
    }
    // In limit: the neighbour reads.
    let small = tar_bytes(&vec![("small".to_owned(), vec![0u8; 1 << 20])]);
    let memory = limited(limits);
    let root = memory.root("a.tar", small);
    assert_eq!(listing(&*memory.vfs, &root).entries.len(), 1);
}

#[test]
fn entries_max_plus_one() {
    let limits = ArchiveLimits {
        entries_max: 10,
        ..ArchiveLimits::default()
    };
    let at = |count: usize| {
        let names: Vec<String> = (0..count).map(|i| format!("m{i}")).collect();
        let raw: Vec<&[u8]> = names.iter().map(|n| n.as_bytes()).collect();
        both(&raw)
    };
    for (name, bytes) in at(11) {
        let memory = limited(limits);
        let root = memory.root(name, bytes);
        expect_limit(
            memory
                .vfs
                .list(&root, &ListOptions::default(), &never())
                .map(drop),
            "entries",
        );
    }
    for (name, bytes) in at(10) {
        let memory = limited(limits);
        let root = memory.root(name, bytes);
        assert_eq!(listing(&*memory.vfs, &root).entries.len(), 10, "{name}");
    }
}

/// Rule 14, and a zip that contains itself by name.
#[test]
fn nesting_max_plus_one() {
    let limits = ArchiveLimits {
        nesting_max: 2,
        ..ArchiveLimits::default()
    };
    let mut bytes = zip_bytes(&vec![("x".to_owned(), b"1".to_vec())]);
    for _ in 0..3 {
        bytes = zip_bytes(&vec![("a.zip".to_owned(), bytes)]);
    }
    let memory = limited(limits);
    let file = memory.put("a.zip", bytes);
    let one = Spec::archive_member(file.clone(), "a.zip");
    // Two archives around a member of `one`: at the limit, it lists and reads.
    let root = Spec::archive_member(one.clone(), "");
    assert_eq!(listing(&*memory.vfs, &root).entries.len(), 1);
    memory
        .vfs
        .open_read(&Spec::archive_member(one.clone(), "a.zip"))
        .unwrap();
    let two = Spec::archive_member(one, "a.zip");
    expect_limit(
        memory
            .vfs
            .stat(&Spec::archive_member(two.clone(), ""), FollowLinks::No),
        "nesting",
    );
    expect_limit(
        memory
            .vfs
            .open_read(&Spec::archive_member(two, "a.zip"))
            .map(drop),
        "nesting",
    );
    // A neighbour at the top still reads.
    memory
        .vfs
        .open_read(&Spec::archive_member(file, "a.zip"))
        .unwrap();
}

/// Rule 15: a stream that asks for more memory than the cap is `ArchiveCorrupt`, before it allocates.
#[test]
fn xz_and_zstd_frames_asking_for_too_much_memory() {
    // A zstd frame header: magic, a descriptor with no content size, a window of 2^31.
    let mut zst = vec![0x28, 0xb5, 0x2f, 0xfd, 0x00, 21 << 3];
    zst.extend_from_slice(&[1, 0, 0]);
    // An xz stream header and a block header with an LZMA2 dictionary of 4 GiB − 1.
    let mut xz = vec![0xfd, b'7', b'z', b'X', b'Z', 0, 0, 1];
    xz.extend_from_slice(&crc32fast::hash(&[0, 1]).to_le_bytes());
    let mut block = vec![0x02, 0x00, 0x21, 0x01, 40, 0, 0, 0];
    block.extend_from_slice(&[0u8; 4][..0]);
    let crc = crc32fast::hash(&block);
    block.extend_from_slice(&crc.to_le_bytes());
    xz.extend_from_slice(&block);
    xz.extend_from_slice(&[0u8; 64]);
    for (name, bytes) in [("big.bin.zst", zst), ("big.bin.xz", xz)] {
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name, bytes);
        let result = memory.vfs.list(&root, &ListOptions::default(), &never());
        assert!(
            matches!(result, Err(VfsError::ArchiveCorrupt { .. })),
            "{name}: {result:?}"
        );
    }
}

// ---- encrypted -----------------------------------------------------------------------------------------------

#[test]
fn encrypted_members_are_listed_and_refused() {
    let mut zipcrypto = RawEntry::new(b"crypt.txt", b"ciphertext");
    zipcrypto.flags = 1;
    let mut aes = RawEntry::new(b"aes.txt", b"ciphertext");
    aes.flags = 1;
    aes.method = 99;
    let mut noflag = RawEntry::new(b"aes2.txt", b"ciphertext");
    noflag.method = 99;
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.zip",
        raw_zip(&[
            zipcrypto,
            aes,
            noflag,
            RawEntry::new(b"plain.txt", b"plain"),
        ]),
    );
    let root = Spec::archive_member(file.clone(), "");
    let vfs = &*memory.vfs;
    let list = listing(vfs, &root);
    assert_eq!(
        names(&list),
        ["aes.txt", "aes2.txt", "crypt.txt", "plain.txt"]
    );
    for name in ["aes.txt", "aes2.txt", "crypt.txt"] {
        let spec = Spec::archive_member(file.clone(), name);
        let entry = vfs.stat(&spec, FollowLinks::No).unwrap();
        assert_eq!(entry.crc32, None, "{name}");
        assert_eq!(entry.size, 10);
        assert!(entry.mtime.is_some(), "{name}");
        let error = vfs.open_read(&spec).map(drop).unwrap_err();
        assert!(
            matches!(error, VfsError::ArchiveEncrypted { .. }),
            "{error}"
        );
        assert_eq!(error.kind(), ErrorKind::Unsupported);
    }
    let plain = Spec::archive_member(file, "plain.txt");
    assert!(vfs.stat(&plain, FollowLinks::No).unwrap().crc32.is_some());
    vfs.open_read(&plain).unwrap();
    let warnings = vfs.take_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].code, WarningCode::ArchiveEncrypted);
    assert!(vfs.take_warnings().is_empty(), "once");
}

#[test]
fn an_unknown_method_is_unsupported_at_read() {
    let mut shrink = RawEntry::new(b"old", b"data");
    shrink.method = 1;
    let mut ppmd = RawEntry::new(b"ppmd", b"data");
    ppmd.method = 98;
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", raw_zip(&[shrink, ppmd]));
    assert_eq!(
        listing(&*memory.vfs, &Spec::archive_member(file.clone(), ""))
            .entries
            .len(),
        2
    );
    for name in ["old", "ppmd"] {
        let error = memory
            .vfs
            .open_read(&Spec::archive_member(file.clone(), name))
            .map(drop)
            .unwrap_err();
        assert!(
            matches!(
                error,
                VfsError::Unsupported {
                    what: "this zip compression method",
                    ..
                }
            ),
            "{error}"
        );
    }
}

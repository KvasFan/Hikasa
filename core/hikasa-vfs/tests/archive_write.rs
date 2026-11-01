//! M2 W5b acceptance A5.14…A5.16 (D-148, D-149): batches, the overlay and the rebuild. After every rebuild the result
//! is read back through a fresh `ArchiveVfs` and through the plain `zip` and `tar` crates.

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex};

use common::archives::{
    OnDisk, OnMemory, RawEntry, RawTar, Seen, TreeData, bytes_of, config, decompress, name_of,
    on_disk, on_memory, pax, raw_tar, raw_zip, read_plain, seal, walk,
};
use common::{ctl, ft, host, read_to_vec};
use hikasa_core::{CancelToken, Ctl, ErrorKind, FileTime, Spec};
use hikasa_vfs::ops::{
    self, Action, CopyOptions, ItemStatus, MoveOptions, RemoveOptions, TouchOptions,
};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveVfs, AtomicWrite, AttrChange, Capabilities, DosAttrs,
    Entry, EntryKind, FileName, Finish, FollowLinks, ListOptions, Listing, MemMeta, MemoryVfs,
    ReadSeek, SetTimes, Vfs, VfsError, VfsOp, WriteOptions,
};

const WRITABLE: [ArchiveFormat; 6] = [
    ArchiveFormat::Zip,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarBz2,
    ArchiveFormat::TarXz,
    ArchiveFormat::TarZst,
];

fn sample() -> TreeData {
    vec![
        ("readme.txt".into(), b"hello\n".to_vec()),
        ("dir/a.txt".into(), b"a".repeat(1000)),
        ("dir/sub/b.bin".into(), (0..=255u8).collect()),
        ("empty/".into(), Vec::new()),
    ]
}

fn member(archive: &Spec, path: &str) -> Spec {
    Spec::archive_member(archive.clone(), path)
}

fn write_member(vfs: &dyn Vfs, spec: &Spec, bytes: &[u8]) -> Result<(), VfsError> {
    let options = WriteOptions {
        overwrite: true,
        ..WriteOptions::default()
    };
    let mut writer = vfs.open_write(spec, &options)?;
    writer.write_all(bytes).unwrap();
    writer.commit()
}

fn read_member(vfs: &dyn Vfs, spec: &Spec) -> Vec<u8> {
    read_to_vec(vfs.open_read(spec).unwrap())
}

fn seen_file(bytes: &[u8]) -> Seen {
    Seen::File(bytes.to_vec())
}

/// The bytes of the archive, read back by a new `ArchiveVfs`, which knows nothing of the rebuild.
fn fresh_view(mem: &Arc<MemoryVfs>, file: &Spec) -> BTreeMap<String, Seen> {
    let dir = tempfile::tempdir().unwrap();
    let vfs = ArchiveVfs::new(
        Arc::clone(mem) as Arc<dyn Vfs>,
        config(dir.path(), ArchiveLimits::default()),
    );
    walk(&vfs, &Spec::archive_member(file.clone(), ""))
}

fn stored(memory: &OnMemory, name: &str) -> Vec<u8> {
    memory
        .mem
        .read_back(&host(&format!("/{name}")))
        .unwrap()
        .to_vec()
}

/// After a rebuild every reader agrees: this back-end, a new one, and the plain crates.
fn assert_all_read(
    memory: &OnMemory,
    format: ArchiveFormat,
    file: &Spec,
    want: &BTreeMap<String, Seen>,
) {
    let name = name_of(format);
    assert_eq!(
        &walk(&*memory.vfs, &member(file, "")),
        want,
        "{format:?}: this back-end"
    );
    assert_eq!(
        &fresh_view(&memory.mem, file),
        want,
        "{format:?}: a new back-end"
    );
    assert_eq!(
        &read_plain(format, &stored(memory, name)),
        want,
        "{format:?}: the plain crate"
    );
}

fn run_copy(vfs: &dyn Vfs, from: &Spec, to: &Spec) -> ops::OpReport {
    let opts = CopyOptions {
        recursive: true,
        preserve: ops::Preserve::default(),
        ..CopyOptions::default()
    };
    let plan = ops::plan_copy(
        vfs,
        vfs,
        &[(from.clone(), to.clone())],
        &opts,
        &Ctl::detached(),
    )
    .unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

// ---- A5.16 rebuild ------------------------------------------------------------------------------------------

/// Every change, for zip and each tar format, read back by everyone.
#[test]
fn every_change_in_every_format_reads_back() {
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &sample()));
        let vfs = &*memory.vfs;
        let mut want = walk(vfs, &member(&file, ""));

        // Add.
        write_member(vfs, &member(&file, "new.txt"), b"new member").unwrap();
        want.insert("new.txt".into(), seen_file(b"new member"));
        assert_all_read(&memory, format, &file, &want);
        // Replace.
        write_member(vfs, &member(&file, "readme.txt"), b"replaced").unwrap();
        want.insert("readme.txt".into(), seen_file(b"replaced"));
        assert_all_read(&memory, format, &file, &want);
        // Remove.
        vfs.remove(&member(&file, "dir/a.txt")).unwrap();
        want.remove("dir/a.txt");
        assert_all_read(&memory, format, &file, &want);
        // Rename, a member and a folder.
        vfs.rename(
            &member(&file, "new.txt"),
            &member(&file, "dir/moved.txt"),
            false,
        )
        .unwrap();
        let moved = want.remove("new.txt").unwrap();
        want.insert("dir/moved.txt".into(), moved);
        assert_all_read(&memory, format, &file, &want);
        vfs.rename(&member(&file, "dir/sub"), &member(&file, "top"), false)
            .unwrap();
        let bin = want.remove("dir/sub/b.bin").unwrap();
        want.remove("dir/sub");
        want.insert("top".into(), Seen::Dir);
        want.insert("top/b.bin".into(), bin);
        assert_all_read(&memory, format, &file, &want);
        // A folder.
        assert!(vfs.create_dir(&member(&file, "made"), false).unwrap());
        want.insert("made".into(), Seen::Dir);
        assert_all_read(&memory, format, &file, &want);
        // A link.
        vfs.create_symlink(
            &member(&file, "made/link"),
            std::ffi::OsStr::new("../readme.txt"),
            false,
        )
        .unwrap();
        want.insert("made/link".into(), Seen::Link("../readme.txt".into()));
        assert_all_read(&memory, format, &file, &want);
        // Times and attributes change nothing a walk sees, and read back as they were set.
        let spec = member(&file, "readme.txt");
        let at = ft(1_700_000_000);
        vfs.set_times(
            &spec,
            &SetTimes {
                modified: Some(at),
                created: None,
            },
        )
        .unwrap();
        vfs.set_attrs(&spec, &AttrChange::Mode(0o600)).unwrap();
        assert_all_read(&memory, format, &file, &want);
        let entry = vfs.stat(&spec, FollowLinks::No).unwrap();
        assert_eq!(entry.mtime, Some(at), "{format:?}");
        assert_eq!(entry.attrs.unix().unwrap().mode, 0o600, "{format:?}");
    }
}

/// `stat` of a member through a new back-end, as text a test can compare.
fn fresh_entry(memory: &OnMemory, spec: &Spec) -> Entry {
    let dir = tempfile::tempdir().unwrap();
    let vfs = ArchiveVfs::new(
        Arc::clone(&memory.mem) as Arc<dyn Vfs>,
        config(dir.path(), ArchiveLimits::default()),
    );
    vfs.stat(spec, FollowLinks::No).unwrap()
}

/// Rebuilds are repeatable: the same inputs give the same bytes.
#[test]
fn two_equal_rebuilds_give_equal_bytes() {
    for format in WRITABLE {
        let rebuilt = || {
            let memory = on_memory(ArchiveLimits::default());
            let file = memory.put(name_of(format), bytes_of(format, &sample()));
            write_member(&*memory.vfs, &member(&file, "n.txt"), b"same").unwrap();
            stored(&memory, name_of(format))
        };
        assert_eq!(rebuilt(), rebuilt(), "{format:?}");
    }
}

/// No net change writes nothing: not the file, not its time.
#[test]
fn an_overlay_with_no_net_change_writes_nothing() {
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &sample()));
        let vfs = &*memory.vfs;
        let at = member(&file, "");
        vfs.begin(&file).unwrap();
        vfs.create_dir(&member(&file, "gone"), false).unwrap();
        vfs.remove(&member(&file, "gone")).unwrap();
        memory.mem.set_journal(true);
        vfs.finish(&at, Finish::Commit).unwrap();
        assert!(
            memory.mem.journal().is_empty(),
            "{:?}",
            memory.mem.journal()
        );
    }
}

/// `mkdir` of a removed folder's parent: a folder that only existed through its members stays when they go.
#[test]
fn an_implicit_folder_outlives_its_last_member() {
    let tree: TreeData = vec![("d/x".into(), b"1".to_vec()), ("y".into(), b"2".to_vec())];
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &tree));
        memory.vfs.remove(&member(&file, "d/x")).unwrap();
        let mut want = BTreeMap::new();
        want.insert("d".to_owned(), Seen::Dir);
        want.insert("y".to_owned(), seen_file(b"2"));
        assert_all_read(&memory, format, &file, &want);
    }
}

// ---- zip records --------------------------------------------------------------------------------------------

/// A zip with one stored member whose local and central extra fields, flags, comment, DOS time, external attributes
/// and "made by" this build does not produce: what it keeps is compared as bytes.
fn rich_zip() -> Vec<u8> {
    let mut plain = RawEntry::new(b"plain.txt", b"plain content");
    plain.extra = vec![0x34, 0x12, 3, 0, 7, 8, 9];
    plain.comment = b"caf\xe9".to_vec();
    let mut other = RawEntry::new(b"other.txt", b"other");
    other.flags = 1 << 3;
    let mut zip = raw_zip(&[plain, other]);
    let at = zip.len() - 22;
    let comment = b"archive comment";
    zip[at + 20] = comment.len() as u8;
    zip.extend_from_slice(comment);
    zip
}

fn local_record(zip: &[u8], name: &[u8]) -> Vec<u8> {
    let at = zip
        .windows(name.len())
        .position(|w| w == name)
        .expect("the name")
        - 30;
    assert_eq!(&zip[at..at + 4], b"PK\x03\x04");
    let name_len = u16::from_le_bytes([zip[at + 26], zip[at + 27]]) as usize;
    let extra_len = u16::from_le_bytes([zip[at + 28], zip[at + 29]]) as usize;
    let size = u32::from_le_bytes(zip[at + 18..at + 22].try_into().unwrap()) as usize;
    zip[at..at + 30 + name_len + extra_len + size].to_vec()
}

fn central_record(zip: &[u8], name: &[u8]) -> Vec<u8> {
    let mut at = zip.len() - 22;
    while &zip[at..at + 4] != b"PK\x05\x06" {
        at -= 1;
    }
    let mut at = u32::from_le_bytes(zip[at + 16..at + 20].try_into().unwrap()) as usize;
    while &zip[at..at + 4] == b"PK\x01\x02" {
        let n = u16::from_le_bytes([zip[at + 28], zip[at + 29]]) as usize;
        let e = u16::from_le_bytes([zip[at + 30], zip[at + 31]]) as usize;
        let c = u16::from_le_bytes([zip[at + 32], zip[at + 33]]) as usize;
        let end = at + 46 + n + e + c;
        if &zip[at + 46..at + 46 + n] == name {
            let mut record = zip[at..end].to_vec();
            record[42..46].fill(0);
            return record;
        }
        at = end;
    }
    panic!("no central record");
}

/// Kept members are the same bytes, in both headers, and the comment stays; a new member goes after them.
#[test]
fn kept_zip_members_stay_byte_identical() {
    let original = rich_zip();
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", original.clone());
    write_member(&*memory.vfs, &member(&file, "new.txt"), b"added").unwrap();
    let rebuilt = stored(&memory, "a.zip");
    for name in [&b"plain.txt"[..], b"other.txt"] {
        assert_eq!(local_record(&rebuilt, name), local_record(&original, name));
        assert_eq!(
            central_record(&rebuilt, name),
            central_record(&original, name)
        );
    }
    let names: Vec<_> = read_plain_names(&rebuilt);
    assert_eq!(
        names,
        ["plain.txt", "other.txt", "new.txt"],
        "archive order, then new"
    );
    let archive = zip::ZipArchive::new(Cursor::new(rebuilt)).unwrap();
    assert_eq!(archive.comment(), b"archive comment");
}

fn read_plain_names(zip: &[u8]) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip.to_vec())).unwrap();
    (0..archive.len())
        .map(|at| archive.by_index(at).unwrap().name().unwrap().into_owned())
        .collect()
}

/// An encrypted member, of either kind, and one of a method this build does not know, survive as they are. The
/// crate's `raw_copy_file` would drop the encryption flag: the reason records are copied in-house.
#[test]
fn members_this_build_cannot_read_survive_a_rebuild() {
    let mut crypto = RawEntry::new(b"crypto.bin", b"0123456789abcdef-encrypted");
    crypto.flags = 1;
    let mut aes = RawEntry::new(b"aes.bin", b"aes-looking bytes");
    aes.method = 99;
    aes.flags = 1;
    let mut odd = RawEntry::new(b"odd.bin", b"ppmd-looking bytes");
    odd.method = 98;
    let plain = RawEntry::new(b"plain.txt", b"plain");
    let original = raw_zip(&[crypto, aes, odd, plain]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", original.clone());
    write_member(&*memory.vfs, &member(&file, "new.txt"), b"added").unwrap();
    let rebuilt = stored(&memory, "a.zip");
    for name in [&b"crypto.bin"[..], b"aes.bin", b"odd.bin", b"plain.txt"] {
        assert_eq!(local_record(&rebuilt, name), local_record(&original, name));
        assert_eq!(
            central_record(&rebuilt, name),
            central_record(&original, name)
        );
    }
    let encrypted = memory.vfs.open_read(&member(&file, "crypto.bin"));
    assert!(matches!(encrypted, Err(VfsError::ArchiveEncrypted { .. })));
    let unknown = memory.vfs.open_read(&member(&file, "odd.bin"));
    assert!(unknown.is_err());
}

/// A rename and a touch of such a member edit its headers: the data is not decoded, which it could not be.
#[test]
fn a_member_this_build_cannot_read_can_be_moved_and_touched() {
    let mut crypto = RawEntry::new(b"crypto.bin", b"0123456789abcdef-encrypted");
    crypto.flags = 1 | (1 << 3);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", raw_zip(&[crypto]));
    let vfs = &*memory.vfs;
    vfs.rename(
        &member(&file, "crypto.bin"),
        &member(&file, "moved.bin"),
        false,
    )
    .unwrap();
    let moved = member(&file, "moved.bin");
    let at = ft(1_700_000_000);
    vfs.set_times(
        &moved,
        &SetTimes {
            modified: Some(at),
            created: None,
        },
    )
    .unwrap();
    let entry = fresh_entry(&memory, &moved);
    assert_eq!(entry.mtime, Some(at));
    assert!(matches!(
        vfs.open_read(&moved),
        Err(VfsError::ArchiveEncrypted { .. })
    ));
    let rebuilt = stored(&memory, "a.zip");
    let local = local_record(&rebuilt, b"moved.bin");
    assert_eq!(local[6] & 1, 1, "still flagged encrypted");
    assert!(local.ends_with(b"0123456789abcdef-encrypted"));
    assert_eq!(
        &local[10..14],
        &raw_zip(&[RawEntry::new(b"x", b"")])[10..14],
        "its DOS time is its check byte"
    );
}

/// Refused members stay, also two of one name, which the crate's writer refuses and its reader shows once.
#[test]
fn refused_zip_members_stay_also_with_the_same_name() {
    let original = raw_zip(&[
        RawEntry::new(b"../evil", b"one"),
        RawEntry::new(b"/abs", b"two"),
        RawEntry::new(b"../evil", b"three"),
        RawEntry::new(b"keep.txt", b"kept"),
    ]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", original);
    write_member(&*memory.vfs, &member(&file, "new.txt"), b"added").unwrap();
    let rebuilt = stored(&memory, "a.zip");
    let names = central_names(&rebuilt);
    assert_eq!(names, ["../evil", "/abs", "../evil", "keep.txt", "new.txt"]);
    let contents = [&b"one"[..], b"two", b"three", b"kept"];
    for content in contents {
        assert!(rebuilt.windows(content.len()).any(|w| w == content));
    }
}

/// Every name of the central directory, duplicates included.
fn central_names(zip: &[u8]) -> Vec<String> {
    let mut at = zip.len() - 22;
    while &zip[at..at + 4] != b"PK\x05\x06" {
        at -= 1;
    }
    let mut at = u32::from_le_bytes(zip[at + 16..at + 20].try_into().unwrap()) as usize;
    let mut names = Vec::new();
    while &zip[at..at + 4] == b"PK\x01\x02" {
        let n = u16::from_le_bytes([zip[at + 28], zip[at + 29]]) as usize;
        let e = u16::from_le_bytes([zip[at + 30], zip[at + 31]]) as usize;
        let c = u16::from_le_bytes([zip[at + 32], zip[at + 33]]) as usize;
        names.push(String::from_utf8_lossy(&zip[at + 46..at + 46 + n]).into_owned());
        at += 46 + n + e + c;
    }
    names
}

/// Shadowed duplicates stay with the member that shadows them (A5.16, D-156): a rebuild that does not touch that member
/// takes nothing away. Each fate is in `archive_review.rs`.
#[test]
fn shadowed_duplicates_stay_through_a_rebuild() {
    let zip = raw_zip(&[
        RawEntry::new(b"a", b"old"),
        RawEntry::new(b"b", b"b"),
        RawEntry::new(b"a", b"new"),
    ]);
    let tar = raw_tar(&[
        RawTar::file(b"a", b"old"),
        RawTar::file(b"b", b"b"),
        RawTar::file(b"a", b"new"),
    ]);
    for (name, bytes) in [("a.zip", zip), ("a.tar", tar)] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes);
        drop(memory.vfs.take_warnings());
        write_member(&*memory.vfs, &member(&file, "c"), b"c").unwrap();
        let warnings = memory.vfs.take_warnings();
        assert!(
            !warnings.iter().any(|w| w.message.contains("went")),
            "{warnings:?}"
        );
        assert_eq!(read_member(&*memory.vfs, &member(&file, "a")), b"new");
        let bytes = stored(&memory, name);
        assert!(bytes.windows(3).any(|w| w == b"old"), "{name}");
    }
}

#[test]
fn a_zip_with_a_prefix_is_refused_and_stays() {
    let mut prefixed = b"#!/bin/sh\nexit 0\n".to_vec();
    let zip = common::archives::zip_bytes(&sample());
    prefixed.extend_from_slice(&zip);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", prefixed.clone());
    let spec = member(&file, "x");
    assert_eq!(
        read_member(&*memory.vfs, &member(&file, "readme.txt")),
        b"hello\n"
    );
    let result = write_member(&*memory.vfs, &spec, b"x");
    assert!(
        matches!(
            result,
            Err(VfsError::Unsupported {
                what: "rewriting a zip with a prefix",
                ..
            })
        ),
        "{result:?}"
    );
    assert_eq!(stored(&memory, "a.zip"), prefixed);
}

/// A stub whose length the offsets count in: `ZipArchive::offset` is 0, and the stub would go without a word.
#[test]
fn a_zip_whose_offsets_count_a_prefix_is_refused_too() {
    let mut entry = RawEntry::new(b"a", b"content");
    let stub = 16u32;
    entry.at = Some(stub);
    let mut bytes = vec![b'#'; stub as usize];
    bytes.extend_from_slice(&raw_zip(std::slice::from_ref(&entry)));
    // The end record states where the directory is: as a writer aware of the stub does.
    let end = bytes.len() - 22;
    let start = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) + stub;
    bytes[end + 16..end + 20].copy_from_slice(&start.to_le_bytes());
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes.clone());
    assert_eq!(read_member(&*memory.vfs, &member(&file, "a")), b"content");
    let result = write_member(&*memory.vfs, &member(&file, "b"), b"x");
    assert!(
        matches!(result, Err(VfsError::Unsupported { .. })),
        "{result:?}"
    );
    assert_eq!(stored(&memory, "a.zip"), bytes);
}

/// Bytes after the end of a zip are no member's, and a rebuild does not lose them either.
#[test]
fn bytes_past_the_end_of_a_zip_stay() {
    let mut bytes = raw_zip(&[RawEntry::new(b"a", b"content")]);
    bytes.extend_from_slice(b"\0\0\0\0\0\0appended");
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes);
    if read_member_checked(&*memory.vfs, &member(&file, "a")).is_none() {
        return; // The crate does not find the end record of this form.
    }
    write_member(&*memory.vfs, &member(&file, "b"), b"x").unwrap();
    let rebuilt = stored(&memory, "a.zip");
    assert!(rebuilt.ends_with(b"appended"));
}

fn read_member_checked(vfs: &dyn Vfs, spec: &Spec) -> Option<Vec<u8>> {
    vfs.open_read(spec).ok().map(read_to_vec)
}

/// Raw names that do not decode stay the bytes they were, CP437 ones too, and a folder that holds one stays put.
#[test]
fn raw_names_stay_raw() {
    let mut flagged = RawEntry::new(b"dir/caf\xc3", b"bad utf-8");
    flagged.flags = 1 << 11;
    let mut folder = RawEntry::new(b"dir2/x", b"x");
    folder.flags = 1 << 11;
    let original = raw_zip(&[flagged, RawEntry::new(b"caf\x82.txt", b"cp437"), folder]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", original.clone());
    let vfs = &*memory.vfs;
    write_member(vfs, &member(&file, "new"), b"n").unwrap();
    let rebuilt = stored(&memory, "a.zip");
    for name in [&b"dir/caf\xc3"[..], b"caf\x82.txt"] {
        assert_eq!(local_record(&rebuilt, name), local_record(&original, name));
    }
    let moved = vfs.rename(&member(&file, "dir"), &member(&file, "other"), false);
    assert!(
        matches!(moved, Err(VfsError::OperationUnsupported { .. })),
        "{moved:?}"
    );
    assert_eq!(stored(&memory, "a.zip"), rebuilt);

    let original = raw_tar(&[RawTar::file(b"bad\xff.txt", b"raw name")]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", original.clone());
    write_member(&*memory.vfs, &member(&file, "new"), b"n").unwrap();
    let rebuilt = stored(&memory, "a.tar");
    assert_eq!(
        blocks_of(&rebuilt, b"bad\xff.txt"),
        blocks_of(&original, b"bad\xff.txt")
    );
}

/// Records out of order and two entries on one record: every entry comes out readable.
#[test]
fn zip_records_need_not_be_in_order_or_apart() {
    let mut second = RawEntry::new(b"second", b"two");
    let first = RawEntry::new(b"first", b"one");
    second.dos_time = 0x6001;
    let mut bytes = raw_zip(&[first, second]);
    // Swap the two central headers: the directory now runs against the file.
    let end = bytes.len() - 22;
    let start = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap()) as usize;
    let one = 46 + 5;
    let swapped: Vec<u8> = [
        &bytes[start + one..start + size],
        &bytes[start..start + one],
    ]
    .concat();
    bytes[start..start + size].copy_from_slice(&swapped);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes);
    let vfs = &*memory.vfs;
    write_member(vfs, &member(&file, "third"), b"three").unwrap();
    assert_eq!(
        central_names(&stored(&memory, "a.zip")),
        ["second", "first", "third"]
    );
    assert_eq!(read_member(vfs, &member(&file, "first")), b"one");
    assert_eq!(read_member(vfs, &member(&file, "second")), b"two");

    let mut alias = RawEntry::new(b"alias", b"same");
    alias.at = Some(0);
    let mut real = RawEntry::new(b"alias", b"same");
    real.name = b"alia2".to_vec();
    let bytes = raw_zip(&[real, alias]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("b.zip", bytes);
    let vfs = &*memory.vfs;
    write_member(vfs, &member(&file, "n"), b"n").unwrap();
    assert_eq!(read_member(vfs, &member(&file, "alia2")), b"same");
}

/// Past 65,535 records the end records go wide, for this reader and the crate's. Kept and new records both count.
#[test]
fn a_zip_with_many_records_goes_wide() {
    let tree: TreeData = (0..65_600).map(|n| (format!("f{n}"), Vec::new())).collect();
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let vfs = &*memory.vfs;
    write_member(vfs, &member(&file, "new"), b"n").unwrap();
    assert_eq!(read_member(vfs, &member(&file, "new")), b"n");
    let rebuilt = stored(&memory, "a.zip");
    let archive = zip::ZipArchive::new(Cursor::new(rebuilt)).unwrap();
    assert_eq!(archive.len(), 65_601);
}

/// Removing every member leaves an empty archive of the format, and it stays one.
#[test]
fn an_archive_can_lose_every_member() {
    let tree: TreeData = vec![("only".into(), b"x".to_vec())];
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &tree));
        let vfs = &*memory.vfs;
        vfs.remove(&member(&file, "only")).unwrap();
        assert!(walk(vfs, &member(&file, "")).is_empty(), "{format:?}");
        let name = name_of(format);
        assert!(
            read_plain(format, &stored(&memory, name)).is_empty(),
            "{format:?}"
        );
        write_member(vfs, &member(&file, "again"), b"y").unwrap();
        assert_eq!(
            read_member(vfs, &member(&file, "again")),
            b"y",
            "{format:?}"
        );
    }
}

// ---- tar blocks ---------------------------------------------------------------------------------------------

fn blocks_of(stream: &[u8], name: &[u8]) -> Vec<u8> {
    let at = stream
        .chunks(512)
        .position(|block| block.starts_with(name) && block[name.len()] == 0)
        .expect("the header");
    let size = u64::from_str_radix(
        std::str::from_utf8(&stream[at * 512 + 124..at * 512 + 135]).unwrap(),
        8,
    )
    .unwrap() as usize;
    stream[at * 512..at * 512 + 512 + size.next_multiple_of(512)].to_vec()
}

/// Kept tar members keep their raw blocks; the stream is otherwise untouched.
#[test]
fn kept_tar_members_keep_their_blocks() {
    let original = raw_tar(&[
        RawTar::file(b"one.txt", b"first"),
        RawTar::file(b"../refused", b"refused data"),
        RawTar::file(b"two.txt", b"second"),
    ]);
    for format in WRITABLE.into_iter().filter(|f| *f != ArchiveFormat::Zip) {
        let memory = on_memory(ArchiveLimits::default());
        let bytes = common::archives::compress(format, &original);
        let file = memory.put(name_of(format), bytes);
        write_member(&*memory.vfs, &member(&file, "new.txt"), b"added").unwrap();
        let stream = decompress(format, &stored(&memory, name_of(format)));
        for name in [&b"one.txt"[..], b"../refused", b"two.txt"] {
            assert_eq!(
                blocks_of(&stream, name),
                blocks_of(&original, name),
                "{format:?}"
            );
        }
        assert!(stream.ends_with(&[0u8; 1024]), "{format:?}: the end marker");
        let position = |name: &[u8]| {
            stream
                .chunks(512)
                .position(|b| b.starts_with(name))
                .unwrap()
        };
        assert!(position(b"one.txt") < position(b"two.txt"));
        assert!(
            position(b"two.txt") < position(b"new.txt"),
            "new ones after the kept"
        );
    }
}

/// A sparse member keeps its blocks; moving or touching it is refused, not done wrong. Refused, device and link records
/// stay as they are.
#[test]
fn tar_members_nobody_may_decode_survive() {
    let mut sparse_record = RawTar::file(b"sparse.img", b"packed data");
    sparse_record.type_flag = b'S';
    let device = RawTar::typed(b"dev", b'3');
    let fifo = RawTar::typed(b"pipe", b'6');
    let link = RawTar::link(b"lnk", b'2', b"/etc/passwd");
    let refused = RawTar::file(b"/etc/hosts", b"absolute");
    let original = raw_tar(&[
        sparse_record,
        device,
        fifo,
        link,
        refused,
        RawTar::file(b"plain.txt", b"plain"),
    ]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", original.clone());
    let vfs = &*memory.vfs;
    write_member(vfs, &member(&file, "new.txt"), b"added").unwrap();
    let rebuilt = stored(&memory, "a.tar");
    for name in [
        &b"sparse.img"[..],
        b"dev",
        b"pipe",
        b"lnk",
        b"/etc/hosts",
        b"plain.txt",
    ] {
        assert_eq!(blocks_of(&rebuilt, name), blocks_of(&original, name));
    }
    let sparse = member(&file, "sparse.img");
    let moved = vfs.rename(&sparse, &member(&file, "moved.img"), false);
    assert!(
        matches!(moved, Err(VfsError::OperationUnsupported { .. })),
        "{moved:?}"
    );
    let touched = vfs.set_attrs(&sparse, &AttrChange::Mode(0o600));
    assert!(
        matches!(touched, Err(VfsError::OperationUnsupported { .. })),
        "{touched:?}"
    );
    assert_eq!(
        stored(&memory, "a.tar"),
        rebuilt,
        "a refused change writes nothing"
    );
}

/// PAX records a rename does not touch stay, and so do the global headers around a removed member.
#[test]
fn pax_records_and_global_headers_stay() {
    let mut global = pax(&[("comment", b"built from abc123")]);
    global[156] = b'g';
    seal(global.first_chunk_mut().unwrap());
    let mut original = global;
    original.extend(RawTar::file(b"first.txt", b"first").bytes());
    original.extend(pax(&[
        ("SCHILY.xattr.user.note", b"kept"),
        ("path", b"named.txt"),
    ]));
    original.extend(raw_tar(&[RawTar::file(b"short", b"content")]));
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", original);
    let vfs = &*memory.vfs;
    vfs.remove(&member(&file, "first.txt")).unwrap();
    let rebuilt = stored(&memory, "a.tar");
    assert!(
        rebuilt.windows(17).any(|w| w == b"built from abc123"),
        "the global header stays"
    );
    vfs.rename(
        &member(&file, "named.txt"),
        &member(&file, "renamed.txt"),
        false,
    )
    .unwrap();
    let rebuilt = stored(&memory, "a.tar");
    assert!(
        rebuilt.windows(4).any(|w| w == b"kept"),
        "the other record stays"
    );
    assert_eq!(read_member(vfs, &member(&file, "renamed.txt")), b"content");
    assert!(
        vfs.stat(&member(&file, "named.txt"), FollowLinks::No)
            .is_err()
    );
}

/// Bytes after the end of a tar are no reader's, and a rebuild does not lose them either.
#[test]
fn bytes_past_the_end_of_a_tar_stay() {
    let mut original = raw_tar(&[RawTar::file(b"a", b"content")]);
    original.extend_from_slice(b"appended by something else");
    original.resize(original.len().next_multiple_of(512), 0);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", original);
    write_member(&*memory.vfs, &member(&file, "b"), b"x").unwrap();
    let rebuilt = stored(&memory, "a.tar");
    assert!(rebuilt.windows(9).any(|w| w == b"something"));
    assert_eq!(read_member(&*memory.vfs, &member(&file, "b")), b"x");
}

fn hard_link_tar() -> Vec<u8> {
    raw_tar(&[
        RawTar::file(b"target", b"shared content"),
        RawTar::link(b"link", b'1', b"target"),
        RawTar::link(b"link2", b'1', b"link"),
    ])
}

/// A hard link does not lose its content with the member it names: whatever happens to that member.
#[test]
fn a_hard_link_keeps_its_content() {
    type Change = fn(&dyn Vfs, &Spec);
    let cases: [(&str, Change); 3] = [
        ("removed", |vfs, file| {
            vfs.remove(&member(file, "target")).unwrap()
        }),
        ("renamed", |vfs, file| {
            vfs.rename(&member(file, "target"), &member(file, "moved"), false)
                .unwrap();
        }),
        ("replaced", |vfs, file| {
            write_member(vfs, &member(file, "target"), b"other").unwrap();
        }),
    ];
    for (name, change) in cases {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put("a.tar", hard_link_tar());
        change(&*memory.vfs, &file);
        let vfs = &*memory.vfs;
        assert_eq!(
            read_member(vfs, &member(&file, "link")),
            b"shared content",
            "{name}"
        );
        assert_eq!(
            read_member(vfs, &member(&file, "link2")),
            b"shared content",
            "{name}"
        );
        let plain = read_plain(ArchiveFormat::Tar, &stored(&memory, "a.tar"));
        assert_eq!(
            plain.get("link"),
            Some(&seen_file(b"shared content")),
            "{name}"
        );
    }
}

/// A hard link that keeps its target stays one, and a touched or renamed link becomes a file with its content.
#[test]
fn a_hard_link_stays_while_it_can() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", hard_link_tar());
    let vfs = &*memory.vfs;
    write_member(vfs, &member(&file, "other"), b"x").unwrap();
    let rebuilt = stored(&memory, "a.tar");
    assert_eq!(
        blocks_of(&rebuilt, b"link"),
        blocks_of(&hard_link_tar(), b"link")
    );
    vfs.rename(&member(&file, "link"), &member(&file, "renamed"), false)
        .unwrap();
    assert_eq!(
        read_member(vfs, &member(&file, "renamed")),
        b"shared content"
    );
    assert_eq!(read_member(vfs, &member(&file, "link2")), b"shared content");
}

// ---- refusals -----------------------------------------------------------------------------------------------

#[test]
fn a_damaged_archive_is_refused_and_stays() {
    let mut damaged = raw_tar(&[RawTar::file(b"a", b"content")]);
    damaged.truncate(damaged.len() - 1024);
    damaged.extend_from_slice(&[b'x'; 512]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", damaged.clone());
    let result = write_member(&*memory.vfs, &member(&file, "b"), b"x");
    assert!(
        matches!(result, Err(VfsError::ArchiveCorrupt { .. })),
        "{result:?}"
    );
    assert_eq!(stored(&memory, "a.tar"), damaged);
}

/// A bad local header cannot be carried over: a rebuild fails rather than write it.
#[test]
fn a_zip_with_a_broken_local_header_is_refused_and_stays() {
    let mut broken = raw_zip(&[RawEntry::new(b"a", b"content"), RawEntry::new(b"b", b"x")]);
    broken[0] = b'X';
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", broken.clone());
    let result = write_member(&*memory.vfs, &member(&file, "b"), b"y");
    assert!(
        matches!(result, Err(VfsError::ArchiveCorrupt { .. })),
        "{result:?}"
    );
    assert_eq!(stored(&memory, "a.zip"), broken);
}

/// The archive changed between the plan and the rebuild: nothing is written, every item fails.
#[test]
fn a_stamp_change_between_plan_and_rebuild_leaves_the_new_archive() {
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let name = name_of(format);
        let file = memory.put(name, bytes_of(format, &sample()));
        let vfs = &*memory.vfs;
        let plan = ops::plan_mkdir(vfs, &[member(&file, "made")], false);
        let other = memory.put(
            name,
            bytes_of(format, &vec![("other".into(), b"o".to_vec())]),
        );
        assert_eq!(other, file);
        memory
            .mem
            .insert_file(
                &host(&format!("/{name}")),
                stored(&memory, name),
                MemMeta {
                    mtime: FileTime::from_unix(1_800_000_000, 0),
                    ..MemMeta::default()
                },
            )
            .unwrap();
        let after = stored(&memory, name);
        let report = ops::execute(vfs, vfs, plan, &Ctl::detached());
        assert!(!report.ok());
        assert_eq!(report.results[0].status, ItemStatus::Failed);
        let error = report.results[0].error.as_ref().unwrap();
        assert!(
            error.message.contains("changed since the plan was made"),
            "{error:?}"
        );
        assert_eq!(stored(&memory, name), after, "{format:?}");
    }
}

/// The same, once the overlay exists: the batch's index would write stale members. This change comes before the commit
/// starts; `a_change_during_the_sync_is_caught_before_the_rename` makes it in the window of the fsync.
#[test]
fn a_stamp_change_inside_a_batch_is_caught_at_commit() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    let plan = ops::plan_mkdir(
        vfs,
        &[member(&file, "made"), member(&file, "second")],
        false,
    );
    vfs.begin(&file).unwrap();
    vfs.create_dir(&member(&file, "made"), false).unwrap();
    let bytes = bytes_of(ArchiveFormat::Zip, &vec![("other".into(), b"o".to_vec())]);
    memory
        .mem
        .insert_file(
            &host("/a.zip"),
            bytes.clone(),
            MemMeta {
                mtime: FileTime::from_unix(1_800_000_000, 0),
                ..MemMeta::default()
            },
        )
        .unwrap();
    let result = vfs.finish(&file, Finish::Commit);
    assert!(
        matches!(result, Err(VfsError::InvalidTarget { reason, .. }) if reason.contains("changed since")),
        "{result:?}"
    );
    assert_eq!(stored(&memory, "a.zip"), bytes);
    drop(plan);
}

// ---- A5.15 overlay ----------------------------------------------------------------------------------------------

/// The primitive table of A3.1, on members of a zip and a `tar.gz`.
#[test]
fn primitive_table_on_members() {
    let tree: TreeData = vec![
        ("file".into(), b"content".to_vec()),
        ("folder/inside".into(), b"x".to_vec()),
        ("empty/".into(), Vec::new()),
    ];
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &tree));
        let vfs = &*memory.vfs;
        vfs.begin(&file).unwrap();
        let at = |path: &str| member(&file, path);
        let kind = |result: Result<(), VfsError>| {
            use hikasa_core::Classify;
            result.map_err(|e| e.kind())
        };
        let times = SetTimes {
            modified: Some(ft(1000)),
            created: None,
        };
        // Missing.
        assert_eq!(kind(vfs.remove(&at("missing"))), Err(ErrorKind::NotFound));
        assert_eq!(
            kind(vfs.set_times(&at("missing"), &times)),
            Err(ErrorKind::NotFound)
        );
        assert_eq!(
            kind(vfs.rename(&at("missing"), &at("x"), false)),
            Err(ErrorKind::NotFound)
        );
        // Files.
        assert_eq!(
            kind(vfs.create_dir(&at("file"), false).map(drop)),
            Err(ErrorKind::AlreadyExists)
        );
        assert_eq!(
            kind(vfs.create_dir(&at("file"), true).map(drop)),
            Err(ErrorKind::NotADirectory)
        );
        assert_eq!(
            kind(vfs.create_dir(&at("file/x"), false).map(drop)),
            Err(ErrorKind::NotADirectory)
        );
        // Folders.
        assert!(!vfs.create_dir(&at("empty"), true).unwrap());
        assert_eq!(
            kind(vfs.create_dir(&at("empty"), false).map(drop)),
            Err(ErrorKind::AlreadyExists)
        );
        assert_eq!(kind(vfs.remove(&at("folder"))), Err(ErrorKind::NotEmpty));
        vfs.remove(&at("empty")).unwrap();
        // A missing parent, with and without `parents`.
        assert_eq!(
            kind(vfs.create_dir(&at("a/b/c"), false).map(drop)),
            Err(ErrorKind::NotFound)
        );
        assert!(vfs.create_dir(&at("a/b/c"), true).unwrap());
        assert_eq!(
            vfs.stat(&at("a/b"), FollowLinks::No).unwrap().kind,
            EntryKind::Dir
        );
        // Existing targets, with and without `replace`.
        vfs.create_dir(&at("dir2"), false).unwrap();
        assert_eq!(
            kind(vfs.rename(&at("file"), &at("folder/inside"), false)),
            Err(ErrorKind::AlreadyExists)
        );
        assert_eq!(
            kind(vfs.rename(&at("file"), &at("folder"), true)),
            Err(ErrorKind::IsADirectory)
        );
        assert_eq!(
            kind(vfs.rename(&at("folder"), &at("file"), true)),
            Err(ErrorKind::NotADirectory)
        );
        assert_eq!(
            kind(vfs.rename(&at("a"), &at("folder"), true)),
            Err(ErrorKind::NotEmpty)
        );
        vfs.rename(&at("a"), &at("dir2"), true).unwrap();
        assert_eq!(
            vfs.stat(&at("dir2/b/c"), FollowLinks::No).unwrap().kind,
            EntryKind::Dir
        );
        // A rename into itself.
        assert_eq!(
            kind(vfs.rename(&at("dir2"), &at("dir2/b/x"), false)),
            Err(ErrorKind::InvalidInput)
        );
        // Links are not followed, and a write replaces one.
        vfs.create_symlink(&at("lnk"), std::ffi::OsStr::new("file"), false)
            .unwrap();
        assert_eq!(
            kind(vfs.create_symlink(&at("lnk"), std::ffi::OsStr::new("x"), false)),
            Err(ErrorKind::AlreadyExists)
        );
        assert_eq!(
            vfs.stat(&at("lnk"), FollowLinks::Yes).unwrap().kind,
            EntryKind::Symlink
        );
        assert_eq!(
            kind(vfs.set_attrs(&at("lnk"), &AttrChange::Mode(0o600))),
            Err(ErrorKind::Unsupported)
        );
        assert_eq!(
            kind(vfs.create_dir(&at("lnk/x"), false).map(drop)),
            Err(ErrorKind::NotADirectory)
        );
        write_member(vfs, &at("lnk"), b"replaced the link").unwrap();
        assert_eq!(
            vfs.stat(&at("lnk"), FollowLinks::No).unwrap().kind,
            EntryKind::File
        );
        assert_eq!(read_member(vfs, &at("file")), b"content", "never followed");
        // `open_write` rules.
        let new_only = WriteOptions::default();
        assert_eq!(
            kind(vfs.open_write(&at("file"), &new_only).map(drop)),
            Err(ErrorKind::AlreadyExists)
        );
        assert_eq!(
            kind(vfs.open_write(&at("folder"), &new_only).map(drop)),
            Err(ErrorKind::IsADirectory)
        );
        assert_eq!(
            kind(vfs.open_write(&at("no/parent"), &new_only).map(drop)),
            Err(ErrorKind::NotFound)
        );
        let parents = WriteOptions {
            create_parents: true,
            ..WriteOptions::default()
        };
        let mut writer = vfs.open_write(&at("no/parent"), &parents).unwrap();
        writer.write_all(b"made").unwrap();
        writer.commit().unwrap();
        assert_eq!(read_member(vfs, &at("no/parent")), b"made");
        // What no format has.
        let created = SetTimes {
            modified: None,
            created: Some(ft(1000)),
        };
        assert_eq!(
            kind(vfs.set_times(&at("file"), &created)),
            Err(ErrorKind::Unsupported)
        );
        assert_eq!(
            kind(vfs.move_to_trash(&at("file"))),
            Err(ErrorKind::Unsupported)
        );
        let hidden = AttrChange::Dos {
            set: DosAttrs::HIDDEN,
            clear: DosAttrs::empty(),
        };
        assert_eq!(
            kind(vfs.set_attrs(&at("file"), &hidden)),
            Err(ErrorKind::Unsupported)
        );
        // A root is no member.
        assert_eq!(kind(vfs.remove(&at(""))), Err(ErrorKind::InvalidInput));
        assert!(!vfs.create_dir(&at(""), true).unwrap());
        vfs.finish(&file, Finish::Abort).unwrap();
    }
}

/// Across archives and into a single stream.
#[test]
fn rename_across_archives_and_writes_on_streams() {
    let memory = on_memory(ArchiveLimits::default());
    let vfs = &*memory.vfs;
    let zip = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let tar = memory.put("b.tar", bytes_of(ArchiveFormat::Tar, &sample()));
    let across = vfs.rename(&member(&zip, "readme.txt"), &member(&tar, "x"), false);
    assert!(
        matches!(across, Err(VfsError::CrossDevice { .. })),
        "{across:?}"
    );
    let stream = memory.put(
        "x.txt.gz",
        common::archives::compress(ArchiveFormat::Gz, b"data"),
    );
    let spec = member(&stream, "x.txt");
    let unsupported = |result: Result<(), VfsError>| {
        assert!(
            matches!(result, Err(VfsError::OperationUnsupported { .. })),
            "{result:?}"
        );
    };
    unsupported(vfs.remove(&spec));
    unsupported(vfs.create_dir(&member(&stream, "d"), false).map(drop));
    unsupported(vfs.set_times(&spec, &SetTimes::default()));
    unsupported(vfs.rename(&spec, &member(&stream, "y"), false));
    unsupported(vfs.open_write(&spec, &WriteOptions::default()).map(drop));
    unsupported(vfs.begin(&stream));
}

/// The overlay is what every reader sees during a batch, and gone after `Abort`.
#[test]
fn the_overlay_is_seen_during_a_batch_and_gone_after_abort() {
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &sample()));
        let original = stored(&memory, name_of(format));
        let vfs = &*memory.vfs;
        let at = |path: &str| member(&file, path);
        let names = |dir: &Spec| {
            let mut listing = vfs
                .list(dir, &ListOptions::default(), &CancelToken::never())
                .unwrap();
            listing.sort_by_name();
            listing
                .entries
                .iter()
                .map(|e| e.name.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        memory.mem.set_journal(true);
        vfs.begin(&file).unwrap();
        write_member(vfs, &at("dir/new.txt"), b"staged").unwrap();
        vfs.remove(&at("readme.txt")).unwrap();
        vfs.rename(&at("dir/a.txt"), &at("dir/b.txt"), false)
            .unwrap();
        assert_eq!(names(&at("")), ["dir", "empty"]);
        assert_eq!(names(&at("dir")), ["b.txt", "new.txt", "sub"]);
        assert_eq!(read_member(vfs, &at("dir/new.txt")), b"staged");
        assert_eq!(
            read_member(vfs, &at("dir/b.txt")),
            b"a".repeat(1000),
            "kept, moved"
        );
        assert!(matches!(
            vfs.stat(&at("readme.txt"), FollowLinks::No),
            Err(VfsError::NotFound { .. })
        ));
        assert_eq!(
            vfs.stat(&at("dir/new.txt"), FollowLinks::No).unwrap().size,
            6
        );
        assert!(
            memory.mem.journal().is_empty(),
            "a member write never touches the archive"
        );
        vfs.finish(&file, Finish::Abort).unwrap();
        assert!(memory.mem.journal().is_empty());
        assert_eq!(stored(&memory, name_of(format)), original);
        assert_eq!(names(&at("dir")), ["a.txt", "sub"]);
        assert_eq!(read_member(vfs, &at("readme.txt")), b"hello\n");
    }
}

/// Staged content follows the cache rules: it counts, it goes with the batch, and it is released on abort.
#[test]
fn staged_files_are_released() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&file, "big"), &vec![7u8; 1 << 16]).unwrap();
    assert_eq!(vfs.cache_used(), 1 << 16);
    vfs.finish(&file, Finish::Abort).unwrap();
    assert_eq!(vfs.cache_used(), 0);
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&file, "big"), &vec![7u8; 1 << 16]).unwrap();
    vfs.finish(&file, Finish::Commit).unwrap();
    assert_eq!(vfs.cache_used(), 0, "nothing stays behind a rebuild");
    let dir = vfs.cache_dir().unwrap();
    let left: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

// ---- A5.14 batches ------------------------------------------------------------------------------------------

/// A `Vfs` that journals `begin` and `finish`, and can panic in a primitive.
struct Journal<'a> {
    inner: &'a dyn Vfs,
    calls: Mutex<Vec<String>>,
    panic_on_remove: bool,
}

impl<'a> Journal<'a> {
    fn new(inner: &'a dyn Vfs) -> Journal<'a> {
        Journal {
            inner,
            calls: Mutex::new(Vec::new()),
            panic_on_remove: false,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Vfs for Journal<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.inner.create_dir(spec, parents)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        assert!(!self.panic_on_remove, "a panic inside execute");
        self.inner.remove(spec)
    }
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.inner.rename(from, to, replace)
    }
    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError> {
        self.inner.set_times(spec, times)
    }
    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError> {
        self.inner.set_attrs(spec, change)
    }
    fn create_symlink(
        &self,
        spec: &Spec,
        target: &std::ffi::OsStr,
        target_is_dir: bool,
    ) -> Result<(), VfsError> {
        self.inner.create_symlink(spec, target, target_is_dir)
    }
    fn begin(&self, at: &Spec) -> Result<(), VfsError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("begin {}", name_tail(at)));
        self.inner.begin(at)
    }
    fn finish(&self, at: &Spec, how: Finish) -> Result<(), VfsError> {
        self.finish_with(at, how, &Ctl::detached())
    }
    fn finish_with(&self, at: &Spec, how: Finish, ctl: &Ctl<'_>) -> Result<(), VfsError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("finish {} {how:?}", name_tail(at)));
        self.inner.finish_with(at, how, ctl)
    }
}

/// The last parts of a spec's text, without the temp folder.
fn name_tail(spec: &Spec) -> String {
    let text = spec.canonical();
    text.rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_owned()
        + &match spec {
            Spec::ArchiveMember { .. } => " (nested)".to_owned(),
            _ => String::new(),
        }
}

fn rm_all(vfs: &dyn Vfs, specs: &[Spec]) -> ops::OpReport {
    let opts = RemoveOptions {
        recursive: true,
        trash: false,
    };
    let plan = ops::plan_remove(vfs, specs, &opts, &Ctl::detached()).unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

/// One rebuild per operation: a tree of items, one write.
#[test]
fn one_rebuild_per_operation() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    memory.mem.set_journal(true);
    let journal = Journal::new(&*memory.vfs);
    let report = rm_all(
        &journal,
        &[member(&file, "dir"), member(&file, "readme.txt")],
    );
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(report.summary.done, 5);
    assert_eq!(journal.calls(), ["begin a.zip", "finish a.zip Commit"]);
    let writes: Vec<_> = memory
        .mem
        .journal()
        .into_iter()
        .filter(|(op, _)| *op == VfsOp::Commit)
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
}

/// Target roots finish before source roots.
#[test]
fn target_roots_finish_before_source_roots() {
    let memory = on_memory(ArchiveLimits::default());
    let source = memory.put("from.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let target = memory.put("to.tar", bytes_of(ArchiveFormat::Tar, &sample()));
    let journal = Journal::new(&*memory.vfs);
    let pairs = [(member(&source, "readme.txt"), member(&target, "moved.txt"))];
    let plan = ops::plan_move(
        &journal,
        &journal,
        &pairs,
        &MoveOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let report = ops::execute(&journal, &journal, plan, &Ctl::detached());
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(
        journal.calls(),
        [
            "begin from.zip",
            "begin to.tar",
            "finish to.tar Commit",
            "finish from.zip Commit"
        ]
    );
    let vfs = &*memory.vfs;
    assert_eq!(read_member(vfs, &member(&target, "moved.txt")), b"hello\n");
    assert!(
        vfs.stat(&member(&source, "readme.txt"), FollowLinks::No)
            .is_err()
    );
}

/// A chain is one batch: items on both levels of an archive, one `begin`, one `finish`, one rebuild.
#[test]
fn a_chain_is_one_root() {
    let memory = on_memory(ArchiveLimits::default());
    let (file, chain) = nested(&memory);
    memory.mem.set_journal(true);
    let journal = Journal::new(&*memory.vfs);
    let specs = [member(&chain, "c.txt"), member(&file, "side.txt")];
    let report = rm_all(&journal, &specs);
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(
        journal.calls(),
        ["begin b.tar.gz (nested)", "finish b.tar.gz (nested) Commit"]
    );
    let commits = memory
        .mem
        .journal()
        .into_iter()
        .filter(|(op, _)| *op == VfsOp::Commit)
        .count();
    assert_eq!(commits, 1);
    let vfs = &*memory.vfs;
    assert!(vfs.stat(&member(&chain, "c.txt"), FollowLinks::No).is_err());
    assert!(
        vfs.stat(&member(&file, "side.txt"), FollowLinks::No)
            .is_err()
    );
    assert_eq!(read_member(vfs, &member(&chain, "d/e")), b"deep");
}

/// A chain with a level that nothing changes is one root and one rebuild too.
#[test]
fn a_chain_with_an_untouched_level_is_one_root() {
    let memory = on_memory(ArchiveLimits::default());
    let inner: TreeData = vec![
        ("c.txt".into(), b"inner".to_vec()),
        ("k".into(), Vec::new()),
    ];
    let middle: TreeData = vec![("c.zip".into(), bytes_of(ArchiveFormat::Zip, &inner))];
    let outer: TreeData = vec![("b.tar".into(), bytes_of(ArchiveFormat::Tar, &middle))];
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &outer));
    let deep = member(&member(&file, "b.tar"), "c.zip");
    memory.mem.set_journal(true);
    let journal = Journal::new(&*memory.vfs);
    let report = rm_all(&journal, &[member(&deep, "c.txt")]);
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(journal.calls().len(), 2, "{:?}", journal.calls());
    let commits = memory.mem.journal();
    let commits = commits.iter().filter(|(op, _)| *op == VfsOp::Commit);
    assert_eq!(commits.count(), 1);
    assert_eq!(memory.vfs.counters().rebuilds, 1);
    let dir = tempfile::tempdir().unwrap();
    let fresh = ArchiveVfs::new(
        Arc::clone(&memory.mem) as Arc<dyn Vfs>,
        config(dir.path(), ArchiveLimits::default()),
    );
    assert!(
        fresh
            .stat(&member(&deep, "c.txt"), FollowLinks::No)
            .is_err()
    );
    assert!(fresh.stat(&member(&deep, "k"), FollowLinks::No).is_ok());
}

#[test]
fn a_second_begin_is_unavailable() {
    let memory = on_memory(ArchiveLimits::default());
    let inner = bytes_of(ArchiveFormat::Tar, &sample());
    let outer: TreeData = vec![("b.tar".into(), inner)];
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &outer));
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    for at in [file.clone(), member(&file, "b.tar")] {
        let second = vfs.begin(&at);
        assert!(
            matches!(&second, Err(VfsError::Unavailable { reason, .. }) if reason == "another operation is changing this archive"),
            "{second:?}"
        );
    }
    vfs.finish(&file, Finish::Abort).unwrap();
    vfs.begin(&file).unwrap();
    vfs.finish(&file, Finish::Abort).unwrap();
}

/// A change from another thread must not land in an operation that may abort: a plain write cannot be lost.
#[test]
fn a_change_from_another_thread_meets_the_batch() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = Arc::clone(&memory.vfs);
    vfs.begin(&file).unwrap();
    let spec = member(&file, "elsewhere");
    let write = std::thread::spawn({
        let vfs = Arc::clone(&vfs);
        move || write_member(&*vfs, &spec, b"x")
    });
    let result = write.join().unwrap();
    assert!(
        matches!(result, Err(VfsError::Unavailable { .. })),
        "{result:?}"
    );
    vfs.finish(&file, Finish::Abort).unwrap();
}

/// A primitive and an `open_write` commit outside a batch rebuild at once.
#[test]
fn a_change_outside_a_batch_rebuilds_at_once() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    memory.mem.set_journal(true);
    vfs.remove(&member(&file, "readme.txt")).unwrap();
    write_member(vfs, &member(&file, "n"), b"x").unwrap();
    let commits = memory
        .mem
        .journal()
        .into_iter()
        .filter(|(op, _)| *op == VfsOp::Commit)
        .count();
    assert_eq!(commits, 2);
    assert_eq!(read_member(vfs, &member(&file, "n")), b"x");
    // A writer dropped without `commit` leaves nothing.
    let mut writer = vfs
        .open_write(&member(&file, "dropped"), &WriteOptions::default())
        .unwrap();
    writer.write_all(b"never").unwrap();
    drop(writer);
    assert!(
        vfs.stat(&member(&file, "dropped"), FollowLinks::No)
            .is_err()
    );
}

/// A panic inside `execute` aborts every batch: the guard.
#[test]
fn a_panic_inside_execute_aborts() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let original = stored(&memory, "a.zip");
    let mut journal = Journal::new(&*memory.vfs);
    journal.panic_on_remove = true;
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rm_all(&journal, &[member(&file, "readme.txt")])
    }));
    assert!(panicked.is_err());
    assert_eq!(journal.calls(), ["begin a.zip", "finish a.zip Abort"]);
    assert_eq!(stored(&memory, "a.zip"), original);
    memory.vfs.begin(&file).unwrap();
    memory.vfs.finish(&file, Finish::Abort).unwrap();
}

/// A failed item does not take the done ones with it (ARCH-101).
#[test]
fn done_items_commit_past_a_failed_one() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    let plan = ops::plan_mkdir(
        vfs,
        &[member(&file, "made"), member(&file, "readme.txt")],
        false,
    );
    let report = ops::execute(vfs, vfs, plan, &Ctl::detached());
    assert_eq!(report.results[0].status, ItemStatus::Done);
    assert_eq!(report.results[1].status, ItemStatus::Failed);
    assert_eq!(
        vfs.stat(&member(&file, "made"), FollowLinks::No)
            .unwrap()
            .kind,
        EntryKind::Dir
    );
    assert_eq!(report.plan.items[0].action, Action::CreateDir);
}

// ---- nested -------------------------------------------------------------------------------------------------

fn nested(memory: &OnMemory) -> (Spec, Spec) {
    let inner: TreeData = vec![
        ("c.txt".into(), b"inner".to_vec()),
        ("d/e".into(), b"deep".to_vec()),
    ];
    let middle: TreeData = vec![
        ("b.tar.gz".into(), bytes_of(ArchiveFormat::TarGz, &inner)),
        ("side.txt".into(), b"side".to_vec()),
    ];
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &middle));
    let chain = member(&file, "b.tar.gz");
    (file, chain)
}

#[test]
fn a_nested_rebuild_goes_inner_then_outer_and_commits_once() {
    let memory = on_memory(ArchiveLimits::default());
    let (file, chain) = nested(&memory);
    let vfs = &*memory.vfs;
    memory.mem.set_journal(true);
    write_member(vfs, &member(&chain, "new.txt"), b"nested write").unwrap();
    assert_eq!(
        read_member(vfs, &member(&chain, "new.txt")),
        b"nested write"
    );
    assert_eq!(read_member(vfs, &member(&chain, "c.txt")), b"inner");
    assert_eq!(read_member(vfs, &member(&file, "side.txt")), b"side");
    let commits = memory
        .mem
        .journal()
        .into_iter()
        .filter(|(op, _)| *op == VfsOp::Commit)
        .count();
    assert_eq!(commits, 1);
    // Plain crates all the way down.
    let outer = read_plain(ArchiveFormat::Zip, &stored(&memory, "a.zip"));
    let Some(Seen::File(inner)) = outer.get("b.tar.gz") else {
        panic!("{outer:?}");
    };
    let inner = read_plain(ArchiveFormat::TarGz, inner);
    assert_eq!(inner.get("new.txt"), Some(&seen_file(b"nested write")));
    assert_eq!(inner.get("d/e"), Some(&seen_file(b"deep")));
}

/// Three levels, in one batch that changes two of them, with one rebuild.
#[test]
fn a_chain_changes_at_every_level_in_one_batch() {
    let memory = on_memory(ArchiveLimits::default());
    let deep: TreeData = vec![("x".into(), b"x".to_vec())];
    let inner: TreeData = vec![("c.zip".into(), bytes_of(ArchiveFormat::Zip, &deep))];
    let middle: TreeData = vec![("b.tar".into(), bytes_of(ArchiveFormat::Tar, &inner))];
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &middle));
    let b = member(&file, "b.tar");
    let c = member(&b, "c.zip");
    let vfs = &*memory.vfs;
    vfs.begin(&c).unwrap();
    write_member(vfs, &member(&c, "level3"), b"3").unwrap();
    write_member(vfs, &member(&b, "level2"), b"2").unwrap();
    write_member(vfs, &member(&file, "level1"), b"1").unwrap();
    assert_eq!(read_member(vfs, &member(&c, "level3")), b"3");
    memory.mem.set_journal(true);
    vfs.finish(&c, Finish::Commit).unwrap();
    assert_eq!(memory.mem.journal().len(), 1);
    assert_eq!(read_member(vfs, &member(&c, "level3")), b"3");
    assert_eq!(read_member(vfs, &member(&c, "x")), b"x");
    assert_eq!(read_member(vfs, &member(&b, "level2")), b"2");
    assert_eq!(read_member(vfs, &member(&file, "level1")), b"1");
}

/// An empty nested archive made in a batch can be written into in that batch, and aborts as a whole.
#[test]
fn a_new_nested_archive_takes_members_in_its_batch() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    let original = stored(&memory, "a.zip");
    let new = member(&file, "new.tar.gz");
    vfs.begin(&file).unwrap();
    vfs.create_archive(&new).unwrap();
    write_member(vfs, &member(&new, "x"), b"in the new one").unwrap();
    assert_eq!(read_member(vfs, &member(&new, "x")), b"in the new one");
    vfs.finish(&file, Finish::Abort).unwrap();
    assert_eq!(stored(&memory, "a.zip"), original);
    assert!(vfs.stat(&new, FollowLinks::No).is_err());
    assert!(vfs.stat(&member(&new, "x"), FollowLinks::No).is_err());
    vfs.begin(&file).unwrap();
    vfs.create_archive(&new).unwrap();
    write_member(vfs, &member(&new, "x"), b"in the new one").unwrap();
    vfs.finish(&file, Finish::Commit).unwrap();
    assert_eq!(read_member(vfs, &member(&new, "x")), b"in the new one");
}

/// A rename over a nested archive whose inside changed: what changed goes with it, and does not come back at commit.
#[test]
fn a_rename_over_a_changed_nested_archive_replaces_it() {
    let memory = on_memory(ArchiveLimits::default());
    let (file, chain) = nested(&memory);
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&chain, "new.txt"), b"inside").unwrap();
    vfs.rename(&member(&file, "side.txt"), &chain, true)
        .unwrap();
    vfs.finish(&file, Finish::Commit).unwrap();
    assert_eq!(read_member(vfs, &chain), b"side");
}

/// Moving, removing or reading a nested archive whose inside changed in the same batch counts what changed.
#[test]
fn what_changed_inside_a_member_moves_with_it() {
    let memory = on_memory(ArchiveLimits::default());
    let (file, chain) = nested(&memory);
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&chain, "new.txt"), b"inside").unwrap();
    let whole = read_member(vfs, &chain);
    assert_eq!(
        read_plain(ArchiveFormat::TarGz, &whole).get("new.txt"),
        Some(&seen_file(b"inside")),
        "a read of the whole member"
    );
    write_member(vfs, &member(&chain, "second.txt"), b"later").unwrap();
    let moved = member(&file, "moved.tar.gz");
    vfs.rename(&chain, &moved, false).unwrap();
    assert_eq!(read_member(vfs, &member(&moved, "second.txt")), b"later");
    vfs.finish(&file, Finish::Commit).unwrap();
    assert_eq!(read_member(vfs, &member(&moved, "new.txt")), b"inside");
    assert_eq!(read_member(vfs, &member(&moved, "c.txt")), b"inner");
    assert!(vfs.stat(&chain, FollowLinks::No).is_err());

    let memory = on_memory(ArchiveLimits::default());
    let (file, chain) = nested(&memory);
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&chain, "new.txt"), b"inside").unwrap();
    vfs.remove(&chain).unwrap();
    vfs.finish(&file, Finish::Commit).unwrap();
    assert!(
        vfs.stat(&chain, FollowLinks::No).is_err(),
        "it does not come back"
    );
    assert_eq!(read_member(vfs, &member(&file, "side.txt")), b"side");
}

/// A member under a chain with a single stream in it has no writer.
#[test]
fn a_chain_with_a_stream_is_read_only() {
    let inner: TreeData = vec![("x".into(), b"x".to_vec())];
    let tar = bytes_of(ArchiveFormat::Tar, &inner);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.tar.gz",
        bytes_of(
            ArchiveFormat::TarGz,
            &vec![(
                "b.gz".to_owned(),
                common::archives::compress(ArchiveFormat::Gz, &tar),
            )],
        ),
    );
    let stream = member(&file, "b.gz");
    let inside = member(&member(&stream, "b"), "x");
    let result = memory.vfs.remove(&inside);
    assert!(result.is_err(), "{result:?}");
    assert!(
        memory.vfs.capabilities(&inside).unwrap() & Capabilities::WRITE == Capabilities::empty()
    );
}

// ---- limits -------------------------------------------------------------------------------------------------

#[test]
fn limits_bind_what_is_written() {
    let limits = ArchiveLimits {
        cache_max: 1 << 20,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let original = stored(&memory, "a.zip");
    let vfs = &*memory.vfs;
    let mut writer = vfs
        .open_write(&member(&file, "big"), &WriteOptions::default())
        .unwrap();
    let chunk = vec![1u8; 1 << 16];
    let mut failed = None;
    for _ in 0..64 {
        if let Err(error) = writer.write_all(&chunk) {
            failed = Some(error);
            break;
        }
    }
    let error = failed.expect("the cache limit");
    assert_eq!(hikasa_core::io_error_kind(&error), ErrorKind::TooLarge);
    drop(writer);
    assert_eq!(vfs.cache_used(), 0);
    assert_eq!(stored(&memory, "a.zip"), original);

    let limits = ArchiveLimits {
        entries_max: 1000,
        depth_max: 16,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let deep = vec!["d"; 20].join("/");
    let result = write_member(&*memory.vfs, &member(&file, &deep), b"x");
    assert!(
        matches!(result, Err(VfsError::ArchiveLimit { limit: "depth", .. })),
        "{result:?}"
    );
}

/// A stream past `stream_max` is one no reader would open: it is not written.
#[test]
fn a_rebuild_past_the_stream_limit_is_refused() {
    let limits = ArchiveLimits {
        stream_max: 1 << 20,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put("a.tar.gz", bytes_of(ArchiveFormat::TarGz, &sample()));
    let original = stored(&memory, "a.tar.gz");
    let result = write_member(&*memory.vfs, &member(&file, "big"), &vec![0u8; 2 << 20]);
    assert!(
        matches!(
            result,
            Err(VfsError::ArchiveLimit {
                limit: "stream",
                ..
            })
        ),
        "{result:?}"
    );
    assert_eq!(stored(&memory, "a.tar.gz"), original);
}

/// Past `ratio_max` a reader would refuse the archive: not written either.
#[test]
fn a_rebuild_past_the_ratio_limit_is_refused() {
    let limits = ArchiveLimits {
        ratio_floor: 1 << 16,
        ratio_max: 8,
        ..ArchiveLimits::default()
    };
    for (name, tree) in [
        ("a.zip", ArchiveFormat::Zip),
        ("a.tar.xz", ArchiveFormat::TarXz),
    ] {
        let memory = on_memory(limits);
        let file = memory.put(name, bytes_of(tree, &sample()));
        let original = stored(&memory, name);
        let result = write_member(&*memory.vfs, &member(&file, "zeros"), &vec![0u8; 1 << 20]);
        assert!(
            matches!(result, Err(VfsError::ArchiveLimit { limit: "ratio", .. })),
            "{name}: {result:?}"
        );
        assert_eq!(stored(&memory, name), original);
    }
}

// ---- metadata -----------------------------------------------------------------------------------------------

/// Written times read back per §1.5.8: DOS in the zone plus 0x5455 for a zip, whole seconds for a tar.
#[test]
fn written_times_read_back() {
    let at = ft(1_792_415_520);
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &sample()));
        let vfs = &*memory.vfs;
        let spec = member(&file, "n");
        vfs.begin(&file).unwrap();
        write_member(vfs, &spec, b"x").unwrap();
        vfs.set_times(
            &spec,
            &SetTimes {
                modified: Some(at),
                created: None,
            },
        )
        .unwrap();
        vfs.finish(&file, Finish::Commit).unwrap();
        assert_eq!(fresh_entry(&memory, &spec).mtime, Some(at), "{format:?}");
        let crc = fresh_entry(&memory, &spec).crc32;
        assert_eq!(crc, Some(crc32fast::hash(b"x")), "{format:?}");
    }
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    let far = ft(1 << 33);
    let spec = member(&file, "readme.txt");
    let times = SetTimes {
        modified: Some(far),
        created: None,
    };
    vfs.set_times(&spec, &times).unwrap();
    let entry = fresh_entry(&memory, &spec);
    assert!(entry.mtime.is_some(), "the DOS time, clamped");
}

/// `R` on a zip member, on a Unix one and on a member without a family.
#[test]
fn the_read_only_bit_of_a_zip_member() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    let spec = member(&file, "readme.txt");
    let set = AttrChange::Dos {
        set: DosAttrs::READONLY,
        clear: DosAttrs::empty(),
    };
    vfs.set_attrs(&spec, &set).unwrap();
    let mode = fresh_entry(&memory, &spec).attrs.unix().unwrap().mode;
    assert_eq!(mode & 0o222, 0, "{mode:o}");
    let clear = AttrChange::Dos {
        set: DosAttrs::empty(),
        clear: DosAttrs::READONLY,
    };
    vfs.set_attrs(&spec, &clear).unwrap();
    let mode = fresh_entry(&memory, &spec).attrs.unix().unwrap().mode;
    assert_eq!(mode & 0o200, 0o200, "{mode:o}");
    let tar = memory.put("b.tar", bytes_of(ArchiveFormat::Tar, &sample()));
    let on_tar = vfs.set_attrs(&member(&tar, "readme.txt"), &set);
    assert!(
        matches!(on_tar, Err(VfsError::OperationUnsupported { .. })),
        "{on_tar:?}"
    );
}

/// A long name and a long link target, each past a tar header's fields.
#[test]
fn long_names_and_targets_read_back() {
    let long = format!("{}/{}", "d".repeat(150), "n".repeat(150));
    let target = "t".repeat(300);
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &sample()));
        let vfs = &*memory.vfs;
        let options = WriteOptions {
            create_parents: true,
            ..WriteOptions::default()
        };
        vfs.begin(&file).unwrap();
        let mut writer = vfs.open_write(&member(&file, &long), &options).unwrap();
        writer.write_all(b"long").unwrap();
        writer.commit().unwrap();
        vfs.create_symlink(&member(&file, "lnk"), std::ffi::OsStr::new(&target), false)
            .unwrap();
        vfs.finish(&file, Finish::Commit).unwrap();
        assert_eq!(
            read_member(vfs, &member(&file, &long)),
            b"long",
            "{format:?}"
        );
        let link = fresh_entry(&memory, &member(&file, "lnk"));
        assert_eq!(
            link.link_target.as_deref(),
            Some(target.as_str()),
            "{format:?}"
        );
        // Renamed: the rewritten header has to say the long name too.
        vfs.rename(
            &member(&file, "readme.txt"),
            &member(&file, &format!("{}/r", "d".repeat(150))),
            false,
        )
        .unwrap();
        assert_eq!(
            read_member(vfs, &member(&file, &format!("{}/r", "d".repeat(150)))),
            b"hello\n",
            "{format:?}"
        );
    }
}

/// Copying between archives keeps a link a link and a mode a mode (rules 5 and 18).
#[test]
fn between_archives_links_and_modes_stay() {
    let memory = on_memory(ArchiveLimits::default());
    let source = memory.put(
        "s.tar",
        raw_tar(&[
            {
                let mut file = RawTar::file(b"tool", b"#!");
                file.mode = 0o750;
                file
            },
            RawTar::link(b"lnk", b'2', b"tool"),
        ]),
    );
    let target = memory.put("t.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    let vfs = &*memory.vfs;
    let report = run_copy(vfs, &member(&source, "tool"), &member(&target, "tool"));
    assert!(report.ok(), "{:?}", report.results);
    let report = run_copy(vfs, &member(&source, "lnk"), &member(&target, "lnk"));
    assert!(report.ok(), "{:?}", report.results);
    let tool = fresh_entry(&memory, &member(&target, "tool"));
    assert_eq!(tool.attrs.unix().unwrap().mode, 0o750);
    assert_eq!(tool.mtime, FileTime::from_unix(1_792_415_520, 0));
    let link = fresh_entry(&memory, &member(&target, "lnk"));
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.link_target.as_deref(), Some("tool"));
}

// ---- on a real disk --------------------------------------------------------------------------------------------

fn siblings(disk: &OnDisk) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&disk.dir)
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The temp file lies next to the archive and is gone after a success, within this process: a killed writer cannot
/// clean up, and what it leaves goes with the next rebuild (`dead_temporary_files_go_and_nothing_else_does`). The lock
/// file is covered too. A5.17 asserts the same after faults, on `MemoryVfs`.
#[test]
fn no_temporary_sibling_stays() {
    for format in WRITABLE {
        let disk = on_disk(ArchiveLimits::default());
        let file = disk.put(name_of(format), &bytes_of(format, &sample()));
        let vfs = &*disk.vfs;
        write_member(vfs, &member(&file, "n"), b"x").unwrap();
        assert_eq!(siblings(&disk), [name_of(format)]);
        assert_eq!(read_member(vfs, &member(&file, "n")), b"x");
        let on_disk_bytes = std::fs::read(disk.dir.join(name_of(format))).unwrap();
        assert_eq!(
            read_plain(format, &on_disk_bytes).get("n"),
            Some(&seen_file(b"x")),
            "{format:?}"
        );
    }
}

/// The permissions of the original stay.
#[cfg(unix)]
#[test]
fn permissions_of_the_archive_stay() {
    use std::os::unix::fs::PermissionsExt;
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &sample()));
    let path = disk.dir.join("a.zip");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    write_member(&*disk.vfs, &member(&file, "n"), b"x").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
}

/// A symbolic link to an archive stays: the file it names is what is replaced (row 21 of §1.5.4). Hard links are another
/// matter: `a_hard_linked_archive_loses_the_link_on_rebuild`.
#[cfg(unix)]
#[test]
fn a_linked_archive_keeps_its_link() {
    let disk = on_disk(ArchiveLimits::default());
    disk.put("real.zip", &bytes_of(ArchiveFormat::Zip, &sample()));
    let link = disk.dir.join("link.zip");
    std::os::unix::fs::symlink("real.zip", &link).unwrap();
    let file = common::archives::local(&link);
    write_member(&*disk.vfs, &member(&file, "n"), b"x").unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        std::path::Path::new("real.zip")
    );
    let real = std::fs::read(disk.dir.join("real.zip")).unwrap();
    assert_eq!(
        read_plain(ArchiveFormat::Zip, &real).get("n"),
        Some(&seen_file(b"x"))
    );
    assert_eq!(siblings(&disk), ["link.zip", "real.zip"]);
}

/// A folder the user cannot write to: a clean error, the original intact, nothing left.
#[cfg(unix)]
#[test]
fn a_read_only_folder_is_a_clean_error() {
    use std::os::unix::fs::PermissionsExt;
    let disk = on_disk(ArchiveLimits::default());
    let bytes = bytes_of(ArchiveFormat::Zip, &sample());
    let file = disk.put("a.zip", &bytes);
    std::fs::set_permissions(&disk.dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = tempfile::tempfile_in(&disk.dir).is_ok();
    let result = write_member(&*disk.vfs, &member(&file, "n"), b"x");
    std::fs::set_permissions(&disk.dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return; // Root writes anywhere.
    }
    assert!(
        matches!(result, Err(VfsError::PermissionDenied { .. })),
        "{result:?}"
    );
    assert_eq!(std::fs::read(disk.dir.join("a.zip")).unwrap(), bytes);
    assert_eq!(siblings(&disk), ["a.zip"]);
}

/// The disk is full: a clean error, the original intact. `/dev/full` needs Linux.
#[test]
fn a_full_disk_is_a_clean_error() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar.gz", bytes_of(ArchiveFormat::TarGz, &sample()));
    let original = stored(&memory, "a.tar.gz");
    memory
        .mem
        .inject_write_error_after(&host("/a.tar.gz"), 16, ErrorKind::Io);
    let result = write_member(&*memory.vfs, &member(&file, "n"), b"x");
    assert!(matches!(result, Err(VfsError::Io { .. })), "{result:?}");
    assert_eq!(stored(&memory, "a.tar.gz"), original);
}

/// Windows cannot replace a file this process holds open without sharing, so a rebuild holds no handle at `commit`.
/// A reader opened by a caller stays valid where the platform lets it.
#[test]
fn a_rebuild_replaces_an_archive_someone_reads() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.tar", &bytes_of(ArchiveFormat::Tar, &sample()));
    let vfs = &*disk.vfs;
    let mut held = vfs.open_read(&member(&file, "readme.txt")).unwrap();
    let result = write_member(vfs, &member(&file, "n"), b"x");
    if cfg!(windows) {
        // Rust opens with `FILE_SHARE_DELETE`; POSIX rename semantics take over where the file system has them.
        drop(held);
        if result.is_err() {
            write_member(vfs, &member(&file, "n"), b"x").unwrap();
        }
    } else {
        result.unwrap();
        let mut text = String::new();
        held.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello\n", "the old bytes, unharmed");
    }
    assert_eq!(read_member(vfs, &member(&file, "n")), b"x");
}

#[test]
fn a_cancelled_commit_writes_nothing() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar.gz", bytes_of(ArchiveFormat::TarGz, &sample()));
    let original = stored(&memory, "a.tar.gz");
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&file, "n"), b"x").unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = vfs.finish_with(&file, Finish::Commit, &ctl(&cancel));
    assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
    assert_eq!(stored(&memory, "a.tar.gz"), original);
    assert!(
        vfs.stat(&member(&file, "n"), FollowLinks::No).is_err(),
        "a cancelled rebuild is an abort"
    );
}

/// A failed or cancelled rebuild on a real disk leaves no temporary sibling.
#[test]
fn a_cancelled_rebuild_leaves_nothing_on_disk() {
    for format in WRITABLE {
        let disk = on_disk(ArchiveLimits::default());
        let bytes = bytes_of(format, &sample());
        let name = name_of(format);
        let file = disk.put(name, &bytes);
        let vfs = &*disk.vfs;
        vfs.begin(&file).unwrap();
        write_member(vfs, &member(&file, "n"), b"x").unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let result = vfs.finish_with(&file, Finish::Commit, &ctl(&cancel));
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        assert_eq!(siblings(&disk), [name]);
        assert_eq!(std::fs::read(disk.dir.join(name)).unwrap(), bytes);
    }
}

/// A removal of the second phase that fails fails its item: the copy stays, in the archive.
#[test]
fn a_source_that_cannot_be_removed_fails_its_item_and_the_copy_stays() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    memory
        .mem
        .insert_file(&host("/src.txt"), &b"moved"[..], MemMeta::default())
        .unwrap();
    memory.mem.inject(hikasa_vfs::Fault {
        path: host("/src.txt"),
        op: VfsOp::Remove,
        kind: ErrorKind::PermissionDenied,
    });
    let vfs = &*memory.vfs;
    let pairs = [(common::mspec("/src.txt"), member(&file, "moved.txt"))];
    let plan = ops::plan_move(vfs, vfs, &pairs, &MoveOptions::default(), &Ctl::detached()).unwrap();
    let report = ops::execute(vfs, vfs, plan, &Ctl::detached());
    assert!(!report.ok());
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(read_member(vfs, &member(&file, "moved.txt")), b"moved");
    assert!(memory.mem.read_back(&host("/src.txt")).is_some());
}

/// The source changed while the rebuild ran: it is not the file that was copied, so it stays.
#[test]
fn a_source_that_changed_during_the_rebuild_stays() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &sample()));
    memory
        .mem
        .insert_file(&host("/s/one"), &b"one"[..], MemMeta::default())
        .unwrap();
    let vfs = &*memory.vfs;
    let source = common::mspec("/s/one");
    let target = member(&file, "one");
    let mut plan = ops::plan_move(
        vfs,
        vfs,
        &[(source.clone(), target)],
        &MoveOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(plan.items[0].action, Action::Move);
    plan.items[0].replaces = false;
    struct Racing<'a> {
        inner: &'a dyn Vfs,
        mem: &'a MemoryVfs,
        path: String,
    }
    impl Vfs for Racing<'_> {
        fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
            self.inner.capabilities(at)
        }
        fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
            self.inner.stat(spec, follow)
        }
        fn list(
            &self,
            dir: &Spec,
            opts: &ListOptions<'_>,
            cancel: &CancelToken,
        ) -> Result<Listing, VfsError> {
            self.inner.list(dir, opts, cancel)
        }
        fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
            self.inner.open_read(spec)
        }
        fn open_write(
            &self,
            spec: &Spec,
            opts: &WriteOptions,
        ) -> Result<Box<dyn AtomicWrite>, VfsError> {
            self.inner.open_write(spec, opts)
        }
        fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
            self.inner.child(dir, name)
        }
        fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
            self.inner.remove(spec)
        }
        fn begin(&self, at: &Spec) -> Result<(), VfsError> {
            self.inner.begin(at)
        }
        fn finish_with(&self, at: &Spec, how: Finish, ctl: &Ctl<'_>) -> Result<(), VfsError> {
            let result = self.inner.finish_with(at, how, ctl);
            self.mem
                .insert_file(&self.path, &b"written meanwhile"[..], MemMeta::default())
                .unwrap();
            result
        }
    }
    let racing = Racing {
        inner: vfs,
        mem: &memory.mem,
        path: host("/s/one"),
    };
    let report = ops::execute(&racing, &racing, plan, &Ctl::detached());
    assert_eq!(
        report.results[0].status,
        ItemStatus::Failed,
        "{:?}",
        report.results
    );
    let error = report.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("changed during the move"),
        "{error:?}"
    );
    assert_eq!(
        &*memory.mem.read_back(&host("/s/one")).unwrap(),
        b"written meanwhile"
    );
    assert_eq!(read_member(vfs, &member(&file, "one")), b"one");
}

/// `AtomicWrite: Seek` (§1.5.10): a local writer and a memory one patch what they wrote; plain writes do not notice.
#[test]
fn writers_seek() {
    use std::io::{Seek, SeekFrom};
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let local = hikasa_vfs::LocalVfs::default();
    let spec = common::archives::local(&root.join("f"));
    let memory = MemoryVfs::default();
    memory.insert_dir(&host("/d"), MemMeta::default()).unwrap();
    let in_memory = common::mspec("/d/f");
    for (vfs, spec) in [
        (&local as &dyn Vfs, &spec),
        (&memory as &dyn Vfs, &in_memory),
    ] {
        let mut writer = vfs.open_write(spec, &WriteOptions::default()).unwrap();
        writer.write_all(b"hello world").unwrap();
        assert_eq!(writer.stream_position().unwrap(), 11);
        writer.seek(SeekFrom::Start(6)).unwrap();
        writer.write_all(b"W").unwrap();
        assert_eq!(writer.seek(SeekFrom::End(0)).unwrap(), 11);
        writer.write_all(b"!").unwrap();
        writer.commit().unwrap();
        assert_eq!(read_member(vfs, spec), b"hello World!");
    }
    let mut writer = memory
        .open_write(
            &in_memory,
            &WriteOptions {
                overwrite: true,
                ..WriteOptions::default()
            },
        )
        .unwrap();
    assert!(writer.seek(SeekFrom::Current(-1)).is_err());
}

/// A pipe has no position: `DirectWriter` answers `Unsupported`.
#[cfg(unix)]
#[test]
fn a_device_writer_does_not_seek() {
    use std::io::{Seek, SeekFrom};
    let local = hikasa_vfs::LocalVfs::default();
    let null = common::lspec(std::path::Path::new("/dev/null"));
    let options = WriteOptions {
        overwrite: true,
        ..WriteOptions::default()
    };
    let mut writer = local.open_write(&null, &options).unwrap();
    let error = writer.seek(SeekFrom::Start(0)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    writer.write_all(b"x").unwrap();
    writer.commit().unwrap();
}

#[test]
fn single_streams_stay_read_only_through_ops() {
    let memory = on_memory(ArchiveLimits::default());
    let stream = memory.put(
        "x.txt.gz",
        common::archives::compress(ArchiveFormat::Gz, b"data"),
    );
    let vfs = &*memory.vfs;
    let report = rm_all(vfs, &[member(&stream, "x.txt")]);
    assert!(!report.ok());
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    let opts = TouchOptions {
        times: SetTimes {
            modified: Some(ft(5)),
            created: None,
        },
        create: false,
    };
    let plan = ops::plan_touch(vfs, &[member(&stream, "x.txt")], &opts);
    assert!(!ops::execute(vfs, vfs, plan, &Ctl::detached()).ok());
}

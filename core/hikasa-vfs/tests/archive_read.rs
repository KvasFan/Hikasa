//! A5.3 and A5.4: every format, on `MemoryVfs` and `LocalVfs` containers, and the content reads of D-139.

mod common;

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use common::archives::{
    self, RawEntry, RawTar, STREAM_CONTENT, STREAM_MEMBER, TreeData, bytes_of, compress,
    masks_with, name_of, on_disk, on_memory, pax, raw_tar, raw_zip, tar_bytes, tree, zip_bytes,
};
use hikasa_core::{CancelToken, Classify, ErrorKind, FileTime, Spec};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, Attrs, Capabilities, EntryKind, FileName, FollowLinks,
    ListOptions, LocalVfs, ReadSeek, Vfs, VfsError, VfsRouter,
};

fn never() -> CancelToken {
    CancelToken::never()
}

fn read_all(vfs: &dyn Vfs, spec: &Spec) -> Vec<u8> {
    let mut out = Vec::new();
    vfs.open_read(spec)
        .unwrap_or_else(|e| panic!("{spec}: {e}"))
        .read_to_end(&mut out)
        .unwrap();
    out
}

/// Every member below `dir`, as `path → content` (folders with a `/` at the end, no content).
fn walk(vfs: &dyn Vfs, dir: &Spec) -> TreeData {
    fn go(vfs: &dyn Vfs, dir: &Spec, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        let mut listing = vfs
            .list(dir, &ListOptions::default(), &never())
            .unwrap_or_else(|e| panic!("{dir}: {e}"));
        listing.sort_by_name();
        assert!(listing.errors.is_empty(), "{:?}", listing.errors);
        for entry in listing.entries {
            let child = vfs.child(dir, &entry.name).unwrap();
            let path = format!("{prefix}{}", entry.name);
            let stated = vfs.stat(&child, FollowLinks::No).unwrap();
            assert_eq!(stated, entry, "stat equals the listing");
            match entry.kind {
                EntryKind::Dir => {
                    let mut seen = BTreeMap::new();
                    go(vfs, &child, &format!("{path}/"), &mut seen);
                    if seen.is_empty() {
                        out.insert(format!("{path}/"), Vec::new());
                    }
                    out.extend(seen);
                }
                _ => {
                    out.insert(path, read_all(vfs, &child));
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    go(vfs, dir, "", &mut out);
    out.into_iter().collect()
}

fn sorted(tree: &TreeData) -> TreeData {
    let mut tree = tree.clone();
    tree.sort();
    tree
}

/// Formats of the tar family and zip.
const TREES: [ArchiveFormat; 6] = [
    ArchiveFormat::Zip,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarBz2,
    ArchiveFormat::TarXz,
    ArchiveFormat::TarZst,
];
const STREAMS: [ArchiveFormat; 4] = [
    ArchiveFormat::Gz,
    ArchiveFormat::Bz2,
    ArchiveFormat::Xz,
    ArchiveFormat::Zst,
];

#[test]
fn every_format_on_memory_and_on_disk() {
    let long: TreeData = vec![
        (format!("{}/x", "d".repeat(150)), b"long".to_vec()),
        ("n".repeat(300), b"long name".to_vec()),
    ];
    let trees = [tree(), long];
    for format in TREES {
        for data in &trees {
            let bytes = bytes_of(format, data);
            let memory = on_memory(ArchiveLimits::default());
            let root = memory.root(name_of(format), bytes.clone());
            assert_eq!(
                walk(&*memory.vfs, &root),
                sorted(data),
                "{format:?}, memory"
            );
            let disk = on_disk(ArchiveLimits::default());
            let root = Spec::archive_member(disk.put(name_of(format), &bytes), "");
            assert_eq!(walk(&*disk.vfs, &root), sorted(data), "{format:?}, disk");
        }
    }
    for format in STREAMS {
        let bytes = bytes_of(format, &tree());
        let want = vec![(STREAM_MEMBER.to_owned(), STREAM_CONTENT.to_vec())];
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name_of(format), bytes.clone());
        assert_eq!(walk(&*memory.vfs, &root), want, "{format:?}, memory");
        let disk = on_disk(ArchiveLimits::default());
        let root = Spec::archive_member(disk.put(name_of(format), &bytes), "");
        assert_eq!(walk(&*disk.vfs, &root), want, "{format:?}, disk");
    }
}

#[test]
fn every_zip_method_reads() {
    use zip::CompressionMethod as M;
    // Deflate64 and LZMA have no writer in the crate.
    for method in [M::Stored, M::Deflated, M::Bzip2, M::Zstd, M::Xz] {
        let bytes = archives::zip_with(&tree(), method);
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root("a.zip", bytes);
        assert_eq!(walk(&*memory.vfs, &root), sorted(&tree()), "{method:?}");
    }
}

#[test]
fn nesting_two_and_three_levels() {
    let inner: TreeData = vec![("c.txt".into(), b"deep".to_vec())];
    let tgz = compress(ArchiveFormat::TarGz, &tar_bytes(&inner));
    let middle: TreeData = vec![("d/b.tar.gz".into(), tgz)];
    let zip = zip_bytes(&middle);
    let memory = on_memory(ArchiveLimits::default());
    let a = memory.put("a.zip", zip.clone());
    let b = Spec::archive_member(a.clone(), "d/b.tar.gz");
    let c = Spec::archive_member(b.clone(), "c.txt");
    assert_eq!(read_all(&*memory.vfs, &c), b"deep");
    assert_eq!(c.archive_depth(), 2);
    let listing = memory
        .vfs
        .list(
            &Spec::archive_member(b, ""),
            &ListOptions::default(),
            &never(),
        )
        .unwrap();
    assert_eq!(listing.entries[0].name.as_str(), "c.txt");

    let outer: TreeData = vec![("a.zip".into(), zip)];
    let tar = tar_bytes(&outer);
    let t = memory.put("t.tar", tar);
    let a = Spec::archive_member(t, "a.zip");
    let b = Spec::archive_member(a, "d/b.tar.gz");
    let c = Spec::archive_member(b, "c.txt");
    assert_eq!(read_all(&*memory.vfs, &c), b"deep");
    assert_eq!(c.archive_depth(), 3);
}

#[test]
fn concatenated_streams_read_to_the_end() {
    for format in STREAMS {
        let mut bytes = compress(format, b"first ");
        bytes.extend(compress(format, b"second"));
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name_of(format), bytes);
        let member = memory
            .vfs
            .child(&root, &FileName::new(STREAM_MEMBER).unwrap())
            .unwrap();
        assert_eq!(
            read_all(&*memory.vfs, &member),
            b"first second",
            "{format:?}"
        );
        let entry = memory.vfs.stat(&member, FollowLinks::No).unwrap();
        assert_eq!(entry.size, 12);
        assert_eq!(entry.crc32, Some(crc32fast::hash(b"first second")));
    }
}

#[test]
fn single_stream_member_names() {
    let memory = on_memory(ArchiveLimits::default());
    let cases = [
        ("x.txt.gz", "x.txt"),
        ("noext", "noext.data"),
        ("x.y.gz", "x.y"),
    ];
    let mut config = archives::config(memory.dir.path(), ArchiveLimits::default());
    config.masks = masks_with(|f| (f == ArchiveFormat::Gz).then_some("*.gz;noext"));
    memory.vfs.set_config(config);
    for (name, member) in cases {
        let root = memory.root(name, compress(ArchiveFormat::Gz, b"x"));
        let listing = memory
            .vfs
            .list(&root, &ListOptions::default(), &never())
            .unwrap();
        assert_eq!(listing.entries[0].name.as_str(), member, "{name}");
    }
}

#[test]
fn masks_decide() {
    let memory = on_memory(ArchiveLimits::default());
    let data = vec![("x".to_owned(), b"1".to_vec())];
    let upper = memory.root("UP.JAR", zip_bytes(&data));
    assert_eq!(walk(&*memory.vfs, &upper), data);

    // A disabled tar_gz makes x.tar.gz a one-member gz.
    let mut config = archives::config(memory.dir.path(), ArchiveLimits::default());
    config.masks = masks_with(|f| (f == ArchiveFormat::TarGz).then_some(""));
    memory.vfs.set_config(config);
    assert_eq!(
        memory.vfs.archive_format("x.tar.gz"),
        Some(ArchiveFormat::Gz)
    );
    let tgz = compress(ArchiveFormat::TarGz, &tar_bytes(&data));
    let root = memory.root("x.tar.gz", tgz.clone());
    let listing = memory
        .vfs
        .list(&root, &ListOptions::default(), &never())
        .unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name.as_str(), "x.tar");

    // A custom mask, and an empty one.
    let mut config = archives::config(memory.dir.path(), ArchiveLimits::default());
    config.masks = masks_with(|f| match f {
        ArchiveFormat::Zip => Some("*.mine"),
        ArchiveFormat::Tar => Some(""),
        _ => None,
    });
    memory.vfs.set_config(config);
    assert_eq!(
        memory.vfs.archive_format("a.mine"),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(memory.vfs.archive_format("a.zip"), None);
    assert_eq!(memory.vfs.archive_format("a.tar"), None);
}

#[test]
fn capability_table() {
    use Capabilities as C;
    let common = C::READ | C::LIST | C::SEEK | C::MTIME | C::CASE_SENSITIVE;
    let write = C::WRITE
        | C::MKDIR
        | C::REMOVE
        | C::RENAME
        | C::SET_TIMES
        | C::SET_ATTRS
        | C::CREATE_SYMLINKS;
    let zip_read = common | C::CRC_PROVIDED | C::DOS_ATTRS | C::UNIX_MODE | C::SYMLINKS;
    let tar_read = common | C::UNIX_MODE | C::UNIX_OWNER | C::SYMLINKS;
    let (zip, tar) = (zip_read | write, tar_read | write);
    let memory = on_memory(ArchiveLimits::default());
    for format in ArchiveFormat::ORDER {
        let member = Spec::archive_member(common::mspec(&format!("/{}", name_of(format))), "x");
        let want = match format {
            ArchiveFormat::Zip => zip,
            f if f.is_single_stream() => common,
            _ => tar,
        };
        assert_eq!(
            memory.vfs.capabilities(&member).unwrap(),
            want,
            "{format:?}"
        );
    }
    // A member of a nested archive gets the bits of the innermost format, and the archive is not opened.
    let a = Spec::archive_member(common::mspec("/missing.zip"), "b.tar.gz");
    let nested = Spec::archive_member(a, "x");
    assert_eq!(memory.vfs.capabilities(&nested).unwrap(), tar);
    let a = Spec::archive_member(common::mspec("/missing.tar"), "b.zip");
    assert_eq!(
        memory
            .vfs
            .capabilities(&Spec::archive_member(a, ""))
            .unwrap(),
        zip
    );
    // The write bits only when every archive around it is writable (§1.5.3).
    let a = Spec::archive_member(common::mspec("/x.tar.gz"), "b.gz");
    let below_a_stream = Spec::archive_member(Spec::archive_member(a, "b"), "x");
    assert_eq!(memory.vfs.capabilities(&below_a_stream).unwrap(), common);
    let a = Spec::archive_member(common::mspec("/x.gz"), "b.tar");
    let in_a_stream = Spec::archive_member(a, "x");
    assert_eq!(memory.vfs.capabilities(&in_a_stream).unwrap(), tar_read);
}

#[test]
fn router_table() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &zip_bytes(&vec![("x".to_owned(), b"1".to_vec())]));
    let member = Spec::archive_member(file.clone(), "x");
    let root = Spec::archive_member(file.clone(), "");
    let plain = VfsRouter::new(LocalVfs::default());
    let router = plain.clone().with_archives(Arc::clone(&disk.vfs));

    assert!(matches!(
        plain.stat(&member, FollowLinks::No),
        Err(VfsError::Unsupported {
            what: "archive specs",
            ..
        })
    ));
    assert_eq!(plain.archive_format("a.zip"), None);
    assert!(plain.capabilities(&member).is_err());

    assert_eq!(router.stat(&member, FollowLinks::No).unwrap().size, 1);
    assert_eq!(
        router
            .list(&root, &ListOptions::default(), &never())
            .unwrap()
            .entries
            .len(),
        1
    );
    assert_eq!(read_all(&router, &member), b"1");
    assert_eq!(router.local_path(&member), None);
    assert_eq!(router.archive_format("a.zip"), Some(ArchiveFormat::Zip));
    assert_eq!(
        router
            .child(&root, &FileName::new("y").unwrap())
            .unwrap()
            .canonical(),
        format!("{file}/y")
    );
    assert_eq!(
        router
            .copy_file_fast(&member, &member, &Default::default(), &never())
            .unwrap(),
        None
    );

    // A recycle bin is one thing a member never has (§1.5.17); two archives and mixed kinds are the router's table.
    let unsupported = |r: Result<(), VfsError>| {
        assert!(
            matches!(r, Err(VfsError::OperationUnsupported { .. })),
            "{r:?}"
        );
    };
    unsupported(router.move_to_trash(&member));
    let other_archive = Spec::archive_member(disk.put("b.zip", b""), "x");
    assert!(matches!(
        router.rename(&member, &other_archive, false),
        Err(VfsError::CrossDevice { .. })
    ));
    unsupported(router.rename(&member, &file, false));
    unsupported(router.rename(&file, &member, false));
    assert!(router.take_warnings().is_empty());
}

#[test]
fn child_of_root_and_folder() {
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.zip", zip_bytes(&tree()));
    let dir = memory
        .vfs
        .child(&root, &FileName::new("dir").unwrap())
        .unwrap();
    assert_eq!(dir, Spec::archive_member(common::mspec("/a.zip"), "dir"));
    let sub = memory
        .vfs
        .child(&dir, &FileName::new("sub").unwrap())
        .unwrap();
    assert_eq!(
        sub,
        Spec::archive_member(common::mspec("/a.zip"), "dir/sub")
    );
    for bad in [
        FileName::from_os("a/b".as_ref()),
        FileName::from_os("..".as_ref()),
    ] {
        assert!(matches!(
            memory.vfs.child(&root, &bad),
            Err(VfsError::InvalidName { .. })
        ));
    }
}

/// §1.5.5: every column of `Entry`, the root's entry and synthesized folders.
#[test]
fn entry_columns() {
    let memory = on_memory(ArchiveLimits::default());
    let text = b"content".to_vec();
    let bytes = raw_tar(&[
        RawTar::file(b"d/f.txt", &text),
        RawTar::link(b"lnk", b'2', b"d/f.txt"),
    ]);
    let file = memory.put("a.tar", bytes);
    let vfs = &*memory.vfs;
    let root = Spec::archive_member(file.clone(), "");
    let entry = vfs.stat(&root, FollowLinks::No).unwrap();
    let container = vfs.stat(&file, FollowLinks::No).unwrap();
    assert_eq!(entry.name.as_str(), "a.tar");
    assert_eq!(entry.kind, EntryKind::Dir);
    assert_eq!(entry.size, 0);
    assert_eq!(entry.mtime, container.mtime);

    let folder = vfs
        .stat(&Spec::archive_member(file.clone(), "d"), FollowLinks::No)
        .unwrap();
    assert_eq!(
        (folder.kind, folder.mtime, folder.size),
        (EntryKind::Dir, None, 0)
    );

    let member = vfs
        .stat(
            &Spec::archive_member(file.clone(), "d/f.txt"),
            FollowLinks::No,
        )
        .unwrap();
    assert_eq!(member.kind, EntryKind::File);
    assert_eq!(member.size, 7);
    assert_eq!(member.mtime, FileTime::from_unix(1_792_415_520, 0));
    assert_eq!(member.crc32, Some(crc32fast::hash(&text)));
    assert_eq!(member.file_id, None);
    assert!(matches!(member.attrs, Attrs::Unix { meta } if meta.mode == 0o644));

    let link = vfs
        .stat(&Spec::archive_member(file, "lnk"), FollowLinks::Yes)
        .unwrap();
    assert_eq!(link.kind, EntryKind::Symlink);
    assert_eq!(link.link_target.as_deref(), Some("d/f.txt"));
    assert!(!link.followed);

    // Zip: the central CRC, a Unix mode or DOS attributes, a link's content as its target.
    let zip = raw_zip(&[
        RawEntry::new(b"plain", b"abc"),
        RawEntry::new(b"unix", b"abc").unix(0o100_640),
        RawEntry::new(b"lnk", b"plain").unix(0o120_777),
        RawEntry::new(b"dir/", b""),
    ]);
    let file = memory.put("z.zip", zip);
    let at = |name: &str| {
        vfs.stat(&Spec::archive_member(file.clone(), name), FollowLinks::Yes)
            .unwrap()
    };
    assert!(matches!(at("plain").attrs, Attrs::Dos { .. }));
    assert_eq!(at("plain").crc32, Some(crc32fast::hash(b"abc")));
    assert!(
        matches!(at("unix").attrs, Attrs::Unix { meta } if meta.mode == 0o640 && meta.uid == 0)
    );
    assert_eq!(at("lnk").kind, EntryKind::Symlink);
    assert_eq!(at("lnk").link_target.as_deref(), Some("plain"));
    assert_eq!(at("dir").kind, EntryKind::Dir);
    assert_eq!(at("dir").crc32, None);
    assert!(memory.vfs.take_warnings().is_empty());
}

/// `FollowLinks::Yes` leaves a link member a link, without an `EntryError`.
#[test]
fn follow_links_leaves_a_link() {
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root(
        "a.tar",
        raw_tar(&[RawTar::link(b"lnk", b'2', b"/etc/passwd")]),
    );
    let options = ListOptions {
        follow: FollowLinks::Yes,
        prefilter: None,
    };
    let listing = memory.vfs.list(&root, &options, &never()).unwrap();
    assert_eq!(listing.entries[0].kind, EntryKind::Symlink);
    assert!(!listing.entries[0].followed);
    assert!(listing.errors.is_empty());
}

#[test]
fn prefilter_is_honoured() {
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root(
        "a.tar",
        raw_tar(&[
            RawTar::file(b"keep", b"1"),
            RawTar::file(b"drop", b"2"),
            RawTar::link(b"lnk", b'2', b"keep"),
        ]),
    );
    let accept = |name: &str, _: EntryKind| name == "keep";
    let options = ListOptions {
        follow: FollowLinks::No,
        prefilter: Some(&accept),
    };
    let mut listing = memory.vfs.list(&root, &options, &never()).unwrap();
    listing.sort_by_name();
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["keep", "lnk"],
        "links are never passed to the prefilter"
    );
    assert_eq!(listing.name_only[0].name.as_str(), "drop");
}

// ---- content reads -------------------------------------------------------------------------------------------

/// The seek and read table every reader passes.
fn check_reader(mut reader: Box<dyn ReadSeek>, data: &[u8]) {
    let len = data.len() as u64;
    let mut all = Vec::new();
    reader.read_to_end(&mut all).unwrap();
    assert_eq!(all, data);
    assert_eq!(reader.read(&mut [0u8; 8]).unwrap(), 0, "at the end");
    assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), len);
    assert_eq!(reader.seek(SeekFrom::Start(0)).unwrap(), 0);
    let mut head = vec![0u8; 3.min(data.len())];
    reader.read_exact(&mut head).unwrap();
    assert_eq!(head, data[..head.len()]);
    if len >= 10 {
        assert_eq!(reader.seek(SeekFrom::End(-4)).unwrap(), len - 4);
        let mut tail = [0u8; 4];
        reader.read_exact(&mut tail).unwrap();
        assert_eq!(tail, data[data.len() - 4..]);
        assert_eq!(reader.seek(SeekFrom::Current(-8)).unwrap(), len - 8);
        let mut middle = [0u8; 2];
        reader.read_exact(&mut middle).unwrap();
        assert_eq!(middle, data[data.len() - 8..data.len() - 6]);
        assert_eq!(reader.seek(SeekFrom::Start(5)).unwrap(), 5);
        let mut one = [0u8; 1];
        reader.read_exact(&mut one).unwrap();
        assert_eq!(one[0], data[5]);
    }
    assert_eq!(reader.seek(SeekFrom::Start(len + 100)).unwrap(), len + 100);
    assert_eq!(reader.read(&mut [0u8; 8]).unwrap(), 0, "past the end");
}

fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn zip_members_take_memory_and_the_cache() {
    let small = noise(1 << 20);
    let large = noise((1 << 20) + 1);
    let data = vec![
        ("small".to_owned(), small.clone()),
        ("large".to_owned(), large.clone()),
    ];
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", zip_bytes(&data));
    let vfs = &memory.vfs;

    let before = vfs.counters();
    check_reader(
        vfs.open_read(&Spec::archive_member(file.clone(), "small"))
            .unwrap(),
        &small,
    );
    let after = vfs.counters();
    assert_eq!(after.memory_reads, before.memory_reads + 1);
    assert_eq!(vfs.cache_used(), 0);

    check_reader(
        vfs.open_read(&Spec::archive_member(file.clone(), "large"))
            .unwrap(),
        &large,
    );
    let cached = vfs.counters();
    assert_eq!(
        cached.memory_reads, after.memory_reads,
        "1 MiB + 1 byte is not in memory"
    );
    assert_eq!(vfs.cache_used(), large.len() as u64);

    // Reuse.
    check_reader(
        vfs.open_read(&Spec::archive_member(file, "large")).unwrap(),
        &large,
    );
    let again = vfs.counters();
    assert_eq!(again.member_decodes, cached.member_decodes);
    assert_eq!(again.cache_hits, cached.cache_hits + 1);
}

#[test]
fn plain_tar_is_a_range() {
    let data = vec![
        ("a".to_owned(), noise(5000)),
        ("b".to_owned(), noise(70_000)),
    ];
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", tar_bytes(&data));
    for (name, content) in &data {
        let reader = memory
            .vfs
            .open_read(&Spec::archive_member(file.clone(), name))
            .unwrap();
        check_reader(reader, content);
    }
    let counters = memory.vfs.counters();
    assert_eq!(counters.range_reads, 2);
    assert_eq!(counters.member_decodes, 0);
    assert_eq!(memory.vfs.cache_used(), 0);
    assert_eq!(memory.vfs.cache_dir(), None, "no cache file");
}

#[test]
fn compressed_tar_spools_then_writes_the_stream_once() {
    let data: TreeData = (0..4).map(|i| (format!("m{i}"), noise(3 << 20))).collect();
    for format in [ArchiveFormat::TarGz, ArchiveFormat::TarZst] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), compress(format, &tar_bytes(&data)));
        let vfs = &memory.vfs;
        let member = |i: usize| Spec::archive_member(file.clone(), format!("m{i}"));

        // The first read decodes up to its member and spools it.
        check_reader(vfs.open_read(&member(1)).unwrap(), &data[1].1);
        let first = vfs.counters();
        assert_eq!((first.member_decodes, first.stream_writes), (1, 0));
        assert_eq!(vfs.cache_used(), 3 << 20);

        // Same member: a hit.
        check_reader(vfs.open_read(&member(1)).unwrap(), &data[1].1);
        assert_eq!(vfs.counters().member_decodes, 1);

        // The second distinct read writes the whole stream once.
        check_reader(vfs.open_read(&member(2)).unwrap(), &data[2].1);
        let second = vfs.counters();
        assert_eq!((second.member_decodes, second.stream_writes), (2, 1));

        // Later reads decode nothing.
        for i in [0, 3, 2, 1] {
            check_reader(vfs.open_read(&member(i)).unwrap(), &data[i].1);
        }
        let last = vfs.counters();
        assert_eq!((last.member_decodes, last.stream_writes), (2, 1));
        assert_eq!(last.index_passes, 1, "one index pass");
    }
}

#[test]
fn a_stream_bigger_than_the_cache_keeps_spooling() {
    let data: TreeData = (0..3).map(|i| (format!("m{i}"), noise(2 << 20))).collect();
    let limits = ArchiveLimits {
        cache_max: 3 << 20,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put(
        "a.tar.gz",
        compress(ArchiveFormat::TarGz, &tar_bytes(&data)),
    );
    let vfs = &memory.vfs;
    for i in [0usize, 1, 2, 0] {
        let member = Spec::archive_member(file.clone(), format!("m{i}"));
        check_reader(vfs.open_read(&member).unwrap(), &data[i].1);
        assert!(vfs.cache_used() <= 3 << 20);
    }
    let counters = vfs.counters();
    assert_eq!(counters.stream_writes, 0);
    assert_eq!(counters.member_decodes, 4, "the first file was evicted");
}

/// A sound stream far past the cache and near `stream_max`: every member reads, however often the stream is decoded
/// from its start (rule 12 binds one pass).
#[test]
fn a_tar_gz_over_the_cache_reads_every_member() {
    let data: TreeData = (0..16)
        .map(|i| (format!("m{i:02}"), noise(2 << 20)))
        .collect();
    let limits = ArchiveLimits {
        stream_max: 40 << 20,
        cache_max: 8 << 20,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put(
        "big.tar.gz",
        compress(ArchiveFormat::TarGz, &tar_bytes(&data)),
    );
    for (name, content) in &data {
        let member = Spec::archive_member(file.clone(), name.as_str());
        check_reader(memory.vfs.open_read(&member).unwrap(), content);
    }
    assert_eq!(memory.vfs.counters().stream_writes, 0);
}

#[test]
fn eight_threads_get_the_same_bytes() {
    let data: TreeData = (0..16)
        .map(|i| (format!("m{i}"), noise(40_000 + i * 3000)))
        .collect();
    for format in [ArchiveFormat::Zip, ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name_of(format), bytes_of(format, &data));
        std::thread::scope(|scope| {
            for thread in 0..8usize {
                let (vfs, file, data) = (&memory.vfs, &file, &data);
                scope.spawn(move || {
                    for i in 0..data.len() {
                        let (name, content) = &data[(i + thread * 5) % data.len()];
                        let spec = Spec::archive_member(file.clone(), name);
                        assert_eq!(read_all(&**vfs, &spec), *content);
                    }
                });
            }
        });
        assert_eq!(memory.vfs.counters().index_passes, 1, "{format:?}");
    }
}

#[test]
fn stream_member_reads_are_reused() {
    let content = noise(2 << 20);
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("x.bin.gz", compress(ArchiveFormat::Gz, &content));
    let member = memory
        .vfs
        .child(&root, &FileName::new("x.bin").unwrap())
        .unwrap();
    check_reader(memory.vfs.open_read(&member).unwrap(), &content);
    check_reader(memory.vfs.open_read(&member).unwrap(), &content);
    assert_eq!(memory.vfs.counters().member_decodes, 1);
}

#[test]
fn folder_and_special_reads() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.tar",
        raw_tar(&[
            RawTar::typed(b"dir/", b'5'),
            RawTar::link(b"lnk", b'2', b"dir"),
            RawTar::typed(b"dev", b'3'),
            RawTar::typed(b"fifo", b'6'),
            RawTar::typed(b"sparse", b'S'),
        ]),
    );
    let open = |name: &str| {
        memory
            .vfs
            .open_read(&Spec::archive_member(file.clone(), name))
            .map(drop)
    };
    assert!(matches!(open(""), Err(VfsError::IsADirectory { .. })));
    assert!(matches!(open("dir"), Err(VfsError::IsADirectory { .. })));
    for name in ["lnk", "dev", "fifo", "sparse"] {
        assert!(
            matches!(open(name), Err(VfsError::OperationUnsupported { .. })),
            "{name}"
        );
    }
    assert!(matches!(open("missing"), Err(VfsError::NotFound { .. })));
    assert!(matches!(open("dev/x"), Err(VfsError::NotADirectory { .. })));
}

// ---- tar_walk ------------------------------------------------------------------------------------------------

#[test]
fn tar_walk_extensions() {
    let mut tar = pax(&[
        ("size", b"3"),
        ("path", b"from/pax.txt"),
        ("mtime", b"1792415520.5"),
    ]);
    tar.extend(
        RawTar {
            size: Some(99),
            ..RawTar::file(b"ignored", b"abc")
        }
        .bytes(),
    );
    // GNU long name and long link.
    let long = "l".repeat(200);
    let mut name = long.clone().into_bytes();
    name.push(0);
    tar.extend(
        RawTar {
            data: name,
            ..RawTar::typed(b"././@LongLink", b'L')
        }
        .bytes(),
    );
    tar.extend(RawTar::file(b"short", b"xy").bytes());
    let mut link = long.clone().into_bytes();
    link.push(0);
    tar.extend(
        RawTar {
            data: link,
            ..RawTar::typed(b"././@LongLink", b'K')
        }
        .bytes(),
    );
    tar.extend(RawTar::link(b"lnk", b'2', b"short").bytes());
    // A global header and a volume label are skipped, base-256 sizes read.
    tar.extend(
        RawTar {
            data: b"12 size=9\n".to_vec(),
            ..RawTar::typed(b"g", b'g')
        }
        .bytes(),
    );
    tar.extend(RawTar::typed(b"label", b'V').bytes());
    let mut header = RawTar::file(b"big", b"12345").header();
    header[124..136].copy_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5]);
    archives::seal(&mut header);
    tar.extend_from_slice(&header);
    let mut body = b"12345".to_vec();
    archives::pad(&mut body);
    tar.extend(body);
    // An old V7 header.
    tar.extend(
        RawTar {
            ustar: false,
            type_flag: 0,
            ..RawTar::file(b"old", b"v7")
        }
        .bytes(),
    );
    tar.extend_from_slice(&[0u8; 1024]);

    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", tar);
    let vfs = &memory.vfs;
    let member = |name: &str| Spec::archive_member(file.clone(), name);
    let pax_member = vfs.stat(&member("from/pax.txt"), FollowLinks::No).unwrap();
    assert_eq!(pax_member.size, 3, "PAX size over the header");
    assert_eq!(
        pax_member.mtime,
        FileTime::from_unix(1_792_415_520, 500_000_000)
    );
    assert_eq!(read_all(&**vfs, &member("from/pax.txt")), b"abc");
    assert_eq!(read_all(&**vfs, &member(&long)), b"xy");
    assert_eq!(read_all(&**vfs, &member("big")), b"12345");
    assert_eq!(read_all(&**vfs, &member("old")), b"v7");
    let link = vfs.stat(&member("lnk"), FollowLinks::No).unwrap();
    assert_eq!(link.link_target.as_deref(), Some(long.as_str()));
    let listing = vfs
        .list(&member(""), &ListOptions::default(), &never())
        .unwrap();
    assert!(listing.errors.is_empty(), "{:?}", listing.errors);
    assert_eq!(listing.entries.len(), 5, "{:?}", listing.entries);
}

#[test]
fn an_extension_body_over_1_mib_is_damage() {
    for (flag, name) in [(b'x', "pax"), (b'L', "long name"), (b'K', "long link")] {
        let good = RawTar::file(b"before", b"ok").bytes();
        let mut tar = good.clone();
        let oversized = vec![b'a'; (1 << 20) + 1];
        tar.extend(
            RawTar {
                data: oversized,
                ..RawTar::typed(b"ext", flag)
            }
            .bytes(),
        );
        tar.extend(RawTar::file(b"after", b"no").bytes());
        tar.extend_from_slice(&[0u8; 1024]);
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root("a.tar", tar);
        let listing = memory
            .vfs
            .list(&root, &ListOptions::default(), &never())
            .unwrap();
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["before"], "{name}");
        assert_eq!(listing.errors.len(), 1, "{name}");
        assert_eq!(listing.errors[0].name.as_str(), "<damaged>");
        assert_eq!(listing.errors[0].error.kind, ErrorKind::InvalidData);
    }
}

#[test]
fn gnu_sparse_is_listed_and_not_read() {
    let mut record = RawTar::file(b"holes", b"data");
    record.type_flag = b'S';
    let mut header = record.header();
    header[257..265].copy_from_slice(b"ustar  \0");
    header[483..495].copy_from_slice(b"00000010000\0");
    header[482] = 1;
    archives::seal(&mut header);
    let mut tar = header.to_vec();
    tar.extend_from_slice(&[0u8; 512]); // the extension block, last flag clear
    tar.extend(b"data");
    archives::pad(&mut tar);
    tar.extend(RawTar::file(b"next", b"ok").bytes());
    tar.extend_from_slice(&[0u8; 1024]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", tar);
    let member = |name: &str| Spec::archive_member(file.clone(), name);
    let entry = memory.vfs.stat(&member("holes"), FollowLinks::No).unwrap();
    assert_eq!((entry.kind, entry.size), (EntryKind::File, 0o10000));
    assert!(matches!(
        memory.vfs.open_read(&member("holes")),
        Err(VfsError::OperationUnsupported { .. })
    ));
    assert_eq!(read_all(&*memory.vfs, &member("next")), b"ok");

    // PAX sparse.
    let mut tar = pax(&[
        ("GNU.sparse.major", b"1"),
        ("GNU.sparse.name", b"named"),
        ("GNU.sparse.realsize", b"77"),
    ]);
    tar.extend(RawTar::file(b"GNUSparseFile/x", b"map").bytes());
    tar.extend_from_slice(&[0u8; 1024]);
    let file = memory.put("b.tar", tar);
    let spec = Spec::archive_member(file, "named");
    assert_eq!(memory.vfs.stat(&spec, FollowLinks::No).unwrap().size, 77);
    assert!(memory.vfs.open_read(&spec).is_err());
}

#[test]
fn zip_with_a_prefix_reads() {
    let data = vec![("x".to_owned(), b"1".to_vec())];
    let mut bytes = b"#!/bin/sh\nexit 0\n".to_vec();
    bytes.extend(zip_bytes(&data));
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.zip", bytes);
    assert_eq!(walk(&*memory.vfs, &root), data);
}

#[test]
fn the_kind_of_the_errors_is_stable() {
    let error = VfsError::ArchiveEncrypted { spec: "s".into() };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(
        error.to_string(),
        "s: the member is encrypted and cannot be read"
    );
    let error = VfsError::ArchiveCorrupt {
        spec: "s".into(),
        reason: "r".into(),
    };
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "s: not a valid archive: r");
    let error = VfsError::ArchiveLimit {
        spec: "s".into(),
        limit: "ratio",
        value: 2,
    };
    assert_eq!(error.kind(), ErrorKind::TooLarge);
    assert_eq!(
        error.to_string(),
        "s: archive limit exceeded (ratio: 2); see archives.limits"
    );
    let io = std::io::Error::from(error);
    assert_eq!(io.kind(), std::io::ErrorKind::FileTooLarge);
    assert_eq!(
        VfsError::from_io("x", hikasa_vfs::VfsOp::Read, io).kind(),
        ErrorKind::TooLarge
    );
}

/// §1.5.5: the tree costs at most 512 bytes per member.
#[test]
fn an_index_costs_at_most_512_bytes_per_member() {
    let names: Vec<String> = (0..4000)
        .map(|i| format!("dir{}/file{i:05}.txt", i % 50))
        .collect();
    let entries: Vec<RawEntry> = names
        .iter()
        .map(|n| RawEntry::new(n.as_bytes(), b"x"))
        .collect();
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", raw_zip(&entries));
    let root = Spec::archive_member(file.clone(), "");
    memory
        .vfs
        .list(&root, &ListOptions::default(), &never())
        .unwrap();
    let bytes = memory.vfs.index_memory(&file).unwrap();
    assert!(bytes / names.len() <= 512, "{bytes} bytes");
}

/// Row 27 on a back-end, and a real item wins over a mask.
#[test]
fn other_specs_pass_through() {
    let memory = on_memory(ArchiveLimits::default());
    let never = never();
    for spec in [Spec::Stdin, Spec::Clipboard] {
        let got = hikasa_vfs::resolve_spec(
            &*memory.vfs,
            spec.clone(),
            hikasa_vfs::SpecUse::Folder,
            &never,
        );
        assert_eq!(got.unwrap(), spec);
    }
}

/// `ops::check_item` refuses a root like a file-system root: an archive is removed as the file it is.
#[test]
fn a_root_is_not_an_item() {
    use hikasa_core::Ctl;
    use hikasa_vfs::ops::{ItemStatus, RemoveOptions, plan_remove};
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.zip", zip_bytes(&tree()));
    let options = RemoveOptions {
        recursive: true,
        ..RemoveOptions::default()
    };
    let plan = plan_remove(&*memory.vfs, &[root], &options, &Ctl::detached()).unwrap();
    assert_eq!(plan.items.len(), 1);
    let report = hikasa_vfs::ops::execute(&*memory.vfs, &*memory.vfs, plan, &Ctl::detached());
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    let error = report.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("is a root folder"),
        "{}",
        error.message
    );
}

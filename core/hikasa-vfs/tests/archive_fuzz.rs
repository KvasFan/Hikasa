//! A5.10: corrupt input never panics (§1.5.7 rules 8–9). One proptest per format of the ten, with its default case
//! count: random bytes under the format's mask, a valid fixture with random byte flips, and a cut at a random length.
//! `list`, `stat`, `open_read` and a full read of every listed member finish and return data or a typed error.

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::OnceLock;

use common::archives::{TreeData, bytes_of, compress, name_of, on_memory, tar_bytes, tree};
use hikasa_core::{CancelToken, Classify, ErrorKind, Spec, WarningCode};
use hikasa_vfs::archive::zip_read::central_times;
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, EntryKind, FollowLinks, ListOptions, Vfs, VfsError,
};
use proptest::prelude::*;

/// The kinds a hostile archive may produce (§1.5.7): `invalid_data`, `too_large`, `unsupported`, and `not_found` for a
/// path that only a damaged listing named.
fn check_kind(error: &VfsError) {
    let kind = error.kind();
    assert!(
        matches!(
            kind,
            ErrorKind::InvalidData
                | ErrorKind::TooLarge
                | ErrorKind::Unsupported
                | ErrorKind::NotFound
        ),
        "{kind:?}: {error}"
    );
}

/// Small limits, so a mutated size cannot make a case slow.
fn limits() -> ArchiveLimits {
    ArchiveLimits {
        entries_max: 4096,
        stream_max: 8 << 20,
        cache_max: 8 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    }
}

fn exercise_dir(vfs: &dyn Vfs, dir: &Spec, depth: u32) {
    let never = CancelToken::never();
    let listing = match vfs.list(dir, &ListOptions::default(), &never) {
        Ok(listing) => listing,
        Err(error) => return check_kind(&error),
    };
    for failed in &listing.errors {
        assert_eq!(failed.error.kind, ErrorKind::InvalidData);
    }
    for entry in &listing.entries {
        let Ok(child) = vfs.child(dir, &entry.name) else {
            panic!("a listed name is a child: {:?}", entry.name);
        };
        if let Err(error) = vfs.stat(&child, FollowLinks::Yes) {
            check_kind(&error);
        }
        match entry.kind {
            EntryKind::Dir if depth < 32 => exercise_dir(vfs, &child, depth + 1),
            EntryKind::Dir => {}
            _ => match vfs.open_read(&child) {
                Ok(mut reader) => {
                    let mut sink = Vec::new();
                    if let Err(error) = reader.read_to_end(&mut sink) {
                        panic!("a read after open: {error}");
                    }
                }
                Err(error) => check_kind(&error),
            },
        }
    }
}

/// Every call on an archive of these bytes, on a fresh back-end.
fn exercise(format: ArchiveFormat, bytes: Vec<u8>) {
    let memory = on_memory(limits());
    let root = memory.root(name_of(format), bytes);
    if let Err(error) = memory.vfs.stat(&root, FollowLinks::No) {
        check_kind(&error);
    }
    exercise_dir(&*memory.vfs, &root, 0);
}

/// A valid fixture of each format, built once.
fn fixture(format: ArchiveFormat) -> &'static [u8] {
    static ALL: OnceLock<BTreeMap<usize, Vec<u8>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        ArchiveFormat::ORDER
            .iter()
            .enumerate()
            .map(|(at, format)| (at, bytes_of(*format, &tree())))
            .collect()
    });
    let at = ArchiveFormat::ORDER
        .iter()
        .position(|f| *f == format)
        .unwrap();
    &all[&at]
}

/// The three inputs of one case.
fn inputs(format: ArchiveFormat) -> impl Strategy<Value = Vec<u8>> {
    let valid = fixture(format).to_vec();
    let len = valid.len();
    let flipped = prop::collection::vec((0..len, 1u8..=255), 1..8).prop_map({
        let valid = valid.clone();
        move |flips| {
            let mut bytes = valid.clone();
            for (at, xor) in flips {
                bytes[at] ^= xor;
            }
            bytes
        }
    });
    let cut = (0..len).prop_map(move |at| valid[..at].to_vec());
    prop_oneof![prop::collection::vec(any::<u8>(), 0..2048), flipped, cut,]
}

macro_rules! fuzz {
    ($($test:ident: $format:ident,)+) => {
        proptest! {
            #![proptest_config(common::config("tests/proptest-regressions/archive_fuzz.txt"))]
            $(
                #[test]
                fn $test(bytes in inputs(ArchiveFormat::$format)) {
                    exercise(ArchiveFormat::$format, bytes);
                }
            )+
        }
    };
}

fuzz! {
    fuzz_zip: Zip,
    fuzz_tar: Tar,
    fuzz_tar_gz: TarGz,
    fuzz_tar_bz2: TarBz2,
    fuzz_tar_xz: TarXz,
    fuzz_tar_zst: TarZst,
    fuzz_gz: Gz,
    fuzz_bz2: Bz2,
    fuzz_xz: Xz,
    fuzz_zst: Zst,
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/archive_fuzz.txt"))]

    #[test]
    fn central_times_on_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096), max in 0usize..64) {
        let found = central_times(Cursor::new(bytes), 0, max);
        prop_assert!(found.len() <= max);
    }

    #[test]
    fn central_times_on_bytes_with_headers(mut bytes in prop::collection::vec(any::<u8>(), 64..4096), at in 0usize..16) {
        bytes[at..at + 4].copy_from_slice(b"PK\x01\x02");
        let _ = central_times(Cursor::new(&bytes[at..]), 0, 64);
    }
}

/// Cuts at every byte of a small `tar.gz`: members complete before the cut read correctly and `<damaged>` appears.
#[test]
fn a_cut_at_every_byte_of_a_tar_gz() {
    let data: TreeData = vec![
        ("first.txt".into(), b"the first member".to_vec()),
        ("second.txt".into(), vec![b's'; 1500]),
        ("third.txt".into(), b"the third member".to_vec()),
    ];
    let bytes = compress(ArchiveFormat::TarGz, &tar_bytes(&data));
    let never = CancelToken::never();
    let mut most = 0;
    for cut in 0..bytes.len() {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put("a.tar.gz", bytes[..cut].to_vec());
        let root = Spec::archive_member(file.clone(), "");
        match memory.vfs.list(&root, &ListOptions::default(), &never) {
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::InvalidData, "cut {cut}: {error}");
            }
            Ok(listing) => {
                assert!(
                    listing
                        .errors
                        .iter()
                        .any(|e| e.name.as_str() == "<damaged>"),
                    "cut {cut}: {:?}",
                    listing.errors
                );
                most = most.max(listing.entries.len());
                for entry in &listing.entries {
                    let (_, want) = data.iter().find(|(n, _)| n == entry.name.as_str()).unwrap();
                    let member = Spec::archive_member(file.clone(), entry.name.as_str());
                    let mut got = Vec::new();
                    memory
                        .vfs
                        .open_read(&member)
                        .unwrap_or_else(|e| panic!("cut {cut}, {}: {e}", entry.name))
                        .read_to_end(&mut got)
                        .unwrap();
                    assert_eq!(&got, want, "cut {cut}");
                }
            }
        }
    }
    assert_eq!(most, 3, "a late cut keeps every member");
}

#[test]
fn a_cut_tar_keeps_the_members_before_it() {
    let data: TreeData = vec![("a".into(), vec![1u8; 700]), ("b".into(), vec![2u8; 700])];
    let bytes = tar_bytes(&data);
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.tar", bytes[..1500].to_vec());
    let listing = memory
        .vfs
        .list(&root, &ListOptions::default(), &CancelToken::never())
        .unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name.as_str(), "a");
    let damaged = &listing.errors[0];
    assert_eq!(damaged.name.as_str(), "<damaged>");
    assert!(
        damaged.error.message.contains("byte"),
        "{}",
        damaged.error.message
    );
}

#[test]
fn rule_8_mask_matches_with_other_content() {
    let never = CancelToken::never();
    for format in ArchiveFormat::ORDER {
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name_of(format), b"this is just text, under a mask".to_vec());
        let result = memory
            .vfs
            .list(&root, &ListOptions::default(), &never)
            .map(drop);
        assert!(
            matches!(result, Err(VfsError::ArchiveCorrupt { .. })),
            "{format:?}: {result:?}"
        );
        // The file itself stays a normal file.
        let file = root.container().unwrap();
        assert_eq!(
            memory.vfs.stat(file, FollowLinks::No).unwrap().kind,
            EntryKind::File
        );

        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root(name_of(format), Vec::new());
        let result = memory.vfs.list(&root, &ListOptions::default(), &never);
        if format == ArchiveFormat::Tar {
            assert!(
                result.unwrap().is_empty(),
                "a zero-length tar is an empty archive"
            );
        } else {
            assert!(
                matches!(result, Err(VfsError::ArchiveCorrupt { .. })),
                "{format:?}"
            );
        }
    }
}

#[test]
fn a_forced_decoder_panic_is_corrupt() {
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("a.tar", tar_bytes(&tree()));
    memory.vfs.inject_decoder_panic();
    let result = memory
        .vfs
        .list(&root, &ListOptions::default(), &CancelToken::never());
    match result {
        Err(VfsError::ArchiveCorrupt { reason, .. }) => assert_eq!(reason, "decoder failed"),
        other => panic!("{other:?}"),
    }
    // The next call works: the lock is not poisoned.
    memory
        .vfs
        .list(&root, &ListOptions::default(), &CancelToken::never())
        .unwrap();
    let member = Spec::archive_member(root.container().unwrap().clone(), "readme.txt");
    memory.vfs.inject_decoder_panic();
    let error = memory.vfs.open_read(&member).map(drop).unwrap_err();
    assert!(matches!(error, VfsError::ArchiveCorrupt { .. }), "{error}");
    memory.vfs.open_read(&member).unwrap();
}

#[test]
fn a_base_error_is_not_corruption() {
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root(
        "a.tar.gz",
        compress(ArchiveFormat::TarGz, &tar_bytes(&tree())),
    );
    memory
        .mem
        .inject_read_error_after(&common::host("/a.tar.gz"), 20, ErrorKind::PermissionDenied);
    let error = memory
        .vfs
        .list(&root, &ListOptions::default(), &CancelToken::never())
        .map(drop)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied, "{error}");
}

#[test]
fn warnings_arrive_once() {
    let memory = on_memory(ArchiveLimits::default());
    let tar = common::archives::raw_tar(&[
        common::archives::RawTar::file(b"../x", b"1"),
        common::archives::RawTar::file(b"ok", b"2"),
    ]);
    let root = memory.root("a.tar", tar);
    let never = CancelToken::never();
    memory
        .vfs
        .list(&root, &ListOptions::default(), &never)
        .unwrap();
    memory
        .vfs
        .list(&root, &ListOptions::default(), &never)
        .unwrap();
    let warnings = memory.vfs.take_warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, WarningCode::ArchiveMemberRefused);
    assert!(memory.vfs.take_warnings().is_empty());
}

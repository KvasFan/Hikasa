//! Golden fixtures (§6.1): the stable serde shapes of the serialisable VFS types (T5; `DosAttrs` as its letter
//! string, D-30) and a deterministic rendering of a `MemoryVfs` tree walked with every listing order, with and
//! without following links and with a prefilter. Run with `HIKASA_BLESS=1` to rewrite them after an intended
//! change.

mod common;

use common::json::{self, J};
use hikasa_vfs::{Attrs, BinaryOutcome, DosAttrs, EntryKind, FollowLinks, UnixMeta, VfsOp};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Shapes {
    kinds: Vec<EntryKind>,
    follow: Vec<FollowLinks>,
    ops: Vec<VfsOp>,
    outcomes: Vec<BinaryOutcome>,
    attrs: Vec<Attrs>,
    all_letters: DosAttrs,
    unix: UnixMeta,
}

#[test]
fn serde_shapes() {
    let unix = UnixMeta {
        mode: 0o755,
        uid: 1000,
        gid: 100,
    };
    let shapes = Shapes {
        kinds: EntryKind::ALL.to_vec(),
        follow: vec![FollowLinks::No, FollowLinks::Yes],
        ops: VfsOp::ALL.to_vec(),
        outcomes: vec![
            BinaryOutcome::Same,
            BinaryOutcome::Different {
                first_difference: Some(4096),
            },
            BinaryOutcome::Different {
                first_difference: None,
            },
        ],
        attrs: vec![
            Attrs::None,
            Attrs::Dos {
                attrs: DosAttrs::HIDDEN | DosAttrs::READONLY | DosAttrs::SYSTEM,
            },
            Attrs::Dos {
                attrs: DosAttrs::empty(),
            },
            Attrs::Unix { meta: unix },
        ],
        all_letters: DosAttrs::all(),
        unix,
    };
    let text = json::to_pretty(&shapes);
    common::assert_golden(&common::fixture("serde", "vfs_types.json"), text.as_bytes());
    let back: Shapes = json::from_value(json::to_value(&shapes)).unwrap();
    assert_eq!(back, shapes);
    // D-30: the letter string, case-insensitive on input; unknown letters are rejected.
    assert_eq!(
        json::to_value(&(DosAttrs::HIDDEN | DosAttrs::READONLY | DosAttrs::SYSTEM)),
        J::Str("HRS".into())
    );
    assert_eq!(
        json::from_value::<DosAttrs>(J::Str("srh".into())).unwrap(),
        DosAttrs::HIDDEN | DosAttrs::READONLY | DosAttrs::SYSTEM
    );
    let err = json::from_value::<DosAttrs>(J::Str("HX".into())).unwrap_err();
    assert!(
        err.to_string().contains("invalid DOS attribute letters"),
        "{err}"
    );
    assert!(json::from_value::<DosAttrs>(J::U(2)).is_err());
}

/// The rendering contains host paths (an absolute link target, error messages), so the listing golden is
/// pinned on Unix; the order-independence it checks is also covered by P-VFS-4 on every platform.
#[cfg(unix)]
mod listing {
    use std::fmt::Write as _;

    use super::common;
    use common::{host, mspec};
    use hikasa_core::{CancelToken, FileTime, Spec};
    use hikasa_vfs::{
        Attrs, DosAttrs, EntryKind, FollowLinks, ListOptions, ListingOrder, MemMeta, MemoryVfs,
        UnixMeta, Vfs,
    };

    fn t(secs: i64) -> Option<FileTime> {
        FileTime::from_unix(secs, 0)
    }

    /// A small project tree with Unix and DOS attributes, CRCs, symlinks (relative, absolute, broken, a loop and a
    /// folder cycle).
    fn tree() -> MemoryVfs {
        let vfs = MemoryVfs::default();
        let unix = |mode| MemMeta {
            mtime: t(1_760_000_000),
            ctime: t(1_750_000_000),
            attrs: Some(Attrs::Unix {
                meta: UnixMeta {
                    mode,
                    uid: 1000,
                    gid: 100,
                },
            }),
            crc32: None,
        };
        vfs.insert_dir(&host("/p"), unix(0o755)).unwrap();
        vfs.insert_dir(&host("/p/src"), unix(0o755)).unwrap();
        vfs.insert_file(
            &host("/p/src/main.rs"),
            b"fn main() {}\n".to_vec(),
            unix(0o644),
        )
        .unwrap();
        vfs.insert_file(&host("/p/src/lib.rs"), b"".to_vec(), unix(0o600))
            .unwrap();
        vfs.insert_file(&host("/p/build.sh"), b"#!/bin/sh\n".to_vec(), unix(0o4755))
            .unwrap();
        vfs.insert_file(
            &host("/p/archive.zip"),
            vec![0x50, 0x4B, 0x03, 0x04],
            MemMeta {
                mtime: t(1_700_000_000),
                ctime: None,
                attrs: Some(Attrs::Dos {
                    attrs: DosAttrs::ARCHIVE | DosAttrs::HIDDEN,
                }),
                crc32: Some(0x0BAD_F00D),
            },
        )
        .unwrap();
        vfs.insert_dir(&host("/p/obj"), MemMeta::default()).unwrap();
        vfs.insert_file(&host("/p/obj/x.o"), vec![0u8; 64], MemMeta::default())
            .unwrap();
        vfs.insert_symlink(&host("/p/current"), "src", unix(0o777))
            .unwrap();
        vfs.insert_symlink(&host("/p/abs"), &host("/p/src/main.rs"), MemMeta::default())
            .unwrap();
        vfs.insert_symlink(&host("/p/dangling"), "gone", MemMeta::default())
            .unwrap();
        vfs.insert_symlink(&host("/p/self"), "self", MemMeta::default())
            .unwrap();
        vfs.insert_symlink(&host("/p/src/back"), "..", MemMeta::default())
            .unwrap();
        vfs
    }

    fn render_entry(out: &mut String, depth: usize, entry: &hikasa_vfs::Entry) {
        let attrs = match entry.attrs {
            Attrs::None => "-".to_owned(),
            Attrs::Dos { attrs } => format!("dos:{}", attrs.letters()),
            Attrs::Unix { meta } => format!(
                "{} {}:{}",
                meta.permissions_string(entry.kind),
                meta.uid,
                meta.gid
            ),
        };
        let time = |t: Option<FileTime>| t.map_or_else(|| "-".to_owned(), |t| t.to_rfc3339());
        let _ = writeln!(
            out,
            "{:indent$}{}{} kind={} size={} attrs={} mtime={} ctime={} link={} followed={} crc={} id={}",
            "",
            entry.name.as_str(),
            if entry.kind == EntryKind::Dir {
                "/"
            } else {
                ""
            },
            entry.kind,
            entry.size,
            attrs,
            time(entry.mtime),
            time(entry.ctime),
            entry.link_target.as_deref().unwrap_or("-"),
            entry.followed,
            entry
                .crc32
                .map_or_else(|| "-".to_owned(), |c| format!("{c:08x}")),
            entry.file_id.map_or(0, |id| id.ino),
            indent = depth * 2,
        );
    }

    /// Walks `dir` (depth-first, sorted by name), descending into real folders only (never through links, so
    /// cycles terminate).
    fn walk(vfs: &dyn Vfs, dir: &Spec, depth: usize, opts: &ListOptions<'_>, out: &mut String) {
        let mut listing = vfs.list(dir, opts, &CancelToken::never()).unwrap();
        listing.sort_by_name();
        for entry in &listing.entries {
            render_entry(out, depth, entry);
            if entry.kind == EntryKind::Dir && !entry.followed {
                let child = vfs.child(dir, &entry.name).unwrap();
                walk(vfs, &child, depth + 1, opts, out);
            }
        }
        for name_only in &listing.name_only {
            let _ = writeln!(
                out,
                "{:indent$}{} (name only, {})",
                "",
                name_only.name.as_str(),
                name_only.kind,
                indent = depth * 2
            );
        }
        for error in &listing.errors {
            let _ = writeln!(
                out,
                "{:indent$}{} ERROR {}: {}",
                "",
                error.name.as_str(),
                error.error.kind,
                without_os_code(&error.error.message),
                indent = depth * 2
            );
        }
    }

    /// `message` without a trailing ` (os error N)`: the loop error carries the platform's `ELOOP` number (40 on
    /// Linux, 62 on macOS and the BSDs), and the golden is shared by every Unix.
    fn without_os_code(message: &str) -> &str {
        match message.rfind(" (os error ") {
            Some(at) if message.ends_with(')') => &message[..at],
            _ => message,
        }
    }

    #[test]
    fn memory_tree_listing() {
        let vfs = tree();
        let decline_obj = |name: &str, _: EntryKind| name != "obj";
        let mut out = String::new();
        let modes: [(&str, FollowLinks, bool); 3] = [
            ("follow=no", FollowLinks::No, false),
            ("follow=yes", FollowLinks::Yes, false),
            ("follow=no prefilter=-obj", FollowLinks::No, true),
        ];
        let orders = [
            ListingOrder::Sorted,
            ListingOrder::Reversed,
            ListingOrder::Shuffled { seed: 99 },
        ];
        for (label, follow, prefilter) in modes {
            let opts = ListOptions {
                follow,
                prefilter: if prefilter { Some(&decline_obj) } else { None },
            };
            let mut renders = orders.iter().map(|&order| {
                vfs.set_listing_order(order);
                let mut text = String::new();
                walk(&vfs, &mspec("/p"), 0, &opts, &mut text);
                text
            });
            let first = renders.next().unwrap();
            for other in renders {
                assert_eq!(
                    other, first,
                    "{label}: every listing order renders the same"
                );
            }
            let _ = writeln!(out, "# {label}");
            out.push_str(&first);
        }
        common::assert_golden(
            &common::fixture("listing", "memory_tree.txt"),
            out.as_bytes(),
        );
    }
}

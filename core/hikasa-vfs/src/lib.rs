//! Virtual file system of Hikasa: every file system access in M1 goes through the [`Vfs`] trait (VFS-001).
//!
//! # Back-ends
//!
//! | Back-end | Specs | Notes |
//! |---|---|---|
//! | [`LocalVfs`] | `Spec::Local` | the local file system; atomic writes; name-only prefiltered listings (D-45) |
//! | [`MemoryVfs`] | `Spec::Local` | a deterministic in-memory fake for tests: listing orders, fault injection, symlinks |
//! | [`Blob`] | `Spec::Stdin`, `Spec::Clipboard` | in-memory content injected by adapters |
//! | [`VfsRouter`] | every spec | dispatches specs to the back-ends above; archive, snapshot, remote, profile and HTTP specs parse but are unsupported in M1 (D-05) |
//!
//! # Content helpers
//!
//! [`crc32_source`] and [`compare_sources`] implement the CRC and binary content tests (§2.20; FC-025,
//! VFS-020, ARCH-081): a same-object shortcut, a length shortcut (never for reported sizes of at most 4096
//! bytes, which pseudo-files get wrong), a memory-mapped fast path for large local regular files that can use
//! [`StreamOptions::threads`] scoped helper threads (results never depend on the count), and the streaming
//! reference implementation. [`CrcComparator`] and [`BinaryComparator`] wrap them as
//! [`hikasa_core::ContentComparator`]s, and [`VfsSource`] makes any file reachable through a [`Vfs`] a
//! [`hikasa_core::ContentSource`].
//!
//! ```
//! use hikasa_core::{CancelToken, CaseSensitivity, ContentComparator, ContentResult, Ctl, LocalPath, Spec};
//! use hikasa_vfs::{BinaryComparator, FollowLinks, ListOptions, MemMeta, MemoryVfs, StreamOptions, Vfs, VfsSource};
//! use std::path::PathBuf;
//!
//! # fn path(p: &str) -> String { if cfg!(windows) { format!("C:{}", p.replace('/', "\\")) } else { p.to_owned() } }
//! let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
//! vfs.insert_file(&path("/left/a.txt"), b"same".to_vec(), MemMeta::default())?;
//! vfs.insert_file(&path("/right/a.txt"), b"same".to_vec(), MemMeta::default())?;
//! let spec = |p: &str| Spec::Local(LocalPath::new(PathBuf::from(path(p))).unwrap());
//!
//! let listing = vfs.list(&spec("/left"), &ListOptions::default(), &CancelToken::never())?;
//! assert_eq!(listing.entries[0].name.as_str(), "a.txt");
//!
//! let source = |p: &str| -> Result<VfsSource<'_>, hikasa_vfs::VfsError> {
//!     let entry = vfs.stat(&spec(p), FollowLinks::No)?;
//!     Ok(VfsSource::new(&vfs, spec(p), entry))
//! };
//! let comparator = BinaryComparator { opts: StreamOptions::default() };
//! let outcome = comparator.compare(&source("/left/a.txt")?, &source("/right/a.txt")?, &Ctl::detached())?;
//! assert_eq!(outcome.result, ContentResult::BinarySame);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Additions to the M1 contract
//!
//! Recorded in contract §1.2.7 ("Accepted additions and deviations (wave 2)"). Additive public API beyond §1.2:
//! [`VfsError::InvalidAttributes`] (the error of [`DosAttrs::parse_letters`]), [`VfsError::from_io`],
//! [`VfsError::spec`], [`VfsError::is_cancelled`], `From<Cancelled> for VfsError`, `From<VfsError> for
//! std::io::Error` (used by [`VfsSource`]'s `ContentSource::open`); [`VfsOp::ALL`]/[`VfsOp::as_str`],
//! [`EntryKind::ALL`]/[`EntryKind::as_str`] and `Display`, `Display for FileName`, [`Attrs::dos`]/[`Attrs::unix`],
//! [`Entry::new`], [`Entry::link_target_os`]; [`Capabilities::names`]/[`Capabilities::parse_name`] (the JSON
//! bit-set form, §4.1 rule 9); [`Listing::sort_by_name`]/[`Listing::len`] (distinct names)/[`Listing::is_empty`];
//! [`LocalVfs::options`]; [`MemoryVfs::set_capabilities`], [`MemoryVfs::clear_faults`],
//! [`MemoryVfs::stat_calls`], [`MemoryVfs::set_kind_unknown`]; [`VfsRouter::local`]; [`VfsSource::spec`],
//! [`VfsSource::entry`]; the [`StreamOptions`] and [`MmapPolicy`] constants, [`StreamOptions::effective_buffer_size`],
//! [`StreamOptions::effective_threads`], [`MmapPolicy::maps`]; [`BinaryOutcome::is_same`].
//!
//! Two new public fields break struct literals written against §1.2 (wave 2, before any consumer existed):
//! [`StreamOptions::threads`] (scoped helper threads of the mapped path, default 1) and
//! [`Entry::link_target_raw`] (the exact target of a lossy `link_target`). Literals use `..Default::default()`
//! or [`Entry::new`].
//!
//! # `unsafe`
//!
//! The crate denies `unsafe` code. The private `mmap` module is the single exemption and the only `unsafe` in
//! Hikasa's library and program code (§1.2.6): it maps local regular files read-only one chunk at a time inside
//! one call, never lets a mapped slice escape, and re-stats both files afterwards, discarding a mapped result
//! when a length or modification time changed. [`MmapPolicy::Never`] disables the mapped path.
//!
//! Specification: `docs/design/m1-contracts.md` §1.2 and the algorithm of §2.20.
#![deny(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod caps;
pub mod compare;
pub mod entry;
pub mod error;
pub mod local;
pub mod memory;
pub mod router;
pub mod source;
pub mod stream;
pub mod vfs;

// The only exemption from `deny(unsafe_code)` (§0.4 rule 2, §1.2.6): read-only memory maps of local regular
// files. Every `unsafe` block inside carries a `// SAFETY:` comment; mapped slices never leave the module.
#[allow(unsafe_code)]
mod mmap;

pub use caps::Capabilities;
pub use compare::{BinaryComparator, BinaryOutcome, CrcComparator, compare_sources, crc32_source};
pub use entry::{Attrs, DosAttrs, Entry, EntryKind, FileId, FileName, UnixMeta};
pub use error::{VfsError, VfsOp};
pub use local::{LocalOptions, LocalVfs};
pub use memory::{Fault, ListingOrder, MemMeta, MemoryVfs};
pub use router::{Blob, VfsRouter};
pub use source::VfsSource;
pub use stream::{MmapPolicy, StreamOptions, compare_readers, crc32_reader};
pub use vfs::{
    AtomicWrite, EntryError, FollowLinks, ListOptions, Listing, NameOnly, Prefilter, ReadSeek, Vfs,
    WriteOptions,
};

//! Archives (M2 W5, VFS-002; contract M2 §1.5): [`ArchiveVfs`], a [`Vfs`] stacked on another one.
//!
//! `ArchiveVfs` serves `ArchiveMember` specs itself and passes every other spec to its base, and it reads containers
//! only through the base, so `MemoryVfs` tests every algorithm, faults included (D-136). Archives are hostile input:
//! names are checked on raw bytes (`name`), the tree does not depend on archive order (`index`), every decoded byte
//! passes the guards of `codec`, and a member never reaches a path (`cache`). Members are `/`-separated and
//! case-sensitive on every platform; a format is chosen by mask, never by content.
//!
//! At the end of W5a archives are read-only: every primitive on a member answers `OperationUnsupported` and no write
//! bit is reported (D-135). W5b adds batches and rebuilds behind those bits: `open_write` and the primitives
//! go through `ArchiveVfs::on_base`, and `Tree::records` lists what a rebuild keeps.
//!
//! | Format | Index | Content of a member |
//! |---|---|---|
//! | zip | central directory, no local header | decoded to memory (≤ 1 MiB) or a cache file, reused |
//! | tar | one pass over the file | a range of the file |
//! | `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst` | one pass over the decoded stream | the first read spools its member, the second distinct one writes the stream to the cache |
//! | `gz`, `bz2`, `xz`, `zst` | one pass that counts and hashes | the stream, spooled and reused |

mod cache;
mod codec;
mod format;
mod index;
pub mod name;
mod resolve;
mod shared;
mod tar_walk;
pub mod time;
pub mod zip_read;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::io::{Cursor, Read, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use hikasa_core::{
    CancelToken, Ctl, ErrorKind, ErrorReport, FileTime, NullProgress, Spec, Warning, WarningCode,
};

pub use format::{ArchiveFormat, ArchiveMasks};
pub use resolve::{Probe, Probed, SpecUse, resolve_spec, resolve_with};

use cache::{Cache, CacheReader, CacheWriter, WHOLE_STREAM};
use codec::{Codec, Counted, Failure, Guard, Input, InputSize, Pass, classify, corrupt, no_panic};
use format::stream_member_name;
use index::{DataRef, MemberKind, Node, RawMember, Tree, build_tree};
use name::{NameRules, check_name, escape_name};
use shared::RangeReader;
use tar_walk::{Damage, TarItem, TarKind, walk};
use zip_read::{ZipData, index_zip};

use crate::caps::Capabilities;
use crate::entry::{Attrs, Entry, EntryKind, FileName, UnixMeta};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{
    AtomicWrite, AttrChange, EntryError, FollowLinks, ListOptions, Listing, NameOnly, ReadSeek,
    SetTimes, Vfs, WriteOptions, op_unsupported,
};

/// A member of at most this many bytes is decoded to memory, a larger one to a cache file.
const MEMORY_MAX: u64 = 1 << 20;
/// When the members of all indexes pass this, the least recently used indexes no call holds are dropped.
const INDEX_MEMBERS_MAX: usize = 10_000_000;

/// Limits of `archives.limits` (D-143; §1.5.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// Members of one archive.
    pub entries_max: u64,
    /// Decoded bytes of one pass, and of all reads of one archive.
    pub stream_max: u64,
    /// Bytes of the temp cache; also the largest member that spools.
    pub cache_max: u64,
    /// Output / input, once past `ratio_floor`; 0 turns the check off.
    pub ratio_max: u32,
    /// Output below which the ratio is not looked at.
    pub ratio_floor: u64,
    /// Archives around a spec.
    pub nesting_max: u32,
}

impl Default for ArchiveLimits {
    /// The table of §1.5.7.
    fn default() -> ArchiveLimits {
        ArchiveLimits {
            entries_max: 1_000_000,
            stream_max: 64 << 30,
            cache_max: 8 << 30,
            ratio_max: 2000,
            ratio_floor: 64 << 20,
            nesting_max: 8,
        }
    }
}

/// What an [`ArchiveVfs`] needs from outside: `hikasa-vfs` cannot name the session (D-136).
pub struct ArchiveConfig {
    /// The masks that decide formats.
    pub masks: ArchiveMasks,
    /// The limits.
    pub limits: ArchiveLimits,
    /// "Now" with the user's zone (T1): the zone of DOS times, the time of a written member. The session passes its
    /// injected clock.
    pub clock: Arc<dyn Fn() -> jiff::Zoned + Send + Sync>,
    /// The folder under which the per-session cache folder lives; created on the first write only.
    pub cache_root: PathBuf,
}

impl fmt::Debug for ArchiveConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArchiveConfig")
            .field("masks", &self.masks)
            .field("limits", &self.limits)
            .field("cache_root", &self.cache_root)
            .finish_non_exhaustive()
    }
}

/// What an [`ArchiveVfs`] has done so far (tests and diagnostics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveCounters {
    /// Index passes over a container.
    pub index_passes: u64,
    /// Member decodes (a pass of a member's bytes through a decoder).
    pub member_decodes: u64,
    /// Whole streams written to the cache.
    pub stream_writes: u64,
    /// Members read to memory.
    pub memory_reads: u64,
    /// Member reads served by a cache file.
    pub cache_hits: u64,
    /// Plain tar members read as a range of the container.
    pub range_reads: u64,
}

#[derive(Debug, Default)]
struct Counters {
    index_passes: AtomicU64,
    member_decodes: AtomicU64,
    stream_writes: AtomicU64,
    memory_reads: AtomicU64,
    cache_hits: AtomicU64,
    range_reads: AtomicU64,
}

fn count(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// The stamp of an outermost file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    mtime: Option<FileTime>,
}

/// The index of one archive at one stamp.
#[derive(Debug)]
struct Index {
    format: ArchiveFormat,
    tree: Tree,
    /// Bytes of the decoded stream (tar family, single streams).
    stream_len: u64,
    zip: Option<ZipData>,
    /// The root's entry.
    root: Entry,
    /// Decoded bytes of all reads of this archive (rule 12).
    decoded: AtomicU64,
}

/// One archive: a lock around its index, built once; other threads wait for it.
#[derive(Debug)]
struct Slot {
    /// Identifies the archive at this stamp in the cache.
    id: u64,
    outer: Spec,
    format: ArchiveFormat,
    index: Mutex<Option<Arc<Index>>>,
    /// The member of the first read of a compressed tar. Its lock keeps one decoder per stream at a time.
    first_read: Mutex<Option<Box<str>>>,
    last_use: AtomicU64,
}

/// The indexes and stamps.
#[derive(Debug, Default)]
struct Indexes {
    slots: HashMap<Spec, Arc<Slot>>,
    stamps: HashMap<Spec, Stamp>,
    next_id: u64,
    tick: u64,
}

/// A `Vfs` of archives, stacked on a base (D-136). See the module documentation.
pub struct ArchiveVfs {
    base: Arc<dyn Vfs>,
    config: RwLock<Arc<ArchiveConfig>>,
    indexes: Mutex<Indexes>,
    cache: Cache,
    warnings: Mutex<Vec<Warning>>,
    counters: Counters,
    panic_next: AtomicBool,
}

impl fmt::Debug for ArchiveVfs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArchiveVfs")
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

/// Locks `mutex`; a poisoned lock only means a decoder panicked, and the data under it stays valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a decoded member goes.
enum Spool<'a> {
    Memory(Vec<u8>),
    File(CacheWriter<'a>),
}

impl Spool<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), VfsError> {
        match self {
            Spool::Memory(buffer) => {
                buffer.extend_from_slice(bytes);
                Ok(())
            }
            Spool::File(writer) => writer.write(bytes),
        }
    }

    /// The reader of the finished spool; a cache file enters the table as `member` of `archive`.
    fn finish(self, archive: u64, member: &str) -> Result<Box<dyn ReadSeek>, VfsError> {
        match self {
            Spool::Memory(buffer) => Ok(Box::new(Cursor::new(buffer))),
            Spool::File(writer) => {
                let file = writer.finish(archive, member)?;
                Ok(Box::new(file.reader()?))
            }
        }
    }
}

impl ArchiveVfs {
    /// A back-end over `base`. Nothing is opened or created.
    pub fn new(base: Arc<dyn Vfs>, config: ArchiveConfig) -> ArchiveVfs {
        ArchiveVfs {
            base,
            config: RwLock::new(Arc::new(config)),
            indexes: Mutex::default(),
            cache: Cache::default(),
            warnings: Mutex::default(),
            counters: Counters::default(),
            panic_next: AtomicBool::new(false),
        }
    }

    /// Masks and limits changed (`Service::reload_settings`): later calls use them, running calls keep theirs.
    pub fn set_config(&self, config: ArchiveConfig) {
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(config);
    }

    /// Removes the session's cache folder (best effort; also on `Drop`). A later spool starts a new session.
    pub fn cleanup(&self) {
        self.cache.cleanup();
    }

    /// What this back-end has done so far.
    pub fn counters(&self) -> ArchiveCounters {
        let c = &self.counters;
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        ArchiveCounters {
            index_passes: get(&c.index_passes),
            member_decodes: get(&c.member_decodes),
            stream_writes: get(&c.stream_writes),
            memory_reads: get(&c.memory_reads),
            cache_hits: get(&c.cache_hits),
            range_reads: get(&c.range_reads),
        }
    }

    /// Bytes the cache holds.
    pub fn cache_used(&self) -> u64 {
        self.cache.used()
    }

    /// The session's cache folder, once something was spooled.
    pub fn cache_dir(&self) -> Option<PathBuf> {
        self.cache.dir()
    }

    /// Bytes of the member tree of the archive `container`, once indexed (the budget of §1.5.5).
    pub fn index_memory(&self, container: &Spec) -> Option<usize> {
        let slot = lock(&self.indexes).slots.get(container).cloned()?;
        let index = lock(&slot.index).clone()?;
        Some(index.tree.memory())
    }

    /// Makes the next decoder pass panic (a test hook of rule 9).
    #[doc(hidden)]
    pub fn inject_decoder_panic(&self) {
        self.panic_next.store(true, Ordering::Relaxed);
    }

    fn forced_panic(&self) {
        if self.panic_next.swap(false, Ordering::Relaxed) {
            std::panic::resume_unwind(Box::new("forced decoder panic"));
        }
    }

    fn config(&self) -> Arc<ArchiveConfig> {
        Arc::clone(&self.config.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn warn(&self, code: WarningCode, message: String, spec: &Spec) {
        lock(&self.warnings).push(Warning {
            code,
            message,
            spec: Some(spec.canonical()),
        });
    }

    /// The base for a spec outside archives; a primitive on a member answers `OperationUnsupported` until W5b.
    fn on_base(&self, spec: &Spec, op: VfsOp) -> Result<&dyn Vfs, VfsError> {
        match spec {
            Spec::ArchiveMember { .. } => Err(op_unsupported(spec, op)),
            _ => Ok(&*self.base),
        }
    }

    /// What every call on a member starts with: the nesting limit, and one `stat` of the outermost file. Another
    /// stamp drops the indexes and the cache files of that file.
    fn enter(&self, spec: &Spec) -> Result<Arc<ArchiveConfig>, VfsError> {
        let config = self.config();
        let nesting_max = config.limits.nesting_max;
        if spec.archive_depth() > nesting_max {
            return Err(VfsError::ArchiveLimit {
                spec: spec.canonical(),
                limit: "nesting",
                value: u64::from(nesting_max),
            });
        }
        let Some(outer) = spec.outermost_file() else {
            return Ok(config);
        };
        let entry = self.base.stat(outer, FollowLinks::Yes)?;
        if entry.kind != EntryKind::File {
            return Err(corrupt(&outer.canonical(), "not a file"));
        }
        let stamp = Stamp {
            len: entry.size,
            mtime: entry.mtime,
        };
        let mut indexes = lock(&self.indexes);
        if indexes.stamps.insert(outer.clone(), stamp) != Some(stamp) {
            let stale: Vec<Spec> = indexes
                .slots
                .iter()
                .filter(|(_, slot)| slot.outer == *outer)
                .map(|(key, _)| key.clone())
                .collect();
            for key in stale {
                if let Some(slot) = indexes.slots.remove(&key) {
                    self.cache.drop_archive(slot.id);
                }
            }
        }
        Ok(config)
    }

    /// The slot of `container`, created on first use, with the indexes trimmed to their budget.
    fn slot(&self, container: &Spec, format: ArchiveFormat) -> Arc<Slot> {
        let mut indexes = lock(&self.indexes);
        indexes.tick += 1;
        let tick = indexes.tick;
        if let Some(slot) = indexes.slots.get(container).filter(|s| s.format == format) {
            slot.last_use.store(tick, Ordering::Relaxed);
            return Arc::clone(slot);
        }
        indexes.next_id += 1;
        let slot = Arc::new(Slot {
            id: indexes.next_id,
            outer: container.outermost_file().unwrap_or(container).clone(),
            format,
            index: Mutex::new(None),
            first_read: Mutex::new(None),
            last_use: AtomicU64::new(tick),
        });
        if let Some(old) = indexes.slots.insert(container.clone(), Arc::clone(&slot)) {
            self.cache.drop_archive(old.id);
        }
        trim_indexes(&mut indexes, &self.cache);
        slot
    }

    /// The index of the archive `container`, built on first need.
    fn index_of(
        &self,
        container: &Spec,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<(Arc<Slot>, Arc<Index>), VfsError> {
        let name = container.file_name().unwrap_or_default();
        let Some(format) = config.masks.detect(&name) else {
            return Err(corrupt(
                &container.canonical(),
                "not an archive by its name",
            ));
        };
        let slot = self.slot(container, format);
        let mut built = lock(&slot.index);
        if let Some(index) = built.as_ref() {
            return Ok((Arc::clone(&slot), Arc::clone(index)));
        }
        let index = Arc::new(self.build(container, format, &name, config, ctl)?);
        *built = Some(Arc::clone(&index));
        drop(built);
        Ok((slot, index))
    }

    /// The entry of the file that holds an archive.
    fn container_entry(
        &self,
        container: &Spec,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<Entry, VfsError> {
        match container {
            Spec::ArchiveMember { archive, member } => {
                let (_, index) = self.index_of(archive, config, ctl)?;
                Ok(lookup(&index, member, container)?.entry())
            }
            _ => self.base.stat(container, FollowLinks::Yes),
        }
    }

    /// A reader of the file that holds an archive.
    fn open_container(
        &self,
        container: &Spec,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        match container {
            Spec::ArchiveMember { archive, member } => {
                self.open_member(container, archive, member, config, ctl)
            }
            _ => self.base.open_read_with(container, ctl.cancel),
        }
    }

    /// One index pass (never a live decoder afterwards), with warnings raised once.
    fn build(
        &self,
        container: &Spec,
        format: ArchiveFormat,
        name: &str,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<Index, VfsError> {
        let text = container.canonical();
        let entry = self.container_entry(container, config, ctl)?;
        let reader = self.open_container(container, config, ctl)?;
        let zone = (config.clock)().time_zone().clone();
        let pass = Pass {
            spec: &text,
            limits: config.limits,
            ctl,
            zone,
        };
        count(&self.counters.index_passes);
        let (tree, stream_len, zip) = no_panic(&text, || {
            self.forced_panic();
            match Codec::of(format) {
                _ if format == ArchiveFormat::Zip => {
                    let (tree, zip) = if entry.size == 0 {
                        return Err(corrupt(&text, "an empty file"));
                    } else {
                        index_zip(reader, entry.size, &pass)?
                    };
                    Ok((tree, 0, Some(zip)))
                }
                codec if format.is_single_stream() => {
                    let (tree, len) = index_single(reader, codec, name, &entry, &pass)?;
                    Ok((tree, len, None))
                }
                codec => {
                    let (tree, len) = index_tar(reader, codec, entry.size, &pass)?;
                    Ok((tree, len, None))
                }
            }
        })?;
        let mut root = Entry::new(FileName::from_os(OsStr::new(name)), EntryKind::Dir);
        root.mtime = entry.mtime;
        root.attrs = tree.root_attrs.unwrap_or(Attrs::None);
        self.index_warnings(&tree, container);
        Ok(Index {
            format,
            tree,
            stream_len,
            zip,
            root,
            decoded: AtomicU64::new(0),
        })
    }

    fn index_warnings(&self, tree: &Tree, container: &Spec) {
        if tree.refused_total > 0 {
            let message = format!("{} archive members were refused", tree.refused_total);
            self.warn(WarningCode::ArchiveMemberRefused, message, container);
        }
        if tree.duplicates > 0 {
            let message = "an archive holds the same path twice: the last one wins".to_owned();
            self.warn(WarningCode::ArchiveDuplicateMember, message, container);
        }
        if tree.encrypted > 0 {
            let message = format!(
                "{} encrypted archive members are listed but cannot be read",
                tree.encrypted
            );
            self.warn(WarningCode::ArchiveEncrypted, message, container);
        }
    }

    /// The content of the member `member` of `archive`.
    fn open_member(
        &self,
        spec: &Spec,
        archive: &Spec,
        member: &str,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        if member.is_empty() {
            return Err(VfsError::IsADirectory {
                spec: spec.canonical(),
            });
        }
        let (slot, index) = self.index_of(archive, config, ctl)?;
        let node = lookup(&index, member, spec)?;
        match node.kind {
            MemberKind::Dir => {
                return Err(VfsError::IsADirectory {
                    spec: spec.canonical(),
                });
            }
            MemberKind::Symlink | MemberKind::Other | MemberKind::Sparse => {
                return Err(op_unsupported(spec, VfsOp::Open));
            }
            MemberKind::File | MemberKind::HardLink => {}
        }
        if node.encrypted {
            return Err(VfsError::ArchiveEncrypted {
                spec: spec.canonical(),
            });
        }
        let text = spec.canonical();
        let pass = Pass {
            spec: &text,
            limits: config.limits,
            ctl,
            zone: jiff::tz::TimeZone::UTC,
        };
        no_panic(&text, || {
            self.forced_panic();
            match (index.format, node.data) {
                (ArchiveFormat::Zip, DataRef::Zip { index: at }) => {
                    self.read_zip(&slot, &index, node, at, archive, config, &pass)
                }
                (ArchiveFormat::Tar, DataRef::Tar { data, .. }) => {
                    count(&self.counters.range_reads);
                    let reader = self.open_container(archive, config, ctl)?;
                    Ok(Box::new(RangeReader::new(reader, data, node.size)))
                }
                (format, DataRef::Tar { data, .. }) => {
                    let codec = Codec::of(format);
                    self.read_tar_member(&slot, &index, node, data, codec, archive, config, &pass)
                }
                (format, DataRef::Stream) => {
                    let codec = Codec::of(format);
                    self.read_stream(&slot, &index, node, codec, archive, config, &pass)
                }
                _ => Err(corrupt(&text, "inconsistent index")),
            }
        })
    }

    /// A new spool for `len` bytes: memory up to 1 MiB, else a cache file.
    fn spool(
        &self,
        len: u64,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
    ) -> Result<Spool<'_>, VfsError> {
        if len <= MEMORY_MAX {
            count(&self.counters.memory_reads);
            return Ok(Spool::Memory(Vec::with_capacity(
                usize::try_from(len).unwrap_or(0),
            )));
        }
        self.cache
            .writer(
                &config.cache_root,
                Some(len),
                config.limits.cache_max,
                pass.spec,
            )
            .map(Spool::File)
    }

    /// A reader of the cached file of `member`, marked as used.
    fn cached(&self, slot: &Slot, member: &str) -> Option<CacheReader> {
        let reader = self.cache.get(slot.id, member)?;
        count(&self.counters.cache_hits);
        Some(reader)
    }

    /// Zip: decode one member, to memory or to a reused cache file.
    #[allow(clippy::too_many_arguments)]
    fn read_zip(
        &self,
        slot: &Slot,
        index: &Index,
        node: &Node,
        at: usize,
        archive: &Spec,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        if let Some(reader) = self.cached(slot, &node.path) {
            return Ok(Box::new(reader));
        }
        let Some(zip) = &index.zip else {
            return Err(corrupt(pass.spec, "inconsistent index"));
        };
        zip.check_ratio(pass)?;
        let _lease = zip.lease(|| self.open_container(archive, config, pass.ctl))?;
        let mut spool = self.spool(node.size, config, pass)?;
        count(&self.counters.member_decodes);
        zip.decode(at, node.size, pass, &index.decoded, &mut |bytes| {
            spool.write(bytes)
        })?;
        spool.finish(slot.id, &node.path)
    }

    /// `decode_range`: decodes the stream of `archive` forward, drops `start` bytes, passes `len` to `sink`.
    #[allow(clippy::too_many_arguments)]
    fn decode_range(
        &self,
        index: &Index,
        codec: Option<Codec>,
        archive: &Spec,
        start: u64,
        len: u64,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), VfsError>,
    ) -> Result<(), VfsError> {
        count(&self.counters.member_decodes);
        let reader = self.open_container(archive, config, pass.ctl)?;
        let input = Arc::new(Input::default());
        let chain = open_stream(reader, codec, &input, pass)?;
        let mut guard = Guard::new(
            chain,
            pass.limits,
            pass.ctl,
            pass.spec,
            InputSize::Live(Arc::clone(&input)),
        )
        .with_total(&index.decoded);
        let fail = |error| codec::read_failure(error, Some(&input), pass.spec);
        codec::pump(&mut guard, start, &mut |_| Ok(())).map_err(fail)?;
        let copied = codec::pump(&mut guard, len, sink).map_err(fail)?;
        if copied != len {
            return Err(corrupt(pass.spec, "size mismatch"));
        }
        Ok(())
    }

    /// A compressed tar: the first read spools its member; a second distinct one writes the whole stream to the
    /// cache when it fits, and every member is then a range of it; otherwise each read spools its own member.
    #[allow(clippy::too_many_arguments)]
    fn read_tar_member(
        &self,
        slot: &Slot,
        index: &Index,
        node: &Node,
        data: u64,
        codec: Option<Codec>,
        archive: &Spec,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        let member: &str = &node.path;
        let mut first_read = lock(&slot.first_read);
        if let Some(stream) = self.cached(slot, WHOLE_STREAM) {
            return Ok(Box::new(RangeReader::new(stream, data, node.size)));
        }
        if let Some(reader) = self.cached(slot, member) {
            return Ok(Box::new(reader));
        }
        let distinct = first_read.as_deref().is_some_and(|first| first != member);
        let whole = index.stream_len;
        if distinct && whole <= self.cache.free(config.limits.cache_max) {
            let mut writer = self.cache.writer(
                &config.cache_root,
                Some(whole),
                config.limits.cache_max,
                pass.spec,
            )?;
            count(&self.counters.stream_writes);
            self.decode_range(
                index,
                codec,
                archive,
                0,
                whole,
                config,
                pass,
                &mut |bytes| writer.write(bytes),
            )?;
            let file = writer.finish(slot.id, WHOLE_STREAM)?;
            return Ok(Box::new(RangeReader::new(file.reader()?, data, node.size)));
        }
        first_read.get_or_insert_with(|| member.into());
        let mut spool = self.spool(node.size, config, pass)?;
        self.decode_range(
            index,
            codec,
            archive,
            data,
            node.size,
            config,
            pass,
            &mut |bytes| spool.write(bytes),
        )?;
        spool.finish(slot.id, member)
    }

    /// A single stream: spooled, and reused from a cache file.
    #[allow(clippy::too_many_arguments)]
    fn read_stream(
        &self,
        slot: &Slot,
        index: &Index,
        node: &Node,
        codec: Option<Codec>,
        archive: &Spec,
        config: &ArchiveConfig,
        pass: &Pass<'_>,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        let member: &str = &node.path;
        let _one_decoder = lock(&slot.first_read);
        if let Some(reader) = self.cached(slot, member) {
            return Ok(Box::new(reader));
        }
        let mut spool = self.spool(node.size, config, pass)?;
        self.decode_range(
            index,
            codec,
            archive,
            0,
            node.size,
            config,
            pass,
            &mut |bytes| spool.write(bytes),
        )?;
        spool.finish(slot.id, member)
    }

    /// The `<refused-N>` and `<damaged>` rows of the root.
    fn root_errors(index: &Index, dir: &Spec) -> Vec<EntryError> {
        let report = |message: String| ErrorReport {
            kind: ErrorKind::InvalidData,
            message,
            spec: Some(dir.canonical()),
            causes: Vec::new(),
        };
        let mut errors: Vec<EntryError> = index
            .tree
            .refused
            .iter()
            .enumerate()
            .filter_map(|(at, row)| {
                Some(EntryError {
                    name: FileName::new(&format!("<refused-{}>", at + 1)).ok()?,
                    error: report(format!(
                        "refused member \"{}\": {}",
                        row.name,
                        row.reason.as_str()
                    )),
                })
            })
            .collect();
        if let (Some(damage), Ok(name)) = (&index.tree.damage, FileName::new("<damaged>")) {
            errors.push(EntryError {
                name,
                error: report(format!(
                    "the archive is damaged at byte {}: {}",
                    damage.offset, damage.reason
                )),
            });
        }
        errors
    }
}

/// Drops the least recently used indexes that no call holds until the members of all indexes fit.
fn trim_indexes(indexes: &mut Indexes, cache: &Cache) {
    let members = |slot: &Slot| {
        slot.index
            .try_lock()
            .ok()
            .and_then(|index| index.as_ref().map(|index| index.tree.nodes.len()))
            .unwrap_or(0)
    };
    let mut total: usize = indexes.slots.values().map(|slot| members(slot)).sum();
    while total > INDEX_MEMBERS_MAX {
        let victim = indexes
            .slots
            .iter()
            .filter(|(_, slot)| Arc::strong_count(slot) == 1 && members(slot) > 0)
            .min_by_key(|(_, slot)| slot.last_use.load(Ordering::Relaxed))
            .map(|(key, _)| key.clone());
        let Some(slot) = victim.and_then(|key| indexes.slots.remove(&key)) else {
            return;
        };
        total -= members(&slot);
        cache.drop_archive(slot.id);
    }
}

/// The node of `member`: `NotADirectory` when a component on the way is not a folder, else `NotFound`.
fn lookup<'a>(index: &'a Index, member: &str, spec: &Spec) -> Result<&'a Node, VfsError> {
    if let Some(node) = index.tree.find(member) {
        return Ok(node);
    }
    let spec = spec.canonical();
    Err(if index.tree.blocked(member) {
        VfsError::NotADirectory { spec }
    } else {
        VfsError::NotFound { spec }
    })
}

/// The decoded stream of a container: the bytes themselves, or a decoder over them.
fn open_stream<'a>(
    reader: Box<dyn ReadSeek>,
    codec: Option<Codec>,
    input: &Arc<Input>,
    pass: &Pass<'_>,
) -> Result<Box<dyn Read + Send + 'a>, VfsError> {
    let counted = Counted::new(reader, Arc::clone(input));
    match codec {
        None => Ok(Box::new(counted)),
        Some(codec) => codec::decoder(codec, Box::new(counted))
            .map_err(|e| codec::read_failure(e, Some(input), pass.spec)),
    }
}

/// A clean tar path as text: valid UTF-8 as it is, otherwise lossy (the raw bytes stay in the container, for a
/// rebuild).
fn tar_text(clean: &[u8]) -> String {
    String::from_utf8_lossy(clean).into_owned()
}

/// A tar record as a [`RawMember`].
fn tar_member(item: TarItem) -> RawMember {
    let kind = match item.kind {
        TarKind::File => MemberKind::File,
        TarKind::Dir => MemberKind::Dir,
        TarKind::Symlink => MemberKind::Symlink,
        TarKind::HardLink => MemberKind::HardLink,
        TarKind::Other => MemberKind::Other,
        TarKind::Sparse => MemberKind::Sparse,
    };
    let path = check_name(&item.name, NameRules::Tar)
        .map(|clean| tar_text(&clean))
        .map_err(|reason| (escape_name(&item.name), reason));
    let link = item.link.as_deref();
    let hard_target = (kind == MemberKind::HardLink)
        .then(|| check_name(link.unwrap_or_default(), NameRules::Tar).ok())
        .flatten()
        .map(|clean| tar_text(&clean));
    RawMember {
        path,
        kind,
        encrypted: false,
        size: item.size,
        mtime: item.mtime,
        ctime: None,
        attrs: Attrs::Unix {
            meta: UnixMeta {
                mode: item.mode & 0o7777,
                uid: item.uid,
                gid: item.gid,
            },
        },
        crc32: item.crc32,
        link_target: (kind == MemberKind::Symlink)
            .then(|| link.map(|l| String::from_utf8_lossy(l).into_owned().into_boxed_str()))
            .flatten(),
        hard_target,
        data: DataRef::Tar {
            first_block: item.first_block,
            data: item.data,
            end: item.end,
        },
    }
}

/// Rule 8 or rule 9: content that fails before a first byte or member is not an archive at all.
fn not_an_archive(
    pass: &Pass<'_>,
    damage: &Option<Damage>,
    members: usize,
) -> Result<(), VfsError> {
    match damage {
        Some(damage) if damage.offset == 0 && members == 0 => {
            Err(corrupt(pass.spec, damage.reason.clone()))
        }
        _ => Ok(()),
    }
}

/// The index pass of the tar family.
fn index_tar(
    reader: Box<dyn ReadSeek>,
    codec: Option<Codec>,
    len: u64,
    pass: &Pass<'_>,
) -> Result<(Tree, u64), VfsError> {
    if len == 0 {
        if codec.is_some() {
            return Err(corrupt(pass.spec, "an empty file"));
        }
        return Ok((Tree::default(), 0));
    }
    let input = Arc::new(Input::default());
    let stream = open_stream(reader, codec, &input, pass)?;
    let mut guard = Guard::new(
        stream,
        pass.limits,
        pass.ctl,
        pass.spec,
        InputSize::Live(Arc::clone(&input)),
    );
    let mut members = Vec::new();
    let entries_max = pass.limits.entries_max;
    let classify_error = |error| classify(error, Some(&input), pass.spec);
    let walked = walk(&mut guard, &classify_error, &mut |item| {
        if members.len() as u64 >= entries_max {
            return Err(VfsError::ArchiveLimit {
                spec: pass.spec.to_owned(),
                limit: "entries",
                value: entries_max,
            });
        }
        members.push(tar_member(item));
        Ok(())
    })?;
    not_an_archive(pass, &walked.damage, members.len())?;
    let mut tree = build_tree(members);
    tree.damage = walked.damage;
    Ok((tree, walked.stream_len))
}

/// The `MTIME` of a gzip header, when set.
fn gzip_mtime(reader: &mut dyn ReadSeek) -> Option<FileTime> {
    let mut header = [0u8; 8];
    codec::read_full(reader, &mut header).ok()?;
    reader.seek(SeekFrom::Start(0)).ok()?;
    if !header.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    let seconds = u32::from_le_bytes(*header.last_chunk()?);
    (seconds != 0).then(|| FileTime::from_unix(i64::from(seconds), 0))?
}

/// The index pass of a single stream: one pass that counts and hashes.
fn index_single(
    mut reader: Box<dyn ReadSeek>,
    codec: Option<Codec>,
    name: &str,
    container: &Entry,
    pass: &Pass<'_>,
) -> Result<(Tree, u64), VfsError> {
    if container.size == 0 {
        return Err(corrupt(pass.spec, "an empty file"));
    }
    let stored = match codec {
        Some(Codec::Gz) => gzip_mtime(&mut *reader),
        _ => None,
    };
    let input = Arc::new(Input::default());
    let stream = open_stream(reader, codec, &input, pass)?;
    let mut guard = Guard::new(
        stream,
        pass.limits,
        pass.ctl,
        pass.spec,
        InputSize::Live(Arc::clone(&input)),
    );
    let mut hasher = crc32fast::Hasher::new();
    let pumped = codec::pump(&mut guard, u64::MAX, &mut |bytes| {
        hasher.update(bytes);
        Ok(())
    });
    let damage = match pumped {
        Ok(_) => None,
        Err(error) => match classify(error, Some(&input), pass.spec) {
            Failure::Fatal(fatal) => return Err(fatal),
            Failure::Damage(reason) => Some(Damage {
                offset: guard.output(),
                reason,
            }),
        },
    };
    let len = guard.output();
    not_an_archive(pass, &damage, 0)?;
    let mut members = Vec::new();
    if damage.is_none() {
        members.push(RawMember {
            path: Ok(stream_member_name(name)),
            kind: MemberKind::File,
            encrypted: false,
            size: len,
            mtime: stored.or(container.mtime),
            ctime: None,
            attrs: Attrs::None,
            crc32: Some(hasher.finalize()),
            link_target: None,
            hard_target: None,
            data: DataRef::Stream,
        });
    }
    let mut tree = build_tree(members);
    tree.damage = damage;
    Ok((tree, len))
}

impl Vfs for ArchiveVfs {
    /// Needs only the masks: the archive is not opened. A member gets the bits of its innermost format.
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        let Spec::ArchiveMember { archive, .. } = at else {
            return self.base.capabilities(at);
        };
        let name = archive.file_name().unwrap_or_default();
        let format = self.config().masks.detect(&name);
        Ok(format.unwrap_or(ArchiveFormat::Gz).capabilities())
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        let Spec::ArchiveMember { archive, member } = spec else {
            return self.base.stat(spec, follow);
        };
        let config = self.enter(spec)?;
        let (_, index) = self.index_of(archive, &config, Ctl::detached())?;
        if member.is_empty() {
            return Ok(index.root.clone());
        }
        Ok(lookup(&index, member, spec)?.entry())
    }

    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        let Spec::ArchiveMember { archive, member } = dir else {
            return self.base.list(dir, opts, cancel);
        };
        let config = self.enter(dir)?;
        let progress = NullProgress;
        let ctl = Ctl {
            cancel,
            progress: &progress,
        };
        let (_, index) = self.index_of(archive, &config, ctl)?;
        let mut listing = Listing::default();
        if member.is_empty() {
            listing.errors = ArchiveVfs::root_errors(&index, dir);
        } else {
            let node = lookup(&index, member, dir)?;
            if node.kind != MemberKind::Dir {
                return Err(VfsError::NotADirectory {
                    spec: dir.canonical(),
                });
            }
        }
        let children = index.tree.children(member);
        listing.entries.reserve_exact(children.len());
        for (at, node) in children.iter().enumerate() {
            if at % 1024 == 0 {
                cancel.check()?;
            }
            let entry = node.entry();
            let declined = opts.prefilter.is_some_and(|accept| {
                entry.kind != EntryKind::Symlink && !accept(entry.name.as_str(), entry.kind)
            });
            if declined {
                listing.name_only.push(NameOnly {
                    name: entry.name,
                    kind: entry.kind,
                });
            } else {
                listing.entries.push(entry);
            }
        }
        Ok(listing)
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.open_read_with(spec, &CancelToken::never())
    }

    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.on_base(spec, VfsOp::Write)?.open_write(spec, opts)
    }

    /// Joins `name` to the member. A name that fails rule 1, or is lossy, is `InvalidName`.
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        let Spec::ArchiveMember { archive, member } = dir else {
            return self.base.child(dir, name);
        };
        let invalid = || VfsError::InvalidName {
            name: name.as_str().to_owned(),
        };
        if name.is_lossy() || name.as_str().contains('/') {
            return Err(invalid());
        }
        let joined = if member.is_empty() {
            name.as_str().to_owned()
        } else {
            format!("{member}/{}", name.as_str())
        };
        match check_name(joined.as_bytes(), NameRules::Tar) {
            Ok(clean) if clean == joined.as_bytes() => {
                Ok(Spec::archive_member((**archive).clone(), joined))
            }
            _ => Err(invalid()),
        }
    }

    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        match spec {
            Spec::ArchiveMember { .. } => None,
            _ => self.base.local_path(spec),
        }
    }

    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.on_base(spec, VfsOp::Mkdir)?.create_dir(spec, parents)
    }

    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.on_base(spec, VfsOp::Remove)?.remove(spec)
    }

    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.on_base(to, VfsOp::Rename)?;
        self.on_base(from, VfsOp::Rename)?.rename(from, to, replace)
    }

    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError> {
        self.on_base(spec, VfsOp::SetTimes)?.set_times(spec, times)
    }

    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError> {
        self.on_base(spec, VfsOp::SetAttrs)?.set_attrs(spec, change)
    }

    fn create_symlink(
        &self,
        spec: &Spec,
        target: &OsStr,
        target_is_dir: bool,
    ) -> Result<(), VfsError> {
        self.on_base(spec, VfsOp::Symlink)?
            .create_symlink(spec, target, target_is_dir)
    }

    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError> {
        self.on_base(spec, VfsOp::Trash)?.move_to_trash(spec)
    }

    fn copy_file_fast(
        &self,
        from: &Spec,
        to: &Spec,
        opts: &WriteOptions,
        cancel: &CancelToken,
    ) -> Result<Option<u64>, VfsError> {
        match (from, to) {
            (Spec::ArchiveMember { .. }, _) | (_, Spec::ArchiveMember { .. }) => Ok(None),
            _ => self.base.copy_file_fast(from, to, opts, cancel),
        }
    }

    /// Builds the index of every archive around `spec`, with cancellation and `Phase::Scanning` progress in bytes.
    fn prepare(&self, spec: &Spec, ctl: &Ctl<'_>) -> Result<(), VfsError> {
        let Spec::ArchiveMember { archive, .. } = spec else {
            return Ok(());
        };
        let config = self.enter(spec)?;
        self.index_of(archive, &config, *ctl).map(|_| ())
    }

    fn open_read_with(
        &self,
        spec: &Spec,
        cancel: &CancelToken,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        let Spec::ArchiveMember { archive, member } = spec else {
            return self.base.open_read_with(spec, cancel);
        };
        let config = self.enter(spec)?;
        let progress = NullProgress;
        let ctl = Ctl {
            cancel,
            progress: &progress,
        };
        self.open_member(spec, archive, member, &config, ctl)
    }

    fn archive_format(&self, name: &str) -> Option<ArchiveFormat> {
        self.config().masks.detect(name)
    }

    fn take_warnings(&self) -> Vec<Warning> {
        std::mem::take(&mut *lock(&self.warnings))
    }
}

impl Drop for ArchiveVfs {
    fn drop(&mut self) {
        self.cache.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::Classify;

    #[test]
    fn default_limits_are_the_table() {
        let limits = ArchiveLimits::default();
        assert_eq!(limits.entries_max, 1_000_000);
        assert_eq!(limits.stream_max, 64 << 30);
        assert_eq!(limits.cache_max, 8 << 30);
        assert_eq!(limits.ratio_max, 2000);
        assert_eq!(limits.ratio_floor, 64 << 20);
        assert_eq!(limits.nesting_max, 8);
        let error = VfsError::ArchiveLimit {
            spec: "s".into(),
            limit: "ratio",
            value: 1,
        };
        assert_eq!(error.kind(), ErrorKind::TooLarge);
    }
}

//! Batches (M2 W5b, D-148; §1.5.10): one rebuild per operation, and no write lost.
//!
//! A batch belongs to the outermost file: `begin` on `a.zip/b.tar` opens the chain, and a change at any level goes to
//! that level's [`Overlay`], made on the first change. `finish` rebuilds the changed levels deepest first, each into the
//! overlay of the level around it, so only the outermost `commit` touches a real file and a chain is all or nothing.
//!
//! A change outside a batch is a batch of its own, so a plain `open_write` cannot be lost. A batch has an owner, the
//! thread that opened it: a change from another thread must not land in an operation that may abort, so it is refused
//! like a second `begin`. Readers of any thread see the overlay.
//!
//! The lock of a batch is held for lookups only, never across I/O: the rebuild reads members through the same paths
//! as any reader.

use std::collections::HashMap;
use std::io::{self, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

use hikasa_core::{Ctl, FileTime, Spec};

use super::cache::{CacheFile, CacheWriter};
use super::codec::{corrupt, no_panic};
use super::format::ArchiveFormat;
use super::name::{NameRules, check_name};
use super::overlay::{Overlay, Scope, check_path};
use super::rebuild::{Rebuild, empty_archive};
use super::{ArchiveConfig, Core, Index, Slot, Stamp, lock};
use crate::entry::{Entry, EntryKind};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{AtomicWrite, Finish, FollowLinks, WriteOptions, op_unsupported};

/// The reason of a change that meets another operation's batch (§1.5.10).
const BUSY: &str = "another operation is changing this archive";
/// The reason when the archive is not the one the batch started from (§1.5.10).
const CHANGED: &str = "changed since the plan was made";

/// One level of a chain: an archive with an overlay.
#[derive(Debug)]
struct Level {
    slot: Arc<Slot>,
    index: Arc<Index>,
    overlay: Arc<Overlay>,
}

/// What a reader or a rebuild sees of a level, without its lock.
#[derive(Debug, Clone)]
pub(super) struct LevelView {
    pub(super) slot: Arc<Slot>,
    pub(super) index: Arc<Index>,
    pub(super) overlay: Arc<Overlay>,
}

#[derive(Debug, Default)]
struct State {
    levels: HashMap<Spec, Level>,
    /// `finish` runs: no more changes.
    finishing: bool,
    /// A nested level is being rebuilt into its member: reading that member must not start it again.
    sealing: bool,
}

/// The batch on one outermost file.
#[derive(Debug)]
pub(super) struct Batch {
    outer: Spec,
    stamp: Stamp,
    owner: ThreadId,
    state: Mutex<State>,
}

/// What a change does to nested archives kept below the path it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Below<'a> {
    /// Nothing: the member stays what it was.
    Stay,
    /// The member goes or is replaced: what changed inside it goes too.
    Discard,
    /// The member moves onto `to`: what changed inside it is rebuilt into it first, and what changed inside a member
    /// it replaces goes.
    Seal { to: &'a str },
}

/// The file outside every archive around `spec`; `spec` itself when it is that file.
pub(super) fn outer_of(spec: &Spec) -> &Spec {
    spec.outermost_file().unwrap_or(spec)
}

/// True when `spec` lies inside the member `path` of `archive`, or names it as an archive.
fn is_below(mut spec: &Spec, archive: &Spec, path: &str) -> bool {
    while let Spec::ArchiveMember {
        archive: around,
        member,
    } = spec
    {
        let inside = member == path
            || member
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('/'));
        if **around == *archive && inside {
            return true;
        }
        spec = around;
    }
    false
}

impl Core {
    /// The batch on the outermost file `outer`.
    fn batch_of(&self, outer: &Spec) -> Option<Arc<Batch>> {
        lock(&self.batches).get(outer).cloned()
    }

    /// The level of the archive `archive`, when a batch changed it.
    pub(super) fn level_view(&self, archive: &Spec) -> Option<LevelView> {
        let batch = {
            let batches = lock(&self.batches);
            if batches.is_empty() {
                return None;
            }
            batches.get(outer_of(archive)).cloned()
        }?;
        let state = lock(&batch.state);
        let level = state.levels.get(archive)?;
        Some(LevelView {
            slot: Arc::clone(&level.slot),
            index: Arc::clone(&level.index),
            overlay: Arc::clone(&level.overlay),
        })
    }

    /// `OperationUnsupported` unless every archive around and including `at` can be written.
    fn writable(&self, at: &Spec, config: &ArchiveConfig, op: VfsOp) -> Result<(), VfsError> {
        if chain_writable(at, config) {
            Ok(())
        } else {
            Err(op_unsupported(at, op))
        }
    }

    /// The stamp of `outer` now.
    fn stamp_now(&self, outer: &Spec) -> Result<Stamp, VfsError> {
        let entry = self.base.stat(outer, FollowLinks::Yes)?;
        if entry.kind != EntryKind::File {
            return Err(corrupt(&outer.canonical(), "not a file"));
        }
        Ok(Stamp {
            len: entry.size,
            mtime: entry.mtime,
        })
    }

    fn open_batch(&self, outer: &Spec, stamp: Stamp) -> Result<Arc<Batch>, VfsError> {
        let mut batches = lock(&self.batches);
        if batches.contains_key(outer) {
            return Err(VfsError::Unavailable {
                spec: outer.canonical(),
                reason: BUSY.to_owned(),
            });
        }
        let batch = Arc::new(Batch {
            outer: outer.clone(),
            stamp,
            owner: thread::current().id(),
            state: Mutex::default(),
        });
        batches.insert(outer.clone(), Arc::clone(&batch));
        Ok(batch)
    }

    /// `Vfs::begin`. The last stamp this back-end saw is the plan's: another one now means the archive changed since
    /// the plan was made, which is cheaper to say here than after the items ran. `finish` checks the stamp again.
    pub(super) fn begin(&self, at: &Spec) -> Result<(), VfsError> {
        let config = self.config();
        self.writable(at, &config, VfsOp::Write)?;
        let outer = outer_of(at);
        if self.batch_of(outer).is_some() {
            return Err(VfsError::Unavailable {
                spec: at.canonical(),
                reason: BUSY.to_owned(),
            });
        }
        let stamp = self.stamp_now(outer).map_err(|error| match error {
            VfsError::NotFound { .. } => changed(outer),
            other => other,
        })?;
        if lock(&self.indexes)
            .stamps
            .get(outer)
            .is_some_and(|seen| *seen != stamp)
        {
            return Err(changed(outer));
        }
        self.open_batch(outer, stamp).map(drop)
    }

    /// The level of `archive` in `batch`, made on the first change.
    fn ensure_level(
        &self,
        batch: &Batch,
        archive: &Spec,
        config: &ArchiveConfig,
    ) -> Result<(), VfsError> {
        if lock(&batch.state).levels.contains_key(archive) {
            return Ok(());
        }
        let (slot, index) = self.index_of(archive, config, Ctl::detached())?;
        let overlay = Arc::new(Overlay::new(&index.tree));
        lock(&batch.state)
            .levels
            .entry(archive.clone())
            .or_insert(Level {
                slot,
                index,
                overlay,
            });
        Ok(())
    }

    /// Runs `apply` on the overlay of the archive around `spec`, in the running batch or in one of its own that
    /// rebuilds at once (§1.5.10).
    pub(super) fn change<T>(
        &self,
        spec: &Spec,
        op: VfsOp,
        below: Below<'_>,
        apply: impl FnOnce(&mut Overlay, &Scope<'_>) -> Result<T, VfsError>,
    ) -> Result<T, VfsError> {
        let Spec::ArchiveMember { archive, member } = spec else {
            return Err(op_unsupported(spec, op));
        };
        let config = self.enter(spec)?;
        self.writable(archive, &config, op)?;
        check_clean(member, spec)?;
        check_path(member, &config.limits, archive)?;
        let outer = outer_of(archive);
        let (batch, own) = match self.batch_of(outer) {
            Some(batch) if batch.owner == thread::current().id() => (batch, false),
            Some(_) => {
                return Err(VfsError::Unavailable {
                    spec: spec.canonical(),
                    reason: BUSY.to_owned(),
                });
            }
            None => (self.open_batch(outer, self.stamp_now(outer)?)?, true),
        };
        let done = self.apply_change(&batch, archive, member, below, &config, apply);
        if !own {
            return done;
        }
        match done {
            Ok(value) => self
                .end_batch(&batch, Finish::Commit, Ctl::detached())
                .map(|()| value),
            Err(error) => {
                let _ = self.end_batch(&batch, Finish::Abort, Ctl::detached());
                Err(error)
            }
        }
    }

    fn apply_change<T>(
        &self,
        batch: &Batch,
        archive: &Spec,
        member: &str,
        below: Below<'_>,
        config: &ArchiveConfig,
        apply: impl FnOnce(&mut Overlay, &Scope<'_>) -> Result<T, VfsError>,
    ) -> Result<T, VfsError> {
        self.ensure_level(batch, archive, config)?;
        if matches!(below, Below::Seal { .. }) {
            self.seal_below(batch, archive, member, config, Ctl::detached())?;
        }
        let now = (config.clock)().timestamp();
        let now = FileTime::from_unix(now.as_second(), now.subsec_nanosecond().unsigned_abs());
        let mut state = lock(&batch.state);
        if state.finishing {
            return Err(VfsError::Unavailable {
                spec: archive.canonical(),
                reason: BUSY.to_owned(),
            });
        }
        let Some(level) = state.levels.get_mut(archive) else {
            return Err(corrupt(&archive.canonical(), "inconsistent batch"));
        };
        let index = Arc::clone(&level.index);
        let scope = Scope {
            archive,
            tree: &index.tree,
            format: index.format,
            limits: config.limits,
            now,
        };
        let result = apply(Arc::make_mut(&mut level.overlay), &scope);
        let gone = match below {
            Below::Discard => Some(member),
            Below::Seal { to } => Some(to),
            Below::Stay => None,
        };
        if let (Ok(_), Some(gone)) = (&result, gone) {
            state
                .levels
                .retain(|spec, _| !is_below(spec, archive, gone));
            drop(state);
            self.forget_below(archive, gone);
            self.forget_below(archive, member);
        }
        result
    }

    /// Drops what was indexed of the archives inside the member `path` of `archive`: their bytes changed.
    fn forget_below(&self, archive: &Spec, path: &str) {
        let mut indexes = lock(&self.indexes);
        let stale: Vec<Spec> = indexes
            .slots
            .keys()
            .filter(|spec| is_below(spec, archive, path))
            .cloned()
            .collect();
        for key in stale {
            indexes.drop_slot(&key, &self.cache);
        }
    }

    /// Rebuilds the changed levels inside the member `path` of `archive` into it, deepest first: the member is about
    /// to move, or to be read as a whole.
    fn seal_below(
        &self,
        batch: &Batch,
        archive: &Spec,
        path: &str,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<(), VfsError> {
        let mut inside: Vec<Spec> = lock(&batch.state)
            .levels
            .keys()
            .filter(|spec| is_below(spec, archive, path))
            .cloned()
            .collect();
        inside.sort_by_key(|spec| std::cmp::Reverse(spec.archive_depth()));
        lock(&batch.state).sealing = true;
        let mut result = Ok(());
        for spec in inside {
            result = self.fold_level(batch, &spec, config, ctl);
            if result.is_err() {
                break;
            }
            lock(&batch.state).levels.remove(&spec);
        }
        lock(&batch.state).sealing = false;
        result
    }

    /// A whole member is read: what changed inside it counts.
    pub(super) fn seal_for_read(
        &self,
        archive: &Spec,
        member: &str,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<(), VfsError> {
        let Some(batch) = self.batch_of(outer_of(archive)) else {
            return Ok(());
        };
        let wanted = {
            let state = lock(&batch.state);
            !state.finishing
                && !state.sealing
                && batch.owner == thread::current().id()
                && state
                    .levels
                    .keys()
                    .any(|spec| is_below(spec, archive, member))
        };
        if wanted {
            self.seal_below(&batch, archive, member, config, ctl)?;
        }
        Ok(())
    }

    /// Rebuilds the nested level `spec` into a staged file and enters it in the level around it. Nothing touches a
    /// real file here (§1.5.10 step 5).
    fn fold_level(
        &self,
        batch: &Batch,
        spec: &Spec,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<(), VfsError> {
        let Spec::ArchiveMember { archive, member } = spec else {
            return Ok(());
        };
        let Some(view) = self.level_of(batch, spec) else {
            return Ok(());
        };
        if !view.overlay.changed(&view.index.tree) {
            return Ok(());
        }
        let file = self.rebuild_to_stage(spec, &view, config, ctl)?;
        self.ensure_level(batch, archive, config)?;
        let mut state = lock(&batch.state);
        let Some(parent) = state.levels.get_mut(&**archive) else {
            return Err(corrupt(&archive.canonical(), "inconsistent batch"));
        };
        let index = Arc::clone(&parent.index);
        let now = (config.clock)().timestamp();
        let scope = Scope {
            archive,
            tree: &index.tree,
            format: index.format,
            limits: config.limits,
            now: FileTime::from_unix(now.as_second(), now.subsec_nanosecond().unsigned_abs()),
        };
        Arc::make_mut(&mut parent.overlay).put_rebuilt(member, file, &scope);
        drop(state);
        self.forget_below(archive, member);
        Ok(())
    }

    fn level_of(&self, batch: &Batch, spec: &Spec) -> Option<LevelView> {
        let state = lock(&batch.state);
        let level = state.levels.get(spec)?;
        Some(LevelView {
            slot: Arc::clone(&level.slot),
            index: Arc::clone(&level.index),
            overlay: Arc::clone(&level.overlay),
        })
    }

    fn check_rebuildable(view: &LevelView, spec: &Spec) -> Result<(), VfsError> {
        match &view.index.tree.damage {
            Some(damage) => Err(corrupt(
                &spec.canonical(),
                format!("it is damaged at byte {}: {}", damage.offset, damage.reason),
            )),
            None => Ok(()),
        }
    }

    fn rebuild_to_stage(
        &self,
        spec: &Spec,
        view: &LevelView,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<Arc<CacheFile>, VfsError> {
        Core::check_rebuildable(view, spec)?;
        let text = spec.canonical();
        let mut stage: CacheWriter =
            self.cache
                .stage(&config.cache_root, config.limits.cache_max, &text)?;
        no_panic(&text, || {
            Rebuild {
                core: self,
                spec,
                view,
                config,
                ctl,
            }
            .run(&mut stage)
        })?;
        stage.finish_staged()
    }

    /// §1.5.10 steps 1–6. `Ok(false)`: no net change, nothing was written.
    fn commit_batch(
        &self,
        batch: &Batch,
        config: &ArchiveConfig,
        ctl: Ctl<'_>,
    ) -> Result<bool, VfsError> {
        let outer = &batch.outer;
        let mut nested: Vec<Spec> = {
            let mut state = lock(&batch.state);
            state.finishing = true;
            state.levels.keys().cloned().collect()
        };
        let any_change = nested.iter().any(|spec| {
            self.level_of(batch, spec)
                .is_some_and(|view| view.overlay.changed(&view.index.tree))
        });
        if !any_change {
            return Ok(false);
        }
        self.check_stamp(batch)?;
        nested.retain(|spec| spec != outer);
        nested.sort_by_key(|spec| std::cmp::Reverse(spec.archive_depth()));
        for spec in &nested {
            self.fold_level(batch, spec, config, ctl)?;
        }
        let Some(view) = self.level_of(batch, outer) else {
            return Ok(false);
        };
        if !view.overlay.changed(&view.index.tree) {
            return Ok(false);
        }
        Core::check_rebuildable(&view, outer)?;
        let options = WriteOptions {
            create_parents: false,
            overwrite: true,
            replace_link: false,
        };
        let mut writer = self.base.open_write(outer, &options)?;
        let text = outer.canonical();
        no_panic(&text, || {
            Rebuild {
                core: self,
                spec: outer,
                view: &view,
                config,
                ctl,
            }
            .run(&mut *writer)
        })?;
        ctl.check()?;
        self.check_stamp(batch)?;
        writer.commit()?;
        Ok(true)
    }

    /// Step 2: the archive is still the one the index read.
    fn check_stamp(&self, batch: &Batch) -> Result<(), VfsError> {
        match self.stamp_now(&batch.outer) {
            Ok(stamp) if stamp == batch.stamp => Ok(()),
            Ok(_) | Err(VfsError::NotFound { .. }) => Err(changed(&batch.outer)),
            Err(error) => Err(error),
        }
    }

    /// Ends `batch`, whatever `how` gives: its overlays and staged files are released.
    fn end_batch(&self, batch: &Batch, how: Finish, ctl: Ctl<'_>) -> Result<(), VfsError> {
        let config = self.config();
        let result = match how {
            Finish::Commit => self.commit_batch(batch, &config, ctl),
            Finish::Abort => Ok(false),
        };
        let outer = &batch.outer;
        lock(&self.batches).remove(outer);
        // After a commit nothing indexed holds. Otherwise the file is as it was, and only a nested archive can have
        // been read from an overlay.
        let wrote = matches!(result, Ok(true));
        let mut indexes = lock(&self.indexes);
        let stale: Vec<Spec> = indexes
            .slots
            .keys()
            .filter(|spec| outer_of(spec) == outer && (wrote || spec.archive_depth() > 0))
            .cloned()
            .collect();
        for key in stale {
            indexes.drop_slot(&key, &self.cache);
        }
        drop(indexes);
        if wrote && let Ok(stamp) = self.stamp_now(outer) {
            lock(&self.indexes).stamps.insert(outer.clone(), stamp);
        }
        result.map(drop)
    }

    /// `Vfs::finish_with`. No batch is nothing to do.
    pub(super) fn finish(&self, at: &Spec, how: Finish, ctl: Ctl<'_>) -> Result<(), VfsError> {
        match self.batch_of(outer_of(at)) {
            Some(batch) => self.end_batch(&batch, how, ctl),
            None => Ok(()),
        }
    }

    /// `Vfs::create_archive`: an empty archive through `open_write`, or a member that is one.
    pub(super) fn create_archive(self: &Arc<Core>, spec: &Spec) -> Result<(), VfsError> {
        let config = self.config();
        let name = spec.file_name().unwrap_or_default();
        let Some(format) = config.masks.detect(&name).filter(|f| f.writable()) else {
            return Err(op_unsupported(spec, VfsOp::Mkdir));
        };
        let text = spec.canonical();
        let bytes =
            empty_archive(format).map_err(|e| VfsError::from_io(text.as_str(), VfsOp::Write, e))?;
        let options = WriteOptions {
            create_parents: false,
            overwrite: false,
            replace_link: false,
        };
        let mut writer = if matches!(spec, Spec::ArchiveMember { .. }) {
            self.open_member_write(spec, &options)?
        } else {
            self.base.open_write(spec, &options)?
        };
        writer
            .write_all(&bytes)
            .map_err(|e| VfsError::from_io(text.as_str(), VfsOp::Write, e))?;
        writer.commit()
    }

    /// `Vfs::open_write` on a member: spools into a staged file, which enters the tree on `commit` (§1.5.10).
    pub(super) fn open_member_write(
        self: &Arc<Core>,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        let Spec::ArchiveMember { archive, member } = spec else {
            return Err(op_unsupported(spec, VfsOp::Write));
        };
        let config = self.enter(spec)?;
        self.writable(archive, &config, VfsOp::Write)?;
        let text = spec.canonical();
        if member.is_empty() {
            return Err(VfsError::IsADirectory { spec: text });
        }
        check_clean(member, spec)?;
        check_path(member, &config.limits, archive)?;
        self.check_target(spec, archive, member, opts)?;
        let file = self
            .cache
            .stage(&config.cache_root, config.limits.cache_max, &text)?;
        Ok(Box::new(MemberWriter {
            core: Arc::clone(self),
            spec: spec.clone(),
            file,
            hasher: Some(crc32fast::Hasher::new()),
            opts: *opts,
        }))
    }

    /// What `open_write` can say before any byte: what `commit` says again against the tree of that moment.
    fn check_target(
        &self,
        spec: &Spec,
        archive: &Spec,
        member: &str,
        opts: &WriteOptions,
    ) -> Result<(), VfsError> {
        let text = spec.canonical();
        match self.member_entry(spec, archive, member) {
            Ok(entry) if entry.kind == EntryKind::Dir => Err(VfsError::IsADirectory { spec: text }),
            Ok(_) if !opts.overwrite => Err(VfsError::AlreadyExists { spec: text }),
            Ok(_) => Ok(()),
            Err(VfsError::NotFound { .. }) => {
                let Some((parent, _)) = member.rsplit_once('/') else {
                    return Ok(());
                };
                let folder = Spec::archive_member(archive.clone(), parent);
                match self.member_entry(&folder, archive, parent) {
                    Ok(entry) if entry.kind == EntryKind::Dir => Ok(()),
                    Ok(_) => Err(VfsError::NotADirectory { spec: text }),
                    Err(VfsError::NotFound { .. }) if opts.create_parents => Ok(()),
                    Err(VfsError::NotFound { .. }) => Err(VfsError::NotFound { spec: text }),
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// The entry of a member as a reader sees it.
    fn member_entry(&self, spec: &Spec, archive: &Spec, member: &str) -> Result<Entry, VfsError> {
        let config = self.config();
        if let Some(view) = self.level_view(archive) {
            let item = view.overlay.lookup(member, spec)?;
            return Ok(Overlay::entry(member, item));
        }
        let (_, index) = self.index_of(archive, &config, Ctl::detached())?;
        Ok(super::lookup(&index, member, spec)?.entry())
    }
}

/// True when every archive around and including `at` is writable.
pub(super) fn chain_writable(at: &Spec, config: &ArchiveConfig) -> bool {
    let mut level = at;
    loop {
        let name = level.file_name().unwrap_or_default();
        if !config
            .masks
            .detect(&name)
            .is_some_and(ArchiveFormat::writable)
        {
            return false;
        }
        match level {
            Spec::ArchiveMember { archive, .. } => level = archive,
            _ => return true,
        }
    }
}

/// `changed since the plan was made`, at the file.
fn changed(outer: &Spec) -> VfsError {
    VfsError::InvalidTarget {
        spec: outer.canonical(),
        reason: CHANGED,
    }
}

/// A member's text as a writer gets it. `child` only makes clean ones, but a spec can come from elsewhere, and a name
/// rule 1 refuses must not reach a header.
pub(super) fn check_clean(member: &str, spec: &Spec) -> Result<(), VfsError> {
    match check_name(member.as_bytes(), NameRules::Tar) {
        Ok(clean) if clean == member.as_bytes() => Ok(()),
        _ => Err(VfsError::InvalidName {
            name: spec.canonical(),
        }),
    }
}

/// The writer of a member: a staged file that enters the tree on `commit`. Its `Arc` keeps the back-end alive, because
/// a commit outside a batch is a rebuild.
struct MemberWriter {
    core: Arc<Core>,
    spec: Spec,
    file: CacheWriter,
    /// The CRC of what was written in order; a writer that seeks has no such thing.
    hasher: Option<crc32fast::Hasher>,
    opts: WriteOptions,
}

impl Write for MemberWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write_all(buf)?;
        if let Some(hasher) = &mut self.hasher {
            hasher.update(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.file)
    }
}

impl Seek for MemberWriter {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.hasher = None;
        self.file.seek(to)
    }
}

impl AtomicWrite for MemberWriter {
    fn commit(self: Box<Self>) -> Result<(), VfsError> {
        let MemberWriter {
            core,
            spec,
            file,
            hasher,
            opts,
        } = *self;
        let file = file.finish_staged()?;
        let crc32 = hasher.map(crc32fast::Hasher::finalize);
        let path = match &spec {
            Spec::ArchiveMember { member, .. } => member.clone(),
            _ => String::new(),
        };
        core.change(&spec, VfsOp::Write, Below::Discard, |overlay, scope| {
            overlay.put_file(
                &path,
                file,
                crc32,
                (opts.overwrite, opts.create_parents),
                scope,
            )
        })
    }
}

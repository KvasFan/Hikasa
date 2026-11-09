//! A save keeps the file (M2 W10b, F2): [`Vfs::open_save`](crate::Vfs::open_save) of [`super::LocalVfs`].
//!
//! A save replaces a file's content, not the file. The atomic writer of `open_write` makes a new file and renames it
//! onto the old one, which then goes with whatever was the file's own: its other hard links (they keep the old
//! content), its owner, its extended attributes (POSIX ACLs, an SELinux label, `user.*`), and a file in a folder this
//! process cannot write cannot be saved at all. So a save of an existing regular file
//!
//! - writes **in place** ([`InPlaceWriter`]) when the rename would lose what the new file cannot take and the file can
//!   be written ([`writes_in_place`]: another hard link, an owner other than the saving user, a folder that cannot be
//!   written; and when making the temporary file is refused, or the new file cannot take the group or an ACL). The
//!   content goes to a spool; `commit` copies the old content aside (the caller's backup, made just before, or a private
//!   copy of its own), truncates the file, writes the spool into it and syncs it. A failure after the truncation writes
//!   the old content back from the copy, into the same file, and gives the file its old modification time again; the
//!   error says so, or, when that fails too, where the old content is kept. Nothing touches the file before `commit`,
//!   and `commit` takes one last look at it (the same file, size and time as when the writer was opened).
//! - otherwise renames as `open_write` does, the new file having taken over on its handle, before any content, the
//!   group (and owner) where they differ and the extended attributes ([`platform::take_identity`]); on Windows it is
//!   created with the hidden, system and archive attributes of the old one.
//!
//! The spool and the copy are private files (`tempfile`: `create_new`, mode 0600) named `.hikasa-save-*`, in the file's
//! folder, or in the system's temporary folder when the file's folder refuses them.

use std::fs::{self, File, FileTimes, Metadata, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hikasa_core::LocalPath;
use tempfile::TempPath;

use super::{Prepared, platform, prepare_write, prepare_write_with, resolve_links, sync_file};
use crate::entry::FileId;
use crate::error::{VfsError, VfsOp};
use crate::vfs::{AtomicWrite, Durability, WriteOptions};

/// The bytes one write of `commit` copies.
const CHUNK: u64 = 1 << 20;
/// The names of the spool and the copy start with this.
const SCRATCH_PREFIX: &str = ".hikasa-save-";

/// What decides whether a save writes in place (W10b, F2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SaveFacts {
    /// The names the file has (hard links).
    pub(crate) links: u64,
    /// Its owner, where the platform says.
    pub(crate) owner: Option<u32>,
    /// The user a new file would belong to (the effective user), where the platform says.
    pub(crate) saver: Option<u32>,
    /// A new file can be made in its folder (as far as can be told without making one).
    pub(crate) folder_writable: bool,
    /// The file itself can be written.
    pub(crate) file_writable: bool,
}

/// True when a save writes into the file instead of renaming a new one onto it: a rename would split its hard links,
/// give it another owner, or cannot be made in its folder, and the file can be written. A file this process cannot
/// write is replaced by a rename through its folder (the user agreed to that: `ReadOnlyTarget`), which is the only way.
pub(crate) fn writes_in_place(facts: &SaveFacts) -> bool {
    let other_owner =
        matches!((facts.owner, facts.saver), (Some(owner), Some(saver)) if owner != saver);
    facts.file_writable && (facts.links > 1 || other_owner || !facts.folder_writable)
}

/// [`Vfs::open_save`](crate::Vfs::open_save) of a local path: an existing regular file (links followed, as `open_write`
/// without `replace_link` does) is saved in place or by a rename that keeps what it can (module documentation); any
/// other target is `open_write`'s.
pub(super) fn prepare_save(
    path: &LocalPath,
    spec_text: &str,
    opts: &WriteOptions,
    keep: Option<PathBuf>,
) -> Result<Prepared, VfsError> {
    let existing = if opts.overwrite && !opts.replace_link {
        existing_file(path.as_path())
    } else {
        None
    };
    let Some((target, meta)) = existing else {
        return prepare_write(path, spec_text, opts);
    };
    let facts = SaveFacts {
        links: platform::links(&meta),
        owner: platform::owner(&meta),
        saver: platform::saver(),
        folder_writable: target.parent().is_none_or(platform::folder_writable),
        file_writable: platform::writable(&target) != Some(false),
    };
    let in_place = |target: PathBuf| {
        InPlaceWriter::open(target, &meta, spec_text, opts.durability, keep.clone())
            .map(Prepared::InPlace)
    };
    if writes_in_place(&facts) {
        return in_place(target);
    }
    match prepare_write_with(path, spec_text, opts, platform::dos_attributes(&meta)) {
        Ok(Prepared::Temp(writer)) => {
            let kept = writer
                .file
                .as_ref()
                .is_none_or(|file| platform::take_identity(file.get_ref(), &target, &meta));
            if kept || !facts.file_writable {
                Ok(Prepared::Temp(writer))
            } else {
                drop(writer);
                in_place(target)
            }
        }
        // The folder refuses a new file although it looked writable (an ACL, a Windows folder): the file itself may not.
        Err(VfsError::PermissionDenied { .. }) if facts.file_writable => in_place(target),
        other => other,
    }
}

/// The regular file a save of `path` replaces, links followed, with its metadata; `None` for anything else.
fn existing_file(path: &Path) -> Option<(PathBuf, Metadata)> {
    let target = resolve_links(path).ok()?;
    let meta = fs::symlink_metadata(&target).ok()?;
    meta.is_file().then_some((target, meta))
}

/// The file as `commit` expects to find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Look {
    id: Option<FileId>,
    len: u64,
    modified: Option<SystemTime>,
}

impl Look {
    fn of(meta: &Metadata) -> Look {
        Look {
            id: platform::file_id(meta),
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

/// A new private file for a spool or a copy: in `dir` (the saved file's folder), else in the system's temporary folder.
fn scratch_file(dir: Option<&Path>) -> io::Result<(File, TempPath)> {
    let mut last = None;
    if let Some(dir) = dir {
        match tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempfile_in(dir)
        {
            Ok(file) => return Ok(file.into_parts()),
            Err(e) => last = Some(e),
        }
    }
    // The system's temporary folder, as `tempfile` finds it (core reads no environment itself, §6.1).
    match tempfile::Builder::new().prefix(SCRATCH_PREFIX).tempfile() {
        Ok(file) => Ok(file.into_parts()),
        Err(e) => Err(last.map_or(e, |first| io::Error::new(first.kind(), first))),
    }
}

/// The writer of a save in place (module documentation).
#[derive(Debug)]
pub(crate) struct InPlaceWriter {
    /// The content until `commit`; `None` once committed. Dropped before `_spool_path`, which removes the file.
    spool: Option<BufWriter<File>>,
    /// Removes the spool file when the writer goes.
    _spool_path: TempPath,
    target: PathBuf,
    spec: String,
    durability: Durability,
    /// The caller's copy of the old content (the save's backup).
    keep: Option<PathBuf>,
    /// The file when the writer was opened.
    seen: Look,
}

impl InPlaceWriter {
    fn open(
        target: PathBuf,
        meta: &Metadata,
        spec: &str,
        durability: Durability,
        keep: Option<PathBuf>,
    ) -> Result<InPlaceWriter, VfsError> {
        let (spool, spool_path) =
            scratch_file(target.parent()).map_err(|e| VfsError::from_io(spec, VfsOp::Write, e))?;
        Ok(InPlaceWriter {
            spool: Some(BufWriter::new(spool)),
            _spool_path: spool_path,
            target,
            spec: spec.to_owned(),
            durability,
            keep,
            seen: Look::of(meta),
        })
    }

    fn spool(&mut self) -> io::Result<&mut BufWriter<File>> {
        self.spool
            .as_mut()
            .ok_or_else(|| io::Error::other("the writer was already committed"))
    }

    fn fail(&self, e: io::Error) -> VfsError {
        VfsError::from_io(self.spec.as_str(), VfsOp::Commit, e)
    }

    /// A failure whose message must stay (`from_io` keeps none for some kinds).
    fn fail_saying(&self, kind: io::ErrorKind, message: String) -> VfsError {
        VfsError::Io {
            spec: self.spec.clone(),
            op: VfsOp::Commit,
            source: io::Error::new(kind, message),
        }
    }

    fn commit_inner(&mut self) -> Result<(), VfsError> {
        let spool = self
            .spool
            .take()
            .ok_or_else(|| self.fail(io::Error::other("the writer was already committed")))?;
        let mut spool = spool.into_inner().map_err(|e| self.fail(e.into_error()))?;
        let len = spool.metadata().map_err(|e| self.fail(e))?.len();
        // The old content, where it can be written back from: the caller's backup, or a copy of this writer's own.
        let (keep, own) = match &self.keep {
            Some(keep) => {
                let copied = fs::metadata(keep).map_err(|e| self.fail(e))?.len();
                if copied != self.seen.len {
                    return Err(self.fail(io::Error::other(format!(
                        "the backup {} does not hold the file",
                        keep.display()
                    ))));
                }
                (keep.clone(), None)
            }
            None => {
                let (mut copy, path) =
                    scratch_file(self.target.parent()).map_err(|e| self.fail(e))?;
                let mut old = File::open(&self.target).map_err(|e| self.fail(e))?;
                io::copy(&mut old, &mut copy).map_err(|e| self.fail(e))?;
                if self.durability != Durability::None {
                    sync_file(&copy).map_err(|e| self.fail(e))?;
                }
                (path.to_path_buf(), Some(path))
            }
        };
        let mut file = OpenOptions::new()
            .write(true)
            .open(&self.target)
            .map_err(|e| self.fail(e))?;
        // One last look: the file this writer was opened on, as it was.
        let now = file.metadata().map_err(|e| self.fail(e))?;
        if Look::of(&now) != self.seen {
            return Err(VfsError::InvalidTarget {
                spec: self.spec.clone(),
                reason: "changed since the plan was made",
            });
        }
        let Err(error) = overwrite(&mut file, &mut spool, len, self.durability) else {
            return Ok(());
        };
        // The file may be cut short: its old content goes back into it.
        let restored = File::open(&keep).and_then(|mut from| {
            let len = from.metadata()?.len();
            overwrite(&mut file, &mut from, len, Durability::FileOnly)?;
            if let Some(modified) = self.seen.modified {
                let _ = file.set_times(FileTimes::new().set_modified(modified));
            }
            Ok(())
        });
        let kind = error.kind();
        Err(match restored {
            Ok(()) => {
                let from = match own {
                    Some(_) => "a copy of its old content".to_owned(),
                    None => format!("its backup {}", keep.display()),
                };
                self.fail_saying(kind, format!("{error}; the file was restored from {from}"))
            }
            Err(back) => {
                if let Some(own) = own {
                    // Kept for the user: the only place the old content is.
                    let _ = own.keep();
                }
                self.fail_saying(
                    kind,
                    format!(
                        "{error}; the file's old content is kept as {} ({back})",
                        keep.display()
                    ),
                )
            }
        })
    }
}

/// Writes the `len` bytes of `from` into `to` from its start, cut to them, and syncs it unless `durability` is `None`.
fn overwrite(to: &mut File, from: &mut File, len: u64, durability: Durability) -> io::Result<()> {
    step(Step::Truncate)?;
    to.set_len(0)?;
    to.seek(SeekFrom::Start(0))?;
    from.seek(SeekFrom::Start(0))?;
    let mut done = 0;
    while done < len {
        step(Step::Write)?;
        // File to file: `copy_file_range` on Linux, a clone where the file system shares blocks.
        let n = io::copy(&mut (&mut *from).take((len - done).min(CHUNK)), to)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    if durability != Durability::None {
        step(Step::Sync)?;
        sync_file(to)?;
    }
    Ok(())
}

/// A step of [`overwrite`] that can fail in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Truncate,
    Write,
    Sync,
}

/// Each step of [`overwrite`] asks here first: tests make the n-th one fail ([`faults`]).
#[cfg(test)]
fn step(step: Step) -> io::Result<()> {
    faults::step(step)
}

#[cfg(not(test))]
fn step(_step: Step) -> io::Result<()> {
    Ok(())
}

impl Write for InPlaceWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.spool()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.spool()?.flush()
    }
}

impl Seek for InPlaceWriter {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.spool()?.seek(to)
    }
}

impl AtomicWrite for InPlaceWriter {
    /// Flushes the spool: it is not the file, and only the file is synced (by `commit`).
    fn sync(&mut self) -> Result<(), VfsError> {
        let flushed = self.spool().and_then(Write::flush);
        flushed.map_err(|e| self.fail(e))
    }

    fn commit(mut self: Box<Self>) -> Result<(), VfsError> {
        self.commit_inner()
    }

    fn in_place(&self) -> bool {
        true
    }
}

impl Drop for InPlaceWriter {
    /// Closes the spool, whose file `_spool_path` then removes (Windows removes no open file).
    fn drop(&mut self) {
        drop(self.spool.take());
    }
}

/// Fault injection of [`overwrite`] in this crate's tests: the steps of this thread are counted from 1, and the ones
/// named fail with an `io` error.
#[cfg(test)]
pub(crate) mod faults {
    use std::io;
    use std::sync::Mutex;
    use std::thread::ThreadId;

    use super::Step;

    /// Per test thread (no `thread_local!` in core, §6.1 check 2): the steps counted, the ones that fail, the ones taken.
    type State = (ThreadId, u32, Vec<u32>, Vec<Step>);
    static STATE: Mutex<Vec<State>> = Mutex::new(Vec::new());

    /// Runs `f` on this thread's state.
    fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
        let me = std::thread::current().id();
        let mut all = STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let at = match all.iter().position(|s| s.0 == me) {
            Some(at) => at,
            None => {
                all.push((me, 0, Vec::new(), Vec::new()));
                all.len() - 1
            }
        };
        f(&mut all[at])
    }

    /// Steps `at` (counted from 1 from now) fail.
    #[cfg(unix)]
    pub(crate) fn fail_at(at: &[u32]) {
        with(|state| {
            state.1 = 0;
            state.2 = at.to_vec();
            state.3.clear();
        });
    }

    /// The steps taken since `fail_at`.
    #[cfg(unix)]
    pub(crate) fn taken() -> Vec<Step> {
        with(|state| state.3.clone())
    }

    pub(super) fn step(step: Step) -> io::Result<()> {
        with(|state| {
            state.1 += 1;
            state.3.push(step);
            if state.2.contains(&state.1) {
                return Err(io::Error::other(format!("injected fault at {step:?}")));
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(links: u64, owner: u32, saver: u32, folder: bool, file: bool) -> SaveFacts {
        SaveFacts {
            links,
            owner: Some(owner),
            saver: Some(saver),
            folder_writable: folder,
            file_writable: file,
        }
    }

    /// The decision is a function of the facts: any of another link, another owner, a folder that takes no new file
    /// makes the save write in place, and only a file that can be written is written in place.
    #[test]
    fn a_save_writes_in_place_when_a_rename_would_lose_the_file() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        assert!(!writes_in_place(&facts(1, 1000, 1000, true, true)));
        assert!(
            writes_in_place(&facts(2, 1000, 1000, true, true)),
            "a hard link"
        );
        assert!(
            writes_in_place(&facts(1, 0, 1000, true, true)),
            "another owner"
        );
        assert!(
            writes_in_place(&facts(1, 1000, 0, true, true)),
            "root saves a user's file"
        );
        assert!(
            writes_in_place(&facts(1, 1000, 1000, false, true)),
            "a read-only folder"
        );
        assert!(writes_in_place(&facts(3, 0, 1000, false, true)));
        for (links, owner, folder) in [
            (2, 1000, true),
            (1, 0, true),
            (1, 1000, false),
            (1, 1000, true),
        ] {
            assert!(
                !writes_in_place(&facts(links, owner, 1000, folder, false)),
                "a file that cannot be written: {links} {owner} {folder}"
            );
        }
        let unknown = SaveFacts {
            owner: None,
            saver: None,
            ..facts(1, 0, 0, true, true)
        };
        assert!(
            !writes_in_place(&unknown),
            "an owner nobody can tell is no reason"
        );
    }

    // ---- on disk: the writer itself ------------------------------------------------------------------------------

    use hikasa_core::{PathStyle, Spec, SpecContext};

    use crate::Vfs;
    use crate::local::LocalVfs;

    /// More than two chunks, so a save makes several write calls.
    #[cfg(unix)]
    const BIG: usize = (2 * CHUNK as usize) + (CHUNK as usize / 2);

    fn spec(path: &Path) -> Spec {
        Spec::parse(
            path.to_str().unwrap(),
            &SpecContext::absolute_only(PathStyle::HOST),
        )
        .unwrap()
    }

    fn save_options() -> WriteOptions {
        WriteOptions {
            create_parents: false,
            overwrite: true,
            replace_link: false,
            durability: Durability::Full,
        }
    }

    /// The old and the new content: different at every chunk.
    #[cfg(unix)]
    fn contents() -> (Vec<u8>, Vec<u8>) {
        let old: Vec<u8> = (0..BIG).map(|i| (i % 251) as u8).collect();
        let new: Vec<u8> = (0..BIG + 4096).map(|i| (i % 241) as u8 ^ 0x5a).collect();
        (old, new)
    }

    /// The scratch files a save left in `dir`.
    #[cfg(unix)]
    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(SCRATCH_PREFIX))
            .collect()
    }

    /// A file with another hard link, saved through one name. `backup`: the caller's copy, made as `run_write` makes it.
    #[cfg(unix)]
    struct Linked {
        _dir: tempfile::TempDir,
        root: PathBuf,
        file: PathBuf,
        link: PathBuf,
        backup: Option<PathBuf>,
        modified: SystemTime,
    }

    #[cfg(unix)]
    fn linked(old: &[u8], backup: bool) -> Linked {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("file.txt");
        let link = root.join("link.txt");
        fs::write(&file, old).unwrap();
        let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        fs::hard_link(&file, &link).unwrap();
        let backup = backup.then(|| {
            let backup = root.join("file.txt.bak");
            fs::copy(&file, &backup).unwrap();
            backup
        });
        Linked {
            _dir: dir,
            root,
            file,
            link,
            backup,
            modified,
        }
    }

    #[cfg(unix)]
    fn open_save(at: &Linked) -> Box<dyn AtomicWrite> {
        let keep = at.backup.as_deref().map(spec);
        LocalVfs::default()
            .open_save(&spec(&at.file), &save_options(), keep.as_ref())
            .unwrap()
    }

    #[cfg(unix)]
    fn inode(path: &Path) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(path).unwrap();
        (meta.ino(), meta.nlink())
    }

    /// A file with another hard link is written in place: both names show the new content, the steps are a truncation,
    /// one write per chunk and a sync, and nothing is left behind.
    #[cfg(unix)]
    #[test]
    fn a_linked_file_is_written_in_place() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        let (old, new) = contents();
        for backup in [false, true] {
            let at = linked(&old, backup);
            let before = inode(&at.file);
            let mut writer = open_save(&at);
            assert!(writer.in_place());
            writer.write_all(&new).unwrap();
            assert_eq!(
                fs::read(&at.file).unwrap(),
                old,
                "nothing before the commit"
            );
            faults::fail_at(&[]);
            writer.commit().unwrap();
            assert_eq!(
                faults::taken(),
                [
                    Step::Truncate,
                    Step::Write,
                    Step::Write,
                    Step::Write,
                    Step::Sync
                ]
            );
            assert_eq!(fs::read(&at.file).unwrap(), new);
            assert_eq!(fs::read(&at.link).unwrap(), new, "the other name");
            assert_eq!(inode(&at.file), before, "the same file, its two names");
            assert!(leftovers(&at.root).is_empty(), "{:?}", leftovers(&at.root));
        }
    }

    /// Fault injection: a failure at every step of the commit after the file was opened (the truncation, each write, the
    /// sync) gives the file its old bytes back, in the same inode, with its old time, from the backup or from the
    /// writer's own copy; the error says so, and the writer's copy goes.
    #[cfg(unix)]
    #[test]
    fn a_failure_at_every_step_restores_the_old_content() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        let (old, new) = contents();
        let steps = 5;
        for backup in [false, true] {
            for at_step in 1..=steps {
                let at = linked(&old, backup);
                let before = inode(&at.file);
                let mut writer = open_save(&at);
                writer.write_all(&new).unwrap();
                faults::fail_at(&[at_step]);
                let error = writer.commit().unwrap_err().to_string();
                let taken = faults::taken();
                // The steps of the save up to the failure, then the restore's: a truncation, three writes, a sync.
                assert_eq!(taken.len() as u32, at_step + steps, "{taken:?}");
                let what = format!("step {at_step}, backup {backup}: {error}");
                assert_eq!(fs::read(&at.file).unwrap(), old, "{what}");
                assert_eq!(fs::read(&at.link).unwrap(), old, "{what}");
                assert_eq!(inode(&at.file), before, "{what}");
                assert_eq!(
                    fs::metadata(&at.file).unwrap().modified().unwrap(),
                    at.modified,
                    "{what}"
                );
                assert!(error.contains("injected fault"), "{what}");
                if backup {
                    assert!(error.contains("restored from its backup"), "{what}");
                    assert_eq!(fs::read(at.backup.as_ref().unwrap()).unwrap(), old);
                } else {
                    assert!(
                        error.contains("restored from a copy of its old content"),
                        "{what}"
                    );
                }
                assert!(leftovers(&at.root).is_empty(), "{:?}", leftovers(&at.root));
            }
        }
    }

    /// When the old content cannot be written back either, the error names where it is: the backup, or the writer's
    /// copy, which then stays.
    #[cfg(unix)]
    #[test]
    fn a_failed_restore_says_where_the_old_content_is() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        let (old, new) = contents();
        for backup in [false, true] {
            let at = linked(&old, backup);
            let mut writer = open_save(&at);
            writer.write_all(&new).unwrap();
            // The second write of the save fails, and the first write of the restore.
            faults::fail_at(&[3, 5]);
            let error = writer.commit().unwrap_err().to_string();
            assert!(
                error.contains("the file's old content is kept as"),
                "{error}"
            );
            let kept = match &at.backup {
                Some(backup) => backup.clone(),
                None => {
                    let left = leftovers(&at.root);
                    assert_eq!(left.len(), 1, "{left:?}");
                    at.root.join(&left[0])
                }
            };
            assert!(error.contains(&kept.display().to_string()), "{error}");
            assert_eq!(fs::read(&kept).unwrap(), old);
        }
    }

    /// Dropping the writer (a cancel, a failed read of the content) leaves the file and no spool.
    #[cfg(unix)]
    #[test]
    fn a_writer_dropped_before_the_commit_leaves_the_file() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        let (old, new) = contents();
        let at = linked(&old, false);
        let mut writer = open_save(&at);
        writer.write_all(&new).unwrap();
        assert!(!leftovers(&at.root).is_empty(), "the spool");
        drop(writer);
        assert_eq!(fs::read(&at.file).unwrap(), old);
        assert!(leftovers(&at.root).is_empty(), "{:?}", leftovers(&at.root));
    }

    /// A file that changed after the writer was opened is not written: the commit's last look refuses it.
    #[cfg(unix)]
    #[test]
    fn a_file_changed_since_the_open_is_not_written() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        let (old, new) = contents();
        let at = linked(&old, false);
        let mut writer = open_save(&at);
        writer.write_all(&new).unwrap();
        fs::write(&at.link, b"changed by someone").unwrap();
        let error = writer.commit().unwrap_err().to_string();
        assert!(error.contains("changed since the plan was made"), "{error}");
        assert_eq!(fs::read(&at.file).unwrap(), b"changed by someone");
    }

    /// A file of one name, owned by this user, in a folder it can write: renamed as before, not in place.
    #[test]
    fn a_plain_file_is_renamed() {
        // The sync counter of `local::tests` is global: one syncing test at a time in a process.
        let _one = crate::local::tests::serial();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.txt");
        fs::write(&file, b"old").unwrap();
        let writer = LocalVfs::default()
            .open_save(&spec(&file), &save_options(), None)
            .unwrap();
        assert!(!writer.in_place());
    }
}

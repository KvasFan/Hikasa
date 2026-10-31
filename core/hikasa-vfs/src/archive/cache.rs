//! The temp cache (D-140; §1.5.6): the only part of `archive/` that calls `std::fs`.
//!
//! `<root>/s<n>.lock` and `<root>/s<n>/` (mode 0700 on Unix); inside, files with numeric names only, so **no member
//! name ever reaches a path**. A held `File::try_lock` marks a live session; a later session sweeps the folders whose
//! lock it can take, so a crash cleans itself without process ids or clocks. Nothing is created by listing.
//!
//! Sessions meet in one folder, and these rules keep them apart on every platform:
//!
//! - `n` is random, never the lowest free number: a dying path cannot come back as someone else's session.
//! - A lock only counts for the file at the path: after `try_lock`, [`is_at`] compares the handle and the path, so a
//!   handle opened before another sweep unlinked that file proves nothing.
//! - The folder goes first and the lock file's name second, both while the lock is held. Unlinking an open file works
//!   on Windows too, because Rust opens files with `FILE_SHARE_DELETE`; where the file system only marks it for
//!   deletion, the name refuses a new open, which [`is_at`] reads as "gone", and a random `n` never asks for it again.
//! - A folder that cannot be removed keeps its lock file, so a later sweep finds it (a file open on Windows).
//! - Each file starts with [`STAMP`] bytes naming its session and number, checked at every open: whatever else goes
//!   wrong in the folder, a reader gets an error and never the bytes of another file.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use same_file::Handle;

use super::codec::STEP;
use super::shared::RangeReader;
use crate::error::{VfsError, VfsOp};

/// Key of a cached file: the id of an archive at one stamp, and a member (or [`WHOLE_STREAM`]).
type Key = (u64, Box<str>);

/// The member text of the cache file that holds a whole decoded stream (a member path is never empty).
pub(crate) const WHOLE_STREAM: &str = "";

/// Bytes at the start of each cache file: the session's number and the file's, little-endian. Not counted in `used`.
const STAMP: usize = 16;

/// One complete file of the cache. Each reader holds an `Arc`, so a held file is never evicted.
#[derive(Debug)]
pub(crate) struct CacheFile {
    path: PathBuf,
    /// Bytes of content, after the stamp.
    len: u64,
    stamp: [u8; STAMP],
    used: Arc<AtomicU64>,
}

impl Drop for CacheFile {
    fn drop(&mut self) {
        // A failure is left to the sweep of `cleanup` or of a later session.
        let _ = fs::remove_file(&self.path);
        self.used.fetch_sub(self.len, Ordering::Relaxed);
    }
}

impl CacheFile {
    /// A reader of the file, which holds it. It checks that the file at the path is the one that was written: the
    /// open handle is what it reads afterwards, so nothing that happens to the path later reaches it.
    pub(crate) fn reader(self: &Arc<CacheFile>) -> Result<CacheReader, VfsError> {
        let open = || {
            let mut file = File::open(&self.path)?;
            let mut stamp = [0u8; STAMP];
            file.read_exact(&mut stamp)?;
            if stamp != self.stamp || file.metadata()?.len() != STAMP as u64 + self.len {
                return Err(io::Error::other("a cache file was replaced"));
            }
            Ok(file)
        };
        let file =
            open().map_err(|e| VfsError::from_io(self.path.to_string_lossy(), VfsOp::Open, e))?;
        Ok(CacheReader {
            file: RangeReader::new(file, STAMP as u64, self.len),
            _hold: Arc::clone(self),
        })
    }
}

/// A reader of a cache file: its content, without the stamp.
pub(crate) struct CacheReader {
    file: RangeReader<File>,
    _hold: Arc<CacheFile>,
}

impl Read for CacheReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for CacheReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.file.seek(to)
    }
}

/// A slot of the table.
struct Slot {
    file: Arc<CacheFile>,
    last_use: u64,
}

/// The session's folder and its held lock.
struct Session {
    number: u64,
    dir: PathBuf,
    lock_path: PathBuf,
    lock: Handle,
    root: PathBuf,
    /// The folders above `root` this session had to create, deepest first.
    made: Vec<PathBuf>,
}

impl Session {
    /// Removes the folder, then the lock file's name while the lock is held (the module's rules), then every folder
    /// that is left empty, so a session leaves nothing behind. Best effort.
    fn close(self) {
        if remove_tree(&self.dir).is_ok() {
            let _ = fs::remove_file(&self.lock_path);
        }
        drop(self.lock);
        for dir in std::iter::once(&self.root).chain(&self.made) {
            if fs::remove_dir(dir).is_err() {
                break;
            }
        }
    }
}

#[derive(Default)]
struct State {
    session: Option<Session>,
    files: HashMap<Key, Slot>,
    tick: u64,
    next: u64,
}

/// The cache of one [`super::ArchiveVfs`].
#[derive(Default)]
pub(crate) struct Cache {
    state: Mutex<State>,
    /// The sum of the lengths of the cache files and of the reservations of the running writes.
    used: Arc<AtomicU64>,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache")
            .field("used", &self.used.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Cache {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Bytes in use.
    pub(crate) fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }

    /// The session's folder, once one exists.
    pub(crate) fn dir(&self) -> Option<PathBuf> {
        self.lock().session.as_ref().map(|s| s.dir.clone())
    }

    /// A reader of the cached file of `key`, marked as used. A file that fails its check leaves the table and counts
    /// as a miss: the caller decodes again.
    pub(crate) fn get(&self, archive: u64, member: &str) -> Option<CacheReader> {
        let mut state = self.lock();
        state.tick += 1;
        let tick = state.tick;
        let key = (archive, Box::<str>::from(member));
        let slot = state.files.get_mut(&key)?;
        slot.last_use = tick;
        let file = Arc::clone(&slot.file);
        drop(state);
        let reader = file.reader().ok();
        if reader.is_none() {
            self.lock().files.remove(&key);
        }
        reader
    }

    /// What a new file can take: the limit minus what readers or writers hold.
    pub(crate) fn free(&self, cache_max: u64) -> u64 {
        let state = self.lock();
        let evictable: u64 = state
            .files
            .values()
            .filter(|slot| Arc::strong_count(&slot.file) == 1)
            .map(|slot| slot.file.len)
            .sum();
        cache_max.saturating_sub(self.used().saturating_sub(evictable))
    }

    /// Makes room for `bytes` more, evicting least recently used files no reader holds; else `ArchiveLimit`.
    fn reserve(&self, bytes: u64, cache_max: u64, spec: &str) -> Result<(), VfsError> {
        let mut state = self.lock();
        while self.used().saturating_add(bytes) > cache_max {
            let victim = state
                .files
                .iter()
                .filter(|(_, slot)| Arc::strong_count(&slot.file) == 1)
                .min_by_key(|(_, slot)| slot.last_use)
                .map(|(key, _)| key.clone());
            let Some(key) = victim else {
                return Err(VfsError::ArchiveLimit {
                    spec: spec.to_owned(),
                    limit: "cache",
                    value: cache_max,
                });
            };
            state.files.remove(&key);
        }
        self.used.fetch_add(bytes, Ordering::Relaxed);
        Ok(())
    }

    /// Starts a cache file. `known` is its length when known: it is reserved at once, so a member over the limit
    /// creates nothing.
    pub(crate) fn writer(
        &self,
        root: &Path,
        known: Option<u64>,
        cache_max: u64,
        spec: &str,
    ) -> Result<CacheWriter<'_>, VfsError> {
        let reserved = known.unwrap_or(0);
        self.reserve(reserved, cache_max, spec)?;
        let fail = |e: io::Error| VfsError::from_io(spec, VfsOp::Write, e);
        let created = (|| {
            let mut state = self.lock();
            if state.session.is_none() {
                state.session = Some(open_session(root).map_err(fail)?);
            }
            state.next += 1;
            let number = state.next;
            let session = state.session.as_ref().map(|s| (s.dir.clone(), s.number));
            drop(state);
            let (dir, session) = session.unwrap_or_default();
            let mut stamp = [0u8; STAMP];
            stamp[..8].copy_from_slice(&session.to_le_bytes());
            stamp[8..].copy_from_slice(&number.to_le_bytes());
            let path = dir.join(format!("m{number}"));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(fail)?;
            file.write_all(&stamp).map_err(fail)?;
            Ok::<_, VfsError>((path, file, stamp))
        })();
        let (path, file, stamp) = match created {
            Ok(done) => done,
            Err(error) => {
                self.used.fetch_sub(reserved, Ordering::Relaxed);
                return Err(error);
            }
        };
        Ok(CacheWriter {
            cache: self,
            file: BufWriter::with_capacity(1 << 16, file),
            path,
            stamp,
            written: 0,
            reserved,
            cache_max,
            spec: spec.to_owned(),
            done: false,
        })
    }

    /// Enters a complete file in the table, replacing an older one of `key`.
    fn enter(&self, key: Key, file: Arc<CacheFile>) {
        let mut state = self.lock();
        state.tick += 1;
        let last_use = state.tick;
        state.files.insert(key, Slot { file, last_use });
    }

    /// Drops every file of the archive `id` (a new stamp).
    pub(crate) fn drop_archive(&self, id: u64) {
        self.lock().files.retain(|key, _| key.0 != id);
    }

    /// Removes the session's folder and lock file (best effort). A later spool opens a new session.
    pub(crate) fn cleanup(&self) {
        let mut state = self.lock();
        state.files.clear();
        if let Some(session) = state.session.take() {
            session.close();
        }
    }
}

/// A cache file being written. Dropped before [`CacheWriter::finish`], it removes the partial file and gives its
/// reservation back.
pub(crate) struct CacheWriter<'a> {
    cache: &'a Cache,
    file: BufWriter<File>,
    path: PathBuf,
    stamp: [u8; STAMP],
    written: u64,
    reserved: u64,
    cache_max: u64,
    spec: String,
    done: bool,
}

impl CacheWriter<'_> {
    /// Appends `bytes`, reserving 1 MiB at a time past what is reserved.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<(), VfsError> {
        let needed = self.written + bytes.len() as u64;
        while needed > self.reserved {
            self.cache.reserve(STEP, self.cache_max, &self.spec)?;
            self.reserved += STEP;
        }
        self.file
            .write_all(bytes)
            .map_err(|e| VfsError::from_io(self.spec.as_str(), VfsOp::Write, e))?;
        self.written = needed;
        Ok(())
    }

    /// Completes the file and enters it in the table.
    pub(crate) fn finish(mut self, archive: u64, member: &str) -> Result<Arc<CacheFile>, VfsError> {
        self.file
            .flush()
            .map_err(|e| VfsError::from_io(self.spec.as_str(), VfsOp::Write, e))?;
        self.done = true;
        let extra = self.reserved - self.written;
        self.cache.used.fetch_sub(extra, Ordering::Relaxed);
        let file = Arc::new(CacheFile {
            path: self.path.clone(),
            len: self.written,
            stamp: self.stamp,
            used: Arc::clone(&self.cache.used),
        });
        self.cache
            .enter((archive, member.into()), Arc::clone(&file));
        Ok(file)
    }
}

impl Drop for CacheWriter<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = fs::remove_file(&self.path);
            self.cache.used.fetch_sub(self.reserved, Ordering::Relaxed);
        }
    }
}

/// `s<n>.lock` → `n`.
fn session_number(name: &str) -> Option<u64> {
    let digits = name.strip_prefix('s')?.strip_suffix(".lock")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Claims a number, sweeps, and creates the session's folder.
fn open_session(root: &Path) -> io::Result<Session> {
    let made = make_root(root)?;
    let (number, lock_path, lock) = claim(root)?;
    let session = Session {
        number,
        dir: root.join(format!("s{number}")),
        lock_path,
        lock,
        root: root.to_path_buf(),
        made,
    };
    if let Err(error) = owned(root, &session.lock_path) {
        session.close();
        return Err(error);
    }
    sweep(root, number)?;
    create_private_dir(&session.dir)?;
    Ok(session)
}

/// Creates `root` and returns the folders above it that did not exist, deepest first.
fn make_root(root: &Path) -> io::Result<Vec<PathBuf>> {
    let made = root
        .ancestors()
        .skip(1)
        .take_while(|dir| !dir.as_os_str().is_empty() && !dir.exists())
        .map(Path::to_path_buf)
        .collect();
    fs::create_dir_all(root)?;
    Ok(made)
}

/// True when `path` names the file behind `lock`. A failure counts as no: a name marked for deletion refuses opens.
fn is_at(lock: &Handle, path: &Path) -> bool {
    Handle::from_path(path).is_ok_and(|at| at == *lock)
}

/// A number no session has: the keys of a `RandomState` come from the operating system.
fn fresh_number() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// A new lock file, locked. Random numbers do not collide, so a retry only follows a sweep that took the new file
/// between `create_new` and `try_lock`, or another session that removed `root` while it left.
fn claim(root: &Path) -> io::Result<(u64, PathBuf, Handle)> {
    const TRIES: u32 = 256;
    for _ in 0..TRIES {
        let number = fresh_number();
        let path = root.join(format!("s{number}.lock"));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(root)?;
                continue;
            }
            // A folder marked for deletion by a leaving session refuses new files until it is gone.
            Err(error) if cfg!(windows) && error.kind() == io::ErrorKind::PermissionDenied => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let lock = Handle::from_file(file)?;
        match lock.as_file().try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => continue,
            Err(TryLockError::Error(error)) => return Err(error),
        }
        if is_at(&lock, &path) {
            return Ok((number, path, lock));
        }
    }
    Err(io::Error::other("no free cache session number"))
}

/// Removes the folder and the lock file of every other session whose lock can be taken: its owner is gone. A lock
/// that fails with any error counts as held.
fn sweep(root: &Path, own: u64) -> io::Result<()> {
    for item in fs::read_dir(root)? {
        let Ok(item) = item else { continue };
        let name = item.file_name();
        let Some(number) = name.to_str().and_then(session_number) else {
            continue;
        };
        if number == own {
            continue;
        }
        let path = item.path();
        let Ok(file) = OpenOptions::new().write(true).open(&path) else {
            continue;
        };
        let Ok(lock) = Handle::from_file(file) else {
            continue;
        };
        if lock.as_file().try_lock().is_ok()
            && is_at(&lock, &path)
            && remove_tree(&root.join(format!("s{number}"))).is_ok()
        {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

/// `remove_dir_all`; a folder that is not there is removed.
fn remove_tree(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// `root` and its parent belong to whoever owns `lock` (just created: this user) or to the system. The fallback root
/// lies in the shared temp folder, where another user may have made it first and could swap what is in it.
#[cfg(unix)]
fn owned(root: &Path, lock: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let me = fs::metadata(lock)?.uid();
    for dir in root.ancestors().take(2) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if !trusted(fs::symlink_metadata(dir)?.uid(), me) {
            let message = format!("{} belongs to another user", dir.display());
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, message));
        }
    }
    Ok(())
}

/// See [`owned`]: Windows keeps the fallback in the user's own temp folder.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn owned(_root: &Path, _lock: &Path) -> io::Result<()> {
    Ok(())
}

/// True when a folder of `owner` can hold the cache of `me`.
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn trusted(owner: u32, me: u32) -> bool {
    owner == me || owner == 0
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_numbers() {
        assert_eq!(session_number("s7.lock"), Some(7));
        assert_eq!(session_number("s.lock"), None);
        assert_eq!(session_number("s7"), None);
        assert_eq!(session_number("x7.lock"), None);
        assert_eq!(session_number("s7a.lock"), None);
    }

    #[test]
    fn numbers_differ() {
        assert_ne!(fresh_number(), fresh_number());
    }

    /// A handle opened before a file left its path proves nothing about the file now at that path.
    #[test]
    fn a_handle_counts_only_for_the_file_at_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s1.lock");
        let lock = Handle::from_file(File::create(&path).unwrap()).unwrap();
        assert!(is_at(&lock, &path));
        fs::rename(&path, dir.path().join("moved")).unwrap();
        assert!(!is_at(&lock, &path), "nothing there");
        File::create(&path).unwrap();
        assert!(!is_at(&lock, &path), "another file there");
    }

    #[test]
    fn a_folder_of_another_user_is_refused() {
        assert!(trusted(1000, 1000));
        assert!(trusted(0, 1000), "the system's /tmp");
        assert!(!trusted(1001, 1000));
        assert!(!trusted(1000, 0), "root never trusts a user's folder");
    }
}

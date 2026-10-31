//! The temp cache (D-140; §1.5.6): the only part of `archive/` that calls `std::fs`.
//!
//! `<root>/s<n>.lock` and `<root>/s<n>/` (mode 0700 on Unix); inside, files with numeric names only, so **no member
//! name ever reaches a path**. A held `File::try_lock` marks a live session; a later session sweeps the folders whose
//! lock it can take, so a crash cleans itself without process ids or clocks. Nothing is created by listing.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::codec::STEP;
use crate::error::{VfsError, VfsOp};

/// Key of a cached file: the id of an archive at one stamp, and a member (or [`WHOLE_STREAM`]).
type Key = (u64, Box<str>);

/// The member text of the cache file that holds a whole decoded stream (a member path is never empty).
pub(crate) const WHOLE_STREAM: &str = "";

/// One complete file of the cache. Each reader holds an `Arc`, so a held file is never evicted.
#[derive(Debug)]
pub(crate) struct CacheFile {
    path: PathBuf,
    len: u64,
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
    /// A reader of the file, which holds it.
    pub(crate) fn reader(self: &Arc<CacheFile>) -> Result<CacheReader, VfsError> {
        let file = File::open(&self.path)
            .map_err(|e| VfsError::from_io(self.path.to_string_lossy(), VfsOp::Open, e))?;
        Ok(CacheReader {
            file,
            _hold: Arc::clone(self),
        })
    }
}

/// A reader of a cache file.
pub(crate) struct CacheReader {
    file: File,
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
    dir: PathBuf,
    lock_path: PathBuf,
    lock: File,
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

    /// The cached file of `key`, marked as used.
    pub(crate) fn get(&self, archive: u64, member: &str) -> Option<Arc<CacheFile>> {
        let mut state = self.lock();
        state.tick += 1;
        let tick = state.tick;
        let slot = state.files.get_mut(&(archive, member.into()))?;
        slot.last_use = tick;
        Some(Arc::clone(&slot.file))
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
            let dir = state.session.as_ref().map(|s| s.dir.clone());
            drop(state);
            let path = dir.unwrap_or_default().join(format!("m{number}"));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(fail)?;
            Ok::<_, VfsError>((path, file))
        })();
        let (path, file) = match created {
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
            let _ = fs::remove_dir_all(&session.dir);
            let Session {
                lock_path, lock, ..
            } = session;
            drop(lock);
            let _ = fs::remove_file(lock_path);
        }
    }
}

/// A cache file being written. Dropped before [`CacheWriter::finish`], it removes the partial file and gives its
/// reservation back.
pub(crate) struct CacheWriter<'a> {
    cache: &'a Cache,
    file: BufWriter<File>,
    path: PathBuf,
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

/// Sweeps, then claims the first free number and creates its folder.
fn open_session(root: &Path) -> io::Result<Session> {
    fs::create_dir_all(root)?;
    let (number, lock_path, lock) = claim(root)?;
    sweep(root, number)?;
    let dir = root.join(format!("s{number}"));
    match fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    create_private_dir(&dir)?;
    Ok(Session {
        dir,
        lock_path,
        lock,
    })
}

/// The first number whose lock file can be created and locked.
fn claim(root: &Path) -> io::Result<(u64, PathBuf, File)> {
    const TRIES: u64 = 1 << 20;
    for number in 1..TRIES {
        let path = root.join(format!("s{number}.lock"));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => continue,
            Err(TryLockError::Error(error)) => return Err(error),
        }
        // A sweep may have taken the new file between `create_new` and `try_lock`.
        if path.exists() {
            return Ok((number, path, file));
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
        let Ok(file) = OpenOptions::new().write(true).open(item.path()) else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        let _ = fs::remove_dir_all(root.join(format!("s{number}")));
        drop(file);
        let _ = fs::remove_file(item.path());
    }
    Ok(())
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
}

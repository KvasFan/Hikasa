//! The writer's lock of a file (M2 W5b; §1.5.10) and what a dead writer leaves next to it.
//!
//! A rebuild reads a whole file and replaces it, so two at once lose one of them whatever the stamps say: both can pass
//! a last look at the file. The lock is an advisory `File::try_lock` on `.<name>.hikasa-lock`, a sibling of the file
//! that is replaced, which is what every spelling and link of it shares. The archive itself cannot carry the lock: a
//! rename leaves the lock on the old inode, and on Windows a lock on the file would keep its readers out.
//!
//! - **Who waits.** Nobody: a held lock is [`Taken::Busy`], at once. A writer that waited would find the file changed
//!   since its plan anyway.
//! - **Who removes it.** Its holder, while it holds it, when it ends. A handle opened before that proves nothing, so
//!   [`take`] compares the handle and the path after locking, as the cache does; a name marked for deletion refuses
//!   opens, which counts as busy. After a crash the file stays until the next writer of that archive, which takes it
//!   over and removes it. The OS drops the lock with the process, so a dead holder blocks nobody.
//! - **Who needs it.** Writers only. A folder that takes no new file is [`Taken::Impossible`], and the write that follows
//!   gives the error it always gave. A file system without locks is too, and the stamp is then all there is.
//! - **Temporary files.** Each one is locked by its writer ([`create_temp`](super::create_temp)), so whoever holds the
//!   writer's lock can tell what a dead writer left from what a live plain write is using, and [`reap`] removes only
//!   the former. Only exact temporary names are looked at, and on Unix only owner-only files, which is what a live or
//!   dead writer leaves: nothing the user made under a similar name. On Windows the handle closes before the rename, so
//!   a plain write of the same file can lose its temporary file to a rebuild in that moment and fail, leaving the
//!   target as it was.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use same_file::Handle;

use super::platform;
use crate::vfs::WriteLock;

/// Longest target name that is embedded in the temporary file name.
const MAX_EMBEDDED_NAME: usize = 200;
/// Temporary names tried for one target.
pub(super) const TEMP_NAMES: u32 = 1000;
/// A run of free names that ends the look for dead temporary files: they are taken lowest first.
const FREE_NAMES_SEEN: u32 = 8;
/// Rounds of "the file at the path is not the file locked" before giving up: each one follows another holder's end.
const TRIES: u32 = 64;

/// The name temporary files of a target `name` start with.
pub(super) fn temp_base(name: &OsStr) -> OsString {
    let mut base = OsString::from(".");
    if name.len() <= MAX_EMBEDDED_NAME {
        base.push(name);
        base.push(".");
    }
    base.push("hikasa-tmp");
    base
}

/// The temporary name number `attempt`.
pub(super) fn temp_name(base: &OsStr, attempt: u32) -> OsString {
    let mut name = base.to_os_string();
    if attempt > 0 {
        name.push(format!("-{attempt}"));
    }
    name
}

/// The lock file of the target `name`. A very long name has no room for a suffix, and shares the folder's.
fn lock_name(name: &OsStr) -> OsString {
    let mut lock = OsString::from(".");
    if name.len() <= MAX_EMBEDDED_NAME {
        lock.push(name);
        lock.push(".");
    }
    lock.push("hikasa-lock");
    lock
}

/// A held lock; dropping it removes the file and ends the lock, in that order.
#[derive(Debug)]
pub(super) struct TargetLock {
    path: PathBuf,
    /// Dropped after the file is gone.
    _handle: Handle,
}

impl WriteLock for TargetLock {}

impl Drop for TargetLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// What [`take`] found.
#[derive(Debug)]
pub(super) enum Taken {
    /// The lock is held.
    Held(TargetLock),
    /// Another writer holds it.
    Busy,
    /// There is no lock to take here.
    Impossible,
}

/// Opens or creates the lock file; one of another user's can still be locked through a read handle.
fn open(path: &Path) -> io::Result<File> {
    let created = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path);
    match created {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            OpenOptions::new().read(true).open(path)
        }
        other => other,
    }
}

/// Takes the writer's lock of `target`, and removes what dead writers left beside it.
pub(super) fn take(target: &Path) -> Taken {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Taken::Impossible;
    };
    let path = dir.join(lock_name(name));
    for _ in 0..TRIES {
        let file = match open(&path) {
            Ok(file) => file,
            // There, and not to be opened: marked for deletion by a holder that just ended, or not ours to use.
            Err(_) if fs::symlink_metadata(&path).is_ok() => return Taken::Busy,
            Err(_) => return Taken::Impossible,
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Taken::Busy,
            Err(TryLockError::Error(_)) => {
                let _ = fs::remove_file(&path);
                return Taken::Impossible;
            }
        }
        let Ok(handle) = Handle::from_file(file) else {
            return Taken::Impossible;
        };
        if Handle::from_path(&path).is_ok_and(|at| at == handle) {
            reap(dir, name);
            return Taken::Held(TargetLock {
                path,
                _handle: handle,
            });
        }
    }
    Taken::Busy
}

/// Removes the temporary files of the target `name` that no writer holds.
fn reap(dir: &Path, name: &OsStr) {
    let base = temp_base(name);
    let mut free = 0;
    for attempt in 0..TEMP_NAMES {
        let path = dir.join(temp_name(&base, attempt));
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && platform::looks_private(&meta) => {
                free = 0;
                if let Ok(file) = OpenOptions::new().write(true).open(&path)
                    && file.try_lock().is_ok()
                {
                    let _ = fs::remove_file(&path);
                }
            }
            Ok(_) => free = 0,
            Err(_) => {
                free += 1;
                if free >= FREE_NAMES_SEEN {
                    return;
                }
            }
        }
    }
}

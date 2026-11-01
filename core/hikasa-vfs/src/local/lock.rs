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
//! - **Temporary files.** Each one is locked by its writer ([`create_temp`](super::create_temp)) before anything else
//!   happens to it, so whoever holds the writer's lock can tell what a dead writer left from what a live plain write is
//!   using, and [`reap`] removes only the former. A writer on Unix holds its temporary file, and so the lock, until the
//!   rename is done. Windows closes the handle first, so a plain write of the same file can lose its temporary file to a
//!   rebuild in that moment, and fails with the target as it was: the one case in which a live writer's file goes. Only
//!   exact temporary names are looked at, and on Unix only owner-only files, which is what a live or dead writer leaves:
//!   nothing the user made under a similar name. A file system that cannot show mode 0600 shows no temporary file as a
//!   writer's, so what a dead writer left on one stays.
//! - **Sharing violations.** On Windows a virus scanner or an indexer can hold any new file for a moment. Removing and
//!   renaming go through [`platform::remove_file`] and [`platform::rename`], which wait a bounded time for that.

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

/// A held lock; dropping it removes the file, while it is still the one locked, and ends the lock, in that order.
#[derive(Debug)]
pub(super) struct TargetLock {
    path: PathBuf,
    /// Dropped after the file is gone.
    handle: Handle,
}

impl WriteLock for TargetLock {}

impl Drop for TargetLock {
    fn drop(&mut self) {
        // Someone may have replaced the file under its name; what stands there now is not this lock to remove.
        if Handle::from_path(&self.path).is_ok_and(|at| at == self.handle) {
            let _ = platform::remove_file(&self.path);
        }
    }
}

/// What [`take`] found.
#[derive(Debug)]
pub(super) enum Taken {
    /// The lock is held.
    Held(TargetLock),
    /// Another writer holds it, or the file in its place cannot be used: the path is in the message.
    Busy(PathBuf),
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
        // Not a file of ours: nothing is created or locked through a link someone put there.
        if fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_file()) {
            return Taken::Busy(path);
        }
        let file = match open(&path) {
            Ok(file) => file,
            // There, and not to be opened: marked for deletion by a holder that just ended, or not ours to use.
            Err(_) if fs::symlink_metadata(&path).is_ok() => return Taken::Busy(path),
            Err(_) => return Taken::Impossible,
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Taken::Busy(path),
            // No locks on this file system. The file stays: it is not ours to remove, and an empty name does no harm.
            Err(TryLockError::Error(_)) => return Taken::Impossible,
        }
        let Ok(handle) = Handle::from_file(file) else {
            return Taken::Impossible;
        };
        if Handle::from_path(&path).is_ok_and(|at| at == handle) {
            reap(dir, name);
            return Taken::Held(TargetLock { path, handle });
        }
    }
    Taken::Busy(path)
}

/// True when `path` still names `file`, as far as that can be told: false after someone removed it.
pub(super) fn still_named(file: &File, path: &Path) -> bool {
    let Ok(held) = file.try_clone().and_then(Handle::from_file) else {
        return true;
    };
    match Handle::from_path(path) {
        Ok(at) => at == held,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

/// True for the name of a temporary file: `base`, or `base-<n>` as [`temp_name`] makes it.
fn is_temp_name(found: &OsStr, base: &OsStr) -> bool {
    let Some(rest) = found
        .as_encoded_bytes()
        .strip_prefix(base.as_encoded_bytes())
    else {
        return false;
    };
    let Some(number) = rest.strip_prefix(b"-") else {
        return rest.is_empty();
    };
    std::str::from_utf8(number)
        .ok()
        .and_then(|number| number.parse::<u32>().ok())
        .is_some_and(|attempt| {
            (1..TEMP_NAMES).contains(&attempt) && temp_name(base, attempt) == found
        })
}

/// Removes the temporary files of the target `name` that no writer holds. The folder is listed, so a file past a run
/// of free numbers is found too.
fn reap(dir: &Path, name: &OsStr) {
    let base = temp_base(name);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !is_temp_name(&entry.file_name(), &base) {
            continue;
        }
        let path = entry.path();
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_file()
            && platform::looks_private(&meta)
            && let Ok(file) = OpenOptions::new().write(true).open(&path)
            && file.try_lock().is_ok()
        {
            let _ = platform::remove_file(&path);
        }
    }
}

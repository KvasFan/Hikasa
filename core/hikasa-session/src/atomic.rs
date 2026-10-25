//! [`write_atomic`]: atomic configuration writes (SES-053).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Attempts at finding a free temporary name before giving up.
const MAX_TEMP_ATTEMPTS: u32 = 10_000;

/// Longest part of the target name kept in the temporary name (bytes): `.` + 200 + `.hikasa-tmp-` + 4 digits
/// stays below the common 255-byte name limit.
pub const TEMP_NAME_PREFIX_MAX: usize = 200;

/// Writes `bytes` to `path` atomically (temp in the same folder, fsync, rename) (SES-053).
///
/// The temporary file is `.<name>.hikasa-tmp-<n>` next to the target, created exclusively (`create_new`), so
/// concurrent writers never share one; `<name>` is cut to at most [`TEMP_NAME_PREFIX_MAX`] bytes (at a char
/// boundary), so every target name the file system accepts (up to 255 bytes) also has a valid temporary name. Readers see either the old content or the new content, never a part of
/// it; after a failure the target is untouched and the temporary file is removed. The parent folder must exist.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{}: not a file path", path.display()),
        )
    })?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let lossy = name.to_string_lossy();
    let mut cut = lossy.len().min(TEMP_NAME_PREFIX_MAX);
    while !lossy.is_char_boundary(cut) {
        cut -= 1;
    }
    let (temp, mut file) = create_temp(&parent, &lossy[..cut])?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|()| fs::rename(&temp, path)) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    sync_dir(&parent);
    Ok(())
}

/// Creates a fresh temporary file in `dir` (exclusive creation; the first free counter wins).
fn create_temp(dir: &Path, name: &str) -> io::Result<(PathBuf, fs::File)> {
    for n in 0..MAX_TEMP_ATTEMPTS {
        let temp = dir.join(format!(".{name}.hikasa-tmp-{n}"));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{}: no free temporary file name", dir.display()),
    ))
}

/// Makes the rename durable (Unix: fsync of the folder; best effort, a failure is not an error).
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Windows has no folder fsync.
#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

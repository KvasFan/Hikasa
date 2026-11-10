//! Loading tables (contract T-4): [`load_bytes`] (pure) and [`load_workbook`] (through the VFS).

use std::io::Read;

use hikasa_core::{CancelToken, Ctl, Phase, Progress, Spec, Unit};
use hikasa_vfs::{EntryKind, FollowLinks, Vfs, VfsError, VfsOp};

use crate::delimited::load_text;
use crate::error::TableError;
use crate::format::{TableFormat, TableKind};
use crate::sheet::Workbook;
use crate::workbook::{SpreadsheetOptions, detect_workbook, load_spreadsheet};

/// Options of a load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadOptions {
    /// Largest input in bytes (default and cap: `u32::MAX`, the arena's offsets).
    pub max_bytes: u64,
    /// Workbook options.
    pub spreadsheet: SpreadsheetOptions,
}

impl Default for LoadOptions {
    fn default() -> LoadOptions {
        LoadOptions {
            max_bytes: u64::from(u32::MAX),
            spreadsheet: SpreadsheetOptions::default(),
        }
    }
}

impl LoadOptions {
    /// `max_bytes` clamped to the cap.
    pub fn effective_max_bytes(&self) -> u64 {
        self.max_bytes.min(u64::from(u32::MAX))
    }
}

/// Loads a table from its bytes. `name` is the file name (format by extension when the kind is Detect, sheet
/// name of text tables, messages). Workbooks are recognized by content whatever the kind says, so an `.xlsx`
/// renamed `.csv` still loads as a workbook (T-9: an encrypted package is refused).
pub fn load_bytes(
    bytes: Vec<u8>,
    name: &str,
    format: &TableFormat,
    opts: &LoadOptions,
    cancel: &CancelToken,
) -> Result<Workbook, TableError> {
    if bytes.len() as u64 > opts.effective_max_bytes() {
        return Err(TableError::TooLarge {
            name: name.to_owned(),
            what: "bytes",
            limit: opts.effective_max_bytes(),
        });
    }
    cancel.check()?;
    let workbook = detect_workbook(&bytes, name)?;
    if workbook.is_some() || format.kind == TableKind::Spreadsheet {
        return load_spreadsheet(&bytes, name, format, &opts.spreadsheet, cancel);
    }
    load_text(bytes, name, format, cancel)
}

/// Stats and reads `spec` through `vfs` (cancellable; progress `reading` per MiB), then [`load_bytes`]. Returns
/// the bytes' CRC-free identity material: the raw bytes are dropped; `bytes_len` is returned for the caller's
/// size check.
pub fn read_spec(
    vfs: &dyn Vfs,
    spec: &Spec,
    max_bytes: u64,
    ctl: &Ctl<'_>,
) -> Result<Vec<u8>, TableError> {
    ctl.check()?;
    let canonical = spec.canonical();
    let load_error = |source: VfsError| -> TableError {
        if source.is_cancelled() {
            TableError::Cancelled
        } else {
            TableError::Load {
                side: hikasa_core::Side::Left,
                source,
            }
        }
    };
    let entry = vfs.stat(spec, FollowLinks::Yes).map_err(load_error)?;
    if entry.kind == EntryKind::Dir {
        return Err(load_error(VfsError::IsADirectory {
            spec: canonical.clone(),
        }));
    }
    if entry.size > max_bytes && entry.size > 4096 {
        return Err(TableError::TooLarge {
            name: canonical,
            what: "bytes",
            limit: max_bytes,
        });
    }
    let mut reader = vfs.open_read_with(spec, ctl.cancel).map_err(load_error)?;
    let mut out = Vec::with_capacity(usize::try_from(entry.size.min(max_bytes)).unwrap_or(0));
    let mut buf = vec![0u8; 1 << 20];
    loop {
        ctl.check()?;
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(load_error(VfsError::from_io(canonical, VfsOp::Read, e)));
            }
        };
        if out.len() as u64 + n as u64 > max_bytes {
            return Err(TableError::TooLarge {
                name: canonical,
                what: "bytes",
                limit: max_bytes,
            });
        }
        out.extend_from_slice(&buf[..n]);
        ctl.progress(Phase::Reading, || Progress {
            phase: Phase::Reading,
            done: out.len() as u64,
            total: Some(entry.size),
            unit: Unit::Bytes,
            current: Some(canonical.clone()),
        });
    }
    Ok(out)
}

/// Loads `spec` through `vfs` (T-4). The file name of the spec chooses presets when `format` is Detect.
pub fn load_workbook(
    vfs: &dyn Vfs,
    spec: &Spec,
    format: &TableFormat,
    opts: &LoadOptions,
    ctl: &Ctl<'_>,
) -> Result<Workbook, TableError> {
    let bytes = read_spec(vfs, spec, opts.effective_max_bytes(), ctl)?;
    let name = spec_name(spec);
    load_bytes(bytes, &name, format, opts, ctl.cancel)
}

/// The file name of a spec (the last component of its canonical form).
pub fn spec_name(spec: &Spec) -> String {
    let canonical = spec.canonical();
    canonical
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&canonical)
        .to_owned()
}

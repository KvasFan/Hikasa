//! The clipboard as a source of `clipboard://` (D-100; contract M2 §1.3.6): the pure [`ClipboardProvider`] seam,
//! the test providers [`NoClipboard`] and [`FixedClipboard`], the conversion of clipboard content into a VFS
//! [`Blob`] ([`clipboard_blob`]) and the in-house PNG encoder of clipboard images ([`encode_png_rgba`]).
//!
//! The operating-system clipboard is read only by `host::SystemClipboard` (adapter-only, like the system clock);
//! the VFS never calls the clipboard itself. Text wins when the clipboard holds both text and an image.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hikasa_core::{Ctl, FileTime};
use hikasa_vfs::{Blob, StreamOptions, VfsError, VfsOp, crc32_reader};

/// The spec text of the clipboard in errors.
pub const CLIPBOARD_SPEC: &str = "clipboard://";

/// The name hint of a clipboard image (format detection, D-15).
pub const CLIPBOARD_IMAGE_NAME: &str = "clipboard.png";

/// Largest image the encoder accepts: 1 GiB of RGBA pixels.
pub const MAX_IMAGE_BYTES: u64 = 1 << 30;

/// Largest clipboard text (and `$HIKASA_CLIPBOARD_FILE`) accepted: 1 GiB of UTF-8, the image limit (§1.3.12).
pub const MAX_TEXT_BYTES: u64 = 1 << 30;

/// What the clipboard holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    /// Text.
    Text(String),
    /// An image: 8-bit RGBA, rows top to bottom, `width * height * 4` bytes.
    Image {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// The pixels.
        rgba: Vec<u8>,
    },
}

/// Why the clipboard could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    /// No clipboard: headless, no display, not supported.
    #[error("no clipboard is available ({0})")]
    Unavailable(String),
    /// The clipboard holds neither text nor an image.
    #[error("the clipboard holds no text or image")]
    Empty,
    /// Reading failed.
    #[error("cannot read the clipboard: {0}")]
    Failed(String),
    /// The content is larger than the limit (bytes): [`MAX_TEXT_BYTES`] or [`MAX_IMAGE_BYTES`].
    #[error("the clipboard content is larger than {0} bytes")]
    TooLarge(u64),
}

impl ClipboardError {
    /// The VFS error of `clipboard://` (§1.3.6): `Unavailable` → `VfsError::Unavailable` (kind `unsupported`),
    /// `Empty` → `NotFound`, `Failed` → `Io` (op `read`), `TooLarge` → `TooLarge`.
    pub fn to_vfs(&self) -> VfsError {
        let spec = CLIPBOARD_SPEC.to_owned();
        match self {
            ClipboardError::Unavailable(_) => VfsError::Unavailable {
                spec,
                reason: self.to_string(),
            },
            ClipboardError::Empty => VfsError::NotFound { spec },
            ClipboardError::Failed(_) => {
                VfsError::from_io(spec, VfsOp::Read, std::io::Error::other(self.to_string()))
            }
            ClipboardError::TooLarge(limit) => VfsError::TooLarge {
                spec,
                limit: *limit,
            },
        }
    }

    /// The VFS error of a report that could not be placed on the clipboard (§1.7.10): `Unavailable` →
    /// `VfsError::Unavailable` (kind `unsupported`), `TooLarge` → `TooLarge`, anything else → `Io` (op `write`)
    /// "cannot write the clipboard: …".
    pub fn to_vfs_write(&self) -> VfsError {
        let spec = CLIPBOARD_SPEC.to_owned();
        match self {
            ClipboardError::Unavailable(_) | ClipboardError::TooLarge(_) => self.to_vfs(),
            ClipboardError::Failed(why) => VfsError::from_io(
                spec,
                VfsOp::Write,
                std::io::Error::other(format!("cannot write the clipboard: {why}")),
            ),
            ClipboardError::Empty => VfsError::from_io(
                spec,
                VfsOp::Write,
                std::io::Error::other(format!("cannot write the clipboard: {self}")),
            ),
        }
    }
}

/// The most a report placed on the clipboard may hold, its text and HTML flavours together (§1.7.9): 128 MiB.
pub const CLIPBOARD_REPORT_MAX: u64 = 128 << 20;

/// A report for the clipboard (§1.7.10): `text` always (a plain-text rendering of an HTML report), `html` for an HTML
/// report. Bytes as a file target's (`\n` line ends on every platform).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload {
    /// The plain text.
    pub text: String,
    /// The HTML document, for an HTML report.
    pub html: Option<String>,
}

/// A source of clipboard content. The service calls it only when a `clipboard://` spec is parsed (once per
/// service) and on `Service::read_clipboard`.
pub trait ClipboardProvider: Send + Sync {
    /// Reads the clipboard now.
    fn read(&self) -> Result<ClipboardContent, ClipboardError>;

    /// Places a report (M2 W7, §1.7.10). Default: `Unavailable`. Called by the adapters only (architecture check 2):
    /// the Service returns the payload in `ReportOutcome::clipboard`.
    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> {
        let _ = payload;
        Err(ClipboardError::Unavailable(
            "this clipboard cannot be written".into(),
        ))
    }

    /// Places a list of files (M2 W9c, D-252): file managers paste them as files. Default: `Unavailable`. Called by
    /// the adapters only (the GUI writes its own provider, D-203).
    fn write_files(&self, paths: &[PathBuf]) -> Result<(), ClipboardError> {
        let _ = paths;
        Err(ClipboardError::Unavailable("file lists".into()))
    }

    /// Reads the list of files the clipboard holds (M2 W9c, D-252): a file manager's copy. `Empty` when it holds
    /// none. Default: `Unavailable`.
    fn read_files(&self) -> Result<Vec<PathBuf>, ClipboardError> {
        Err(ClipboardError::Unavailable("file lists".into()))
    }

    /// Places `text` as the primary selection (M2 W11a, D-298: X11 and Wayland, what a middle click pastes). Default:
    /// `Unavailable` (Windows, macOS, a clipboard without one). Called by the adapters only.
    fn write_primary(&self, text: &str) -> Result<(), ClipboardError> {
        let _ = text;
        Err(ClipboardError::Unavailable("no primary selection".into()))
    }

    /// Reads the primary selection's text (M2 W11a, D-298). Default: `Unavailable`.
    fn read_primary(&self) -> Result<String, ClipboardError> {
        Err(ClipboardError::Unavailable("no primary selection".into()))
    }
}

/// No clipboard: every read is `Unavailable("no clipboard provider")` (the default of `ServiceInit::new`).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoClipboard;

impl ClipboardProvider for NoClipboard {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        Err(ClipboardError::Unavailable("no clipboard provider".into()))
    }
}

/// What a [`FixedClipboard`] answers.
#[derive(Debug, Clone)]
enum Fixed {
    Content(ClipboardContent),
    /// A list of files (M2 W9c): `read` sees an empty clipboard, `read_files` the list.
    Files(Vec<PathBuf>),
    Empty,
    Unavailable(String),
    Failed(String),
}

/// A clipboard with fixed content or a fixed error, counting its reads and recording every payload and file list
/// placed on it (tests). A clipboard that is unavailable or failing refuses payloads and lists the same way. A list
/// placed becomes the content (`read_files` gives it back, `read` sees no text), as on a real clipboard.
#[derive(Debug)]
pub struct FixedClipboard {
    fixed: parking_lot::Mutex<Fixed>,
    reads: AtomicUsize,
    payloads: parking_lot::Mutex<Vec<ClipboardPayload>>,
    files: parking_lot::Mutex<Vec<Vec<PathBuf>>>,
    /// The primary selection (M2 W11a): every text placed, in order; the last one is what `read_primary` gives.
    primaries: parking_lot::Mutex<Vec<String>>,
}

impl FixedClipboard {
    fn new(fixed: Fixed) -> FixedClipboard {
        FixedClipboard {
            fixed: parking_lot::Mutex::new(fixed),
            reads: AtomicUsize::new(0),
            payloads: parking_lot::Mutex::new(Vec::new()),
            files: parking_lot::Mutex::new(Vec::new()),
            primaries: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Every primary selection placed, in order (M2 W11a; [`FixedClipboard::set_primary`] counts as one).
    pub fn primaries(&self) -> Vec<String> {
        self.primaries.lock().clone()
    }

    /// Places `text` as the primary selection (what another program's selection would be).
    pub fn set_primary(&self, text: impl Into<String>) {
        self.primaries.lock().push(text.into());
    }

    /// Holds a list of files (a file manager's copy; M2 W9c).
    pub fn files(paths: Vec<PathBuf>) -> FixedClipboard {
        FixedClipboard::new(Fixed::Files(paths))
    }

    /// Replaces the content by a list of files.
    pub fn set_files(&self, paths: Vec<PathBuf>) {
        *self.fixed.lock() = Fixed::Files(paths);
    }

    /// Every file list placed, in order.
    pub fn file_lists(&self) -> Vec<Vec<PathBuf>> {
        self.files.lock().clone()
    }

    /// Every payload placed, in order.
    pub fn payloads(&self) -> Vec<ClipboardPayload> {
        self.payloads.lock().clone()
    }

    /// Holds `text`.
    pub fn text(text: impl Into<String>) -> FixedClipboard {
        FixedClipboard::new(Fixed::Content(ClipboardContent::Text(text.into())))
    }

    /// Holds an RGBA image.
    pub fn image(width: u32, height: u32, rgba: Vec<u8>) -> FixedClipboard {
        FixedClipboard::new(Fixed::Content(ClipboardContent::Image {
            width,
            height,
            rgba,
        }))
    }

    /// Holds nothing (`Empty`).
    pub fn empty() -> FixedClipboard {
        FixedClipboard::new(Fixed::Empty)
    }

    /// No clipboard (`Unavailable(reason)`).
    pub fn unavailable(reason: impl Into<String>) -> FixedClipboard {
        FixedClipboard::new(Fixed::Unavailable(reason.into()))
    }

    /// Reading fails (`Failed(reason)`).
    pub fn failing(reason: impl Into<String>) -> FixedClipboard {
        FixedClipboard::new(Fixed::Failed(reason.into()))
    }

    /// Replaces the content (a later `read_clipboard` sees it).
    pub fn set_text(&self, text: impl Into<String>) {
        *self.fixed.lock() = Fixed::Content(ClipboardContent::Text(text.into()));
    }

    /// How often the clipboard was read.
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }
}

impl ClipboardProvider for FixedClipboard {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        match &*self.fixed.lock() {
            Fixed::Content(content) => Ok(content.clone()),
            Fixed::Empty | Fixed::Files(_) => Err(ClipboardError::Empty),
            Fixed::Unavailable(reason) => Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => Err(ClipboardError::Failed(reason.clone())),
        }
    }

    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> {
        match &*self.fixed.lock() {
            Fixed::Unavailable(reason) => return Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => return Err(ClipboardError::Failed(reason.clone())),
            Fixed::Content(_) | Fixed::Files(_) | Fixed::Empty => {}
        }
        self.payloads.lock().push(payload.clone());
        Ok(())
    }

    fn write_files(&self, paths: &[PathBuf]) -> Result<(), ClipboardError> {
        let mut fixed = self.fixed.lock();
        match &*fixed {
            Fixed::Unavailable(reason) => return Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => return Err(ClipboardError::Failed(reason.clone())),
            Fixed::Content(_) | Fixed::Files(_) | Fixed::Empty => {}
        }
        self.files.lock().push(paths.to_vec());
        *fixed = Fixed::Files(paths.to_vec());
        Ok(())
    }

    fn read_files(&self) -> Result<Vec<PathBuf>, ClipboardError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        match &*self.fixed.lock() {
            Fixed::Files(paths) if !paths.is_empty() => Ok(paths.clone()),
            Fixed::Files(_) | Fixed::Content(_) | Fixed::Empty => Err(ClipboardError::Empty),
            Fixed::Unavailable(reason) => Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => Err(ClipboardError::Failed(reason.clone())),
        }
    }

    /// Records the text (an unavailable or failing clipboard refuses it, as its other writes).
    fn write_primary(&self, text: &str) -> Result<(), ClipboardError> {
        match &*self.fixed.lock() {
            Fixed::Unavailable(reason) => return Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => return Err(ClipboardError::Failed(reason.clone())),
            Fixed::Content(_) | Fixed::Files(_) | Fixed::Empty => {}
        }
        self.primaries.lock().push(text.to_owned());
        Ok(())
    }

    /// The last primary selection placed; `Empty` when none was.
    fn read_primary(&self) -> Result<String, ClipboardError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        match &*self.fixed.lock() {
            Fixed::Unavailable(reason) => return Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => return Err(ClipboardError::Failed(reason.clone())),
            Fixed::Content(_) | Fixed::Files(_) | Fixed::Empty => {}
        }
        self.primaries
            .lock()
            .last()
            .cloned()
            .ok_or(ClipboardError::Empty)
    }
}

impl<P: ClipboardProvider + ?Sized> ClipboardProvider for Arc<P> {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        (**self).read()
    }

    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> {
        (**self).write_payload(payload)
    }

    fn write_files(&self, paths: &[PathBuf]) -> Result<(), ClipboardError> {
        (**self).write_files(paths)
    }

    fn read_files(&self) -> Result<Vec<PathBuf>, ClipboardError> {
        (**self).read_files()
    }

    fn write_primary(&self, text: &str) -> Result<(), ClipboardError> {
        (**self).write_primary(text)
    }

    fn read_primary(&self) -> Result<String, ClipboardError> {
        (**self).read_primary()
    }
}

/// The clipboard writes of a window in the order they were asked, the latest winning (M2 W9c after review F12): each
/// command takes a [`Ticket`] when it is invoked, its job writes in turn; a write whose ticket is older than the last
/// one taken is skipped (a later Copy or Copy Filename already replaces it), and writes never interleave. Plain calls of
/// the [`ClipboardProvider`] methods (a script's `output-to:clipboard`) take their ticket when they write.
pub struct OrderedClipboard {
    inner: Arc<dyn ClipboardProvider>,
    issued: std::sync::atomic::AtomicU64,
    turn: parking_lot::Mutex<()>,
}

/// The place of a write in an [`OrderedClipboard`]'s order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket(u64);

impl OrderedClipboard {
    /// The writes to `inner`, in order.
    pub fn new(inner: Arc<dyn ClipboardProvider>) -> OrderedClipboard {
        OrderedClipboard {
            inner,
            issued: std::sync::atomic::AtomicU64::new(0),
            turn: parking_lot::Mutex::new(()),
        }
    }

    /// A ticket for a write about to be asked (taken when the command runs, on the window's thread).
    pub fn ticket(&self) -> Ticket {
        Ticket(self.issued.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// Runs `write` in turn: `Ok(false)` without writing when a later ticket was taken meanwhile.
    pub fn in_turn(
        &self,
        ticket: Ticket,
        write: impl FnOnce(&dyn ClipboardProvider) -> Result<(), ClipboardError>,
    ) -> Result<bool, ClipboardError> {
        let _turn = self.turn.lock();
        if self.issued.load(Ordering::SeqCst) != ticket.0 {
            return Ok(false);
        }
        write(&*self.inner).map(|()| true)
    }
}

impl std::fmt::Debug for OrderedClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrderedClipboard")
            .field("issued", &self.issued)
            .finish_non_exhaustive()
    }
}

impl ClipboardProvider for OrderedClipboard {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        self.inner.read()
    }

    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> {
        let ticket = self.ticket();
        self.in_turn(ticket, |c| c.write_payload(payload)).map(drop)
    }

    fn write_files(&self, paths: &[PathBuf]) -> Result<(), ClipboardError> {
        let ticket = self.ticket();
        self.in_turn(ticket, |c| c.write_files(paths)).map(drop)
    }

    fn read_files(&self) -> Result<Vec<PathBuf>, ClipboardError> {
        self.inner.read_files()
    }

    /// The primary selection is not the clipboard: its writes are not ordered with the clipboard's (the latest
    /// selection is placed at once).
    fn write_primary(&self, text: &str) -> Result<(), ClipboardError> {
        self.inner.write_primary(text)
    }

    fn read_primary(&self) -> Result<String, ClipboardError> {
        self.inner.read_primary()
    }
}

/// The paths a clipboard text names (D-252's Paste of text; M2 W9c after review F7): each line, trimmed, that is an
/// absolute local path or a `file://` URI with no host or `localhost` (percent-decoded; on Unix its bytes, so a name
/// that is not UTF-8 survives; on Windows `file:///C:/x` is `C:\x`). Every other line (prose, a relative path, a URL
/// of another host or scheme, `-`, `clipboard://`) is left out.
pub fn pasted_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(pasted_path)
        .collect()
}

/// One line of [`pasted_paths`].
fn pasted_path(line: &str) -> Option<PathBuf> {
    let scheme = line.get(..7).filter(|s| s.eq_ignore_ascii_case("file://"));
    let path = match scheme {
        Some(_) => {
            let rest = &line[7..];
            let slash = rest.find('/')?;
            let host = &rest[..slash];
            if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
                return None;
            }
            path_of_uri(&percent_decoded(&rest[slash..])?)?
        }
        None => PathBuf::from(line),
    };
    path.is_absolute().then_some(path)
}

/// The bytes of a URI path with its `%XX` escapes decoded (`None`: a `%` not followed by two hex digits).
pub fn percent_decoded(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The local path of a decoded URI path (`/home/a`; `/C:/x` on Windows is `C:\x`).
fn path_of_uri(bytes: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        let text = std::str::from_utf8(bytes).ok()?;
        let drive = text
            .as_bytes()
            .get(1..3)
            .is_some_and(|d| d[0].is_ascii_alphabetic() && d[1] == b':');
        // Windows' own separators (W9 close): `/` is never part of a Windows name, and the path then reads as the
        // same path typed or dropped does.
        let text = if drive { &text[1..] } else { text };
        Some(PathBuf::from(text.replace('/', "\\")))
    }
}

/// The blob of clipboard content: text → its UTF-8 bytes without a name hint (format detection by content);
/// an image → a PNG ([`encode_png_rgba`]) named [`CLIPBOARD_IMAGE_NAME`]. `mtime` is the injected clock at capture
/// (T1). Empty text is an empty clipboard (`NotFound`, as [`ClipboardError::Empty`]); text above
/// [`MAX_TEXT_BYTES`] is `TooLarge`.
pub fn clipboard_blob(content: ClipboardContent, mtime: FileTime) -> Result<Blob, VfsError> {
    let (bytes, name) = match content {
        ClipboardContent::Text(text) if text.is_empty() => {
            return Err(ClipboardError::Empty.to_vfs());
        }
        ClipboardContent::Text(text) if text.len() as u64 > MAX_TEXT_BYTES => {
            return Err(ClipboardError::TooLarge(MAX_TEXT_BYTES).to_vfs());
        }
        ClipboardContent::Text(text) => (text.into_bytes(), None),
        ClipboardContent::Image {
            width,
            height,
            rgba,
        } => (
            encode_png_rgba(width, height, &rgba)?,
            Some(CLIPBOARD_IMAGE_NAME.to_owned()),
        ),
    };
    Ok(Blob {
        bytes: Arc::from(bytes),
        mtime,
        name,
    })
}

/// The CRC-32 of `bytes` (the VFS's CRC; no other CRC crate in this crate).
fn crc32(bytes: &[u8]) -> u32 {
    crc32_reader(
        &mut Cursor::new(bytes),
        &StreamOptions::default(),
        &Ctl::detached(),
    )
    .unwrap_or(0)
}

/// The largest stored deflate block.
const STORED_MAX: usize = 65_535;

/// Adler-32 (RFC 1950), fed in pieces.
struct Adler32 {
    a: u32,
    b: u32,
}

impl Adler32 {
    const MOD: u32 = 65_521;

    fn new() -> Adler32 {
        Adler32 { a: 1, b: 0 }
    }

    fn update(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(5552) {
            for &byte in chunk {
                self.a += u32::from(byte);
                self.b += self.a;
            }
            self.a %= Self::MOD;
            self.b %= Self::MOD;
        }
    }

    fn finish(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

/// Adler-32 of `bytes` at once.
#[cfg(test)]
fn adler32(bytes: &[u8]) -> u32 {
    let mut adler = Adler32::new();
    adler.update(bytes);
    adler.finish()
}

/// Writes a zlib stream of stored deflate blocks into `out` as the bytes arrive: each block's 5-byte header is
/// reserved when it starts and filled in when it is full (or the stream ends).
struct Stored<'a> {
    out: &'a mut Vec<u8>,
    adler: Adler32,
    /// Bytes of the stream not yet pushed.
    remaining: usize,
    /// Bytes in the open block.
    in_block: usize,
    /// Offset of the open block's header in `out`.
    header: usize,
}

impl Stored<'_> {
    fn push(&mut self, mut bytes: &[u8]) {
        self.adler.update(bytes);
        while !bytes.is_empty() {
            if self.in_block == 0 {
                self.header = self.out.len();
                self.out.extend_from_slice(&[0; 5]);
            }
            let take = bytes.len().min(STORED_MAX - self.in_block);
            let (now, rest) = bytes.split_at(take);
            self.out.extend_from_slice(now);
            self.in_block += take;
            self.remaining -= take;
            bytes = rest;
            if self.in_block == STORED_MAX || self.remaining == 0 {
                let len = self.in_block as u16;
                let mut head = [u8::from(self.remaining == 0), 0, 0, 0, 0];
                head[1..3].copy_from_slice(&len.to_le_bytes());
                head[3..5].copy_from_slice(&(!len).to_le_bytes());
                if let Some(slot) = self.out.get_mut(self.header..self.header + 5) {
                    slot.copy_from_slice(&head);
                }
                self.in_block = 0;
            }
        }
    }
}

/// Appends one PNG chunk.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(out.get(start..).unwrap_or_default());
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes 8-bit RGBA pixels as a PNG: colour type 6, one IDAT holding a zlib stream of **stored** deflate blocks
/// (no compression), every row with filter 0; CRC-32 and Adler-32 checksums. Deterministic. More than
/// [`MAX_IMAGE_BYTES`] of pixels → `TooLarge`; a pixel buffer whose length is not `width * height * 4` →
/// `invalid_data`. The bytes are "a PNG that decodes to the pixels", not a stable format (W12 may compress).
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, VfsError> {
    let spec = || CLIPBOARD_SPEC.to_owned();
    let pixels = u64::from(width) * u64::from(height) * 4;
    if pixels > MAX_IMAGE_BYTES {
        return Err(VfsError::TooLarge {
            spec: spec(),
            limit: MAX_IMAGE_BYTES,
        });
    }
    if rgba.len() as u64 != pixels || width == 0 || height == 0 {
        return Err(VfsError::from_io(
            spec(),
            VfsOp::Read,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "an image of {width}x{height} pixels needs {pixels} bytes, got {}",
                    rgba.len()
                ),
            ),
        ));
    }
    // One output buffer: the zlib stream of stored blocks is written straight into the IDAT chunk from the
    // rows (a filter byte, then the row), so the pixels are held once besides the PNG itself.
    let row = width as usize * 4;
    let raw_len = height as usize * (row + 1);
    let blocks = raw_len.div_ceil(STORED_MAX).max(1);
    let idat_len = 2 + blocks * 5 + raw_len + 4;
    let mut png = Vec::with_capacity(8 + 25 + 12 + idat_len + 12);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    png.extend_from_slice(&(idat_len as u32).to_be_bytes());
    let idat_start = png.len();
    png.extend_from_slice(b"IDAT");
    // zlib: CMF 0x78 (deflate, 32 KiB window), FLG 0x01 (no dictionary, level 0; 0x7801 % 31 == 0).
    png.extend_from_slice(&[0x78, 0x01]);
    let mut stored = Stored {
        out: &mut png,
        adler: Adler32::new(),
        remaining: raw_len,
        in_block: 0,
        header: 0,
    };
    for line in rgba.chunks(row) {
        stored.push(&[0]);
        stored.push(line);
    }
    let adler = stored.adler.finish();
    png.extend_from_slice(&adler.to_be_bytes());
    let crc = crc32(png.get(idat_start..).unwrap_or_default());
    png.extend_from_slice(&crc.to_be_bytes());
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler_of_wikipedia() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn crc_of_iend() {
        // The CRC of the empty IEND chunk is a well-known constant.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }
}

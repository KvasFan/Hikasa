//! The clipboard as a source of `clipboard://` (D-100; contract M2 §1.3.6): the pure [`ClipboardProvider`] seam,
//! the test providers [`NoClipboard`] and [`FixedClipboard`], the conversion of clipboard content into a VFS
//! [`Blob`] ([`clipboard_blob`]) and the in-house PNG encoder of clipboard images ([`encode_png_rgba`]).
//!
//! The operating-system clipboard is read only by `host::SystemClipboard` (adapter-only, like the system clock);
//! the VFS never calls the clipboard itself. Text wins when the clipboard holds both text and an image.

use std::io::Cursor;
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
}

/// A source of clipboard content. The service calls it only when a `clipboard://` spec is parsed (once per
/// service) and on `Service::read_clipboard`.
pub trait ClipboardProvider: Send + Sync {
    /// Reads the clipboard now.
    fn read(&self) -> Result<ClipboardContent, ClipboardError>;
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
    Empty,
    Unavailable(String),
    Failed(String),
}

/// A clipboard with fixed content or a fixed error, counting its reads (tests).
#[derive(Debug)]
pub struct FixedClipboard {
    fixed: parking_lot::Mutex<Fixed>,
    reads: AtomicUsize,
}

impl FixedClipboard {
    fn new(fixed: Fixed) -> FixedClipboard {
        FixedClipboard {
            fixed: parking_lot::Mutex::new(fixed),
            reads: AtomicUsize::new(0),
        }
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
            Fixed::Empty => Err(ClipboardError::Empty),
            Fixed::Unavailable(reason) => Err(ClipboardError::Unavailable(reason.clone())),
            Fixed::Failed(reason) => Err(ClipboardError::Failed(reason.clone())),
        }
    }
}

impl<P: ClipboardProvider + ?Sized> ClipboardProvider for Arc<P> {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        (**self).read()
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

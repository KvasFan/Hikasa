//! Frame counts from the container's structure (no pixel is decoded): GIF image descriptors, WebP `ANMF` chunks,
//! APNG's `acTL`, TIFF's IFD chain. Every walk is bounded by the bytes and stops at the first malformed block.

use crate::model::{FrameCount, ImageFormatName};

/// Most IFDs or frames walked; beyond it the count is reported as this many.
const MAX_WALK: u32 = 65_536;

/// The frame count of `bytes` in `format`.
pub(crate) fn count(format: ImageFormatName, bytes: &[u8]) -> FrameCount {
    let n = match format {
        ImageFormatName::Gif => gif(bytes),
        ImageFormatName::WebP => webp(bytes),
        ImageFormatName::Png => apng(bytes),
        ImageFormatName::Tiff => tiff(bytes),
        _ => Some(1),
    };
    match n {
        Some(0 | 1) => FrameCount::One,
        Some(n) => FrameCount::Many(n),
        None => FrameCount::Unknown,
    }
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u32_be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Skips GIF data sub-blocks from `at`; the position after the terminator.
fn sub_blocks(b: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let size = *b.get(at)? as usize;
        at += 1;
        if size == 0 {
            return Some(at);
        }
        at += size;
    }
}

/// Image descriptors of a GIF up to the trailer (`None` when the header is malformed).
fn gif(b: &[u8]) -> Option<u32> {
    gif_walk(b, |_, _| {})
}

/// The largest frame of a GIF in pixels (0 when no image descriptor can be read): the decoder allocates a frame
/// that does not match the logical screen apart from the picture (W12 review F4), so `decode_image` refuses a
/// frame above `max_pixels` before decoding.
pub(crate) fn gif_largest_frame(b: &[u8]) -> u64 {
    let mut largest = 0u64;
    gif_walk(b, |width, height| {
        largest = largest.max(u64::from(width) * u64::from(height));
    });
    largest
}

/// Walks the image descriptors of a GIF up to the trailer, calling `frame(width, height)` for each one whose size
/// can be read (before the walk stops at a malformed block); the number of whole descriptors (`None` when the
/// header is malformed).
fn gif_walk(b: &[u8], mut frame: impl FnMut(u16, u16)) -> Option<u32> {
    let packed = *b.get(10)?;
    let mut at = 13;
    if packed & 0x80 != 0 {
        at += 3 << ((packed & 7) + 1);
    }
    let mut frames = 0u32;
    while frames < MAX_WALK {
        match b.get(at) {
            Some(0x21) => at = sub_blocks(b, at + 2)?,
            Some(0x2C) => {
                // Left, top, width, height (little endian), then the packed byte.
                if let (Some(w), Some(h)) = (u16_le(b, at + 5), u16_le(b, at + 7)) {
                    frame(w, h);
                }
                let local = *b.get(at + 9)?;
                at += 10;
                if local & 0x80 != 0 {
                    at += 3 << ((local & 7) + 1);
                }
                frames += 1;
                // The LZW minimum code size, then the image data.
                match sub_blocks(b, at + 1) {
                    Some(next) => at = next,
                    None => break,
                }
            }
            _ => break,
        }
    }
    Some(frames)
}

/// `ANMF` chunks of an animated WebP (1 for a still one).
fn webp(b: &[u8]) -> Option<u32> {
    if b.get(..4)? != b"RIFF" || b.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut at = 12usize;
    let mut frames = 0u32;
    while let (Some(fourcc), Some(size)) = (b.get(at..at + 4), u32_le(b, at + 4)) {
        if fourcc == b"ANMF" {
            frames += 1;
            if frames >= MAX_WALK {
                break;
            }
        }
        at = at.checked_add(8 + size as usize + (size as usize & 1))?;
    }
    Some(frames.max(1))
}

/// APNG's `acTL` frame count (1 for a plain PNG).
fn apng(b: &[u8]) -> Option<u32> {
    let mut at = 8usize;
    while let (Some(len), Some(kind)) = (u32_be(b, at), b.get(at + 4..at + 8)) {
        match kind {
            b"acTL" => return Some(u32_be(b, at + 8)?.min(MAX_WALK)),
            b"IDAT" | b"IEND" => return Some(1),
            _ => {}
        }
        at = at.checked_add(12 + len as usize)?;
    }
    Some(1)
}

/// IFDs of a classic TIFF (`None` for BigTIFF or a malformed header).
fn tiff(b: &[u8]) -> Option<u32> {
    let le = match b.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let raw: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(raw)
        } else {
            u16::from_be_bytes(raw)
        })
    };
    let u32_at = |at: usize| {
        let raw: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(raw)
        } else {
            u32::from_be_bytes(raw)
        })
    };
    let mut next = u32_at(4)? as usize;
    let mut pages = 0u32;
    let mut seen = Vec::new();
    while next != 0 && pages < MAX_WALK {
        if seen.contains(&next) {
            break;
        }
        seen.push(next);
        let Some(entries) = u16_at(next) else { break };
        pages += 1;
        match u32_at(next + 2 + 12 * entries as usize) {
            Some(n) => next = n as usize,
            None => break,
        }
    }
    Some(pages)
}

/// True when an ICC profile describes sRGB: its description (`desc`, ASCII or UTF-16 text) names `sRGB`.
pub(crate) fn is_srgb_profile(profile: &[u8]) -> bool {
    let ascii = profile.windows(4).any(|w| w == b"sRGB");
    let utf16 = profile
        .windows(8)
        .any(|w| w == b"\0s\0R\0G\0B" || w == b"s\0R\0G\0B\0");
    ascii || utf16
}

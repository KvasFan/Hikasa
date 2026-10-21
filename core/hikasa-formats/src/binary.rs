//! Binary detection (TC-003, §2.9): [`BinaryVerdict`], [`BinaryReason`], [`MagicKind`] and [`sniff`], with the
//! in-house magic-number table that replaces the `infer` crate (§0.7).
//!
//! ```text
//! sniff(bytes):
//!   empty                                  → text   (Empty)
//!   a byte-order mark                      → text   (Bom)
//!   a known signature at offset 0          → BINARY (Magic)
//!   w = the first SNIFF_LEN (8192) bytes
//!   BOM-less UTF-16/32 pattern in w[..4096] → text   (UtfPattern; the same function as encoding detection)
//!   a NUL byte in w                        → BINARY (NulByte)
//!   control bytes > 10 % of w              → BINARY (ControlRatio)
//!   otherwise                              → text   (Text)
//! ```

use serde::{Deserialize, Serialize};

use crate::encoding::{UTF_PATTERN_SAMPLE, bom_sniff, utf_pattern};

/// Binary-content verdict (TC-003, §2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BinaryVerdict {
    /// True when the content is classified binary.
    pub is_binary: bool,
    /// Rule that decided.
    pub reason: BinaryReason,
}

/// The rule that decided a `BinaryVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum BinaryReason {
    /// Empty content (text).
    Empty,
    /// A byte-order mark (text).
    Bom,
    /// A known binary file signature at offset 0 (binary).
    Magic {
        /// The signature.
        kind: MagicKind,
    },
    /// A BOM-less UTF-16/32 pattern (`utf_pattern`; text).
    UtfPattern,
    /// A NUL byte (binary).
    NulByte {
        /// Offset of the first NUL byte.
        offset: u32,
    },
    /// Too many control bytes (binary).
    ControlRatio {
        /// Control bytes per thousand bytes of the sniffed prefix.
        permille: u16,
    },
    /// None of the above (text).
    Text,
}

/// Signatures recognised by `sniff` (the in-house replacement for `infer`, §0.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MagicKind {
    /// PNG image.
    Png,
    /// GIF image.
    Gif,
    /// JPEG image.
    Jpeg,
    /// PDF document.
    Pdf,
    /// ZIP archive (also docx, xlsx, jar, …).
    Zip,
    /// gzip stream.
    Gzip,
    /// ELF executable or library.
    Elf,
    /// 7z archive.
    SevenZip,
    /// xz stream.
    Xz,
    /// Zstandard frame.
    Zstd,
}

/// The magic-number table (signatures at offset 0), in test order.
pub const MAGIC: &[(MagicKind, &[u8])] = &[
    (
        MagicKind::Png,
        &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
    ),
    (MagicKind::Gif, b"GIF87a"),
    (MagicKind::Gif, b"GIF89a"),
    (MagicKind::Jpeg, &[0xFF, 0xD8, 0xFF]),
    (MagicKind::Pdf, b"%PDF-"),
    (MagicKind::Zip, &[0x50, 0x4B, 0x03, 0x04]),
    (MagicKind::Zip, &[0x50, 0x4B, 0x05, 0x06]),
    (MagicKind::Zip, &[0x50, 0x4B, 0x07, 0x08]),
    (MagicKind::Gzip, &[0x1F, 0x8B]),
    (MagicKind::Elf, &[0x7F, 0x45, 0x4C, 0x46]),
    (MagicKind::SevenZip, &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]),
    (MagicKind::Xz, &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]),
    (MagicKind::Zstd, &[0x28, 0xB5, 0x2F, 0xFD]),
];

/// The signature at offset 0, if any.
pub fn magic(bytes: &[u8]) -> Option<MagicKind> {
    MAGIC
        .iter()
        .find(|(_, signature)| bytes.starts_with(signature))
        .map(|(kind, _)| *kind)
}

/// Bytes examined by `sniff` (prefix).
pub const SNIFF_LEN: usize = 8192;

/// Classifies content as text or binary from its first `SNIFF_LEN` bytes.
pub fn sniff(bytes: &[u8]) -> BinaryVerdict {
    let text = |reason| BinaryVerdict {
        is_binary: false,
        reason,
    };
    let binary = |reason| BinaryVerdict {
        is_binary: true,
        reason,
    };
    if bytes.is_empty() {
        return text(BinaryReason::Empty);
    }
    if bom_sniff(bytes).is_some() {
        return text(BinaryReason::Bom);
    }
    if let Some(kind) = magic(bytes) {
        return binary(BinaryReason::Magic { kind });
    }
    let w = &bytes[..bytes.len().min(SNIFF_LEN)];
    if utf_pattern(&w[..w.len().min(UTF_PATTERN_SAMPLE)]).is_some() {
        return text(BinaryReason::UtfPattern);
    }
    if let Some(offset) = memchr::memchr(0, w) {
        return binary(BinaryReason::NulByte {
            offset: offset as u32,
        });
    }
    let ctrl = w.iter().filter(|&&b| is_control(b)).count();
    if ctrl * 1000 > 100 * w.len() {
        let permille = (ctrl * 1000 / w.len()) as u16;
        return binary(BinaryReason::ControlRatio { permille });
    }
    text(BinaryReason::Text)
}

/// Control bytes that do not occur in ordinary text: C0 except HT, LF, VT, FF, CR, BS, SUB and ESC, plus DEL.
fn is_control(b: u8) -> bool {
    (b < 0x20 && !matches!(b, 0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x08 | 0x1A | 0x1B)) || b == 0x7F
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_in_order() {
        assert_eq!(sniff(b"").reason, BinaryReason::Empty);
        assert_eq!(sniff(b"\xEF\xBB\xBF\x00\x00").reason, BinaryReason::Bom);
        assert_eq!(
            sniff(b"\x1F\x8Bxyz"),
            BinaryVerdict {
                is_binary: true,
                reason: BinaryReason::Magic {
                    kind: MagicKind::Gzip
                }
            }
        );
        assert_eq!(sniff(b"a\x00b\x00").reason, BinaryReason::UtfPattern);
        assert_eq!(
            sniff(b"abc\x00def").reason,
            BinaryReason::NulByte { offset: 3 }
        );
        assert_eq!(
            sniff(b"\x1B[1mbold\x1B[0m\r\n\x08\x0C").reason,
            BinaryReason::Text
        );
        let mut eleven = vec![b'a'; 89];
        eleven.extend_from_slice(&[0x01; 11]);
        assert_eq!(
            sniff(&eleven),
            BinaryVerdict {
                is_binary: true,
                reason: BinaryReason::ControlRatio { permille: 110 }
            }
        );
        let mut ten = vec![b'a'; 90];
        ten.extend_from_slice(&[0x7F; 10]);
        assert_eq!(sniff(&ten).reason, BinaryReason::Text);
    }

    #[test]
    fn only_the_prefix_is_examined() {
        let mut late_nul = vec![b'a'; SNIFF_LEN];
        late_nul.push(0);
        assert!(!sniff(&late_nul).is_binary);
    }

    #[test]
    fn serde_shape() {
        let v = sniff(b"\x89PNG\r\n\x1a\n....");
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"is_binary":true,"reason":{"rule":"magic","kind":"png"}}"#
        );
    }
}

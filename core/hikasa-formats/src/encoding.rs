//! Encoding detection and decoding (TC-001, FF-032; §2.8, D-42): [`TextEncoding`], [`DetectedEncoding`],
//! [`EncodingSource`], [`DecodeOptions`], [`detect`], [`decode`], [`bom_sniff`] and [`utf_pattern`] (shared
//! with binary sniffing, so both always agree).
//!
//! Detection order: override → BOM → the format's fixed encoding → BOM-less UTF-16/32 byte pattern → pure
//! ASCII (format fallback, then the injected locale encoding, each only if ASCII-compatible, else UTF-8) →
//! valid UTF-8 → mostly valid UTF-8 (a few stray bytes; decoded lossily) → `chardetng` guess (UTF-8 and
//! ISO-2022-JP denied). The locale is never read here: the adapter passes it in
//! [`DecodeOptions::locale_fallback`] (T1).

use std::fmt;

use encoding_rs::{DecoderResult, Encoding};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::FormatError;
use crate::model::EncodingSetting;

/// A text encoding: every WHATWG encoding (`encoding_rs`) plus UTF-32 (own decoder; FF-032 lists it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    /// An `encoding_rs` encoding.
    Whatwg(&'static Encoding),
    /// UTF-32 little endian.
    Utf32Le,
    /// UTF-32 big endian.
    Utf32Be,
}

impl TextEncoding {
    /// UTF-8.
    pub const UTF_8: TextEncoding = TextEncoding::Whatwg(&encoding_rs::UTF_8_INIT);
    /// UTF-16 little endian.
    pub const UTF_16LE: TextEncoding = TextEncoding::Whatwg(&encoding_rs::UTF_16LE_INIT);
    /// UTF-16 big endian.
    pub const UTF_16BE: TextEncoding = TextEncoding::Whatwg(&encoding_rs::UTF_16BE_INIT);

    /// Canonical name ("UTF-8", "windows-1252", "UTF-16LE", "UTF-32BE", …).
    pub fn name(self) -> &'static str {
        match self {
            TextEncoding::Whatwg(e) => e.name(),
            TextEncoding::Utf32Le => "UTF-32LE",
            TextEncoding::Utf32Be => "UTF-32BE",
        }
    }

    /// Lookup by label (WHATWG labels plus "utf-32le"/"utf-32be", case-insensitive). Labels that WHATWG maps
    /// to the "replacement" encoding (ISO-2022-KR, HZ-GB-2312, …) are not decodable and give `None`.
    pub fn for_label(label: &str) -> Option<TextEncoding> {
        let trimmed = label.trim();
        if trimmed.eq_ignore_ascii_case("utf-32le")
            || trimmed.eq_ignore_ascii_case("utf-32")
            || trimmed.eq_ignore_ascii_case("utf32le")
            || trimmed.eq_ignore_ascii_case("utf32")
        {
            return Some(TextEncoding::Utf32Le);
        }
        if trimmed.eq_ignore_ascii_case("utf-32be") || trimmed.eq_ignore_ascii_case("utf32be") {
            return Some(TextEncoding::Utf32Be);
        }
        match Encoding::for_label(trimmed.as_bytes()) {
            Some(e) if e != encoding_rs::REPLACEMENT => Some(TextEncoding::Whatwg(e)),
            _ => None,
        }
    }

    /// True when ASCII bytes decode to the same ASCII chars (false for UTF-16, UTF-32 and ISO-2022-JP).
    pub fn is_ascii_compatible(self) -> bool {
        match self {
            TextEncoding::Whatwg(e) => e.is_ascii_compatible(),
            TextEncoding::Utf32Le | TextEncoding::Utf32Be => false,
        }
    }

    /// The byte-order mark of this encoding (empty for encodings without one).
    pub fn bom(self) -> &'static [u8] {
        match self {
            TextEncoding::Whatwg(e) if e == encoding_rs::UTF_8 => &[0xEF, 0xBB, 0xBF],
            TextEncoding::Whatwg(e) if e == encoding_rs::UTF_16LE => &[0xFF, 0xFE],
            TextEncoding::Whatwg(e) if e == encoding_rs::UTF_16BE => &[0xFE, 0xFF],
            TextEncoding::Whatwg(_) => &[],
            TextEncoding::Utf32Le => &[0xFF, 0xFE, 0x00, 0x00],
            TextEncoding::Utf32Be => &[0x00, 0x00, 0xFE, 0xFF],
        }
    }
}

impl fmt::Display for TextEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for TextEncoding {
    /// The canonical name.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for TextEncoding {
    /// Any label accepted by [`TextEncoding::for_label`].
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<TextEncoding, D::Error> {
        let label = String::deserialize(deserializer)?;
        TextEncoding::for_label(&label)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown encoding {label:?}")))
    }
}

/// How the encoding was chosen (shown in the File Info header and in JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingSource {
    /// Per-pane or command-line override.
    Override,
    /// Byte-order mark.
    Bom,
    /// The format's fixed encoding.
    Fixed,
    /// BOM-less UTF-16/32 byte pattern (`utf_pattern`).
    UtfPattern,
    /// Pure ASCII content: the format's fallback label.
    FormatFallback,
    /// Pure ASCII content: the adapter's locale encoding.
    LocaleFallback,
    /// Pure ASCII content without an ASCII-compatible fallback: UTF-8.
    Ascii,
    /// The content is valid UTF-8.
    Utf8Valid,
    /// The content is valid UTF-8 apart from a few malformed sequences (§2.8 step 4d, D-42): decoded as UTF-8,
    /// each malformed sequence becomes U+FFFD and is counted (warning `lossy_decode`).
    Utf8Lossy,
    /// chardetng's guess.
    Detector,
}

/// A detected encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DetectedEncoding {
    /// Encoding.
    pub encoding: TextEncoding,
    /// BOM length stripped from the content (0 = no BOM).
    pub bom_len: u8,
    /// How it was chosen.
    pub source: EncodingSource,
}

/// Decoding options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Per-pane override (TC-001); wins over everything except that a matching BOM is still stripped.
    pub override_encoding: Option<TextEncoding>,
    /// "Current locale" encoding, supplied by the adapter (core never reads the locale).
    pub locale_fallback: TextEncoding,
    /// Bytes given to `chardetng`. Default 1 MiB.
    pub detect_limit: usize,
}

impl Default for DecodeOptions {
    /// No override, UTF-8 as the locale encoding, 1 MiB for the detector.
    fn default() -> DecodeOptions {
        DecodeOptions {
            override_encoding: None,
            locale_fallback: TextEncoding::UTF_8,
            detect_limit: 1 << 20,
        }
    }
}

/// Bytes of the BOM-less UTF pattern sample (§2.8 step 4a).
pub const UTF_PATTERN_SAMPLE: usize = 4096;

/// Sniffs a BOM: (encoding, BOM length).
pub fn bom_sniff(bytes: &[u8]) -> Option<(TextEncoding, u8)> {
    match bytes {
        [0xEF, 0xBB, 0xBF, ..] => Some((TextEncoding::UTF_8, 3)),
        [0xFF, 0xFE, 0x00, 0x00, ..] => Some((TextEncoding::Utf32Le, 4)),
        [0x00, 0x00, 0xFE, 0xFF, ..] => Some((TextEncoding::Utf32Be, 4)),
        [0xFF, 0xFE, ..] => Some((TextEncoding::UTF_16LE, 2)),
        [0xFE, 0xFF, ..] => Some((TextEncoding::UTF_16BE, 2)),
        _ => None,
    }
}

fn label_encoding(label: &str) -> Result<TextEncoding, FormatError> {
    TextEncoding::for_label(label).ok_or_else(|| FormatError::UnknownEncoding {
        label: label.to_owned(),
    })
}

/// Detects the encoding (§2.8, D-42). Pure.
///
/// Fails only for an unknown label in `setting` (formats validate their labels when they are compiled).
pub fn detect(
    bytes: &[u8],
    setting: &EncodingSetting,
    opts: &DecodeOptions,
) -> Result<DetectedEncoding, FormatError> {
    let found = |encoding, bom_len, source| DetectedEncoding {
        encoding,
        bom_len,
        source,
    };
    // 1. Override (a matching BOM is still stripped).
    if let Some(e) = opts.override_encoding {
        let bom_len = match bom_sniff(bytes) {
            Some((b, n)) if b == e => n,
            _ => 0,
        };
        return Ok(found(e, bom_len, EncodingSource::Override));
    }
    // 2. BOM.
    if let Some((e, n)) = bom_sniff(bytes) {
        return Ok(found(e, n, EncodingSource::Bom));
    }
    match setting {
        // 3. Fixed.
        EncodingSetting::Fixed { label } => {
            Ok(found(label_encoding(label)?, 0, EncodingSource::Fixed))
        }
        EncodingSetting::Auto { fallback } => {
            // 4a. BOM-less UTF-16/32 pattern.
            let sample = &bytes[..bytes.len().min(UTF_PATTERN_SAMPLE)];
            if let Some(e) = utf_pattern(sample) {
                return Ok(found(e, 0, EncodingSource::UtfPattern));
            }
            // 4b. Pure ASCII.
            if bytes.is_ascii() {
                if let Some(label) = fallback {
                    let e = label_encoding(label)?;
                    if e.is_ascii_compatible() {
                        return Ok(found(e, 0, EncodingSource::FormatFallback));
                    }
                }
                if opts.locale_fallback.is_ascii_compatible() {
                    return Ok(found(
                        opts.locale_fallback,
                        0,
                        EncodingSource::LocaleFallback,
                    ));
                }
                return Ok(found(TextEncoding::UTF_8, 0, EncodingSource::Ascii));
            }
            // 4c. Valid UTF-8.
            if std::str::from_utf8(bytes).is_ok() {
                return Ok(found(TextEncoding::UTF_8, 0, EncodingSource::Utf8Valid));
            }
            // 4d. Mostly valid UTF-8: a few stray bytes must not re-decode the whole file in a legacy encoding.
            if mostly_utf8(bytes) {
                return Ok(found(TextEncoding::UTF_8, 0, EncodingSource::Utf8Lossy));
            }
            // 4e. Detector.
            let mut detector =
                chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
            let window = &bytes[..bytes.len().min(opts.detect_limit)];
            detector.feed(window, window.len() == bytes.len());
            let guess = detector.guess(None, chardetng::Utf8Detection::Deny);
            Ok(found(
                TextEncoding::Whatwg(guess),
                0,
                EncodingSource::Detector,
            ))
        }
    }
}

/// Malformed sequences allowed by the mostly-UTF-8 rule (§2.8 step 4d).
pub(crate) const MOSTLY_UTF8_MAX_MALFORMED: u64 = 16;

/// §2.8 step 4d (D-42): the content is UTF-8 with a few stray bytes. Over the whole content, `V` = valid
/// multi-byte sequences (chars ≥ U+0080) and `M` = malformed sequences (maximal subparts, the units that
/// [`decode`] replaces with U+FFFD, a truncated sequence at the end included). True when `V ≥ 1`,
/// `M ≤ 16` and `8 × M ≤ V`: valid multi-byte UTF-8 is strong evidence (legacy single- and double-byte text
/// almost never forms it, while almost every one of its non-ASCII bytes is malformed UTF-8), so a file with
/// eight times more of it than of stray bytes is decoded as UTF-8 with replacement characters instead of
/// being re-decoded wholesale in a legacy encoding.
pub(crate) fn mostly_utf8(bytes: &[u8]) -> bool {
    let mut valid = 0u64;
    let mut malformed = 0u64;
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                valid += lead_bytes(text.as_bytes());
                break;
            }
            Err(e) => {
                let good = e.valid_up_to();
                valid += lead_bytes(rest.get(..good).unwrap_or(&[]));
                malformed += 1;
                if malformed > MOSTLY_UTF8_MAX_MALFORMED {
                    return false;
                }
                let skip = good + e.error_len().unwrap_or(rest.len() - good);
                rest = rest.get(skip..).unwrap_or(&[]);
                if rest.is_empty() {
                    break;
                }
            }
        }
    }
    valid >= 1 && malformed * 8 <= valid
}

/// Lead bytes of multi-byte sequences in valid UTF-8 (one per char ≥ U+0080).
fn lead_bytes(valid: &[u8]) -> u64 {
    valid.iter().filter(|&&b| b >= 0xC0).count() as u64
}

/// The BOM-less UTF-32/UTF-16 byte-pattern heuristic of §2.8 step 4, on `sample` (callers pass at most the first
/// 4096 bytes). Shared by `detect` and `sniff`, so both always agree.
///
/// Candidates, in order: UTF-32LE, UTF-32BE, UTF-16LE and UTF-16BE by their zero-byte patterns, then BOM-less
/// UTF-16 text in one non-Latin alphabet (a constant high byte in `0x01..=0x2F`, for example Cyrillic 0x04 or
/// Greek 0x03, mixed with ASCII). A candidate is accepted only when its decoded sample is plausible text: at
/// most 10 % of the units are control characters outside HT, LF, VT, FF, CR, BS, SUB and ESC (C0, DEL and
/// C1) or unpaired surrogates. So arrays of small 16- or 32-bit integers stay binary.
pub fn utf_pattern(sample: &[u8]) -> Option<TextEncoding> {
    // UTF-32 first: ASCII UTF-32 also has many zero bytes at odd offsets.
    let units = sample.len() / 4;
    if units >= 2 {
        let mut le = 0usize;
        let mut be = 0usize;
        let mut le_valid = true;
        let mut be_valid = true;
        for &[a, b, c, d] in sample.as_chunks::<4>().0 {
            if c == 0 && d == 0 && (a, b) != (0, 0) {
                le += 1;
            }
            if a == 0 && b == 0 && (c, d) != (0, 0) {
                be += 1;
            }
            le_valid &= is_scalar(u32::from_le_bytes([a, b, c, d]));
            be_valid &= is_scalar(u32::from_be_bytes([a, b, c, d]));
        }
        if le_valid && le * 100 >= 90 * units && plausible(sample, TextEncoding::Utf32Le) {
            return Some(TextEncoding::Utf32Le);
        }
        if be_valid && be * 100 >= 90 * units && plausible(sample, TextEncoding::Utf32Be) {
            return Some(TextEncoding::Utf32Be);
        }
    }
    let pairs = sample.len() / 2;
    if pairs >= 2 {
        let mut zero_odd = 0usize;
        let mut zero_even = 0usize;
        let mut odd_values = [0usize; 256];
        let mut even_values = [0usize; 256];
        for &[even, odd] in sample.as_chunks::<2>().0 {
            zero_even += usize::from(even == 0);
            zero_odd += usize::from(odd == 0);
            odd_values[usize::from(odd)] += 1;
            even_values[usize::from(even)] += 1;
        }
        let le = TextEncoding::UTF_16LE;
        let be = TextEncoding::UTF_16BE;
        if zero_odd * 100 >= 40 * pairs && zero_even * 100 <= 5 * pairs && plausible(sample, le) {
            return Some(le);
        }
        if zero_even * 100 >= 40 * pairs && zero_odd * 100 <= 5 * pairs && plausible(sample, be) {
            return Some(be);
        }
        if one_alphabet(&odd_values, zero_even, pairs) && plausible(sample, le) {
            return Some(le);
        }
        if one_alphabet(&even_values, zero_odd, pairs) && plausible(sample, be) {
            return Some(be);
        }
    }
    None
}

/// BOM-less UTF-16 text in one non-Latin alphabet mixed with ASCII (§2.8): `high` counts the high bytes of the
/// `pairs` units, `zero_low` the zero low bytes. The most frequent non-zero high byte `v` (the smallest on a
/// tie) lies in `0x01..=0x2F` and covers ≥ 30 % of the units; zero high bytes (ASCII: spaces, digits,
/// punctuation, line breaks) cover ≥ 5 %; together they cover ≥ 90 %; and ≤ 5 % of the low bytes are zero.
fn one_alphabet(high: &[usize; 256], zero_low: usize, pairs: usize) -> bool {
    let mut best = (0usize, 0usize);
    for (v, &count) in high.iter().enumerate().skip(1) {
        if count > best.1 {
            best = (v, count);
        }
    }
    let (v, count) = best;
    let ascii = high[0];
    (0x01..=0x2F).contains(&v)
        && count * 100 >= 30 * pairs
        && ascii * 100 >= 5 * pairs
        && (ascii + count) * 100 >= 90 * pairs
        && zero_low * 100 <= 5 * pairs
}

/// True when at most 10 % of the units of `sample` decoded as `enc` are implausible in text: control characters
/// other than HT, LF, VT, FF, CR, BS, SUB and ESC (the C0 set of §2.9), DEL, the C1 controls U+0080–U+009F,
/// and (UTF-16) unpaired surrogates or (UTF-32) invalid scalar values.
fn plausible(sample: &[u8], enc: TextEncoding) -> bool {
    let bad_char = |v: u32| match v {
        0x08..=0x0D | 0x1A | 0x1B => false,
        0x00..=0x1F | 0x7F..=0x9F => true,
        _ => false,
    };
    let (units, bad) = match enc {
        TextEncoding::Utf32Le | TextEncoding::Utf32Be => {
            let chunks = sample.as_chunks::<4>().0;
            let bad = chunks
                .iter()
                .filter(|&&b| {
                    let v = if enc == TextEncoding::Utf32Be {
                        u32::from_be_bytes(b)
                    } else {
                        u32::from_le_bytes(b)
                    };
                    !is_scalar(v) || bad_char(v)
                })
                .count();
            (chunks.len(), bad)
        }
        TextEncoding::Whatwg(_) => {
            let big = enc == TextEncoding::UTF_16BE;
            let chunks = sample.as_chunks::<2>().0;
            let code_units = chunks.iter().map(|&b| {
                if big {
                    u16::from_be_bytes(b)
                } else {
                    u16::from_le_bytes(b)
                }
            });
            let bad = char::decode_utf16(code_units)
                .filter(|c| c.as_ref().map_or(true, |&c| bad_char(u32::from(c))))
                .count();
            (chunks.len(), bad)
        }
    };
    bad * 10 <= units
}

fn is_scalar(v: u32) -> bool {
    v <= 0x10FFFF && !(0xD800..=0xDFFF).contains(&v)
}

/// Decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Text without the BOM.
    pub text: String,
    /// Encoding used.
    pub encoding: DetectedEncoding,
    /// Number of malformed sequences replaced (0 = lossless decode).
    pub malformed: u64,
}

/// Decodes. Malformed sequences become U+FFFD and are counted (INV-ENC-1: never panics, total).
pub fn decode(bytes: &[u8], enc: &DetectedEncoding) -> Decoded {
    let body = bytes.get(usize::from(enc.bom_len)..).unwrap_or(&[]);
    let (text, malformed) = match enc.encoding {
        TextEncoding::Whatwg(e) => decode_whatwg(body, e),
        TextEncoding::Utf32Le => decode_utf32(body, false),
        TextEncoding::Utf32Be => decode_utf32(body, true),
    };
    Decoded {
        text,
        encoding: *enc,
        malformed,
    }
}

fn decode_whatwg(body: &[u8], encoding: &'static Encoding) -> (String, u64) {
    if encoding == encoding_rs::UTF_8
        && let Ok(text) = std::str::from_utf8(body)
    {
        return (text.to_owned(), 0);
    }
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let capacity = decoder
        .max_utf8_buffer_length_without_replacement(body.len())
        .unwrap_or(body.len().saturating_mul(3));
    let mut out = String::with_capacity(capacity);
    let mut malformed = 0u64;
    let mut pos = 0usize;
    loop {
        let (result, read) =
            decoder.decode_to_string_without_replacement(&body[pos..], &mut out, true);
        pos += read;
        match result {
            DecoderResult::InputEmpty => break,
            DecoderResult::OutputFull => {
                let more = decoder
                    .max_utf8_buffer_length_without_replacement(body.len() - pos)
                    .unwrap_or(4096)
                    .max(16);
                out.reserve(more);
            }
            DecoderResult::Malformed(_, _) => {
                malformed += 1;
                out.push('\u{FFFD}');
            }
        }
    }
    (out, malformed)
}

/// Hikasa's UTF-32 decoder: 4-byte units; an invalid scalar value or 1–3 trailing bytes → one U+FFFD each.
fn decode_utf32(body: &[u8], big_endian: bool) -> (String, u64) {
    let mut out = String::with_capacity(body.len() / 4 + 4);
    let mut malformed = 0u64;
    let (units, rest) = body.as_chunks::<4>();
    for &bytes in units {
        let v = if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        };
        match char::from_u32(v) {
            Some(c) => out.push(c),
            None => {
                malformed += 1;
                out.push('\u{FFFD}');
            }
        }
    }
    if !rest.is_empty() {
        malformed += 1;
        out.push('\u{FFFD}');
    }
    (out, malformed)
}

/// Encodes `text` (no BOM). UTF-16 and UTF-32 are written by Hikasa (encoding_rs only encodes into
/// ASCII-compatible encodings); other encodings use encoding_rs, where unmappable chars become HTML numeric
/// character references. Returns the bytes and whether some char was unmappable.
pub fn encode(text: &str, enc: TextEncoding) -> (Vec<u8>, bool) {
    match enc {
        TextEncoding::Utf32Le | TextEncoding::Utf32Be => {
            let mut out = Vec::with_capacity(text.len() * 4);
            for c in text.chars() {
                let v = u32::from(c);
                if enc == TextEncoding::Utf32Be {
                    out.extend_from_slice(&v.to_be_bytes());
                } else {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            (out, false)
        }
        TextEncoding::Whatwg(e) if e == encoding_rs::UTF_16LE || e == encoding_rs::UTF_16BE => {
            let mut out = Vec::with_capacity(text.len() * 2);
            for unit in text.encode_utf16() {
                if e == encoding_rs::UTF_16BE {
                    out.extend_from_slice(&unit.to_be_bytes());
                } else {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
            }
            (out, false)
        }
        TextEncoding::Whatwg(e) => {
            let (bytes, _, unmappable) = e.encode(text);
            (bytes.into_owned(), unmappable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto() -> EncodingSetting {
        EncodingSetting::Auto { fallback: None }
    }

    #[test]
    fn labels_and_names() {
        assert_eq!(TextEncoding::for_label("UTF-8"), Some(TextEncoding::UTF_8));
        assert_eq!(
            TextEncoding::for_label(" utf-32BE "),
            Some(TextEncoding::Utf32Be)
        );
        assert_eq!(
            TextEncoding::for_label("utf-32"),
            Some(TextEncoding::Utf32Le)
        );
        assert_eq!(
            TextEncoding::for_label("latin1").map(TextEncoding::name),
            Some("windows-1252")
        );
        assert_eq!(TextEncoding::for_label("hz-gb-2312"), None);
        assert_eq!(TextEncoding::for_label("cp437"), None);
        assert_eq!(TextEncoding::UTF_16BE.name(), "UTF-16BE");
        assert!(!TextEncoding::UTF_16LE.is_ascii_compatible());
        assert!(TextEncoding::UTF_8.is_ascii_compatible());
        assert_eq!(
            serde_json::to_string(&TextEncoding::Utf32Le).unwrap(),
            "\"UTF-32LE\""
        );
        let back: TextEncoding = serde_json::from_str("\"shift_jis\"").unwrap();
        assert_eq!(back.name(), "Shift_JIS");
        assert!(serde_json::from_str::<TextEncoding>("\"nope\"").is_err());
    }

    #[test]
    fn every_whatwg_name_is_a_label() {
        let all = [
            encoding_rs::BIG5,
            encoding_rs::EUC_JP,
            encoding_rs::EUC_KR,
            encoding_rs::GBK,
            encoding_rs::GB18030,
            encoding_rs::IBM866,
            encoding_rs::ISO_2022_JP,
            encoding_rs::ISO_8859_2,
            encoding_rs::ISO_8859_8_I,
            encoding_rs::KOI8_R,
            encoding_rs::KOI8_U,
            encoding_rs::MACINTOSH,
            encoding_rs::SHIFT_JIS,
            encoding_rs::UTF_16BE,
            encoding_rs::UTF_16LE,
            encoding_rs::UTF_8,
            encoding_rs::WINDOWS_874,
            encoding_rs::WINDOWS_1252,
            encoding_rs::WINDOWS_1258,
            encoding_rs::X_MAC_CYRILLIC,
            encoding_rs::X_USER_DEFINED,
        ];
        for e in all {
            let t = TextEncoding::Whatwg(e);
            assert_eq!(TextEncoding::for_label(t.name()), Some(t), "{}", t.name());
        }
        for t in [TextEncoding::Utf32Le, TextEncoding::Utf32Be] {
            assert_eq!(TextEncoding::for_label(t.name()), Some(t));
        }
    }

    #[test]
    fn override_strips_only_a_matching_bom() {
        let opts = DecodeOptions {
            override_encoding: Some(TextEncoding::UTF_8),
            ..DecodeOptions::default()
        };
        let d = detect(b"\xEF\xBB\xBFa", &auto(), &opts).unwrap();
        assert_eq!((d.bom_len, d.source), (3, EncodingSource::Override));
        let d = detect(b"\xFF\xFEa\x00", &auto(), &opts).unwrap();
        assert_eq!(d.bom_len, 0);
        assert_eq!(decode(b"\xEF\xBB\xBFa", &d).text, "\u{FEFF}a");
    }

    #[test]
    fn fixed_and_fallbacks() {
        let fixed = EncodingSetting::Fixed {
            label: "koi8-r".into(),
        };
        let d = detect(b"abc", &fixed, &DecodeOptions::default()).unwrap();
        assert_eq!(
            (d.encoding.name(), d.source),
            ("KOI8-R", EncodingSource::Fixed)
        );
        // A BOM still wins over Fixed.
        let d = detect(b"\xFE\xFF\x00a", &fixed, &DecodeOptions::default()).unwrap();
        assert_eq!(d.source, EncodingSource::Bom);
        // A non-ASCII-compatible fallback is skipped; so is a non-ASCII-compatible locale.
        let utf16_fallback = EncodingSetting::Auto {
            fallback: Some("utf-16le".into()),
        };
        let opts = DecodeOptions {
            locale_fallback: TextEncoding::UTF_16LE,
            ..DecodeOptions::default()
        };
        let d = detect(b"abc", &utf16_fallback, &opts).unwrap();
        assert_eq!(
            (d.encoding, d.source),
            (TextEncoding::UTF_8, EncodingSource::Ascii)
        );
        let bad = EncodingSetting::Auto {
            fallback: Some("nope".into()),
        };
        assert!(matches!(
            detect(b"abc", &bad, &DecodeOptions::default()),
            Err(FormatError::UnknownEncoding { .. })
        ));
        // Empty content is pure ASCII.
        let d = detect(b"", &auto(), &DecodeOptions::default()).unwrap();
        assert_eq!(d.source, EncodingSource::LocaleFallback);
    }

    #[test]
    fn decoding_counts_malformed_sequences() {
        let utf8 = DetectedEncoding {
            encoding: TextEncoding::UTF_8,
            bom_len: 0,
            source: EncodingSource::Override,
        };
        let d = decode(b"a\xFFb\xC3", &utf8);
        assert_eq!(d.text, "a\u{FFFD}b\u{FFFD}");
        assert_eq!(d.malformed, 2);
        let utf32 = DetectedEncoding {
            encoding: TextEncoding::Utf32Le,
            bom_len: 0,
            source: EncodingSource::Override,
        };
        let d = decode(b"a\x00\x00\x00\x00\xD8\x00\x00\x62\x00", &utf32);
        assert_eq!(d.text, "a\u{FFFD}\u{FFFD}");
        assert_eq!(d.malformed, 2);
        let utf16 = DetectedEncoding {
            encoding: TextEncoding::UTF_16LE,
            bom_len: 2,
            source: EncodingSource::Bom,
        };
        let d = decode(b"\xFF\xFEa\x00\x00\xD8b\x00", &utf16);
        assert_eq!(d.text, "a\u{FFFD}b");
        assert_eq!(d.malformed, 1);
        // A BOM length past the end is total.
        let d = decode(b"\xEF", &DetectedEncoding { bom_len: 3, ..utf8 });
        assert_eq!(d.text, "");
    }

    #[test]
    fn encode_round_trips() {
        for enc in [
            TextEncoding::UTF_8,
            TextEncoding::UTF_16LE,
            TextEncoding::UTF_16BE,
            TextEncoding::Utf32Le,
            TextEncoding::Utf32Be,
        ] {
            let text = "a\u{e9}\u{1F600}\r\n";
            let (bytes, unmappable) = encode(text, enc);
            assert!(!unmappable);
            let d = DetectedEncoding {
                encoding: enc,
                bom_len: 0,
                source: EncodingSource::Override,
            };
            assert_eq!(decode(&bytes, &d).text, text, "{enc}");
        }
        let (bytes, unmappable) = encode("\u{1F600}", TextEncoding::for_label("latin1").unwrap());
        assert!(unmappable);
        assert_eq!(bytes, b"&#128512;");
        assert_eq!(TextEncoding::Utf32Be.bom(), [0, 0, 0xFE, 0xFF]);
    }

    #[test]
    fn utf_patterns() {
        assert_eq!(utf_pattern(b"a\x00b\x00"), Some(TextEncoding::UTF_16LE));
        assert_eq!(utf_pattern(b"\x00a\x00b"), Some(TextEncoding::UTF_16BE));
        assert_eq!(
            utf_pattern(b"a\x00\x00\x00b\x00\x00\x00"),
            Some(TextEncoding::Utf32Le)
        );
        assert_eq!(
            utf_pattern(b"\x00\x00\x00a\x00\x00\x00b"),
            Some(TextEncoding::Utf32Be)
        );
        assert_eq!(utf_pattern(b"a\x00"), None);
        assert_eq!(utf_pattern(&[0u8; 64]), None);
        assert_eq!(utf_pattern(b"hello world"), None);
        // An invalid UTF-32 unit (> 0x10FFFF) rules UTF-32 out, even when 90 % of the units look right.
        let mut sample = b"a\x00\x00\x00".repeat(9);
        sample.extend_from_slice(b"\x00\x00\x11\x00");
        assert_eq!(utf_pattern(&sample), None);
        sample.truncate(36);
        sample.extend_from_slice(b"b\x00\x00\x00");
        assert_eq!(utf_pattern(&sample), Some(TextEncoding::Utf32Le));
    }

    /// Arrays of small 16- and 32-bit integers have the zero-byte pattern of ASCII UTF-16/32 but decode to
    /// control characters: not text. BOM-less UTF-16 in one non-Latin alphabet is text (§2.8; wave-2 review).
    #[test]
    fn utf_pattern_rejects_integers_and_accepts_one_alphabet() {
        let u16s: Vec<u8> = (0u16..2048)
            .map(|i| i % 200)
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(utf_pattern(&u16s[..UTF_PATTERN_SAMPLE]), None);
        let u16be: Vec<u8> = (0u16..2048)
            .map(|i| i % 200)
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(utf_pattern(&u16be[..UTF_PATTERN_SAMPLE]), None);
        let u32s: Vec<u8> = (0u32..1024)
            .map(|i| i % 200)
            .flat_map(u32::to_le_bytes)
            .collect();
        assert_eq!(utf_pattern(&u32s), None);
        // ASCII text with tabs and line breaks stays UTF-16/32 (no implausible control characters).
        let text: Vec<u8> = "a\tb\r\nc\x0c\x1b[0m\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(utf_pattern(&text), Some(TextEncoding::UTF_16LE));
        let cyrillic = "Привет мир, это текст.\n".repeat(20);
        let le: Vec<u8> = cyrillic.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = cyrillic.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(utf_pattern(&le), Some(TextEncoding::UTF_16LE));
        assert_eq!(utf_pattern(&be), Some(TextEncoding::UTF_16BE));
        let greek: Vec<u8> = "Καλημέρα κόσμε, καλή μέρα.\n"
            .repeat(10)
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(utf_pattern(&greek), Some(TextEncoding::UTF_16LE));
        // Without any ASCII (no spaces or line breaks) a constant high byte is not enough.
        let letters: Vec<u8> = "Приветмир"
            .repeat(20)
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(utf_pattern(&letters), None);
        // CJK spreads over many high bytes: not recognised without a BOM (documented limitation).
        let cjk: Vec<u8> = "漢字の文章、テキスト。\n"
            .repeat(10)
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(utf_pattern(&cjk), None);
        // ASCII bytes alone never look like one alphabet (a constant odd byte, but no zero high bytes).
        assert_eq!(utf_pattern(&b"a,b,c,d,e,f,g,h,".repeat(8)), None);
    }

    /// §2.8 step 4d (D-42): a few stray bytes in UTF-8 keep UTF-8; legacy text does not become UTF-8.
    #[test]
    fn mostly_utf8_rule() {
        let ru = "Привет, как дела? Это тестовый текст на русском языке.\n".repeat(10);
        let mut bytes = ru.clone().into_bytes();
        bytes.push(0xFF);
        let d = detect(&bytes, &auto(), &DecodeOptions::default()).unwrap();
        assert_eq!(d.encoding, TextEncoding::UTF_8);
        assert_eq!(d.source, EncodingSource::Utf8Lossy);
        let decoded = decode(&bytes, &d);
        assert_eq!(decoded.malformed, 1);
        assert_eq!(decoded.text, format!("{ru}\u{FFFD}"));
        // A truncated sequence at the end counts as one malformed sequence.
        let mut cut = "é".repeat(8).into_bytes();
        cut.push(0xC3);
        assert!(mostly_utf8(&cut));
        // The bounds: 8 valid sequences per malformed one, at most 16 malformed, at least one valid.
        let with = |valid: usize, stray: usize| {
            let mut b = "é".repeat(valid).into_bytes();
            for _ in 0..stray {
                b.extend_from_slice(b" \xFF");
            }
            b
        };
        assert!(mostly_utf8(&with(8, 1)));
        assert!(!mostly_utf8(&with(7, 1)));
        assert!(mostly_utf8(&with(128, 16)));
        assert!(!mostly_utf8(&with(1000, 17)));
        assert!(!mostly_utf8(&with(0, 1)));
        assert!(!mostly_utf8(b"caf\xE9 cr\xE8me"));
        // Legacy text with many non-ASCII bytes still goes to the detector.
        let latin1: Vec<u8> = "Le café est très bon, déjà vu à Noël.\n"
            .repeat(5)
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap())
            .collect();
        let d = detect(&latin1, &auto(), &DecodeOptions::default()).unwrap();
        assert_eq!(d.source, EncodingSource::Detector);
    }
}

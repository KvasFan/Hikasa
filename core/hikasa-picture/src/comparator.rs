//! [`PictureComparator`]: the Rules content test of Picture-kind formats (D-344, §1.12.21).
//!
//! A folder comparison and `quick --rules` compare a pair whose format is of kind Picture by its pixels: both files
//! are read (at most [`PictureComparator::limit`] bytes each), identical bytes are `binary_same` without decoding,
//! otherwise both decode and compare under the tolerance and Auto Scale of the Picture Compare defaults session
//! (the Service fills [`PictureComparator::options`] and [`PictureComparator::decode`]). A pair the decoders do not
//! support (`unsupported`: DDS, AVIF, …) falls back to the bytes with `fell_back_to_binary` (plan A8); a pair that
//! fails to decode (`invalid_data`, `too_large`) is `error` for that pair, never a fallback (T-21's rule). Files
//! larger than the limit compare by bytes with `rules_size_limit`, as the text comparator's.

use std::sync::Arc;

use hikasa_core::{
    Cancelled, Classify, ContentComparator, ContentMethod, ContentOutcome, ContentResult,
    ContentSource, Ctl, ErrorReport, ReadError, Warning, WarningCode,
};

use crate::codec::{DecodeOptions, Encoded, decode_two, decode_vec};
use crate::compare::{PictureOptions, PictureVerdict, compare_pictures};
use crate::error::PictureError;
use crate::model::Picture;

/// A Picture format's load conversion, run on a side before it decodes (D-336, D-337; W12c review fixes, F1). The
/// Service implements it with the same code `picture compare` converts with (T11): the format of `name` (its own
/// detection, or a forced format), its External program through the Service's runner and temp folder.
pub trait PictureConverter: Send + Sync + std::fmt::Debug {
    /// The program's output for `input`, the content of `name`; `Ok(None)` when `name`'s format has no conversion
    /// (the input decodes as it is).
    ///
    /// # Errors
    ///
    /// [`PictureError::Cancelled`]; any other error is the pair's `error` (a failed program, an empty output).
    fn convert(
        &self,
        name: &str,
        input: &[u8],
        ctl: &Ctl<'_>,
    ) -> Result<Option<Vec<u8>>, PictureError>;
}

/// The Rules content comparator of Picture-kind formats (D-344).
#[derive(Debug, Clone)]
pub struct PictureComparator {
    /// Tolerance and geometry (only Auto Scale applies to a folder row; orientations and the offset stay identity).
    pub options: PictureOptions,
    /// Decoding: the tone map and `max_pixels` of the defaults session.
    pub decode: DecodeOptions,
    /// Largest file read (default 512 MiB): a larger file compares by bytes.
    pub limit: u64,
    /// The load conversions of the sides' Picture formats (`None`: every side decodes as it is).
    pub converter: Option<Arc<dyn PictureConverter>>,
}

impl Default for PictureComparator {
    fn default() -> PictureComparator {
        PictureComparator {
            options: PictureOptions::default(),
            decode: DecodeOptions::default(),
            limit: PictureComparator::DEFAULT_LIMIT,
            converter: None,
        }
    }
}

/// What a pair of read files gave.
enum Answer {
    /// The result, with the comparison's warnings.
    Result(ContentResult, Vec<Warning>),
    /// An unsupported container: the bytes decide (they differ), with this message.
    Fallback(String),
}

impl PictureComparator {
    /// The default size limit: 512 MiB a file.
    pub const DEFAULT_LIMIT: u64 = 512 << 20;

    /// A side ready to decode: the conversion's output when its format has one (no name hint: the output's content
    /// decides), else its own bytes.
    fn prepare(&self, bytes: Vec<u8>, name: &str, ctl: &Ctl<'_>) -> Result<Encoded, PictureError> {
        if let Some(converter) = &self.converter
            && let Some(output) = converter.convert(name, &bytes, ctl)?
        {
            return Ok(Encoded {
                bytes: output,
                name: None,
            });
        }
        Ok(Encoded {
            bytes,
            name: Some(name.to_owned()),
        })
    }

    /// Compares two pictures held in memory, named `names` (the container by content, then the name). Each side
    /// is converted first when its format has a load conversion; its bytes are dropped once decoded.
    ///
    /// # Errors
    ///
    /// [`Cancelled`].
    fn compare_bytes(
        &self,
        left: (Vec<u8>, &str),
        right: (Vec<u8>, &str),
        ctl: &Ctl<'_>,
    ) -> Result<Answer, Cancelled> {
        if left.0 == right.0 {
            return Ok(Answer::Result(ContentResult::BinarySame, Vec::new()));
        }
        let (ln, rn) = (left.1, right.1);
        let (l, r) = rayon::join(
            || self.prepare(left.0, ln, ctl),
            || self.prepare(right.0, rn, ctl),
        );
        let (lp, rp) = match (l, r) {
            (Ok(l), Ok(r)) => {
                let pair = decode_two(l, r, &self.decode, ctl);
                (pair.left, pair.right)
            }
            (l, r) => {
                let one = |e: Encoded| decode_vec(e.bytes, e.name.as_deref(), &self.decode, ctl);
                (l.and_then(one), r.and_then(one))
            }
        };
        let error = |e: PictureError, name: &str| -> Result<Answer, Cancelled> {
            match e {
                PictureError::Cancelled => Err(Cancelled),
                PictureError::Unsupported { format } => Ok(Answer::Fallback(format!(
                    "{name}: the {format} picture format is not decoded; compared by bytes"
                ))),
                other => Ok(Answer::Result(
                    ContentResult::Error {
                        error: ErrorReport {
                            kind: other.kind(),
                            message: format!("{name}: {other}"),
                            spec: Some(name.to_owned()),
                            causes: Vec::new(),
                        },
                    },
                    Vec::new(),
                )),
            }
        };
        // An unsupported side falls back whatever the other side is (the bytes decide); a decode error of
        // either side is the pair's error.
        let (lp, rp) = match (lp, rp) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(PictureError::Cancelled), _) | (_, Err(PictureError::Cancelled)) => {
                return Err(Cancelled);
            }
            (Err(e @ PictureError::Unsupported { .. }), _) => return error(e, ln),
            (_, Err(e @ PictureError::Unsupported { .. })) => return error(e, rn),
            (Err(e), _) => return error(e, ln),
            (_, Err(e)) => return error(e, rn),
        };
        // The decoder's and the comparison's warnings that are not informational (F6): an ICC profile ignored,
        // Auto Scale of differently proportioned pictures.
        let mut warnings: Vec<Warning> = Vec::new();
        let mut keep = |w: &crate::error::PictureWarning| {
            if !w.code.is_informational() {
                warnings.push(w.to_warning());
            }
        };
        let side_warnings = |p: &Picture, name: &str| p.warnings(name);
        side_warnings(&lp, ln).iter().for_each(&mut keep);
        side_warnings(&rp, rn).iter().for_each(&mut keep);
        let comparison = match compare_pictures(Arc::new(lp), Arc::new(rp), &self.options, ctl) {
            Ok(c) => c,
            Err(e) => return error(e, ln),
        };
        comparison.warnings().iter().for_each(&mut keep);
        // The bytes differ (checked above), so an all-same comparison is `rules_same`.
        let verdict = PictureVerdict::of(comparison.stats(), false, || Ok::<_, Cancelled>(false))?;
        let result = match verdict.kind {
            hikasa_core::ContentKind::BinarySame => ContentResult::BinarySame,
            hikasa_core::ContentKind::RulesSame => ContentResult::RulesSame,
            hikasa_core::ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
            _ => ContentResult::ImportantDifferences,
        };
        Ok(Answer::Result(result, warnings))
    }
}

fn outcome(result: ContentResult, warnings: Vec<Warning>) -> ContentOutcome {
    ContentOutcome {
        method: ContentMethod::Rules,
        result,
        warnings,
    }
}

fn read_error(source: &dyn ContentSource, e: &ReadError) -> ContentResult {
    ContentResult::Error {
        error: ErrorReport {
            kind: e.kind(),
            message: format!("{}: {e}", source.display()),
            spec: Some(source.display()),
            causes: Vec::new(),
        },
    }
}

/// Compares two sources by streaming their bytes in 1 MiB reads (files over the limit).
fn stream_bytes(
    left: &dyn ContentSource,
    right: &dyn ContentSource,
    ctl: &Ctl<'_>,
) -> Result<ContentResult, Cancelled> {
    use std::io::Read;
    if let (Some(a), Some(b)) = (left.len(), right.len())
        && a != b
    {
        return Ok(ContentResult::BinaryDifferent);
    }
    let (mut a, mut b) = match (left.open(), right.open()) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) => return Ok(read_error(left, &ReadError::Io(e))),
        (_, Err(e)) => return Ok(read_error(right, &ReadError::Io(e))),
    };
    let mut ba = vec![0u8; 1 << 20];
    let mut bb = vec![0u8; 1 << 20];
    loop {
        ctl.check()?;
        let na = match read_full(&mut a, &mut ba) {
            Ok(n) => n,
            Err(e) => return Ok(read_error(left, &ReadError::Io(e))),
        };
        let nb = match read_full(&mut b, &mut bb) {
            Ok(n) => n,
            Err(e) => return Ok(read_error(right, &ReadError::Io(e))),
        };
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(ContentResult::BinaryDifferent);
        }
        if na == 0 {
            return Ok(ContentResult::BinarySame);
        }
    }

    fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            match r.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(n)
    }
}

impl ContentComparator for PictureComparator {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn size_limit(&self) -> Option<u64> {
        Some(self.limit)
    }

    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, Cancelled> {
        ctl.check()?;
        let size_limit = |source: &dyn ContentSource| Warning {
            code: WarningCode::RulesSizeLimit,
            message: format!(
                "{} is larger than the picture comparison limit of {} bytes; compared by bytes",
                source.display(),
                self.limit
            ),
            spec: None,
        };
        for source in [left, right] {
            if source.len().is_some_and(|len| len > self.limit) {
                let result = stream_bytes(left, right, ctl)?;
                return Ok(outcome(result, vec![size_limit(source)]));
            }
        }
        let read = |source: &dyn ContentSource| source.read_all(self.limit, ctl.cancel);
        let (lb, rb) = match (read(left), read(right)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(ReadError::Cancelled), _) | (_, Err(ReadError::Cancelled)) => {
                return Err(Cancelled);
            }
            (Err(ReadError::TooLarge { .. }), _) => {
                let result = stream_bytes(left, right, ctl)?;
                return Ok(outcome(result, vec![size_limit(left)]));
            }
            (_, Err(ReadError::TooLarge { .. })) => {
                let result = stream_bytes(left, right, ctl)?;
                return Ok(outcome(result, vec![size_limit(right)]));
            }
            (Err(e), _) => return Ok(outcome(read_error(left, &e), Vec::new())),
            (_, Err(e)) => return Ok(outcome(read_error(right, &e), Vec::new())),
        };
        let (ln, rn) = (left.display(), right.display());
        let names = (name_of(left, &ln), name_of(right, &rn));
        match self.compare_bytes((lb, names.0), (rb, names.1), ctl)? {
            Answer::Result(result, warnings) => Ok(outcome(result, warnings)),
            // The bytes were found to differ before decoding.
            Answer::Fallback(message) => Ok(outcome(
                ContentResult::BinaryDifferent,
                vec![Warning {
                    code: WarningCode::FellBackToBinary,
                    message,
                    spec: None,
                }],
            )),
        }
    }
}

/// The name hint of a source: its name, else its display text.
fn name_of<'a>(source: &'a dyn ContentSource, display: &'a str) -> &'a str {
    let name = source.name();
    if name.is_empty() { display } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{PngLevel, encode_png};
    use crate::model::RgbaImage;
    use hikasa_core::CancelToken;

    struct Bytes(&'static str, Vec<u8>);

    impl ContentSource for Bytes {
        fn name(&self) -> &str {
            self.0
        }
        fn display(&self) -> String {
            self.0.to_owned()
        }
        fn len(&self) -> Option<u64> {
            Some(self.1.len() as u64)
        }
        fn open(&self) -> std::io::Result<Box<dyn std::io::Read + Send + '_>> {
            Ok(Box::new(std::io::Cursor::new(self.1.clone())))
        }
        fn read_all(&self, limit: u64, _: &CancelToken) -> Result<Vec<u8>, ReadError> {
            if self.1.len() as u64 > limit {
                return Err(ReadError::TooLarge { limit });
            }
            Ok(self.1.clone())
        }
    }

    fn png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
        let image = RgbaImage {
            width: w,
            height: h,
            rgba: rgba.to_vec(),
        };
        encode_png(&image, PngLevel::Fast).unwrap()
    }

    fn run(c: &PictureComparator, l: Bytes, r: Bytes) -> ContentOutcome {
        c.compare(&l, &r, &Ctl::detached()).unwrap()
    }

    #[test]
    fn classes_fallback_and_errors() {
        let a = png(&[10, 20, 30, 255, 0, 0, 0, 255], 2, 1);
        let b = png(&[12, 20, 30, 255, 0, 0, 0, 255], 2, 1);
        let mut c = PictureComparator::default();
        let o = run(&c, Bytes("a.png", a.clone()), Bytes("b.png", a.clone()));
        assert_eq!(o.result, ContentResult::BinarySame);
        let o = run(&c, Bytes("a.png", a.clone()), Bytes("b.png", b.clone()));
        assert_eq!(o.result, ContentResult::ImportantDifferences);
        c.options.tolerance = 2;
        let o = run(&c, Bytes("a.png", a.clone()), Bytes("b.png", b.clone()));
        assert_eq!(o.result, ContentResult::UnimportantOnly);
        let dds = Bytes("a.dds", b"DDS |rest of a header".to_vec());
        let o = run(&c, dds, Bytes("b.dds", b"DDS |other".to_vec()));
        assert_eq!(o.result, ContentResult::BinaryDifferent);
        assert_eq!(o.warnings[0].code, WarningCode::FellBackToBinary);
        let o = run(&c, Bytes("a.png", a[..20].to_vec()), Bytes("b.png", b));
        assert!(
            matches!(&o.result, ContentResult::Error { error } if error.kind == hikasa_core::ErrorKind::InvalidData),
            "{o:?}"
        );
        c.limit = 10;
        let o = run(&c, Bytes("a.png", a.clone()), Bytes("b.png", a));
        assert_eq!(o.result, ContentResult::BinarySame);
        assert_eq!(o.warnings[0].code, WarningCode::RulesSizeLimit);
    }

    /// W12c review fixes (N38–N40): the error names the side that failed; a file of exactly the limit is decoded;
    /// an unsupported side falls back to the bytes even when the other side does not decode.
    #[test]
    fn errors_name_their_side_the_limit_and_the_fallback_precedence() {
        let a = png(&[10, 20, 30, 255], 1, 1);
        let c = PictureComparator::default();
        let o = run(
            &c,
            Bytes("good.png", a.clone()),
            Bytes("broken.png", a[..20].to_vec()),
        );
        match &o.result {
            ContentResult::Error { error } => {
                assert_eq!(error.spec.as_deref(), Some("broken.png"));
                assert!(
                    error.message.starts_with("broken.png: "),
                    "{}",
                    error.message
                );
            }
            other => panic!("{other:?}"),
        }
        let o = run(
            &c,
            Bytes("broken.png", a[..20].to_vec()),
            Bytes("good.png", a.clone()),
        );
        assert!(
            matches!(&o.result, ContentResult::Error { error } if error.spec.as_deref() == Some("broken.png")),
            "{o:?}"
        );
        // Exactly the limit: decoded (the same pixels encoded otherwise are rules_same, not a byte comparison
        // with a warning).
        let rgba: Vec<u8> = (0..64 * 64 * 4).map(|i| (i % 251) as u8).collect();
        let image = RgbaImage {
            width: 64,
            height: 64,
            rgba,
        };
        let fast = encode_png(&image, PngLevel::Fast).unwrap();
        let best = encode_png(&image, PngLevel::Best).unwrap();
        assert_ne!(fast, best);
        let c = PictureComparator {
            limit: fast.len().max(best.len()) as u64,
            ..PictureComparator::default()
        };
        let o = run(
            &c,
            Bytes("a.png", fast.clone()),
            Bytes("b.png", best.clone()),
        );
        assert_eq!(o.result, ContentResult::RulesSame, "{o:?}");
        assert!(o.warnings.is_empty(), "{o:?}");
        // An unsupported right side beside a left side that does not decode: the bytes decide (plan A8).
        let o = run(
            &PictureComparator::default(),
            Bytes("broken.png", a[..20].to_vec()),
            Bytes("t.dds", b"DDS |x".to_vec()),
        );
        assert_eq!(o.result, ContentResult::BinaryDifferent, "{o:?}");
        assert_eq!(o.warnings[0].code, WarningCode::FellBackToBinary);
    }

    /// A converter of the test: a `*.conv` file holds `r,g,b` text, converted to a 2 × 2 PNG of that colour;
    /// `fail` fails; it counts its calls.
    #[derive(Debug, Default)]
    struct Conv(std::sync::atomic::AtomicUsize);

    impl PictureConverter for Conv {
        fn convert(
            &self,
            name: &str,
            input: &[u8],
            _ctl: &Ctl<'_>,
        ) -> Result<Option<Vec<u8>>, PictureError> {
            if !name.ends_with(".conv") {
                return Ok(None);
            }
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let text = String::from_utf8_lossy(input);
            if text.starts_with("fail") {
                return Err(PictureError::Decode {
                    format: "conversion".into(),
                    message: "the program failed".into(),
                });
            }
            let c: Vec<u8> = text
                .trim()
                .split(',')
                .map(|v| v.trim().parse().unwrap())
                .collect();
            Ok(Some(png(&[c[0], c[1], c[2], 255].repeat(4), 2, 2)))
        }
    }

    /// F1: a side whose format has a load conversion decodes from the program's output, never from its own bytes;
    /// identical inputs are `binary_same` without a conversion; a failed conversion is the pair's error.
    #[test]
    fn sides_with_a_conversion_decode_the_output() {
        let conv = Arc::new(Conv::default());
        let c = PictureComparator {
            converter: Some(Arc::clone(&conv) as Arc<dyn PictureConverter>),
            ..PictureComparator::default()
        };
        let o = run(
            &c,
            Bytes("a.conv", b"1,2,3".to_vec()),
            Bytes("b.conv", b"1, 2, 3 ".to_vec()),
        );
        assert_eq!(o.result, ContentResult::RulesSame, "{o:?}");
        assert_eq!(conv.0.load(std::sync::atomic::Ordering::SeqCst), 2);
        let o = run(
            &c,
            Bytes("a.conv", b"1,2,3".to_vec()),
            Bytes("b.conv", b"9,2,3".to_vec()),
        );
        assert_eq!(o.result, ContentResult::ImportantDifferences, "{o:?}");
        // One side converted, the other a PNG of the same pixels.
        let o = run(
            &c,
            Bytes("a.conv", b"1,2,3".to_vec()),
            Bytes("b.png", png(&[1, 2, 3, 255].repeat(4), 2, 2)),
        );
        assert_eq!(o.result, ContentResult::RulesSame, "{o:?}");
        let before = conv.0.load(std::sync::atomic::Ordering::SeqCst);
        let o = run(
            &c,
            Bytes("a.conv", b"1,2,3".to_vec()),
            Bytes("b.conv", b"1,2,3".to_vec()),
        );
        assert_eq!(o.result, ContentResult::BinarySame);
        assert_eq!(conv.0.load(std::sync::atomic::Ordering::SeqCst), before);
        let o = run(
            &c,
            Bytes("a.conv", b"1,2,3".to_vec()),
            Bytes("b.conv", b"fail".to_vec()),
        );
        assert!(
            matches!(&o.result, ContentResult::Error { error } if error.spec.as_deref() == Some("b.conv")
                && error.message.contains("the program failed")),
            "{o:?}"
        );
        // Without the converter the same files are no pictures: the bytes decide.
        let o = run(
            &PictureComparator::default(),
            Bytes("a.conv", b"1,2,3".to_vec()),
            Bytes("b.conv", b"1, 2, 3 ".to_vec()),
        );
        assert_eq!(o.result, ContentResult::BinaryDifferent, "{o:?}");
    }

    /// F6: the comparison's warnings that are not informational reach the outcome (Auto Scale of differently
    /// proportioned pictures).
    #[test]
    fn auto_scale_of_other_proportions_warns() {
        let mut c = PictureComparator::default();
        c.options.geometry.auto_scale = true;
        let sq = png(&[5, 5, 5, 255].repeat(16), 4, 4);
        let wide = png(&[5, 5, 5, 255].repeat(64), 16, 4);
        let o = run(&c, Bytes("sq.png", sq), Bytes("wide.png", wide));
        assert_eq!(o.result, ContentResult::RulesSame, "{o:?}");
        assert_eq!(
            o.warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
            [WarningCode::AspectRatioDiffers],
            "{o:?}"
        );
    }
}

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

use crate::codec::{DecodeOptions, decode_bytes};
use crate::compare::{PictureOptions, PictureVerdict, compare_pictures};
use crate::error::PictureError;

/// The Rules content comparator of Picture-kind formats (D-344).
#[derive(Debug, Clone)]
pub struct PictureComparator {
    /// Tolerance and geometry (only Auto Scale applies to a folder row; orientations and the offset stay identity).
    pub options: PictureOptions,
    /// Decoding: the tone map and `max_pixels` of the defaults session.
    pub decode: DecodeOptions,
    /// Largest file read (default 512 MiB): a larger file compares by bytes.
    pub limit: u64,
}

impl Default for PictureComparator {
    fn default() -> PictureComparator {
        PictureComparator {
            options: PictureOptions::default(),
            decode: DecodeOptions::default(),
            limit: PictureComparator::DEFAULT_LIMIT,
        }
    }
}

/// What a pair of read files gave.
enum Answer {
    Result(ContentResult),
    Fallback(String),
}

impl PictureComparator {
    /// The default size limit: 512 MiB a file.
    pub const DEFAULT_LIMIT: u64 = 512 << 20;

    /// Compares two pictures held in memory, named `names` (the container by content, then the name).
    ///
    /// # Errors
    ///
    /// [`Cancelled`].
    fn compare_bytes(
        &self,
        left: (&[u8], &str),
        right: (&[u8], &str),
        ctl: &Ctl<'_>,
    ) -> Result<Answer, Cancelled> {
        if left.0 == right.0 {
            return Ok(Answer::Result(ContentResult::BinarySame));
        }
        let (lp, rp) = rayon::join(
            || decode_bytes(left.0, Some(left.1), &self.decode, ctl),
            || decode_bytes(right.0, Some(right.1), &self.decode, ctl),
        );
        let error = |e: PictureError, name: &str| -> Result<Answer, Cancelled> {
            match e {
                PictureError::Cancelled => Err(Cancelled),
                PictureError::Unsupported { format } => Ok(Answer::Fallback(format!(
                    "{name}: the {format} picture format is not decoded; compared by bytes"
                ))),
                other => Ok(Answer::Result(ContentResult::Error {
                    error: ErrorReport {
                        kind: other.kind(),
                        message: format!("{name}: {other}"),
                        spec: Some(name.to_owned()),
                        causes: Vec::new(),
                    },
                })),
            }
        };
        // An unsupported side falls back whatever the other side is (the bytes decide); a decode error of
        // either side is the pair's error.
        let (lp, rp) = match (lp, rp) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(PictureError::Cancelled), _) | (_, Err(PictureError::Cancelled)) => {
                return Err(Cancelled);
            }
            (Err(e @ PictureError::Unsupported { .. }), _) => return error(e, left.1),
            (_, Err(e @ PictureError::Unsupported { .. })) => return error(e, right.1),
            (Err(e), _) => return error(e, left.1),
            (_, Err(e)) => return error(e, right.1),
        };
        let comparison = match compare_pictures(Arc::new(lp), Arc::new(rp), &self.options, ctl) {
            Ok(c) => c,
            Err(e) => return error(e, left.1),
        };
        // The bytes differ (checked above), so an all-same comparison is `rules_same`.
        let verdict = PictureVerdict::of(comparison.stats(), false, || Ok::<_, Cancelled>(false))?;
        Ok(Answer::Result(match verdict.kind {
            hikasa_core::ContentKind::BinarySame => ContentResult::BinarySame,
            hikasa_core::ContentKind::RulesSame => ContentResult::RulesSame,
            hikasa_core::ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
            _ => ContentResult::ImportantDifferences,
        }))
    }
}

fn outcome(result: ContentResult, warnings: Vec<Warning>) -> ContentOutcome {
    ContentOutcome {
        method: ContentMethod::Rules,
        result,
        warnings,
    }
}

/// The byte comparison of two buffers already read.
fn by_bytes(left: &[u8], right: &[u8]) -> ContentResult {
    if left == right {
        ContentResult::BinarySame
    } else {
        ContentResult::BinaryDifferent
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
        match self.compare_bytes((&lb, names.0), (&rb, names.1), ctl)? {
            Answer::Result(result) => Ok(outcome(result, Vec::new())),
            Answer::Fallback(message) => Ok(outcome(
                by_bytes(&lb, &rb),
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
}

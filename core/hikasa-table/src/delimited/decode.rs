//! Decoding of text tables (contract §2.2 rule 1): BOM, the format's encoding, UTF-8, chardetng.

use encoding_rs::Encoding;

/// Decoded text.
#[derive(Debug)]
pub(crate) struct Decoded {
    /// The text (BOM removed).
    pub text: String,
    /// Encoding name.
    pub encoding: &'static str,
    /// Replacement characters introduced by malformed input (0 when the input was well-formed).
    pub malformed: u64,
}

/// Decodes `bytes` (consumed, so valid UTF-8 is never copied). `label`: an explicit encoding.
pub(crate) fn decode(bytes: Vec<u8>, label: Option<&str>) -> Result<Decoded, String> {
    if let Some((enc, bom)) = Encoding::for_bom(&bytes) {
        return Ok(with(enc, &bytes[bom..]));
    }
    if let Some(label) = label {
        let enc = Encoding::for_label(label.trim().as_bytes())
            .ok_or_else(|| format!("unknown encoding `{label}`"))?;
        if enc == encoding_rs::UTF_8 {
            return Ok(utf8(bytes));
        }
        return Ok(with(enc, &bytes));
    }
    if std::str::from_utf8(&bytes).is_ok() {
        return Ok(utf8(bytes));
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(&bytes, true);
    let enc = detector.guess(None, chardetng::Utf8Detection::Deny);
    Ok(with(enc, &bytes))
}

fn utf8(bytes: Vec<u8>) -> Decoded {
    match String::from_utf8(bytes) {
        Ok(text) => Decoded {
            text,
            encoding: encoding_rs::UTF_8.name(),
            malformed: 0,
        },
        Err(e) => with(encoding_rs::UTF_8, e.as_bytes()),
    }
}

fn with(enc: &'static Encoding, bytes: &[u8]) -> Decoded {
    let (text, had_errors) = enc.decode_without_bom_handling(bytes);
    let malformed = if had_errors {
        text.chars().filter(|c| *c == '\u{FFFD}').count() as u64
    } else {
        0
    };
    Decoded {
        text: text.into_owned(),
        encoding: enc.name(),
        malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boms_labels_and_detection() {
        let d = decode(b"\xEF\xBB\xBFa,b".to_vec(), None).unwrap();
        assert_eq!((d.text.as_str(), d.encoding), ("a,b", "UTF-8"));
        let d = decode(vec![0xFF, 0xFE, b'a', 0, b',', 0, b'b', 0], None).unwrap();
        assert_eq!((d.text.as_str(), d.encoding), ("a,b", "UTF-16LE"));
        let d = decode(b"caf\xE9".to_vec(), Some("windows-1252")).unwrap();
        assert_eq!(d.text, "café");
        let d = decode(b"caf\xE9,na\xEFve,\xE0 la carte".to_vec(), None).unwrap();
        assert_eq!(d.malformed, 0);
        assert!(d.text.contains("café"), "{}", d.text);
        assert!(decode(b"x".to_vec(), Some("no-such")).is_err());
        let d = decode(b"a\xFFb".to_vec(), Some("utf-8")).unwrap();
        assert_eq!((d.text.as_str(), d.malformed), ("a\u{FFFD}b", 1));
    }
}

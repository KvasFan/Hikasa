//! Links from a folder report to its pair reports (M2 W7, D-200): the [`PairLinks`] a writer answers, and [`href`],
//! the one way a link is spelled. The renderer neither names nor writes files: the plan of the writer
//! (`hikasa_session::api::report_write`) answers which row links where.

use std::fmt::Write as _;

use hikasa_folder::NodeId;

/// What a row of a folder report links to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairLink {
    /// The pair report, relative to the folder report (an [`href`]).
    Href(String),
    /// No pair report: skipped for this reason (`too_large`, `binary`).
    Skipped(String),
    /// No pair report: its comparison or rendering failed with this message.
    Failed(String),
}

impl PairLink {
    /// `written`, `skipped`, `failed` (XML `file-report/@status`).
    pub const fn status(&self) -> &'static str {
        match self {
            PairLink::Href(_) => "written",
            PairLink::Skipped(_) => "skipped",
            PairLink::Failed(_) => "failed",
        }
    }
}

/// The links of a folder report's rows, answered by the writer's plan.
pub trait PairLinks {
    /// The link of the row `node`; `None` for a row that gets no pair report.
    fn link(&self, node: NodeId) -> Option<PairLink>;
}

/// True for the RFC 3986 unreserved bytes, which an href keeps.
const fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// A relative link from its segments (normative, §1.7.8): each segment's bytes, the RFC 3986 unreserved bytes
/// (`A–Z a–z 0–9 - . _ ~`) kept and every other byte `%XX` (upper-case hex), the segments joined by `/`. So a link is
/// never absolute, never `file:`, never holds `\`, `\\?\` or a drive letter, and a name that is not UTF-8 still links.
/// A segment `..` stays `..` (the back link of a pair report); the caller passes final names only.
pub fn href<S: AsRef<[u8]>>(segments: &[S]) -> String {
    let mut out = String::new();
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        for &b in segment.as_ref() {
            if unreserved(b) {
                out.push(char::from(b));
            } else {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// The bytes of a link's segments back: `%XX` decoded, split at `/` (the inverse of [`href`], for tests and for a
/// check of a written report's links). `None` for a malformed escape.
pub fn href_segments(link: &str) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    for segment in link.split('/') {
        let bytes = segment.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                let hex = segment.get(i + 1..i + 3)?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            } else {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
        out.push(decoded);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn href_keeps_unreserved_bytes_only() {
        assert_eq!(href(&["a_files", "001-x.c.html"]), "a_files/001-x.c.html");
        assert_eq!(href(&[b"a b#%?&+".as_slice()]), "a%20b%23%25%3F%26%2B");
        assert_eq!(href(&["\u{e9}"]), "%C3%A9");
        assert_eq!(href(&[b"\xff\xfe".as_slice()]), "%FF%FE");
        assert_eq!(href(&["..", "r.html"]), "../r.html");
        assert_eq!(href(&["C:\\x"]), "C%3A%5Cx");
        assert_eq!(href::<&str>(&[]), "");
        assert_eq!(
            href_segments("a%20b/%FF").unwrap(),
            [b"a b".to_vec(), vec![0xff]]
        );
        assert!(href_segments("%G1").is_none());
        assert!(href_segments("%4").is_none());
    }
}

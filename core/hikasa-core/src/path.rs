//! [`RelPath`]: a normalised path relative to a base folder, on any back-end.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Classify, ErrorKind};

/// A path relative to a base folder: components joined by `'/'`, none empty, `"."` or `".."`, none containing
/// NUL; the empty path is the base folder itself (INV-RELPATH-1).
///
/// `'\'` is an ordinary character here (it is legal in Unix file names); only `'/'` separates components.
/// The derived order is the byte order of the `/`-joined text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

/// RelPath construction errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid relative path {input:?}: {reason}")]
pub struct RelPathError {
    /// The rejected text (a whole path for [`RelPath::parse`], one name for [`RelPath::join`]).
    pub input: String,
    /// Stable reason ("empty component", "'.' or '..' component", "contains NUL", "contains '/'").
    pub reason: &'static str,
}

impl Classify for RelPathError {
    /// Always `invalid_input`.
    fn kind(&self) -> ErrorKind {
        ErrorKind::InvalidInput
    }
}

/// Validates one component; `Err(reason)` on failure.
fn check_component(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        Err("empty component")
    } else if name == "." || name == ".." {
        Err("'.' or '..' component")
    } else if name.contains('\0') {
        Err("contains NUL")
    } else {
        Ok(())
    }
}

impl RelPath {
    /// The base folder (empty path).
    pub const fn root() -> RelPath {
        RelPath(String::new())
    }

    /// Parses a `'/'`-separated relative path (`""` is the root). Leading, trailing or doubled `'/'`, `"."`,
    /// `".."` and NUL are rejected.
    pub fn parse(s: &str) -> Result<RelPath, RelPathError> {
        if s.is_empty() {
            return Ok(RelPath::root());
        }
        for component in s.split('/') {
            check_component(component).map_err(|reason| RelPathError {
                input: s.to_owned(),
                reason,
            })?;
        }
        Ok(RelPath(s.to_owned()))
    }

    /// Appends one name (must not contain `'/'` or NUL and must not be empty, `"."` or `".."`).
    pub fn join(&self, name: &str) -> Result<RelPath, RelPathError> {
        if name.contains('/') {
            return Err(RelPathError {
                input: name.to_owned(),
                reason: "contains '/'",
            });
        }
        check_component(name).map_err(|reason| RelPathError {
            input: name.to_owned(),
            reason,
        })?;
        if self.0.is_empty() {
            return Ok(RelPath(name.to_owned()));
        }
        let mut joined = String::with_capacity(self.0.len() + 1 + name.len());
        joined.push_str(&self.0);
        joined.push('/');
        joined.push_str(name);
        Ok(RelPath(joined))
    }

    /// Components from the base downwards (none for the root).
    pub fn components(&self) -> impl Iterator<Item = &str> + '_ {
        let text = self.0.as_str();
        (!text.is_empty())
            .then(|| text.split('/'))
            .into_iter()
            .flatten()
    }

    /// Last component (`None` for the root).
    pub fn name(&self) -> Option<&str> {
        if self.0.is_empty() {
            return None;
        }
        self.0.rsplit('/').next()
    }

    /// Parent (`None` for the root; the root for a one-component path).
    pub fn parent(&self) -> Option<RelPath> {
        if self.0.is_empty() {
            return None;
        }
        Some(RelPath(self.parent_str().to_owned()))
    }

    /// Number of components (root = 0).
    pub fn depth(&self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            self.0.bytes().filter(|&b| b == b'/').count() + 1
        }
    }

    /// The `'/'`-joined string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True for the base folder.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The parent's text without allocating (`""` for the root and for one-component paths).
    pub(crate) fn parent_str(&self) -> &str {
        match self.0.rfind('/') {
            Some(at) => self.0.get(..at).unwrap_or(""),
            None => "",
        }
    }
}

impl fmt::Display for RelPath {
    /// Writes the `'/'`-joined text (the root is the empty string).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RelPath {
    type Error = RelPathError;

    fn try_from(value: String) -> Result<RelPath, RelPathError> {
        if value.is_empty() {
            return Ok(RelPath::root());
        }
        for component in value.split('/') {
            check_component(component).map_err(|reason| RelPathError {
                input: value.clone(),
                reason,
            })?;
        }
        Ok(RelPath(value))
    }
}

impl From<RelPath> for String {
    fn from(value: RelPath) -> String {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_accessors() {
        let p = RelPath::parse("a/b/c.txt").unwrap();
        assert_eq!(p.components().collect::<Vec<_>>(), ["a", "b", "c.txt"]);
        assert_eq!(p.name(), Some("c.txt"));
        assert_eq!(p.parent().unwrap().as_str(), "a/b");
        assert_eq!(p.depth(), 3);
        assert!(!p.is_root());
        let root = RelPath::root();
        assert!(root.is_root());
        assert_eq!(root.depth(), 0);
        assert_eq!(root.components().count(), 0);
        assert_eq!(root.name(), None);
        assert_eq!(root.parent(), None);
        assert_eq!(RelPath::parse("x").unwrap().parent(), Some(RelPath::root()));
        assert_eq!(RelPath::parse("").unwrap(), RelPath::root());
    }

    #[test]
    fn invalid_paths() {
        for bad in ["/a", "a/", "a//b", "./a", "a/../b", "a/.", "..", "a\0b"] {
            assert!(RelPath::parse(bad).is_err(), "{bad:?}");
        }
        assert!(
            RelPath::parse("a\\b").is_ok(),
            "backslash is an ordinary character"
        );
        assert!(RelPath::parse("with space/ü").is_ok());
    }

    #[test]
    fn join_validates() {
        let p = RelPath::root().join("a").unwrap().join("b").unwrap();
        assert_eq!(p.as_str(), "a/b");
        for bad in ["", ".", "..", "x/y", "nul\0"] {
            assert!(p.join(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn errors_are_invalid_input() {
        let err = RelPath::parse("a//b").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            err.to_string(),
            "invalid relative path \"a//b\": empty component"
        );
        let report = crate::error::ErrorReport::capture(&err, None);
        assert_eq!(report.kind, ErrorKind::InvalidInput);
        assert_eq!(
            RelPath::root().join("x/y").unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn serde_round_trip() {
        let p = RelPath::parse("dir/file").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"dir/file\"");
        assert_eq!(serde_json::from_str::<RelPath>(&json).unwrap(), p);
        assert!(serde_json::from_str::<RelPath>("\"a//b\"").is_err());
    }
}

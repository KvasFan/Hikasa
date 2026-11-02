//! Rename masks (FC-074; D-180): pure functions, tables of §1.6.13. The plan of a rename is `build.rs`'s.

use std::ops::Range;

use hikasa_formats::{Pattern, RegexLimits};

use crate::error::FolderError;

/// A name split by D-07: `stem`, `ext`. `.bashrc` and `a.` have no extension.
fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    }
}

/// `expand(part, mask)`: position `i = 0`; `?` copies `part[i]` if there is one and steps; `*` copies the rest of `part`;
/// any other character is written and steps. Characters are Unicode scalar values.
fn expand(part: &str, mask: &str) -> String {
    let chars: Vec<char> = part.chars().collect();
    let mut out = String::with_capacity(part.len() + mask.len());
    let mut i = 0;
    for m in mask.chars() {
        match m {
            '?' => {
                out.extend(chars.get(i));
                i += 1;
            }
            '*' => {
                out.extend(chars.iter().skip(i));
                i = chars.len();
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    out
}

/// The DOS rename of `name` by `mask` (normative table of §1.6.13). A mask with a `.` is split at its last `.`, and the
/// stem and the extension are expanded apart; a mask without one is expanded over the whole name.
pub fn dos_rename(name: &str, mask: &str) -> String {
    let Some((mask_stem, mask_ext)) = mask.rsplit_once('.') else {
        return expand(name, mask);
    };
    let (stem, ext) = split_name(name);
    let mut out = expand(stem, mask_stem);
    let ext = expand(ext.unwrap_or(""), mask_ext);
    if !ext.is_empty() {
        out.push('.');
        out.push_str(&ext);
    }
    out
}

/// The mask of one part of `parts`: per position that character when all agree, else `?`; `*` after the common length
/// when lengths differ.
fn part_mask(parts: &[&str]) -> String {
    let chars: Vec<Vec<char>> = parts.iter().map(|part| part.chars().collect()).collect();
    let shortest = chars.iter().map(Vec::len).min().unwrap_or(0);
    let longest = chars.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = String::with_capacity(longest);
    for i in 0..shortest {
        let first = chars.first().and_then(|name| name.get(i));
        if chars.iter().all(|name| name.get(i) == first) {
            out.extend(first);
        } else {
            out.push('?');
        }
    }
    if longest != shortest {
        out.push('*');
    }
    out
}

/// FC-074's *computed base mask*: per part, `?` where the names differ, `*` where their lengths differ. Stems and
/// extensions apart; extensions are joined with `.` when any name has one.
pub fn base_mask(names: &[&str]) -> String {
    let splits: Vec<(&str, Option<&str>)> = names.iter().map(|name| split_name(name)).collect();
    let stems: Vec<&str> = splits.iter().map(|(stem, _)| *stem).collect();
    let mut out = part_mask(&stems);
    if splits.iter().any(|(_, ext)| ext.is_some()) {
        let exts: Vec<&str> = splits.iter().map(|(_, ext)| ext.unwrap_or("")).collect();
        out.push('.');
        out.push_str(&part_mask(&exts));
    }
    out
}

/// One piece of a replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Text as written.
    Text(String),
    /// A group by index.
    Group(usize),
}

/// A compiled regular expression rename: `old` over the whole name, `new` expanded in-house, the same over both engines.
#[derive(Debug, Clone)]
pub struct RegexRename {
    pattern: Pattern,
    pieces: Vec<Piece>,
}

/// The error of a bad replacement.
fn bad_new(new: &str, why: &str) -> FolderError {
    FolderError::InvalidRules(format!("rename `{new}`: {why}"))
}

/// Parses `new`: `$1`, `${1}`, `${name}`, `$$`; anything else after a `$`, or a group that does not exist, is an error.
fn parse_new(new: &str, names: &[Option<String>]) -> Result<Vec<Piece>, FolderError> {
    let group = |index: usize| {
        if index < names.len() {
            Ok(Piece::Group(index))
        } else {
            Err(bad_new(new, &format!("there is no group {index}")))
        }
    };
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut chars = new.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            text.push(c);
            continue;
        }
        let piece = match chars.peek().copied() {
            Some('$') => {
                chars.next();
                text.push('$');
                continue;
            }
            Some('{') => {
                chars.next();
                let mut word = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => word.push(c),
                        None => return Err(bad_new(new, "a `${` is not closed")),
                    }
                }
                if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                    group(
                        word.parse()
                            .map_err(|_| bad_new(new, "a group number is too large"))?,
                    )?
                } else {
                    let index = names.iter().position(|name| name.as_deref() == Some(&word));
                    let index = index
                        .ok_or_else(|| bad_new(new, &format!("there is no group `{word}`")))?;
                    Piece::Group(index)
                }
            }
            Some(digit) if digit.is_ascii_digit() => {
                let mut number = String::new();
                while let Some(digit) = chars.peek().copied().filter(char::is_ascii_digit) {
                    number.push(digit);
                    chars.next();
                }
                group(
                    number
                        .parse()
                        .map_err(|_| bad_new(new, "a group number is too large"))?,
                )?
            }
            _ => return Err(bad_new(new, "a `$` needs a group or a second `$`")),
        };
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut text)));
        }
        pieces.push(piece);
    }
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

impl RegexRename {
    /// Compiles `old`, which matches the **whole** name: `regex` first, `fancy-regex` only where backreferences or
    /// look-around ask for it, with the limits of user patterns (T9). `(?i)` is written in the pattern.
    pub fn new(old: &str, new: &str) -> Result<RegexRename, FolderError> {
        let pattern = Pattern::compile(&format!("^(?:{old})$"), false, &RegexLimits::default())
            .map_err(|e| FolderError::InvalidRules(e.to_string()))?;
        let pieces = parse_new(new, &pattern.group_names())?;
        Ok(RegexRename { pattern, pieces })
    }

    /// The new name of `name`, `None` without a match. A backtrack limit at run time is an error, of that item alone.
    pub fn apply(&self, name: &str) -> Result<Option<String>, FolderError> {
        let found = self
            .pattern
            .captures_at(name, 0)
            .map_err(|e| FolderError::InvalidRules(e.to_string()))?;
        let Some(groups) = found else {
            return Ok(None);
        };
        let of = |index: usize| -> &str {
            let range: Option<&Range<usize>> = groups.get(index).and_then(Option::as_ref);
            range
                .and_then(|range| name.get(range.clone()))
                .unwrap_or("")
        };
        let mut out = String::with_capacity(name.len());
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Group(index) => out.push_str(of(*index)),
            }
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollar_words() {
        let names = [None, Some("n".to_owned())];
        assert_eq!(
            parse_new("a$1b$$", &names).unwrap(),
            [
                Piece::Text("a".into()),
                Piece::Group(1),
                Piece::Text("b$".into())
            ]
        );
        assert_eq!(parse_new("${n}", &names).unwrap(), [Piece::Group(1)]);
        assert_eq!(parse_new("${1}", &names).unwrap(), [Piece::Group(1)]);
        for bad in ["$", "$a", "${", "${x}", "$2"] {
            assert!(parse_new(bad, &names).is_err(), "{bad}");
        }
    }
}

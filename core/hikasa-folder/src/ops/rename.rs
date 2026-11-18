//! Rename masks (FC-074; D-180): pure functions, tables of §1.6.13. The plan of a rename is `build.rs`'s.

use hikasa_formats::{Pattern, RegexLimits, Template};

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

/// A compiled regular expression rename: `old` over the whole name, `new` expanded by [`Template`] (D-180, the
/// expansion text replace uses too), the same over both engines.
#[derive(Debug, Clone)]
pub struct RegexRename {
    pattern: Pattern,
    template: Template,
}

impl RegexRename {
    /// Compiles `old`, which matches the **whole** name: `regex` first, `fancy-regex` only where backreferences or
    /// look-around ask for it, with the limits of user patterns (T9). `(?i)` is written in the pattern.
    pub fn new(old: &str, new: &str) -> Result<RegexRename, FolderError> {
        let pattern = Pattern::compile(&format!("^(?:{old})$"), false, &RegexLimits::default())
            .map_err(|e| FolderError::InvalidRules(e.to_string()))?;
        let template = Template::parse(new, &pattern.group_names())
            .map_err(|e| FolderError::InvalidRules(format!("rename `{new}`: {}", e.message)))?;
        Ok(RegexRename { pattern, template })
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
        let out = self.template.expand(name, &groups);
        Ok(Some(out))
    }
}

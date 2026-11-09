//! Replacement templates (D-180, moved here by M2 W10c, §1.10.18): the `$` expansion shared by the folder
//! rename (FC-074) and the text replace (TC-064), so both read `$1`, `${1}`, `${name}` and `$$` by one rule (T11).
//!
//! | Written | Meaning |
//! |---|---|
//! | `$1`, `$12` | the group of that number (the longest run of digits) |
//! | `${1}` | the group of that number |
//! | `${name}` | the named group |
//! | `$$` | a `$` |
//!
//! Any other `$`, an unclosed `${`, a group the pattern does not have: [`TemplateError`] at [`Template::parse`]. A
//! group that took part in no match expands to nothing. The expansion is the same over both engines of
//! [`crate::Pattern`] (the crates' own expansions differ on `$1a`).

use std::ops::Range;

/// One piece of a template.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Text as written.
    Text(String),
    /// A group by index (0 = the whole match).
    Group(usize),
}

/// A replacement template that does not parse (the text and why).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{template}`: {message}")]
pub struct TemplateError {
    /// The template as written.
    pub template: String,
    /// Why it is refused (`there is no group 2`, `a `$` needs a group or a second `$``, …).
    pub message: String,
}

/// A parsed replacement template (D-180).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pieces: Vec<Piece>,
}

impl Template {
    /// Parses `text` against the group names of a pattern by index, the whole match first
    /// ([`crate::Pattern::group_names`]).
    pub fn parse(text: &str, names: &[Option<String>]) -> Result<Template, TemplateError> {
        let bad = |why: String| TemplateError {
            template: text.to_owned(),
            message: why,
        };
        let group = |index: usize| {
            if index < names.len() {
                Ok(Piece::Group(index))
            } else {
                Err(bad(format!("there is no group {index}")))
            }
        };
        let number = |digits: &str| {
            digits
                .parse::<usize>()
                .map_err(|_| bad("a group number is too large".to_owned()))
        };
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '$' {
                literal.push(c);
                continue;
            }
            let piece = match chars.peek().copied() {
                Some('$') => {
                    chars.next();
                    literal.push('$');
                    continue;
                }
                Some('{') => {
                    chars.next();
                    let mut word = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) => word.push(c),
                            None => return Err(bad("a `${` is not closed".to_owned())),
                        }
                    }
                    if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                        group(number(&word)?)?
                    } else {
                        let index = names
                            .iter()
                            .position(|name| name.as_deref() == Some(word.as_str()));
                        Piece::Group(
                            index.ok_or_else(|| bad(format!("there is no group `{word}`")))?,
                        )
                    }
                }
                Some(digit) if digit.is_ascii_digit() => {
                    let mut digits = String::new();
                    while let Some(digit) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(digit);
                        chars.next();
                    }
                    group(number(&digits)?)?
                }
                _ => return Err(bad("a `$` needs a group or a second `$`".to_owned())),
            };
            if !literal.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut literal)));
            }
            pieces.push(piece);
        }
        if !literal.is_empty() {
            pieces.push(Piece::Text(literal));
        }
        Ok(Template { pieces })
    }

    /// The template's text when it names no group (its expansion never depends on the match).
    pub fn literal(&self) -> Option<String> {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Group(_) => return None,
            }
        }
        Some(out)
    }

    /// Appends the expansion for one match to `out`: `groups[i]` is the byte range of group `i` in `haystack`
    /// (`None` when it took part in no match; [`crate::Pattern::captures_at`]).
    pub fn expand_into(&self, haystack: &str, groups: &[Option<Range<usize>>], out: &mut String) {
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                Piece::Group(index) => {
                    let text = groups
                        .get(*index)
                        .and_then(Option::as_ref)
                        .and_then(|range| haystack.get(range.clone()))
                        .unwrap_or("");
                    out.push_str(text);
                }
            }
        }
    }

    /// The expansion for one match ([`Template::expand_into`]).
    pub fn expand(&self, haystack: &str, groups: &[Option<Range<usize>>]) -> String {
        let mut out = String::with_capacity(haystack.len().min(256));
        self.expand_into(haystack, groups, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollar_words() {
        let names = [None, Some("n".to_owned())];
        let t = Template::parse("a$1b$$", &names).unwrap();
        assert_eq!(
            t.pieces,
            [
                Piece::Text("a".into()),
                Piece::Group(1),
                Piece::Text("b$".into())
            ]
        );
        assert_eq!(
            Template::parse("${n}", &names).unwrap().pieces,
            [Piece::Group(1)]
        );
        assert_eq!(
            Template::parse("${1}", &names).unwrap().pieces,
            [Piece::Group(1)]
        );
        for bad in ["$", "$a", "${", "${x}", "$2", "$99999999999999999999999"] {
            assert!(Template::parse(bad, &names).is_err(), "{bad}");
        }
        assert_eq!(
            Template::parse("$2", &names).unwrap_err().to_string(),
            "`$2`: there is no group 2"
        );
        let groups = [Some(0..3), Some(1..2)];
        assert_eq!(t.expand("xyz", &groups), "ayb$");
        assert_eq!(t.expand("xyz", &[Some(0..3), None]), "ab$");
        assert_eq!(t.literal(), None);
        assert_eq!(
            Template::parse("a$$b", &names)
                .unwrap()
                .literal()
                .as_deref(),
            Some("a$b")
        );
    }
}

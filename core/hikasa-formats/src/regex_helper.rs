//! The regex helper's menu (FF-028, D-320): the pieces of a regular expression a ▸ button beside every regex field
//! offers. Each item replaces the field's selection with [`HelperItem::insert`] and puts the caret
//! [`HelperItem::caret`] bytes into it (inside the brackets of *Any of* and *None of*). The table is data: the GUI
//! draws the menu from it and the help topic `regex-helper` lists it.

/// One item of the regex helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelperItem {
    /// The menu caption.
    pub caption: &'static str,
    /// The text inserted.
    pub insert: &'static str,
    /// Where the caret goes, in bytes from the start of `insert` (`insert.len()`: after it).
    pub caret: usize,
}

/// An item whose caret goes after the inserted text.
const fn after(caption: &'static str, insert: &'static str) -> HelperItem {
    HelperItem {
        caption,
        insert,
        caret: insert.len(),
    }
}

/// The items, in menu order (D-320).
pub const ITEMS: &[HelperItem] = &[
    after("Tab", "\\t"),
    after("Any character", "."),
    after("Word character", "\\w"),
    after("Not a word character", "\\W"),
    after("Digit", "\\d"),
    after("Not a digit", "\\D"),
    after("White space", "\\s"),
    HelperItem {
        caption: "Any of",
        insert: "[ ]",
        caret: 1,
    },
    HelperItem {
        caption: "None of",
        insert: "[^ ]",
        caret: 2,
    },
    after("Line start", "^"),
    after("Line end", "$"),
    after("Form feed", "\\x0C"),
    after("Or", "|"),
    after("Zero or more", "*"),
    after("One or more", "+"),
    after("Optional", "?"),
];

/// `text` with the byte range `selection` (clamped to `text`, snapped to character boundaries) replaced by `item`'s
/// text, and the caret's byte offset after the insertion.
pub fn apply(text: &str, selection: std::ops::Range<usize>, item: &HelperItem) -> (String, usize) {
    let snap = |mut at: usize| {
        at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let (a, b) = (snap(selection.start), snap(selection.end));
    let (start, end) = (a.min(b), a.max(b));
    let mut out = String::with_capacity(text.len() + item.insert.len());
    out.push_str(&text[..start]);
    out.push_str(item.insert);
    out.push_str(&text[end..]);
    (out, start + item.caret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pattern, RegexLimits};

    #[test]
    fn every_item_compiles_alone_and_in_context() {
        let limits = RegexLimits::default();
        assert_eq!(ITEMS.len(), 16);
        for item in ITEMS {
            assert!(item.caret <= item.insert.len(), "{}", item.caption);
            // Brackets get a character, quantifiers something to repeat, as a user would complete them.
            let (filled, _) = apply("a", 1..1, item);
            let source = match item.insert {
                "*" | "+" | "?" => filled,
                "[ ]" | "[^ ]" => item.insert.replace(' ', "x"),
                _ => item.insert.to_owned(),
            };
            Pattern::compile(&source, false, &limits)
                .unwrap_or_else(|e| panic!("{}: {source:?}: {e}", item.caption));
        }
    }

    #[test]
    fn the_caret_goes_inside_the_brackets() {
        let any = ITEMS.iter().find(|i| i.caption == "Any of").unwrap();
        let (text, caret) = apply("ab", 1..1, any);
        assert_eq!((text.as_str(), caret), ("a[ ]b", 2));
        let none = ITEMS.iter().find(|i| i.caption == "None of").unwrap();
        let (text, caret) = apply("x", 0..1, none);
        assert_eq!((text.as_str(), caret), ("[^ ]", 2));
        let tab = &ITEMS[0];
        let (text, caret) = apply("é", 1..9, tab);
        assert_eq!(
            (text.as_str(), caret),
            ("\\t", 2),
            "a selection inside a character snaps back"
        );
    }
}

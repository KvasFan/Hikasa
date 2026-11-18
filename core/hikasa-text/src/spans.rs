//! Syntax colour spans of one document (a Text Edit view, M2 W11b review fix; contract §1.11.35 W11b review note 14):
//! [`DocumentSpans`] classifies a line of a single document with its format, as [`crate::TextComparison::spans`] does
//! for a side of a comparison. A grammar with multi-line items needs the lines' tokenizer start states
//! ([`DocumentSpans::start_states`], one sequential pass a view runs on a worker); without them every line starts in
//! the normal state.

use std::sync::Arc;

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_formats::{CompiledFormat, ElementId, LineState, Tokenizer};

use crate::classify::{CharClass, ClassifiedSpan, RuleSet};
use crate::compare::SPAN_LIMIT_BYTES;
use crate::document::{LineIdx, TextDocument};
use crate::rules::TextRules;

/// The lines a start-state pass checks the cancellation between.
const CHUNK_LINES: u32 = 4096;

/// The syntax spans of documents of one format (the default rules: the spans' elements and classes, which the
/// colours are; no unimportant pattern).
#[derive(Debug)]
pub struct DocumentSpans {
    format: Arc<CompiledFormat>,
    set: RuleSet,
}

impl DocumentSpans {
    /// The spans of documents of `format`.
    pub fn new(format: Arc<CompiledFormat>) -> DocumentSpans {
        let set = RuleSet::new(&TextRules::default(), &format, Arc::from(Vec::new()));
        DocumentSpans { format, set }
    }

    /// The format.
    pub fn format(&self) -> &Arc<CompiledFormat> {
        &self.format
    }

    /// True when the format's grammar has multi-line items: the lines need their start states
    /// ([`DocumentSpans::start_states`]) to be classified as the whole document says.
    pub fn needs_states(&self) -> bool {
        self.format.grammar().has_multiline()
    }

    /// The tokenizer start state of every line of `doc` (one sequential pass; empty when the grammar has no
    /// multi-line item). Cancellable between chunks of lines and inside a long line.
    pub fn start_states(
        &self,
        doc: &TextDocument,
        cancel: &CancelToken,
    ) -> Result<Vec<LineState>, Cancelled> {
        if !self.needs_states() {
            return Ok(Vec::new());
        }
        let mut tokenizer = Tokenizer::new(self.format.grammar());
        let mut tokens = Vec::new();
        let mut states = Vec::with_capacity(doc.line_count() as usize);
        let mut state = LineState::Normal;
        let mut cursor = doc.cursor();
        for line in 0..doc.line_count() {
            if line % CHUNK_LINES == 0 {
                cancel.check()?;
            }
            states.push(state);
            let text = cursor.line(line);
            state = tokenizer
                .tokenize_line_cancellable(text, line, state, &mut tokens, cancel)?
                .end_state;
        }
        Ok(states)
    }

    /// The classified spans of `line` of `doc` from its start state `state` (`LineState::Normal` for a grammar without
    /// multi-line items). Bounded on very long lines as [`crate::TextComparison::spans`] is: the first
    /// [`SPAN_LIMIT_BYTES`] are classified, the rest is one span of `Everything Else`.
    pub fn spans(
        &self,
        doc: &TextDocument,
        line: LineIdx,
        state: LineState,
    ) -> Vec<ClassifiedSpan> {
        if line.0 >= doc.line_count() {
            return Vec::new();
        }
        let text = doc.line(line);
        let mut cut = text.len().min(SPAN_LIMIT_BYTES);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        let head = &text[..cut];
        let mut tokens = Vec::new();
        Tokenizer::new(self.format.grammar()).tokenize_line(head, line.0, state, &mut tokens);
        let mut errors = 0;
        let mut out: Vec<ClassifiedSpan> = self
            .set
            .classify(head, &tokens, &CancelToken::never(), &mut errors)
            .into_iter()
            .map(|s| ClassifiedSpan {
                range: s.start..s.end,
                element: s.element,
                class: s.class,
                important: s.important,
            })
            .collect();
        if cut < text.len() {
            out.push(ClassifiedSpan {
                range: cut as u32..text.len() as u32,
                element: ElementId::EVERYTHING_ELSE,
                class: CharClass::Other,
                important: true,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompareInput, compare};
    use hikasa_core::{Ctl, Side};

    /// The spans of a document equal the spans of the same document as a comparison's side (the colours of a Text
    /// Edit view are those of a Text Compare pane).
    #[test]
    fn spans_equal_a_comparisons() {
        let format = hikasa_formats::FormatRegistry::builtin()
            .expect("registry")
            .compiled(&hikasa_formats::FormatId::new("c-cpp").expect("id"))
            .expect("compiled");
        let text = "int a; /* one\n two */ int b;\n\"str\" // c\n";
        let doc = Arc::new(TextDocument::from_text(text));
        let input = CompareInput {
            left: Arc::clone(&doc),
            right: Arc::clone(&doc),
            format: Arc::clone(&format),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: true,
            anchors: Vec::new(),
            right_format: None,
            ignored: Default::default(),
        };
        let cmp = compare(input, None, &Ctl::detached(), None).expect("cmp");
        let spans = DocumentSpans::new(format);
        assert!(spans.needs_states());
        let states = spans
            .start_states(&doc, &CancelToken::never())
            .expect("states");
        assert_eq!(states.len(), doc.line_count() as usize);
        for line in 0..doc.line_count() {
            let ours = spans.spans(&doc, LineIdx(line), states[line as usize]);
            let theirs = cmp.spans(Side::Left, LineIdx(line));
            let key = |v: &[ClassifiedSpan]| -> Vec<(std::ops::Range<u32>, ElementId, CharClass)> {
                v.iter()
                    .map(|s| (s.range.clone(), s.element, s.class))
                    .collect()
            };
            assert_eq!(key(&ours), key(&theirs), "line {line}");
        }
        // The second line starts inside the block comment.
        assert_ne!(states[1], LineState::Normal);
    }
}

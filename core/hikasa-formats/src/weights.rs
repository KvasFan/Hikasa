//! Line weights (TC-012, §2.10.7): `line_weight(line)` is the maximum priority over the weights whose
//! pattern is found anywhere in the raw line (substring with case per `match_case`, or regex search); 0 when
//! none matches.

use crate::error::FormatError;
use crate::model::LineWeight;
use crate::regex_engine::Pattern;
use crate::tokenize::scan::literal_ci;

#[derive(Debug, Clone)]
enum WeightPattern {
    Literal { text: Box<str>, ci: bool },
    Regex(Pattern),
}

#[derive(Debug, Clone)]
struct CompiledWeight {
    pattern: WeightPattern,
    priority: u8,
}

/// The compiled line weights of a format.
#[derive(Debug, Clone, Default)]
pub(crate) struct CompiledWeights {
    items: Vec<CompiledWeight>,
}

impl CompiledWeights {
    /// Compiles `weights`; `validate(k, w)` checks one weight and returns its compiled regex (`None` for a
    /// literal).
    pub(crate) fn compile(
        weights: &[LineWeight],
        mut validate: impl FnMut(usize, &LineWeight) -> Result<Option<Pattern>, FormatError>,
    ) -> Result<CompiledWeights, FormatError> {
        let mut items = Vec::with_capacity(weights.len());
        for (k, w) in weights.iter().enumerate() {
            let pattern = match validate(k, w)? {
                Some(re) => WeightPattern::Regex(re),
                None => WeightPattern::Literal {
                    text: w.pattern.as_str().into(),
                    ci: !w.match_case,
                },
            };
            items.push(CompiledWeight {
                pattern,
                priority: w.priority,
            });
        }
        // Highest priority first, so the search can stop at the first hit of the best possible priority.
        items.sort_by_key(|w| std::cmp::Reverse(w.priority));
        Ok(CompiledWeights { items })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Weight (0..=4) of a raw line and the number of fancy-regex runtime-limit hits: a weight regex whose
    /// search hits the backtracking limit counts as "not found" and is counted (§2.10.5, §5.5 rule 3).
    pub(crate) fn weight(&self, line: &str) -> (u8, u32) {
        let mut errors = 0u32;
        for w in &self.items {
            let found = match &w.pattern {
                WeightPattern::Literal { text, ci: false } => {
                    memchr::memmem::find(line.as_bytes(), text.as_bytes()).is_some()
                }
                WeightPattern::Literal { text, ci: true } => line
                    .char_indices()
                    .any(|(at, _)| literal_ci(text, line, at).is_some()),
                WeightPattern::Regex(re) => match re.find_at(line, 0) {
                    Ok(found) => found.is_some(),
                    Err(_) => {
                        errors = errors.saturating_add(1);
                        false
                    }
                },
            };
            if found {
                return (w.priority, errors);
            }
        }
        (0, errors)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::compile::CompiledFormat;
    use crate::model::{FileFormat, FormatId, FormatKind, LineWeight};
    use crate::regex_engine::RegexLimits;

    #[test]
    fn maximum_priority_of_matching_weights() {
        let mut f = FileFormat::new(FormatId::new("w").unwrap(), "W", FormatKind::Text);
        f.line_weights = vec![
            LineWeight {
                pattern: r"^\s*object".into(),
                regex: true,
                match_case: true,
                priority: 4,
            },
            LineWeight {
                pattern: "Class".into(),
                regex: false,
                match_case: false,
                priority: 2,
            },
            LineWeight {
                pattern: "end".into(),
                regex: false,
                match_case: true,
                priority: 1,
            },
        ];
        let c = CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap();
        assert!(c.has_line_weights());
        assert_eq!(c.line_weight("  object Form1: TForm1"), 4);
        assert_eq!(c.line_weight("x = class(TObject)"), 2);
        assert_eq!(c.line_weight("  end"), 1);
        assert_eq!(c.line_weight("  END"), 0);
        assert_eq!(c.line_weight("  CLASS end object"), 2);
        assert_eq!(c.line_weight(""), 0);
        assert_eq!(c.line_weight_with_errors("  CLASS end object"), (2, 0));
    }

    /// A weight regex that hits the backtracking limit counts as "not found" and is counted, never silently
    /// dropped (§5.5 rule 3; wave-2 review).
    #[test]
    fn runtime_limit_hits_are_counted() {
        let mut f = FileFormat::new(FormatId::new("w").unwrap(), "W", FormatKind::Text);
        f.line_weights = vec![
            LineWeight {
                pattern: r"(a*)*\1b".into(),
                regex: true,
                match_case: true,
                priority: 4,
            },
            LineWeight {
                pattern: "b".into(),
                regex: false,
                match_case: true,
                priority: 1,
            },
        ];
        let limits = RegexLimits {
            size_limit: 8 << 20,
            backtrack_limit: 1000,
        };
        let c = CompiledFormat::compile(Arc::new(f), &limits).unwrap();
        let line = format!("{} b", "a".repeat(30));
        assert_eq!(c.line_weight_with_errors(&line), (1, 1));
        assert_eq!(c.line_weight(&line), 1);
        assert_eq!(c.line_weight_with_errors("ab"), (4, 0));
    }
}

//! Help topics (docs/17 §4.3, DoD "every new command has a help topic"; M2 plan W0, Q13a): the Markdown files of
//! `help/` at the repository root, embedded at build time, with a registry, a resolver for `hikasa help <words>`
//! and a small Markdown reader shared by the CLI (`--json` links), the GUI help panel (F1) and
//! `cargo xtask help-check`.
//!
//! A topic is one file `help/<id>.md`. Its first line is `# <title>`. Links to other topics are written
//! `[label](topic:<id>)`; every link must resolve ([`problems`]). The reader knows headings (`#`, `##`, `###`),
//! paragraphs, list items (`- `), fenced code blocks and table lines (kept verbatim), and inline code and links.

/// One help topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpTopic {
    /// Stable id (`text-compare`, `view-folder-compare`, …): the file name without `.md`.
    pub id: &'static str,
    /// The Markdown text.
    pub source: &'static str,
}

macro_rules! topics {
    ($($id:literal),* $(,)?) => {
        /// Every topic, in contents order.
        const TOPICS: &[HelpTopic] = &[
            $(HelpTopic { id: $id, source: include_str!(concat!("../../../help/", $id, ".md")) },)*
        ];
    };
}

topics!(
    "index",
    "cli",
    "exit-codes",
    "json-output",
    "quick",
    "text-compare",
    "folder-compare",
    "format",
    "settings",
    "session",
    "workspace",
    "vfs",
    "archives",
    "gui",
    "completions",
    "version",
    "help",
    "main-window",
    "view-home",
    "dialog-session-settings",
    "dialog-save-session",
    "dialog-workspaces",
    "dialog-options",
    "wizard-export",
    "wizard-import",
    "wizard-restore",
    "view-folder-compare",
    "view-text-compare",
);

/// The topic shown when no topic is named (`hikasa help`, Help ▸ Contents).
pub const INDEX_TOPIC: &str = "index";

/// Every topic, in contents order.
pub fn topics() -> &'static [HelpTopic] {
    TOPICS
}

/// The topic `id`.
pub fn topic(id: &str) -> Option<&'static HelpTopic> {
    TOPICS.iter().find(|t| t.id == id)
}

/// A `hikasa help` request that names no topic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown help topic {word:?}{}", suggestion_text(.suggestions))]
pub struct UnknownTopic {
    /// The words as one id (`folder-bogus`).
    pub word: String,
    /// Topics with a similar id.
    pub suggestions: Vec<&'static str>,
}

fn suggestion_text(suggestions: &[&str]) -> String {
    if suggestions.is_empty() {
        "; `hikasa help --list` lists the topics".to_owned()
    } else {
        format!("; did you mean: {}?", suggestions.join(", "))
    }
}

/// The topic of `hikasa help <words…>`: no words is the index; the words joined with `-` name the topic
/// (`text compare` → `text-compare`, case-insensitive); when no topic has that id, the leading words are tried
/// (`format list` → `format`, `gui open` → `gui`).
pub fn resolve(words: &[String]) -> Result<&'static HelpTopic, UnknownTopic> {
    let words: Vec<String> = words
        .iter()
        .flat_map(|w| w.split_whitespace())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        return topic(INDEX_TOPIC).ok_or_else(|| UnknownTopic {
            word: INDEX_TOPIC.into(),
            suggestions: Vec::new(),
        });
    }
    for n in (1..=words.len()).rev() {
        if let Some(found) = topic(&words[..n].join("-")) {
            return Ok(found);
        }
    }
    let word = words.join("-");
    let suggestions = suggestions(&word);
    Err(UnknownTopic { word, suggestions })
}

/// Topics whose id contains `word` or is within two edits of it (at most five).
pub fn suggestions(word: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = TOPICS
        .iter()
        .map(|t| t.id)
        .filter(|id| (word.len() >= 3 && id.contains(word)) || edit_distance(id, word) <= 2)
        .collect();
    out.truncate(5);
    out
}

/// Levenshtein distance over chars.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            let next = (row[j] + 1).min(row[j + 1] + 1).min(prev + cost);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// Inline content of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// Inline code (`` `…` ``).
    Code(String),
    /// A link to another topic (`[label](topic:id)`).
    Link {
        /// Shown text.
        label: String,
        /// Target topic id.
        topic: String,
    },
}

/// One block of a topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A heading (`#` = 1, `##` = 2, `###` = 3).
    Heading {
        /// Level 1–3.
        level: u8,
        /// Content.
        text: Vec<Inline>,
    },
    /// A paragraph (consecutive lines joined with a space).
    Paragraph(Vec<Inline>),
    /// A list item (`- `; indented continuation lines belong to it).
    Item(Vec<Inline>),
    /// Verbatim lines: a fenced code block or a table.
    Verbatim(String),
}

/// The inline content of one line: `` `code` `` and `[label](topic:id)`; everything else is text. A link to
/// anything but a topic, or an unclosed construct, stays text.
pub fn parse_inline(line: &str) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut text = String::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if c == '`'
            && let Some(end) = rest[1..].find('`')
        {
            flush(&mut text, &mut out);
            out.push(Inline::Code(rest[1..1 + end].to_owned()));
            rest = &rest[end + 2..];
            continue;
        }
        if c == '['
            && let Some(close) = rest.find("](topic:")
            && let Some(end) = rest[close..].find(')')
        {
            let label = &rest[1..close];
            let target = &rest[close + 8..close + end];
            if !label.contains(['[', ']']) && !target.is_empty() && !target.contains(' ') {
                flush(&mut text, &mut out);
                out.push(Inline::Link {
                    label: label.to_owned(),
                    topic: target.to_owned(),
                });
                rest = &rest[close + end + 1..];
                continue;
            }
        }
        text.push(c);
        rest = &rest[c.len_utf8()..];
    }
    flush(&mut text, &mut out);
    out
}

fn flush(text: &mut String, out: &mut Vec<Inline>) {
    if !text.is_empty() {
        out.push(Inline::Text(std::mem::take(text)));
    }
}

/// Closes the open paragraph or list item (`(is_item, text)`).
fn close_block(open: &mut Option<(bool, String)>, blocks: &mut Vec<Block>) {
    if let Some((item, text)) = open.take() {
        let inline = parse_inline(&text);
        blocks.push(if item {
            Block::Item(inline)
        } else {
            Block::Paragraph(inline)
        });
    }
}

/// The blocks of a topic.
pub fn parse_blocks(source: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut lines = source.lines().peekable();
    let mut open: Option<(bool, String)> = None;
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("```") {
            close_block(&mut open, &mut blocks);
            let mut code = String::new();
            for inner in lines.by_ref() {
                if inner.trim_end().starts_with("```") {
                    break;
                }
                code.push_str(inner);
                code.push('\n');
            }
            blocks.push(Block::Verbatim(code));
        } else if trimmed.starts_with('|') {
            close_block(&mut open, &mut blocks);
            let mut table = format!("{trimmed}\n");
            while let Some(next) = lines.next_if(|l| l.trim_end().starts_with('|')) {
                table.push_str(next.trim_end());
                table.push('\n');
            }
            blocks.push(Block::Verbatim(table));
        } else if let Some((level, text)) = heading(trimmed) {
            close_block(&mut open, &mut blocks);
            blocks.push(Block::Heading {
                level,
                text: parse_inline(text),
            });
        } else if trimmed.is_empty() {
            close_block(&mut open, &mut blocks);
        } else if let Some(item) = trimmed.strip_prefix("- ") {
            close_block(&mut open, &mut blocks);
            open = Some((true, item.trim().to_owned()));
        } else if let Some((_, text)) = open.as_mut() {
            text.push(' ');
            text.push_str(trimmed.trim());
        } else {
            open = Some((false, trimmed.trim().to_owned()));
        }
    }
    close_block(&mut open, &mut blocks);
    blocks
}

/// `(level, text)` of a heading line.
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let text = line[hashes..].strip_prefix(' ')?;
    u8::try_from(hashes)
        .ok()
        .filter(|level| (1..=3).contains(level))
        .map(|level| (level, text.trim()))
}

/// Plain text of inline content (links show their label, code its text).
pub fn plain(inline: &[Inline]) -> String {
    inline
        .iter()
        .map(|i| match i {
            Inline::Text(t) | Inline::Code(t) => t.as_str(),
            Inline::Link { label, .. } => label.as_str(),
        })
        .collect()
}

/// Terminal text of inline content: links become their label, inline code keeps its backquotes.
fn terminal(inline: &[Inline]) -> String {
    inline
        .iter()
        .map(|i| match i {
            Inline::Text(t) => t.clone(),
            Inline::Code(t) => format!("`{t}`"),
            Inline::Link { label, .. } => label.clone(),
        })
        .collect()
}

impl HelpTopic {
    /// The title: the text of the first `#` heading (the id when there is none).
    pub fn title(&self) -> String {
        self.source
            .lines()
            .find_map(|l| heading(l.trim_end()).filter(|(level, _)| *level == 1))
            .map_or_else(
                || self.id.to_owned(),
                |(_, text)| plain(&parse_inline(text)),
            )
    }

    /// The blocks of the topic.
    pub fn blocks(&self) -> Vec<Block> {
        parse_blocks(self.source)
    }

    /// The topics this topic links to, in order of appearance, without repeats.
    pub fn links(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for line in self.source.lines() {
            for inline in parse_inline(line) {
                if let Inline::Link { topic, .. } = inline
                    && !out.contains(&topic)
                {
                    out.push(topic);
                }
            }
        }
        out
    }

    /// The topic as terminal text: the title underlined with `=`, blocks separated by a blank line (consecutive
    /// list items are not), list items with `- `, verbatim blocks indented by four spaces.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let mut after_item = false;
        for block in self.blocks() {
            let item = matches!(block, Block::Item(_));
            if !out.is_empty() && !(item && after_item) {
                out.push('\n');
            }
            after_item = item;
            match block {
                Block::Heading { level, text } => {
                    let line = terminal(&text);
                    out.push_str(&line);
                    out.push('\n');
                    if level == 1 {
                        out.push_str(&"=".repeat(line.chars().count()));
                        out.push('\n');
                    }
                }
                Block::Paragraph(text) => {
                    out.push_str(&terminal(&text));
                    out.push('\n');
                }
                Block::Item(text) => {
                    out.push_str("- ");
                    out.push_str(&terminal(&text));
                    out.push('\n');
                }
                Block::Verbatim(lines) => {
                    for line in lines.lines() {
                        out.push_str("    ");
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
        }
        out
    }
}

/// Every problem of the topic set: a topic whose first line is not a `# ` title, a link to a topic that does not
/// exist, and a topic that no other topic links to (the index excepted).
pub fn problems() -> Vec<String> {
    let mut problems = Vec::new();
    for t in TOPICS {
        if !t.source.lines().next().is_some_and(|l| l.starts_with("# ")) {
            problems.push(format!(
                "help/{}.md: the first line is not a `# ` title",
                t.id
            ));
        }
        for link in t.links() {
            if topic(&link).is_none() {
                problems.push(format!(
                    "help/{}.md: link to the unknown topic {link:?}",
                    t.id
                ));
            }
        }
        let linked = TOPICS
            .iter()
            .any(|other| other.id != t.id && other.links().iter().any(|l| l == t.id));
        if t.id != INDEX_TOPIC && !linked {
            problems.push(format!("help/{}.md: no other topic links to it", t.id));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_topic_set_is_consistent() {
        assert_eq!(problems(), Vec::<String>::new());
        assert!(topic(INDEX_TOPIC).is_some());
        for t in topics() {
            assert!(!t.title().is_empty(), "{}", t.id);
            assert!(t.render_text().starts_with(&t.title()), "{}", t.id);
        }
    }

    #[test]
    fn words_resolve_to_topics() {
        let id = |w: &[&str]| resolve(&words(w)).map(|t| t.id);
        assert_eq!(id(&[]), Ok(INDEX_TOPIC));
        assert_eq!(id(&["text", "compare"]), Ok("text-compare"));
        assert_eq!(id(&["Folder", "Compare"]), Ok("folder-compare"));
        assert_eq!(id(&["format", "list"]), Ok("format"));
        assert_eq!(id(&["text compare"]), Ok("text-compare"));
        let err = resolve(&words(&["fodler-compare"])).unwrap_err();
        assert_eq!(err.word, "fodler-compare");
        assert!(err.suggestions.contains(&"folder-compare"), "{err:?}");
        assert!(err.to_string().contains("did you mean"), "{err}");
        let none = resolve(&words(&["zzzzzzzz"])).unwrap_err();
        assert!(none.suggestions.is_empty() && none.to_string().contains("--list"));
    }

    #[test]
    fn inline_content() {
        assert_eq!(
            parse_inline("Run `hikasa quick` or see [exit codes](topic:exit-codes)."),
            vec![
                Inline::Text("Run ".into()),
                Inline::Code("hikasa quick".into()),
                Inline::Text(" or see ".into()),
                Inline::Link {
                    label: "exit codes".into(),
                    topic: "exit-codes".into()
                },
                Inline::Text(".".into()),
            ]
        );
        assert_eq!(
            parse_inline("a [web](https://x) `open"),
            vec![Inline::Text("a [web](https://x) `open".into())]
        );
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn blocks() {
        let source = "# T\n\nOne\ntwo.\n\n- a\n  b\n- c\n\n| x |\n|---|\n\n```\nq\n```\n## S\n";
        let blocks = parse_blocks(source);
        assert_eq!(blocks.len(), 7, "{blocks:?}");
        assert_eq!(
            blocks[1],
            Block::Paragraph(vec![Inline::Text("One two.".into())])
        );
        assert_eq!(blocks[2], Block::Item(vec![Inline::Text("a b".into())]));
        assert_eq!(blocks[4], Block::Verbatim("| x |\n|---|\n".into()));
        assert_eq!(blocks[5], Block::Verbatim("q\n".into()));
        assert!(matches!(blocks[6], Block::Heading { level: 2, .. }));
    }
}

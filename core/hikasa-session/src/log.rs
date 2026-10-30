//! The log book of a GUI process (M2 W4a, D-116; FC-100 panel, OPT-018): a pure, bounded list of entries that
//! every window shares and the folder views' Log panel shows.
//!
//! The filter of OPT-018 applies when an entry is pushed (errors are always kept); `display_up_to` is the
//! capacity (the oldest entries are dropped). With `save_to_file` set, the lines pushed since the last
//! [`LogBook::take_unsaved`] are handed to the adapter, which appends them with `Service::log_append` off the UI
//! thread. No clock is read here (T1): every entry carries the time its producer took from the injected clock.

use std::collections::VecDeque;

use hikasa_core::Warning;

use crate::settings::Options;

/// The kind of a log entry (OPT-018 classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LogKind {
    /// An operation summary (a finished comparison, a settings reload).
    Summary,
    /// An operation detail.
    Detail,
    /// A warning.
    Warning,
    /// An error (always kept).
    Error,
    /// A debug message (`hikasa::*` tracing events with `debug_messages`).
    Debug,
}

impl LogKind {
    /// The word of the saved line (`summary`, `detail`, `warning`, `error`, `debug`).
    pub fn as_str(self) -> &'static str {
        match self {
            LogKind::Summary => "summary",
            LogKind::Detail => "detail",
            LogKind::Warning => "warning",
            LogKind::Error => "error",
            LogKind::Debug => "debug",
        }
    }
}

/// One entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// When it happened (the injected clock of the producer).
    pub time: jiff::Zoned,
    /// Its kind.
    pub kind: LogKind,
    /// Where it comes from (a tab caption, `settings`, `notification`, …).
    pub source: String,
    /// The text (one line; a warning keeps its stable code: `[unknown_config_key] …`).
    pub text: String,
}

/// What the book keeps (OPT-018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogFilter {
    /// Operation summaries.
    pub summaries: bool,
    /// Operation details.
    pub details: bool,
    /// Warnings.
    pub warnings: bool,
    /// Debug messages.
    pub debug: bool,
    /// Entries kept (`display_up_to`; at least 1).
    pub capacity: usize,
}

impl Default for LogFilter {
    /// The defaults of `[tweaks.log_panel]`.
    fn default() -> LogFilter {
        LogFilter::from_options(&Options::default())
    }
}

impl LogFilter {
    /// The filter of `[tweaks.log_panel]` (OPT-018).
    pub fn from_options(o: &Options) -> LogFilter {
        let p = &o.tweaks.log_panel;
        LogFilter {
            summaries: p.operation_summaries,
            details: p.operation_details,
            warnings: p.warnings,
            debug: p.debug_messages,
            capacity: usize::try_from(p.display_up_to)
                .unwrap_or(usize::MAX)
                .max(1),
        }
    }

    /// True when an entry of `kind` is kept (errors always are).
    pub fn keeps(&self, kind: LogKind) -> bool {
        match kind {
            LogKind::Summary => self.summaries,
            LogKind::Detail => self.details,
            LogKind::Warning => self.warnings,
            LogKind::Error => true,
            LogKind::Debug => self.debug,
        }
    }
}

/// The bounded log of a GUI process.
#[derive(Debug, Clone, Default)]
pub struct LogBook {
    entries: VecDeque<LogEntry>,
    filter: LogFilter,
    save: bool,
    generation: u64,
    unsaved: Vec<String>,
}

impl LogBook {
    /// An empty book with `filter`; `save` = `save_to_file` is set.
    pub fn new(filter: LogFilter, save: bool) -> LogBook {
        LogBook {
            entries: VecDeque::new(),
            filter,
            save,
            generation: 0,
            unsaved: Vec::new(),
        }
    }

    /// Changes the filter (the options were reloaded or applied): a smaller capacity drops the oldest entries;
    /// entries already kept stay. Turning saving off forgets the unsaved lines.
    pub fn set_filter(&mut self, filter: LogFilter, save: bool) {
        self.filter = filter;
        self.save = save;
        if !save {
            self.unsaved.clear();
        }
        if self.trim() {
            self.generation += 1;
        }
    }

    /// The filter.
    pub fn filter(&self) -> LogFilter {
        self.filter
    }

    /// Adds an entry; `false` when the filter drops it.
    pub fn push(&mut self, e: LogEntry) -> bool {
        if !self.filter.keeps(e.kind) {
            return false;
        }
        if self.save {
            self.unsaved.push(format_line(&e));
        }
        self.entries.push_back(e);
        self.trim();
        self.generation += 1;
        true
    }

    /// Adds one `Warning` entry per warning, its stable code in the text (`[code] message`).
    pub fn push_warnings(&mut self, time: &jiff::Zoned, source: &str, w: &[Warning]) {
        for warning in w {
            self.push(LogEntry {
                time: time.clone(),
                kind: LogKind::Warning,
                source: source.to_owned(),
                text: format!("[{}] {}", warning.code, warning.message),
            });
        }
    }

    /// The entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the book is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bumped by every change (the panel redraws when it differs).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Removes every entry (the panel's Clear; lines not saved yet are still saved).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.generation += 1;
    }

    /// The formatted lines pushed since the last call (only with `save_to_file`; empty otherwise).
    pub fn take_unsaved(&mut self) -> Vec<String> {
        std::mem::take(&mut self.unsaved)
    }

    /// Drops the oldest entries beyond the capacity; true when something was dropped.
    fn trim(&mut self) -> bool {
        let mut dropped = false;
        while self.entries.len() > self.filter.capacity {
            self.entries.pop_front();
            dropped = true;
        }
        dropped
    }
}

/// One saved line: `2026-10-30 14:03:07  warning  <source>: <text>` (line breaks of the text become spaces).
pub fn format_line(e: &LogEntry) -> String {
    let text = e.text.replace(['\r', '\n'], " ");
    format!(
        "{}  {}  {}: {}",
        e.time.strftime("%Y-%m-%d %H:%M:%S"),
        e.kind.as_str(),
        e.source,
        text
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::WarningCode;

    fn time() -> jiff::Zoned {
        "2026-10-30T14:03:07+01:00[+01:00]".parse().expect("a time")
    }

    fn entry(kind: LogKind, text: &str) -> LogEntry {
        LogEntry {
            time: time(),
            kind,
            source: "a ⇄ b".into(),
            text: text.into(),
        }
    }

    const KINDS: [LogKind; 5] = [
        LogKind::Summary,
        LogKind::Detail,
        LogKind::Warning,
        LogKind::Error,
        LogKind::Debug,
    ];

    #[test]
    fn each_filter_keeps_its_kind_and_errors_always() {
        let none = LogFilter {
            summaries: false,
            details: false,
            warnings: false,
            debug: false,
            capacity: 10,
        };
        for (flag, kind) in [
            (0, LogKind::Summary),
            (1, LogKind::Detail),
            (2, LogKind::Warning),
            (3, LogKind::Debug),
        ] {
            let mut filter = none;
            match flag {
                0 => filter.summaries = true,
                1 => filter.details = true,
                2 => filter.warnings = true,
                _ => filter.debug = true,
            }
            let mut book = LogBook::new(filter, false);
            let kept: Vec<LogKind> = KINDS
                .iter()
                .filter(|k| book.push(entry(**k, "x")))
                .copied()
                .collect();
            let mut want = vec![kind, LogKind::Error];
            want.sort();
            assert_eq!(kept, want);
        }
        let mut book = LogBook::new(none, false);
        assert!(book.push(entry(LogKind::Error, "e")));
        assert_eq!(book.len(), 1);
        // The defaults of OPT-018: summaries and warnings, no details, no debug, 1000 entries.
        let d = LogFilter::default();
        assert!(d.summaries && d.warnings && !d.details && !d.debug);
        assert_eq!(d.capacity, 1000);
    }

    #[test]
    fn capacity_drops_the_oldest_and_set_filter_shrinks() {
        let filter = LogFilter {
            capacity: 3,
            ..LogFilter::default()
        };
        let mut book = LogBook::new(filter, false);
        for i in 0..5 {
            book.push(entry(LogKind::Summary, &i.to_string()));
        }
        let texts: Vec<&str> = book.entries().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["2", "3", "4"]);
        let before = book.generation();
        book.set_filter(
            LogFilter {
                capacity: 1,
                ..filter
            },
            false,
        );
        assert!(book.generation() > before);
        let texts: Vec<&str> = book.entries().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["4"]);
        book.clear();
        assert!(book.is_empty());
        let mut zero = Options::default();
        zero.tweaks.log_panel.display_up_to = 0;
        assert_eq!(LogFilter::from_options(&zero).capacity, 1);
    }

    #[test]
    fn lines_are_formatted_and_saved_only_with_save_to_file() {
        let e = entry(LogKind::Warning, "[unknown_config_key] two\nlines");
        assert_eq!(
            format_line(&e),
            "2026-10-30 14:03:07  warning  a ⇄ b: [unknown_config_key] two lines"
        );
        let mut book = LogBook::new(LogFilter::default(), false);
        book.push(entry(LogKind::Summary, "s"));
        assert!(book.take_unsaved().is_empty());
        book.set_filter(LogFilter::default(), true);
        book.push(entry(LogKind::Summary, "s"));
        book.push(entry(LogKind::Detail, "dropped by the filter"));
        assert_eq!(
            book.take_unsaved(),
            vec!["2026-10-30 14:03:07  summary  a ⇄ b: s".to_owned()]
        );
        assert!(book.take_unsaved().is_empty());
        book.push(entry(LogKind::Error, "e"));
        book.set_filter(LogFilter::default(), false);
        assert!(book.take_unsaved().is_empty());
    }

    #[test]
    fn warnings_keep_their_codes() {
        let mut book = LogBook::new(LogFilter::default(), false);
        book.push_warnings(
            &time(),
            "settings",
            &[Warning {
                code: WarningCode::UnknownConfigKey,
                message: "options.toml: `x`".into(),
                spec: None,
            }],
        );
        let e = book.entries().next().expect("one entry");
        assert_eq!(e.kind, LogKind::Warning);
        assert_eq!(e.text, "[unknown_config_key] options.toml: `x`");
        assert_eq!(e.source, "settings");
    }
}

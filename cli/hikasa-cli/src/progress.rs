//! Progress on stderr (contract §1.9.1, §5.4): one updating line when stderr is a terminal and `--quiet` is not
//! set; nothing otherwise. The service throttles the reports (≤ 20 Hz per phase), so the sink draws every report
//! it receives. The line is cleared with spaces (no ANSI sequence) before anything else is written.

use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex, PoisonError};

use hikasa_core::{NullProgress, Progress, ProgressSink, Unit};
use hikasa_report::escape_name;

/// Longest progress line (characters), so that it fits a narrow terminal without wrapping.
const MAX_LINE: usize = 79;

/// The progress of one invocation.
#[derive(Clone)]
pub enum ProgressLine {
    /// No progress (stderr is not a terminal, or `--quiet`).
    Off,
    /// One updating line on the terminal.
    Terminal(Arc<StderrProgress>),
}

impl ProgressLine {
    /// Progress for this process: a terminal line only when stderr is a terminal and not `quiet`.
    pub fn new(quiet: bool) -> ProgressLine {
        if !quiet && io::stderr().is_terminal() {
            ProgressLine::Terminal(Arc::new(StderrProgress::default()))
        } else {
            ProgressLine::Off
        }
    }

    /// The sink to pass to `Service::job`.
    pub fn sink(&self) -> Arc<dyn ProgressSink> {
        match self {
            ProgressLine::Off => Arc::new(NullProgress),
            ProgressLine::Terminal(sink) => Arc::clone(sink) as Arc<dyn ProgressSink>,
        }
    }

    /// Removes the progress line (before output, warnings or errors are written).
    pub fn clear(&self) {
        if let ProgressLine::Terminal(sink) = self {
            sink.clear();
        }
    }

    /// One line for the person at the terminal, in place of the progress line; nothing where progress is off.
    pub fn note(&self, text: &str) {
        if let ProgressLine::Terminal(sink) = self {
            sink.clear();
            let mut err = io::stderr().lock();
            let _ = writeln!(err, "{text}");
            let _ = err.flush();
        }
    }
}

/// [`ProgressSink`] drawing one line on stderr (`\r` + text + padding).
#[derive(Debug, Default)]
pub struct StderrProgress {
    /// Characters of the line on screen (0 = nothing drawn).
    drawn: Mutex<usize>,
}

impl StderrProgress {
    /// Overwrites the line with spaces and returns to its start.
    pub fn clear(&self) {
        let mut drawn = self.drawn.lock().unwrap_or_else(PoisonError::into_inner);
        if *drawn > 0 {
            let mut err = io::stderr().lock();
            let _ = write!(err, "\r{:width$}\r", "", width = *drawn);
            let _ = err.flush();
            *drawn = 0;
        }
    }
}

impl ProgressSink for StderrProgress {
    fn report(&self, progress: Progress) {
        let line = render(&progress);
        let width = line.chars().count();
        let mut drawn = self.drawn.lock().unwrap_or_else(PoisonError::into_inner);
        let pad = drawn.saturating_sub(width);
        let mut err = io::stderr().lock();
        let _ = write!(err, "\r{line}{:pad$}", "");
        let _ = err.flush();
        *drawn = width;
    }
}

/// The unit's word: hikasa-core's own name of the unit (its serde name, as in the JSON documents, §4.1 rule 2),
/// so no word table lives in the adapter (T11).
fn unit_word(unit: Unit) -> String {
    match serde_json::to_value(unit) {
        Ok(serde_json::Value::String(word)) => word,
        Ok(_) | Err(_) => String::new(),
    }
}

/// `<phase> <done>[/<total>] <unit>[ <current>]`, at most [`MAX_LINE`] characters.
fn render(progress: &Progress) -> String {
    let mut line = format!("{} {}", progress.phase.as_str(), progress.done);
    if let Some(total) = progress.total {
        line.push_str(&format!("/{total}"));
    }
    line.push(' ');
    line.push_str(&unit_word(progress.unit));
    if let Some(current) = &progress.current {
        line.push(' ');
        line.push_str(&escape_name(current));
    }
    if line.chars().count() > MAX_LINE {
        line = line.chars().take(MAX_LINE - 1).collect();
        line.push('…');
    }
    line
}

#[cfg(test)]
mod tests {
    use hikasa_core::Phase;

    use super::*;

    #[test]
    fn lines_are_short_and_escaped() {
        let p = Progress {
            phase: Phase::Scanning,
            done: 3,
            total: Some(10),
            unit: Unit::Folders,
            current: Some("a\nb".into()),
        };
        assert_eq!(render(&p), "scanning 3/10 folders a\\nb");
        let long = Progress {
            current: Some("x".repeat(200)),
            ..p
        };
        let line = render(&long);
        assert_eq!(line.chars().count(), MAX_LINE);
        assert!(line.ends_with('…'));
        let open = Progress {
            phase: Phase::Reading,
            done: 5,
            total: None,
            unit: Unit::Bytes,
            current: None,
        };
        assert_eq!(render(&open), "reading 5 bytes");
    }
}

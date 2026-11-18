//! The log of a script (§1.8.9, D-224): one function makes every line, for every sink; the file of `log [append:]<file>`
//! is written and flushed line by line. The one place of the script code that opens a file without the VFS (check 2): the
//! VFS has no append primitive and W8 adds none.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use hikasa_report::escape_controls;
use hikasa_script::{LineKind, LocalTime};

/// `<kind> <where> <text>`: the line as the stream shows it (§1.8.9), `<where>` = `L<line>` or `-`; the text through
/// `escape_controls`.
pub fn line(kind: LineKind, line_no: Option<u32>, text: &str) -> String {
    let at = line_no.map_or_else(|| "-".to_owned(), |n| format!("L{n}"));
    format!("{} {at} {}", kind.as_str(), escape_controls(text))
}

/// `<YYYY-MM-DD> <HH:MM:SS> <kind> <where> <text>`: the line of the file and the window.
pub fn stamped(time: LocalTime, kind: LineKind, line_no: Option<u32>, text: &str) -> String {
    format!(
        "{} {} {}",
        time.date(),
        time.time(),
        line(kind, line_no, text)
    )
}

/// An open log file: each line is written and flushed as it comes (a crash keeps what was logged; no `fsync`).
#[derive(Debug)]
pub struct LogFile {
    file: File,
    /// A write failed: the file is given up, the stream goes on.
    broken: bool,
}

impl LogFile {
    /// Opens `path`: `append` appends (created when absent), otherwise it is created or truncated.
    pub fn open(path: &Path, append: bool) -> std::io::Result<LogFile> {
        let mut options = OpenOptions::new();
        options.create(true);
        if append {
            options.append(true);
        } else {
            options.write(true).truncate(true);
        }
        Ok(LogFile {
            file: options.open(path)?,
            broken: false,
        })
    }

    /// Writes one line and flushes it; after a failure nothing more is written and the error is returned once.
    pub fn write_line(&mut self, text: &str) -> std::io::Result<()> {
        if self.broken {
            return Ok(());
        }
        let result = self
            .file
            .write_all(text.as_bytes())
            .and_then(|()| self.file.write_all(b"\n"))
            .and_then(|()| self.file.flush());
        if result.is_err() {
            self.broken = true;
        }
        result
    }
}

/// A sink a test hands the log instead of a file (A8.9's write fault): lines go to `write`.
pub trait LineSink: Send {
    /// Writes one whole line.
    fn write_line(&mut self, text: &str) -> std::io::Result<()>;
}

impl LineSink for LogFile {
    fn write_line(&mut self, text: &str) -> std::io::Result<()> {
        LogFile::write_line(self, text)
    }
}

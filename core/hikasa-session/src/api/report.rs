//! Reports and output files (§3.2, §3.4; RPT-004, RPT-011, D-24): [`Service::render_text`],
//! [`Service::render_folder`], [`Service::write_output`] and their `_with` variants with [`ReportSettings`].

use std::ffi::OsStr;
use std::io::Write;

use hikasa_core::{Spec, Warning};
use hikasa_report::{
    FolderLayout, FolderReportOptions, HtmlStyle, ReportMeta, TextReportOptions,
    render_folder_report_with, render_text_report_with,
};
use hikasa_vfs::{Vfs, VfsError, VfsOp, WriteOptions};

use super::responses::{FolderCompareResponse, TextCompareResponse};
use super::{ApiError, Service};
use crate::jobs::run_guarded;

/// Report inputs that the option structs of hikasa-report do not carry (additive, wave 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSettings {
    /// `--title <text>`: the report title (`None` = "Text Compare" / "Folder Compare"; RPT-004).
    pub title: Option<String>,
    /// HTML styling (`html-custom` stylesheet).
    pub style: HtmlStyle,
    /// Folder `List` layout: write the closing `# <n> same, …` summary line (§4.3.3). `false` for `--quiet`.
    pub list_summary: bool,
}

impl Default for ReportSettings {
    /// No title, the built-in stylesheet, the list summary line written.
    fn default() -> ReportSettings {
        ReportSettings {
            title: None,
            style: HtmlStyle::default(),
            list_summary: true,
        }
    }
}

impl Service {
    /// Renders a text report with `ReportMeta::for_text(…, Some(clock.now()), "hikasa <version>")`.
    pub fn render_text(
        &self,
        resp: &TextCompareResponse,
        opts: &TextReportOptions,
        out: &mut dyn Write,
    ) -> Result<(), ApiError> {
        self.render_text_with(resp, opts, &ReportSettings::default(), out)
            .map(|_| ())
    }

    /// [`Service::render_text`] with a title and HTML style; returns the warnings of the rendering itself
    /// (`patch_lossy_eol`), which the CLI adds to its JSON/stderr warnings. Additive.
    pub fn render_text_with(
        &self,
        resp: &TextCompareResponse,
        opts: &TextReportOptions,
        settings: &ReportSettings,
        out: &mut dyn Write,
    ) -> Result<Vec<Warning>, ApiError> {
        run_guarded(|| {
            let mut meta = ReportMeta::for_text(
                &resp.comparison,
                opts,
                Some(self.clock().now()),
                &self.generator(),
            );
            meta.title.clone_from(&settings.title);
            Ok(render_text_report_with(
                &resp.comparison,
                opts,
                &meta,
                &settings.style,
                out,
            )?)
        })
    }

    /// Renders a folder report with `ReportMeta::for_folder(…)`.
    pub fn render_folder(
        &self,
        resp: &FolderCompareResponse,
        opts: &FolderReportOptions,
        out: &mut dyn Write,
    ) -> Result<(), ApiError> {
        self.render_folder_with(resp, opts, &ReportSettings::default(), out)
    }

    /// [`Service::render_folder`] with a title, an HTML style and the list summary switch. Additive.
    ///
    /// hikasa-report has no switch for the list's summary line; with `list_summary == false` the service drops
    /// the `# …` line from the `List` output (list rows start with a marker, never with `# `).
    pub fn render_folder_with(
        &self,
        resp: &FolderCompareResponse,
        opts: &FolderReportOptions,
        settings: &ReportSettings,
        out: &mut dyn Write,
    ) -> Result<(), ApiError> {
        run_guarded(|| {
            let mut meta = ReportMeta::for_folder(
                &resp.comparison,
                opts,
                Some(self.clock().now()),
                &self.generator(),
            );
            meta.title.clone_from(&settings.title);
            if opts.layout == FolderLayout::List && !settings.list_summary {
                let mut filter = DropSummaryLines::new(out);
                render_folder_report_with(
                    &resp.comparison,
                    opts,
                    &meta,
                    &settings.style,
                    &mut filter,
                )?;
                filter.finish().map_err(|e| ApiError::Report(e.into()))?;
                Ok(())
            } else {
                Ok(render_folder_report_with(
                    &resp.comparison,
                    opts,
                    &meta,
                    &settings.style,
                    out,
                )?)
            }
        })
    }

    /// Writes report output to a spec atomically (`--output`).
    ///
    /// The target is replaced through the VFS's atomic writer (temporary file, fsync, rename): on any error the
    /// target is untouched. Its folder must exist.
    pub fn write_output(&self, target: &OsStr, bytes: &[u8]) -> Result<Spec, ApiError> {
        run_guarded(|| {
            let spec = self.parse_spec(target)?;
            let options = WriteOptions {
                create_parents: false,
                overwrite: true,
            };
            let mut writer = self
                .vfs()
                .open_write(&spec, &options)
                .map_err(ApiError::from_vfs)?;
            writer.write_all(bytes).map_err(|e| {
                ApiError::from_vfs(VfsError::from_io(spec.canonical(), VfsOp::Write, e))
            })?;
            writer.commit().map_err(ApiError::from_vfs)?;
            Ok(spec)
        })
    }
}

/// A writer that drops every line starting with `# ` (the folder list's summary line), streaming.
struct DropSummaryLines<'a> {
    out: &'a mut dyn Write,
    at_line_start: bool,
    state: LineState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    /// Copying.
    Normal,
    /// A `#` at a line start is held back until the next byte decides.
    Hash,
    /// Inside a dropped line (up to and including its newline).
    Dropping,
}

impl<'a> DropSummaryLines<'a> {
    fn new(out: &'a mut dyn Write) -> DropSummaryLines<'a> {
        DropSummaryLines {
            out,
            at_line_start: true,
            state: LineState::Normal,
        }
    }

    /// Writes a held-back `#` and flushes.
    fn finish(self) -> std::io::Result<()> {
        if self.state == LineState::Hash {
            self.out.write_all(b"#")?;
        }
        self.out.flush()
    }
}

impl Write for DropSummaryLines<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut kept = Vec::with_capacity(buf.len());
        for &b in buf {
            match self.state {
                LineState::Normal => {
                    if self.at_line_start && b == b'#' {
                        self.state = LineState::Hash;
                    } else {
                        kept.push(b);
                        self.at_line_start = b == b'\n';
                    }
                }
                LineState::Hash => {
                    if b == b' ' {
                        self.state = LineState::Dropping;
                    } else {
                        kept.push(b'#');
                        kept.push(b);
                        self.at_line_start = b == b'\n';
                        self.state = LineState::Normal;
                    }
                }
                LineState::Dropping => {
                    if b == b'\n' {
                        self.state = LineState::Normal;
                        self.at_line_start = true;
                    }
                }
            }
        }
        self.out.write_all(&kept)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_lines_are_dropped() {
        let mut out = Vec::new();
        let mut filter = DropSummaryLines::new(&mut out);
        for piece in [
            &b"=  a\n!  b/"[..],
            b"\n#",
            b" 1 same, 0 left newer\n",
            b"#x\n",
        ] {
            filter.write_all(piece).unwrap();
        }
        filter.finish().unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "=  a\n!  b/\n#x\n");
    }
}

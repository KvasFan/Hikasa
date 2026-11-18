//! The format editor's preview (M2 W11c; contract §1.11.27, **open default**: shown on Text formats): a sample text
//! (typed, or the first 64 KiB of a file loaded through the browse dialog) tokenized with the edited format by
//! `Service::format_preview` — a job started [`PAUSE`] after the last change, a newer one cancelling an older — so a
//! grammar is tried before it is saved. The drawing colours each token by its element name (`ui/formats.rs`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hikasa_core::JobKind;
use hikasa_formats::FileFormat;
use hikasa_session::Service;
use hikasa_session::api::format_editor::PreviewLine;

use crate::job::{Job, Wake};

/// The pause after the last change before the preview runs again (§1.11.27).
pub const PAUSE: Duration = Duration::from_millis(300);

/// The sample a new editor shows.
pub const SAMPLE: &str =
    "/* A sample: type or load your own. */\nint main(void) {\n    return 0; // done\n}\n";

/// The preview of one editor.
pub struct Preview {
    /// The sample text.
    pub sample: String,
    /// The tokens of the last run (its lines).
    pub lines: Vec<PreviewLine>,
    /// Core's message of the last run (a format that does not compile).
    pub error: Option<String>,
    /// The last change (format or sample) not yet tokenized.
    changed_at: Option<Instant>,
    /// What the running or last job tokenized.
    ran: Option<(FileFormat, String)>,
    job: Option<Job<Vec<PreviewLine>>>,
    loading: Option<Job<String>>,
}

impl Default for Preview {
    fn default() -> Preview {
        Preview {
            sample: SAMPLE.to_owned(),
            lines: Vec::new(),
            error: None,
            changed_at: None,
            ran: None,
            job: None,
            loading: None,
        }
    }
}

impl Preview {
    /// The format or the sample changed at `now`.
    pub fn touch(&mut self, now: Instant) {
        self.changed_at = Some(now);
    }

    /// True while a job runs.
    pub fn running(&self) -> bool {
        self.job.is_some() || self.loading.is_some()
    }

    /// Reads a sample file (its first 64 KiB, decoded as a document of `format`) on a job.
    pub fn load_file(&mut self, svc: &Service, path: PathBuf, format: FileFormat, wake: &Wake) {
        self.loading = Some(Job::spawn(
            svc,
            JobKind::VfsRead,
            "read the sample".into(),
            wake,
            move |svc, _| svc.format_sample(&path, &format),
        ));
    }

    /// One frame at `now`: lands a finished job, and starts one when the format (`None`: a field that does not read)
    /// or the sample changed and the pause has passed. True when something changed.
    pub fn tick(
        &mut self,
        svc: &Service,
        wake: &Wake,
        format: Option<&FileFormat>,
        now: Instant,
    ) -> bool {
        let mut changed = false;
        if let Some(loading) = &self.loading
            && let Some(result) = loading.try_result()
        {
            self.loading = None;
            changed = true;
            match result {
                Ok(text) => {
                    self.sample = text;
                    self.changed_at = Some(now - PAUSE);
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
        if let Some(job) = &self.job
            && let Some(result) = job.try_result()
        {
            self.job = None;
            changed = true;
            match result {
                Ok(lines) => {
                    self.lines = lines;
                    self.error = None;
                }
                Err(hikasa_session::ApiError::Cancelled) => {}
                Err(e) => {
                    self.lines.clear();
                    self.error = Some(e.to_string());
                }
            }
        }
        let Some(format) = format else {
            return changed;
        };
        let wanted = (format.clone(), self.sample.clone());
        let stale = self.ran.as_ref() != Some(&wanted);
        let due = self
            .changed_at
            .is_none_or(|at| now.saturating_duration_since(at) >= PAUSE);
        if stale && due {
            if let Some(old) = self.job.take() {
                old.cancel();
            }
            let (f, sample) = wanted.clone();
            self.job = Some(Job::spawn(
                svc,
                JobKind::FormatTokens,
                "preview the format".into(),
                wake,
                move |svc, job| svc.format_preview(&f, &sample, job),
            ));
            self.ran = Some(wanted);
            self.changed_at = None;
            changed = true;
        }
        changed
    }
}

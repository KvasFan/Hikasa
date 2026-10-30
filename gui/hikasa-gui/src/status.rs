//! The status bar, the job indicator and the message panel (M2 W4a, D-115; UI-030): toolkit-free data that the
//! views fill and `ui/status.rs` draws.

use hikasa_core::Phase;
use hikasa_session::ipc::ViewId;

/// A view's status bar texts: the context text on the left, the state items on the right (comparison state,
/// then encodings and line endings for file views).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusItems {
    /// The context text.
    pub left: String,
    /// The state items, left to right.
    pub right: Vec<String>,
}

/// One running job of a view (the job indicator lists them; Cancel cancels the view's jobs).
#[derive(Debug, Clone, PartialEq)]
pub struct JobStatus {
    /// The view.
    pub view: ViewId,
    /// The job's title.
    pub title: String,
    /// The last phase reported.
    pub phase: Option<Phase>,
    /// Done / total when the total is known (0.0…1.0).
    pub fraction: Option<f32>,
}

/// The aggregate of the job indicator of a window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JobSummary {
    /// Running jobs.
    pub count: usize,
    /// The first job's title and phase (the indicator's text).
    pub text: String,
    /// The mean fraction of the jobs that know theirs.
    pub fraction: Option<f32>,
}

impl JobSummary {
    /// The summary of `jobs`.
    pub fn of(jobs: &[JobStatus]) -> JobSummary {
        let Some(first) = jobs.first() else {
            return JobSummary::default();
        };
        let mut text = first.title.clone();
        if let Some(phase) = first.phase {
            text.push_str(&format!(" ({})", phase.as_str().replace('_', " ")));
        }
        if jobs.len() > 1 {
            text.push_str(&format!(" and {} more", jobs.len() - 1));
        }
        let known: Vec<f32> = jobs.iter().filter_map(|j| j.fraction).collect();
        let fraction = (!known.is_empty()).then(|| known.iter().sum::<f32>() / known.len() as f32);
        JobSummary {
            count: jobs.len(),
            text,
            fraction,
        }
    }
}

/// What a view tells the log book when a job of it ends (D-116): the app adds the time and the tab caption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogNote {
    /// The kind (summary, warning, error).
    pub kind: hikasa_session::log::LogKind,
    /// The text (warnings keep their stable code: `[code] message`).
    pub text: String,
}

/// The kind of a message.
#[allow(dead_code)] // variants for W4b/W4c (contract seams)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKind {
    /// Information.
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
}

/// The one-line message panel of a view (D-115).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Kind.
    pub kind: MessageKind,
    /// Text.
    pub text: String,
    /// "Do not show again", bound to a boolean `options.toml` key set to `false` when chosen
    /// (`next_difference.show_message_panel`).
    pub do_not_show: Option<&'static str>,
}

impl Message {
    /// An information message.
    pub fn info(text: impl Into<String>) -> Message {
        Message {
            kind: MessageKind::Info,
            text: text.into(),
            do_not_show: None,
        }
    }

    /// A warning message.
    #[allow(dead_code)] // later waves
    pub fn warning(text: impl Into<String>) -> Message {
        Message {
            kind: MessageKind::Warning,
            text: text.into(),
            do_not_show: None,
        }
    }

    /// An error message.
    pub fn error(text: impl Into<String>) -> Message {
        Message {
            kind: MessageKind::Error,
            text: text.into(),
            do_not_show: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_indicator_aggregates_jobs() {
        assert_eq!(JobSummary::of(&[]), JobSummary::default());
        let jobs = [
            JobStatus {
                view: ViewId(1),
                title: "a ⇄ b".into(),
                phase: Some(Phase::Aligning),
                fraction: Some(0.5),
            },
            JobStatus {
                view: ViewId(2),
                title: "c ⇄ d".into(),
                phase: None,
                fraction: Some(1.0),
            },
            JobStatus {
                view: ViewId(2),
                title: "x".into(),
                phase: None,
                fraction: None,
            },
        ];
        let s = JobSummary::of(&jobs);
        assert_eq!(s.count, 3);
        assert_eq!(s.text, "a ⇄ b (aligning) and 2 more");
        assert_eq!(s.fraction, Some(0.75));
    }
}

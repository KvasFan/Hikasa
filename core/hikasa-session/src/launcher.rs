//! Starting programs (FC-062; D-186, §1.6.18): a pure trait, as `ClipboardProvider` is (D-100). Only adapters name
//! `host::SystemLauncher`; a test never starts a real program.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::Serialize;

use crate::api::ApiError;

/// One program with its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommand {
    /// The program.
    pub program: OsString,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// The folder it starts in.
    pub cwd: PathBuf,
}

/// An [`OpenCommand`] as JSON (`commands` of `hikasa.folder_op.v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenCommandDto {
    /// The program.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// The folder it starts in.
    pub cwd: String,
}

impl From<&OpenCommand> for OpenCommandDto {
    fn from(command: &OpenCommand) -> OpenCommandDto {
        OpenCommandDto {
            program: command.program.to_string_lossy().into_owned(),
            args: command
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            cwd: command.cwd.display().to_string(),
        }
    }
}

/// What starts programs.
pub trait AppLauncher: Send + Sync {
    /// Starts the program and returns when it has ended (a job thread waits: `refresh_when_finished`,
    /// `wait_for_previous`). `Err` when it cannot start; its own exit code is the program's.
    fn run(&self, command: &OpenCommand) -> Result<i32, ApiError>;
    /// Starts the program and returns once it started (additive, W9b review): `Err` when it cannot start, else what
    /// waits for its end. Programs that do not wait for each other start one after the other, so the first that cannot
    /// start stops the rest. The default runs the program to its end here (a launcher that cannot tell the two apart).
    fn start<'a>(&'a self, command: &'a OpenCommand) -> Result<Started<'a>, ApiError> {
        let code = self.run(command)?;
        Ok(Box::new(move || Ok(code)))
    }
    /// FC-062 *Associated Application*.
    fn open_associated(&self, path: &Path) -> Result<(), ApiError>;
    /// FC-063 *Open Folder in File Manager* (D-254; W9c note 34): the system handler opens the folder `path`. Default:
    /// [`AppLauncher::open_associated`].
    fn open_folder(&self, path: &Path) -> Result<(), ApiError> {
        self.open_associated(path)
    }
    /// FC-063 *Show in File Manager* (M2 W9c, D-254): the file manager shows `path` selected in its folder. Default:
    /// `unsupported`.
    fn reveal(&self, path: &Path) -> Result<(), ApiError> {
        Err(ApiError::Unsupported(format!(
            "this program cannot show {} in a file manager",
            path.display()
        )))
    }
}

/// What [`AppLauncher::start`] gives back: waits for the started program's end, with its exit code.
pub type Started<'a> = Box<dyn FnOnce() -> Result<i32, ApiError> + Send + 'a>;

/// No programs here: every call is `unsupported`. The default of `ServiceInit::new`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLauncher;

impl AppLauncher for NoLauncher {
    fn run(&self, command: &OpenCommand) -> Result<i32, ApiError> {
        Err(ApiError::Unsupported(format!(
            "this program cannot start {}",
            command.program.to_string_lossy()
        )))
    }

    fn open_associated(&self, path: &Path) -> Result<(), ApiError> {
        Err(ApiError::Unsupported(format!(
            "this program cannot open {}",
            path.display()
        )))
    }
}

/// One call a [`RecordingLauncher`] took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// [`AppLauncher::run`].
    Run(OpenCommand),
    /// [`AppLauncher::open_associated`].
    Associated(PathBuf),
    /// [`AppLauncher::reveal`] (M2 W9c).
    Reveal(PathBuf),
    /// [`AppLauncher::open_folder`] (W9c note 34).
    OpenFolder(PathBuf),
}

/// For tests: records each call and answers from a script: one answer per call, in order, then exit code 0. An answer
/// `None` is a program that cannot start.
#[derive(Debug, Default)]
pub struct RecordingLauncher {
    calls: Mutex<Vec<Recorded>>,
    answers: Mutex<Vec<Option<i32>>>,
}

impl RecordingLauncher {
    /// A launcher that answers `answers`, the first one first.
    pub fn with_answers(answers: Vec<Option<i32>>) -> RecordingLauncher {
        let mut answers = answers;
        answers.reverse();
        RecordingLauncher {
            calls: Mutex::new(Vec::new()),
            answers: Mutex::new(answers),
        }
    }

    /// The calls so far.
    pub fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().clone()
    }

    fn answer(&self, program: &str) -> Result<i32, ApiError> {
        match self.answers.lock().pop() {
            Some(None) => Err(ApiError::InvalidArgument(format!("cannot start {program}"))),
            Some(Some(code)) => Ok(code),
            None => Ok(0),
        }
    }
}

impl AppLauncher for RecordingLauncher {
    fn run(&self, command: &OpenCommand) -> Result<i32, ApiError> {
        self.calls.lock().push(Recorded::Run(command.clone()));
        self.answer(&command.program.to_string_lossy())
    }

    fn open_associated(&self, path: &Path) -> Result<(), ApiError> {
        self.calls
            .lock()
            .push(Recorded::Associated(path.to_path_buf()));
        self.answer(&path.display().to_string()).map(drop)
    }

    fn reveal(&self, path: &Path) -> Result<(), ApiError> {
        self.calls.lock().push(Recorded::Reveal(path.to_path_buf()));
        self.answer(&path.display().to_string()).map(drop)
    }

    fn open_folder(&self, path: &Path) -> Result<(), ApiError> {
        self.calls
            .lock()
            .push(Recorded::OpenFolder(path.to_path_buf()));
        self.answer(&path.display().to_string()).map(drop)
    }
}

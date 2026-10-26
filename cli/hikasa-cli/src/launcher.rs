//! [`ProcessGuiLauncher`] (contract §1.9.1, §3.6): starts `hikasa-gui` detached with null stdio and the arguments
//! and environment the session chose (`Service::gui_launch`: `--config-dir` for explicit and portable
//! configurations, `HIKASA_IPC_ENDPOINT` when it is overridden), and sleeps between connection attempts.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use hikasa_session::api::{GuiLaunch, GuiLauncher, LaunchError};

/// The GUI launcher of the CLI.
#[derive(Debug, Clone)]
pub struct ProcessGuiLauncher {
    /// The executable (`find_gui_exe`: `$HIKASA_GUI_EXE` alone when set, else the sibling of the CLI, else
    /// `PATH`; an absolute path, so `Command` never searches `PATH` again); `None` → [`LaunchError::NotFound`]
    /// (exit 103). A spawn failure is [`LaunchError::Spawn`] naming the executable (also exit 103).
    pub exe: Option<PathBuf>,
}

impl GuiLauncher for ProcessGuiLauncher {
    fn launch(&self, launch: &GuiLaunch) -> Result<(), LaunchError> {
        let exe = self.exe.as_ref().ok_or(LaunchError::NotFound)?;
        let mut command = Command::new(exe);
        command
            .args(&launch.args)
            .envs(launch.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut command);
        // An executable that exists but cannot be started (no execute permission, a missing interpreter, …) is
        // no usable GUI executable: the session reports `GuiNotStartable` (exit 103) with this message.
        let mut child = command
            .spawn()
            .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", exe.display())))?;
        // Reap the GUI if it exits while the CLI still runs (no zombie during a long `--wait`); the thread ends
        // with the process otherwise.
        let _ = std::thread::Builder::new()
            .name("hikasa-gui-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });
        Ok(())
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Unix: a process group of its own, so a Ctrl-C in the CLI's terminal does not end the GUI.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows: no console of its own and a new process group.
#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// Other targets: nothing to do.
#[cfg(not(any(unix, windows)))]
fn detach(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_executable_is_not_found() {
        let launcher = ProcessGuiLauncher { exe: None };
        assert!(matches!(
            launcher.launch(&GuiLaunch::default()),
            Err(LaunchError::NotFound)
        ));
    }

    #[test]
    fn a_spawn_failure_names_the_executable() {
        let exe = std::env::temp_dir().join("hikasa-no-such-gui-executable");
        let launcher = ProcessGuiLauncher {
            exe: Some(exe.clone()),
        };
        match launcher.launch(&GuiLaunch::default()) {
            Err(LaunchError::Spawn(e)) => {
                assert!(e.to_string().contains(&*exe.to_string_lossy()), "{e}");
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("{other:?}"),
        }
    }
}

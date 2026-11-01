//! Ctrl-C through the real binary (D-159): each test spawns `hikasa` and sends it SIGINT by its process id.
//!
//! Unix only: a console control event on Windows reaches every process of the console, the test runner too, so a
//! Ctrl-C cannot be aimed at one child there. The handler's state is unit-tested in `src/interrupt.rs` on every OS.

#![cfg(unix)]

mod common;

use std::io::Write;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use common::{Sandbox, hikasa_exe};

/// What a machine under load may take to end after a cancel; the check is that it ends, G4 is measured elsewhere.
const LIMIT: Duration = Duration::from_secs(60);

/// `len` bytes that do not compress.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// `hikasa <args>` as a child, set up as `Sandbox::cmd` sets it up. `stdin` is piped so that a test can hold it open.
fn spawn(s: &Sandbox, args: &[&str]) -> Child {
    Command::new(hikasa_exe())
        .current_dir(s.path())
        .env("HIKASA_IPC_ENDPOINT", s.socket())
        .env("HIKASA_GUI_EXE", s.join("no-such-gui"))
        .env("HIKASA_CLIPBOARD_FILE", s.join("no-clipboard"))
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .env_remove("NO_COLOR")
        .arg("--config-dir")
        .arg(s.config())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hikasa")
}

/// Sends SIGINT to this one child.
fn sigint(child: &Child) {
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("run kill");
    assert!(status.success(), "the child is gone");
}

/// The child's end, within `limit`; a child that overruns is killed by its handle and fails the test.
fn wait(child: &mut Child, limit: Duration) -> ExitStatus {
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            return status;
        }
        if began.elapsed() > limit {
            child.kill().expect("kill the child");
            child.wait().expect("wait");
            panic!("the process did not end within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Waits until `name` is in the sandbox's top folder; the child must still run.
fn wait_for_name(s: &Sandbox, child: &mut Child, name: &str) {
    let began = Instant::now();
    while !names(s).iter().any(|n| n.contains(name)) {
        assert!(
            child.try_wait().expect("wait").is_none(),
            "the process ended before {name} appeared"
        );
        assert!(began.elapsed() < LIMIT, "{name} never appeared");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn names(s: &Sandbox) -> Vec<String> {
    std::fs::read_dir(s.path())
        .expect("list")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// A stored zip of one member.
fn small_zip() -> Vec<u8> {
    let body = b"alpha\n";
    let local = |out: &mut Vec<u8>| {
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0x60, 0xef, 0x5c]);
        out.extend_from_slice(&[0xec, 0x6e, 0x60, 0x9f]); // CRC-32 of the body
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
    };
    let mut out = b"PK\x03\x04".to_vec();
    local(&mut out);
    out.push(b'x');
    out.extend_from_slice(body);
    let start = out.len() as u32;
    out.extend_from_slice(b"PK\x01\x02\x14\x03");
    local(&mut out);
    out.extend_from_slice(&[0; 10]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(b'x');
    let central = out.len() as u32 - start;
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
    out.extend_from_slice(&central.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

/// A rebuild cancelled in flight: exit 100, the archive as it was, its temporary and lock files gone.
#[test]
fn cancel_of_a_rebuild_keeps_the_archive_and_leaves_no_files() {
    let s = Sandbox::new();
    let archive = small_zip();
    s.file("a.zip", &archive);
    s.file("big.bin", noise(64 << 20));
    let check = s.cmd().args(["vfs", "cat", "a.zip/x"]).output().unwrap();
    assert_eq!(check.stdout, b"alpha\n", "the fixture reads");

    let mut child = spawn(&s, &["vfs", "cp", "big.bin", "a.zip/big.bin"]);
    wait_for_name(&s, &mut child, "hikasa-tmp");
    sigint(&child);
    let status = wait(&mut child, LIMIT);

    assert_eq!(status.code(), Some(100), "{status:?}");
    assert_eq!(std::fs::read(s.join("a.zip")).unwrap(), archive);
    let left: Vec<_> = names(&s)
        .into_iter()
        .filter(|n| n.contains("hikasa-tmp") || n.contains("hikasa-lock"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

/// A plain copy cancelled in flight: exit 100, no temporary file, no partial target.
#[test]
fn cancel_of_a_copy_leaves_no_partial_target() {
    let s = Sandbox::new();
    // A folder of many files, so that the cancel falls inside the run.
    let source = noise(16 << 20);
    for k in 0..16 {
        s.file(&format!("tree/f{k}"), &source);
    }
    s.dir("out");
    let mut child = spawn(&s, &["vfs", "cp", "-r", "tree", "out/tree"]);
    let began = Instant::now();
    while !s.join("out/tree").exists() {
        assert!(began.elapsed() < LIMIT, "the copy never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    sigint(&child);
    let status = wait(&mut child, LIMIT);

    assert_eq!(status.code(), Some(100), "{status:?}");
    let copied: Vec<_> = std::fs::read_dir(s.join("out/tree"))
        .unwrap()
        .flatten()
        .collect();
    assert!(copied.len() < 16, "the cancel came too late to test");
    for entry in copied {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.contains("hikasa-tmp"), "{name}");
        assert_eq!(
            entry.metadata().unwrap().len(),
            source.len() as u64,
            "{name}"
        );
    }
}

/// A comparison cancelled in flight exits 100. It leaves no file to wait for, so the signal goes out halfway through
/// a timed run, and a run the signal missed (not armed yet, or done already) is run again.
#[test]
fn cancel_of_a_folder_compare_exits_100() {
    let s = Sandbox::new();
    let content = noise(4 << 20);
    for k in 0..24 {
        s.file(&format!("l/f{k}"), &content);
        s.file(&format!("r/f{k}"), &content);
    }
    let args = ["folder", "compare", "l", "r", "--content", "binary"];
    let mut timed = spawn(&s, &args);
    let began = Instant::now();
    let done = wait(&mut timed, LIMIT);
    let whole = began.elapsed();
    assert!(done.success() || done.code().is_some(), "{done:?}");

    let mut seen = Vec::new();
    for attempt in 1..=8u32 {
        let mut child = spawn(&s, &args);
        std::thread::sleep(whole.mul_f64(0.25 + 0.5 * f64::from(attempt % 3) / 2.0));
        if child.try_wait().expect("wait").is_some() {
            seen.push(None);
            continue;
        }
        sigint(&child);
        let status = wait(&mut child, LIMIT);
        if status.code() == Some(100) {
            return;
        }
        seen.push(status.code());
    }
    panic!("no run ended in a cancel: {seen:?}");
}

/// The second Ctrl-C does not wait for the job. A job blocked on standard input cannot see the cancel, so the first
/// leaves it running and only the second ends it. Standard input takes more than a pipe holds, so when the write
/// returns the job is reading, which is after the handler was armed.
#[test]
fn the_second_ctrl_c_exits_at_once_where_a_cancel_cannot_be_seen() {
    let s = Sandbox::new();
    s.file("b.txt", "other\n");
    let mut child = spawn(&s, &["quick", "-", "b.txt"]);
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(&vec![b'x'; 1 << 20]).expect("feed stdin");
    stdin.flush().expect("flush");

    sigint(&child);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().expect("wait").is_none(),
        "the first Ctrl-C ended a job that cannot see it"
    );
    sigint(&child);
    let status = wait(&mut child, LIMIT);
    assert_eq!(status.code(), Some(100), "{status:?}");
    drop(stdin);
}

/// A command without a job installs nothing: Ctrl-C ends it the way the OS does, also when it waits.
#[test]
fn a_command_without_a_job_keeps_the_default_ctrl_c() {
    use std::os::unix::process::ExitStatusExt;

    let s = Sandbox::new();
    let mut child = spawn(&s, &["gui", "open", "--no-launch", "--wait", "-"]);
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(&vec![b'x'; 1 << 20]).expect("feed stdin");
    stdin.flush().expect("flush");
    sigint(&child);
    let status = wait(&mut child, LIMIT);
    assert_eq!(status.signal(), Some(2), "{status:?}");
    drop(stdin);
}

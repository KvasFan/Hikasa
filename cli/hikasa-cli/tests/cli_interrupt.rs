//! Ctrl-C through the real binary (D-159): each test spawns `hikasa` and sends it SIGINT by its process id.
//!
//! Nothing here is timed. A job is one that cannot end by itself before the test has acted: a source of 64 GiB that
//! is one hole, standard input held open, or a conversion that waits. The signal goes out once the job is seen to be
//! where the test needs it, by a file it made or a line of its `--log` file, and `LIMIT` only ends a test that failed.
//!
//! Unix only: a console control event on Windows reaches every process of the console, the test runner too, so a
//! Ctrl-C cannot be aimed at one child there. The handler's state is unit-tested in `src/interrupt.rs` on every OS.

#![cfg(unix)]

mod common;

use std::io::{Seek, SeekFrom, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use common::{Sandbox, hikasa_exe};

/// What a machine under load may take to end after a cancel; the check is that it ends, G4 is measured elsewhere.
const LIMIT: Duration = Duration::from_secs(60);

/// A source no copy gets through before the test: a hole, so it costs no disk until something copies it.
const HUGE: u64 = 64 << 30;

/// Makes the file `name` of [`HUGE`] bytes.
fn huge(s: &Sandbox, name: &str) {
    let file = std::fs::File::create(s.join(name)).expect("create");
    file.set_len(HUGE).expect("a hole");
}

/// The name of the log every child writes.
const LOG: &str = "hikasa.log";
/// What `src/interrupt.rs` logs.
const ARMED: &str = "Ctrl-C handler set";
const CANCELLING: &str = "Ctrl-C: cancelling";
const EXITING: &str = "Ctrl-C again: exiting";
const INHERITED: &str = "Ctrl-C keeps the action it inherited";

/// `hikasa <args>` as a child, set up as `Sandbox::cmd` sets it up, logging to [`LOG`]. `stdin` is piped so that a test
/// can hold it open.
fn spawn(s: &Sandbox, args: &[&str]) -> Child {
    configure(s, Command::new(hikasa_exe()), args)
}

fn configure(s: &Sandbox, mut command: Command, args: &[&str]) -> Child {
    command
        .current_dir(s.path())
        .env("HIKASA_IPC_ENDPOINT", s.socket())
        .env("HIKASA_GUI_EXE", s.join("no-such-gui"))
        .env("HIKASA_CLIPBOARD_FILE", s.join("no-clipboard"))
        .env("LC_ALL", "C.UTF-8")
        .env("TZ", "UTC")
        .env_remove("NO_COLOR")
        .arg("--config-dir")
        .arg(s.config())
        .args(["--verbose", "--log"])
        .arg(s.join(LOG))
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

/// Waits until `seen` holds; the child must still run.
fn wait_until(child: &mut Child, what: &str, seen: impl Fn() -> bool) {
    let began = Instant::now();
    while !seen() {
        let ended = child.try_wait().expect("wait");
        assert!(ended.is_none(), "ended before {what}: {ended:?}");
        if began.elapsed() > LIMIT {
            child.kill().expect("kill the child");
            child.wait().expect("wait");
            panic!("{what} never happened");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// True when the child's log has `line`.
fn logged(s: &Sandbox, line: &str) -> bool {
    std::fs::read_to_string(s.join(LOG)).is_ok_and(|log| log.contains(line))
}

/// Waits until the child's log has `line`.
fn wait_for_log(s: &Sandbox, child: &mut Child, line: &str) {
    wait_until(child, line, || logged(s, line));
}

/// The names in `dir` of the sandbox.
fn names(s: &Sandbox, dir: &str) -> Vec<String> {
    std::fs::read_dir(s.join(dir))
        .expect("list")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Waits until a name in `dir` contains `part`.
fn wait_for_name(s: &Sandbox, child: &mut Child, dir: &str, part: &str) {
    wait_until(child, part, || {
        names(s, dir).iter().any(|n| n.contains(part))
    });
}

/// The temporary and lock files in `dir`.
fn leftovers(s: &Sandbox, dir: &str) -> Vec<String> {
    let mut left = names(s, dir);
    left.retain(|n| n.contains("hikasa-tmp") || n.contains("hikasa-lock"));
    left
}

/// A format whose conversion writes `marker` and waits to be killed: a comparison of `*.slow` files stops there.
fn slow_format(s: &Sandbox, marker: &str) {
    s.file(
        "slow.sh",
        format!(
            "#!/bin/sh\n: > '{}'\nexec sleep 600\n",
            s.join(marker).display()
        ),
    );
    s.file(
        "cfg/formats/slow.toml",
        format!(
            "schema_version = 1\nid = \"slow\"\nname = \"Slow\"\nkind = \"text\"\nmask = \"*.slow\"\n\n\
             [conversion]\nmethod = \"external\"\nload = 'sh \"{}\" %s %t'\n",
            s.join("slow.sh").display()
        ),
    );
}

/// A zip of one stored member that is a hole of [`HUGE`] bytes (ZIP64, so its numbers fit): the index reads the end of
/// the file only, and a rebuild's raw copy of the member takes longer than any test.
fn huge_zip(s: &Sandbox, name: &str) {
    const ON_64: u32 = u32::MAX;
    let extra: Vec<u8> = [
        &[1u8, 0, 16, 0],
        &HUGE.to_le_bytes()[..],
        &HUGE.to_le_bytes()[..],
    ]
    .concat();
    let fields = |out: &mut Vec<u8>| {
        out.extend_from_slice(&[45, 0, 0, 0, 0, 0, 0, 0x60, 0xef, 0x5c]); // version, flags, stored, time, date
        out.extend_from_slice(&0u32.to_le_bytes()); // the checksum: a raw copy does not read it
        out.extend_from_slice(&ON_64.to_le_bytes());
        out.extend_from_slice(&ON_64.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    };
    let mut local = b"PK\x03\x04".to_vec();
    fields(&mut local);
    local.push(b'x');
    local.extend_from_slice(&extra);
    let start = local.len() as u64 + HUGE;

    let mut tail = b"PK\x01\x02\x2d\x03".to_vec();
    fields(&mut tail);
    tail.extend_from_slice(&[0; 10]);
    tail.extend_from_slice(&0u32.to_le_bytes());
    tail.push(b'x');
    tail.extend_from_slice(&extra);
    let central = tail.len() as u64;
    let end = start + central;
    tail.extend_from_slice(b"PK\x06\x06");
    tail.extend_from_slice(&44u64.to_le_bytes());
    tail.extend_from_slice(&[45, 3, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    tail.extend_from_slice(&1u64.to_le_bytes());
    tail.extend_from_slice(&1u64.to_le_bytes());
    tail.extend_from_slice(&central.to_le_bytes());
    tail.extend_from_slice(&start.to_le_bytes());
    tail.extend_from_slice(b"PK\x06\x07\0\0\0\0");
    tail.extend_from_slice(&end.to_le_bytes());
    tail.extend_from_slice(&1u32.to_le_bytes());
    tail.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
    tail.extend_from_slice(&(central as u32).to_le_bytes());
    tail.extend_from_slice(&ON_64.to_le_bytes());
    tail.extend_from_slice(&[0; 2]);

    let mut file = std::fs::File::create(s.join(name)).expect("create");
    file.write_all(&local).expect("the header");
    file.seek(SeekFrom::Start(start)).expect("a hole");
    file.write_all(&tail).expect("the end");
}

/// A rebuild cancelled in flight: exit 100, the archive as it was, its temporary and lock files gone.
#[test]
fn cancel_of_a_rebuild_keeps_the_archive_and_leaves_no_files() {
    let s = Sandbox::new();
    huge_zip(&s, "a.zip");
    s.file("new", b"new\n");
    let before = std::fs::metadata(s.join("a.zip")).unwrap();
    let listed = s.cmd().args(["vfs", "ls", "a.zip"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(" x"),
        "the fixture reads: {listed:?}"
    );

    let mut child = spawn(&s, &["vfs", "cp", "new", "a.zip/new"]);
    wait_for_name(&s, &mut child, ".", "hikasa-tmp");
    sigint(&child);
    let status = wait(&mut child, LIMIT);

    assert_eq!(status.code(), Some(100), "{status:?}");
    let after = std::fs::metadata(s.join("a.zip")).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(leftovers(&s, "."), Vec::<String>::new());
}

/// A folder's copy cancelled in the middle of a file: exit 100, what was done stays whole, the file under way leaves
/// no temporary file and no partial target, and nothing starts after it.
#[test]
fn cancel_of_a_copy_leaves_no_partial_target() {
    let s = Sandbox::new();
    s.file("tree/a", b"first\n");
    huge(&s, "tree/b");
    s.file("tree/c", b"last\n");
    s.dir("out");
    let mut child = spawn(&s, &["vfs", "cp", "-r", "tree", "out/tree"]);
    wait_until(&mut child, "the copy of the hole", || {
        s.join("out/tree").exists()
            && names(&s, "out/tree")
                .iter()
                .any(|n| n.contains("b.hikasa-tmp"))
    });
    sigint(&child);
    let status = wait(&mut child, LIMIT);

    assert_eq!(status.code(), Some(100), "{status:?}");
    assert_eq!(names(&s, "out/tree"), ["a"]);
    assert_eq!(std::fs::read(s.join("out/tree/a")).unwrap(), b"first\n");
}

/// A comparison cancelled after it found a difference: exit 100, not the verdict of the part it saw, and `--output`
/// keeps the report it had. `one` is only on the left, `two.slow` stops the scan in its conversion.
#[test]
fn cancel_of_a_folder_compare_exits_100_and_keeps_the_report() {
    for json in [false, true] {
        let s = Sandbox::new();
        slow_format(&s, "converting");
        s.file("l/one", b"only here\n");
        s.file("l/two.slow", b"left\n");
        s.file("r/two.slow", b"right\n");
        s.file("report.txt", b"the old report\n");
        let mut args = vec![
            "folder",
            "compare",
            "l",
            "r",
            "--content",
            "rules",
            "--output",
            "report.txt",
        ];
        if json {
            args.insert(0, "--json");
        }
        let mut child = spawn(&s, &args);
        wait_for_name(&s, &mut child, ".", "converting");
        sigint(&child);
        let status = wait(&mut child, LIMIT);

        assert_eq!(status.code(), Some(100), "{status:?}");
        assert_eq!(
            std::fs::read(s.join("report.txt")).unwrap(),
            b"the old report\n"
        );
        assert_eq!(leftovers(&s, "."), Vec::<String>::new());
    }
}

/// The same through `text compare`, whose comparison fails by itself when it sees the cancel.
#[test]
fn cancel_of_a_text_compare_exits_100_and_keeps_the_report() {
    let s = Sandbox::new();
    slow_format(&s, "converting");
    s.file("a.slow", b"left\n");
    s.file("b.slow", b"right\n");
    s.file("report.txt", b"the old report\n");
    let mut child = spawn(
        &s,
        &[
            "text",
            "compare",
            "a.slow",
            "b.slow",
            "--output",
            "report.txt",
        ],
    );
    wait_for_name(&s, &mut child, ".", "converting");
    sigint(&child);
    let status = wait(&mut child, LIMIT);

    assert_eq!(status.code(), Some(100), "{status:?}");
    assert_eq!(
        std::fs::read(s.join("report.txt")).unwrap(),
        b"the old report\n"
    );
}

/// The second Ctrl-C does not wait for the job. A job blocked on standard input cannot see the cancel, so the first
/// leaves it running and only the second ends it: the log tells which press the process saw, and that it was the
/// second that ended it. Standard input stays open throughout, so the job cannot end in any other way.
#[test]
fn the_second_ctrl_c_exits_at_once_where_a_cancel_cannot_be_seen() {
    let s = Sandbox::new();
    s.file("b.txt", "other\n");
    let mut child = spawn(&s, &["quick", "-", "b.txt"]);
    let stdin = child.stdin.take().expect("stdin");
    wait_for_log(&s, &mut child, ARMED);

    sigint(&child);
    wait_for_log(&s, &mut child, CANCELLING);
    sigint(&child);
    let status = wait(&mut child, LIMIT);

    assert_eq!(status.code(), Some(100), "{status:?}");
    assert!(logged(&s, EXITING), "the first Ctrl-C ended the job");
    drop(stdin);
}

/// A command without a job installs nothing: Ctrl-C ends it the way the OS does, also when it waits. Standard input
/// takes more than a pipe holds, so when the write returns the command is reading, and it reads until the end.
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

/// A process that inherits SIGINT as ignored (`cmd &` without job control) keeps it ignored: the job does not see
/// the signal, and ends as it would have.
#[test]
fn an_inherited_ignored_ctrl_c_stays_ignored() {
    let s = Sandbox::new();
    s.file("b.txt", "other\n");
    let mut shell = Command::new("sh");
    shell
        .args(["-c", "trap '' INT; exec \"$0\" \"$@\""])
        .arg(hikasa_exe());
    let mut child = configure(&s, shell, &["quick", "-", "b.txt"]);
    let mut stdin = child.stdin.take().expect("stdin");
    wait_until(&mut child, "the handler's decision", || {
        logged(&s, INHERITED) || logged(&s, ARMED)
    });

    sigint(&child);
    stdin.write_all(b"text\n").expect("feed stdin");
    drop(stdin);
    let status = wait(&mut child, LIMIT);

    assert!(!logged(&s, CANCELLING));
    assert_ne!(status.code(), Some(100), "{status:?}");
    assert!(status.code().is_some(), "{status:?}");
}

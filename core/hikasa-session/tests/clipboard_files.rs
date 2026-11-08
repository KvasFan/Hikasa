//! M2 W9c acceptance A9.20 (core, D-252): the clipboard's file lists. `FixedClipboard` and `FileClipboard` round
//! trips, the `Unavailable` defaults of a provider that has no lists, and refusals that pass through.

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_session::clipboard::{
    ClipboardContent, ClipboardError, ClipboardProvider, FixedClipboard, NoClipboard,
};
use hikasa_session::host::FileClipboard;

fn paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/data/left/a file.txt"),
        PathBuf::from("/data/left/sub"),
        PathBuf::from("/data/left/b,c;d#e%.bin"),
    ]
}

#[test]
fn a9_20_the_fixed_clipboard_records_and_serves_file_lists() {
    let clipboard = FixedClipboard::text("some text");
    assert!(matches!(clipboard.read_files(), Err(ClipboardError::Empty)));
    clipboard.write_files(&paths()).unwrap();
    assert_eq!(clipboard.file_lists(), vec![paths()]);
    // The list is now the content: a file manager reads it back, text readers see none.
    assert_eq!(clipboard.read_files().unwrap(), paths());
    assert!(matches!(clipboard.read(), Err(ClipboardError::Empty)));
    // Text placed later replaces it.
    clipboard.set_text("again");
    assert!(matches!(clipboard.read_files(), Err(ClipboardError::Empty)));
    assert_eq!(
        clipboard.read().unwrap(),
        ClipboardContent::Text("again".into())
    );
    let copied = FixedClipboard::files(vec![PathBuf::from("/x")]);
    assert_eq!(copied.read_files().unwrap(), vec![PathBuf::from("/x")]);
    copied.set_files(Vec::new());
    assert!(matches!(copied.read_files(), Err(ClipboardError::Empty)));
}

#[test]
fn a9_20_refusals_pass_through_and_the_defaults_are_unavailable() {
    let unavailable = FixedClipboard::unavailable("no display");
    assert!(matches!(
        unavailable.write_files(&paths()),
        Err(ClipboardError::Unavailable(why)) if why == "no display"
    ));
    assert!(matches!(
        unavailable.read_files(),
        Err(ClipboardError::Unavailable(_))
    ));
    assert!(unavailable.file_lists().is_empty());
    let failing = FixedClipboard::failing("busy");
    assert!(matches!(
        failing.write_files(&paths()),
        Err(ClipboardError::Failed(_))
    ));
    assert!(matches!(
        failing.read_files(),
        Err(ClipboardError::Failed(_))
    ));
    // A provider without lists says so (the trait's defaults), through an `Arc` too.
    let none: Arc<dyn ClipboardProvider> = Arc::new(NoClipboard);
    assert!(matches!(
        none.write_files(&paths()),
        Err(ClipboardError::Unavailable(why)) if why == "file lists"
    ));
    assert!(matches!(
        none.read_files(),
        Err(ClipboardError::Unavailable(why)) if why == "file lists"
    ));
    let shared: Arc<FixedClipboard> = Arc::new(FixedClipboard::empty());
    let provider: Arc<dyn ClipboardProvider> = shared.clone();
    provider.write_files(&paths()).unwrap();
    assert_eq!(shared.file_lists(), vec![paths()]);
    assert_eq!(provider.read_files().unwrap(), paths());
}

#[test]
fn a9_20_the_file_clipboard_keeps_one_path_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("clipboard.txt");
    let clipboard = FileClipboard(file.clone());
    assert!(matches!(
        clipboard.read_files(),
        Err(ClipboardError::Unavailable(_))
    ));
    clipboard.write_files(&paths()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "/data/left/a file.txt\n/data/left/sub\n/data/left/b,c;d#e%.bin\n"
    );
    assert_eq!(clipboard.read_files().unwrap(), paths());
    // The same file read as text holds the lines (a Paste of text sees them as specs).
    assert!(matches!(
        clipboard.read().unwrap(),
        ClipboardContent::Text(text) if text.lines().count() == 3
    ));
    // `\r\n` line ends and empty lines are taken too.
    std::fs::write(&file, "/a\r\n\r\n/b\n").unwrap();
    assert_eq!(
        clipboard.read_files().unwrap(),
        vec![PathBuf::from("/a"), PathBuf::from("/b")]
    );
    std::fs::write(&file, "\n\n").unwrap();
    assert!(matches!(clipboard.read_files(), Err(ClipboardError::Empty)));
    // A path with a line break cannot be one line.
    let err = clipboard
        .write_files(&[PathBuf::from("/a\nb")])
        .unwrap_err();
    assert!(matches!(err, ClipboardError::Failed(_)), "{err:?}");
    // A carriage return neither (it would end a line read back; review F6, M25).
    let err = clipboard
        .write_files(&[PathBuf::from("/a\rb")])
        .unwrap_err();
    assert!(matches!(err, ClipboardError::Failed(_)), "{err:?}");
}

/// Linux: a name that is not valid UTF-8 keeps its bytes through the file.
#[cfg(target_os = "linux")]
#[test]
fn a9_20_a_name_that_is_not_unicode_keeps_its_bytes() {
    use std::os::unix::ffi::OsStrExt as _;
    let dir = tempfile::tempdir().unwrap();
    let clipboard = FileClipboard(dir.path().join("clipboard.txt"));
    let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/data/caf\xe9.txt"));
    clipboard.write_files(std::slice::from_ref(&odd)).unwrap();
    assert_eq!(clipboard.read_files().unwrap(), vec![odd]);
}

/// A clipboard text's paths (W9c review F7): absolute local paths and `file://` URIs of this computer, decoded;
/// prose, relative paths, other hosts and schemes, `-` and `clipboard://` are not paths.
#[test]
fn pasted_text_names_only_local_absolute_paths() {
    use hikasa_session::clipboard::pasted_paths;
    #[cfg(unix)]
    {
        let text = "  /data/a b.txt  \r\nfile:///data/c%20d.txt\nFILE://localhost/data/e.txt\n\
                    file://server/share/f.txt\nhttp://example.com/x\nrelative.txt\nsome words here\n-\n\
                    clipboard://\n\nfile:///data/bad%zz\n";
        assert_eq!(
            pasted_paths(text),
            [
                PathBuf::from("/data/a b.txt"),
                PathBuf::from("/data/c d.txt"),
                PathBuf::from("/data/e.txt"),
            ]
        );
        use std::os::unix::ffi::OsStrExt as _;
        assert_eq!(
            pasted_paths("file:///d/caf%E9\n"),
            [PathBuf::from(std::ffi::OsStr::from_bytes(b"/d/caf\xe9"))]
        );
    }
    #[cfg(windows)]
    {
        let text = " C:\\data\\a b.txt \r\nfile:///C:/data/c%20d.txt\n\\data\\no-drive.txt\nfile://srv/s/x\n";
        // Compared as text: a path with `/` would equal one with `\` as a `PathBuf` (W9 close).
        let texts: Vec<std::ffi::OsString> = pasted_paths(text)
            .into_iter()
            .map(PathBuf::into_os_string)
            .collect();
        assert_eq!(texts, ["C:\\data\\a b.txt", "C:\\data\\c d.txt"]);
    }
    assert!(pasted_paths("The clipboard holds prose.\nAnd more.").is_empty());
}

/// Clipboard writes in the order they were asked, the latest winning (review F12): a write whose ticket is older than
/// the last one taken is skipped; plain writes take their ticket when they write.
#[test]
fn ordered_writes_let_the_latest_win() {
    use hikasa_session::clipboard::{ClipboardPayload, OrderedClipboard};
    let fixed = Arc::new(FixedClipboard::text("t"));
    let ordered = OrderedClipboard::new(Arc::clone(&fixed) as Arc<dyn ClipboardProvider>);
    let first = ordered.ticket();
    let second = ordered.ticket();
    // The second job ends first: it writes; the first one, older, writes nothing.
    let payload = ClipboardPayload {
        text: "names".into(),
        html: None,
    };
    assert!(
        ordered
            .in_turn(second, |c| c.write_payload(&payload))
            .unwrap()
    );
    assert!(!ordered.in_turn(first, |c| c.write_files(&paths())).unwrap());
    assert_eq!(fixed.payloads(), [payload]);
    assert!(fixed.file_lists().is_empty());
    // In order, each writes; a plain write is a ticket of its own.
    let third = ordered.ticket();
    assert!(ordered.in_turn(third, |c| c.write_files(&paths())).unwrap());
    ordered.write_files(&paths()).unwrap();
    assert_eq!(fixed.file_lists().len(), 2);
    // Reads pass through; a refusal reaches the writer.
    assert_eq!(ordered.read_files().unwrap(), paths());
    let refusing = OrderedClipboard::new(Arc::new(FixedClipboard::failing("no")));
    let t = refusing.ticket();
    assert!(matches!(
        refusing.in_turn(t, |c| c.write_files(&paths())),
        Err(ClipboardError::Failed(_))
    ));
}

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

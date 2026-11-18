//! M2 W11c acceptance A11.32 (automated part): a non-UTF-8 file name (Linux) through open, save and Find in Files;
//! Windows drive, UNC and `\\?\` paths as Save As and the header's path edit take them (a pure table on every
//! platform: both resolve the typed text as a spec, `Service::parse_spec`; on Windows a Save As to a drive path on
//! disk too); CRLF documents keep their line endings through an edit and a save.

use hikasa_core::Side;
use hikasa_core::spec::{PathStyle, Spec, SpecContext};

use crate::commands::CommandId;
use crate::tests_text::Fixture;

/// The texts a Windows user types in Save As or the header's path edit, and the local path each one is.
#[test]
fn a11_32_windows_paths_are_local_files() {
    let ctx = SpecContext {
        style: PathStyle::Windows,
        cwd: Some(std::path::PathBuf::from("C:\\work")),
        home: None,
    };
    for (typed, canonical) in [
        ("C:\\data\\out.txt", "C:\\data\\out.txt"),
        ("c:/data/out.txt", "C:\\data\\out.txt"),
        (
            "\\\\server\\share\\dir\\out.txt",
            "\\\\server\\share\\dir\\out.txt",
        ),
        ("\\\\?\\C:\\very\\long\\out.txt", "C:\\very\\long\\out.txt"),
        ("out.txt", "C:\\work\\out.txt"),
    ] {
        let spec = Spec::parse(typed, &ctx).unwrap_or_else(|e| panic!("{typed}: {e}"));
        assert!(matches!(spec, Spec::Local(_)), "{typed}: {spec:?}");
        let shown = spec.canonical();
        assert!(
            shown.eq_ignore_ascii_case(canonical) || shown.ends_with(&canonical[2..]),
            "{typed}: {shown}"
        );
    }
}

/// On Windows (CI, wine): Save As to a drive-letter path and to its `\\?\` form writes the file there.
#[cfg(windows)]
#[test]
fn a11_32_save_as_to_a_drive_path_on_disk() {
    let mut f = Fixture::new("a\r\nb\r\n", "a\r\nc\r\n");
    for (k, verbatim) in [false, true].into_iter().enumerate() {
        let target = f.world.path(&format!("out/saved{k}.txt"));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let typed = if verbatim {
            format!("\\\\?\\{}", target.display())
        } else {
            target.display().to_string()
        };
        f.place(Side::Left, 0, 0);
        f.type_text("x");
        f.settle();
        f.run(CommandId::SaveAs);
        f.settle();
        f.browse_answer(
            crate::browse::BrowseTarget::TextSaveAs {
                view: f.view,
                side: Side::Left,
                local: false,
            },
            Some(std::path::Path::new(&typed)),
        );
        f.settle();
        let saved = std::fs::read_to_string(&target).expect("saved");
        assert!(saved.starts_with("x"), "{saved:?}");
    }
}

/// CRLF documents: an edit and a save keep every line's CR LF (every platform).
#[test]
fn a11_32_crlf_documents_keep_their_line_endings() {
    let mut f = Fixture::new("one\r\ntwo\r\nthree\r\n", "one\r\nTWO\r\nthree\r\n");
    f.place(Side::Left, 1, 0);
    f.type_text("x");
    f.settle();
    f.run(CommandId::Save);
    f.settle();
    let saved = std::fs::read(&f.paths[0]).unwrap();
    assert_eq!(saved, b"one\r\nxtwo\r\nthree\r\n");
}

/// Linux: a file whose name is not UTF-8 opens, saves and is found by Find in Files.
#[cfg(target_os = "linux")]
#[test]
fn a11_32_a_non_utf8_file_name_opens_saves_and_is_found() {
    use std::os::unix::ffi::OsStrExt;
    let world = crate::tests::World::new();
    let dir = world.path("raw");
    std::fs::create_dir_all(&dir).unwrap();
    let left = dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt"));
    let right = dir.join("plain.txt");
    std::fs::write(&left, "needle one\nsame\n").unwrap();
    std::fs::write(&right, "needle two\nsame\n").unwrap();
    // The view's request carries the name as the OS gives it (the command line's IPC words are UTF-8, D-101: a view
    // of such a file is opened by the window itself, from its request).
    let mut app = world.app(None);
    let plain = right.to_string_lossy().into_owned();
    let mut request = world
        .service
        .open_view_request(
            hikasa_session::ipc::ViewKind::TextCompare,
            &[plain.clone(), plain.clone()],
            &hikasa_session::ipc::OpenOptions::default(),
        )
        .expect("a request");
    if let hikasa_session::api::ViewRequest::Text { request, .. } = &mut request {
        request.left = left.clone().into_os_string();
    }
    let target = app.open_target();
    let view = app.open_local(
        hikasa_session::ipc::ViewKind::TextCompare,
        request,
        &hikasa_session::ipc::OpenOptions::default(),
        vec![left.to_string_lossy().into_owned(), plain],
        target,
    );
    let mut f = Fixture {
        world,
        app,
        view,
        clip: std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::text("")),
        paths: [left.clone(), right],
        now: 1.0,
    };
    f.settle();
    assert!(f.m().loaded(), "the view opened the file");
    f.place(Side::Left, 1, 0);
    f.type_text("z");
    f.settle();
    f.run(CommandId::Save);
    f.settle();
    assert_eq!(std::fs::read(&left).unwrap(), b"needle one\nzsame\n");
    // Find in Files over the folder: a hit in each file, the non-UTF-8 one included.
    let w = f.app.win();
    let wake: crate::job::Wake = std::sync::Arc::new(|| {});
    let svc = f.world.service.clone();
    if let Some(win) = f.app.0.window_mut(w) {
        win.find_files.start(
            &svc,
            &wake,
            hikasa_session::api::TextFindRequest {
                pattern: "needle".into(),
                roots: vec![dir.into_os_string()],
                recursive: true,
                ..hikasa_session::api::TextFindRequest::default()
            },
        );
    }
    let began = std::time::Instant::now();
    loop {
        f.settle();
        let p = &mut f.app.0.window_mut(w).unwrap().find_files;
        p.poll();
        if !p.running() {
            break;
        }
        assert!(began.elapsed() < std::time::Duration::from_secs(30));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let p = &f.app.0.window_mut(w).unwrap().find_files;
    // Both files hit (and the save's backup, which holds the old text).
    assert!(p.error.is_none(), "{:?}", p.error);
    let names: Vec<&str> = p.files.iter().map(|e| e.spec.as_str()).collect();
    assert!(names.iter().any(|n| n.ends_with("plain.txt")), "{names:?}");
    assert!(
        names
            .iter()
            .any(|n| n.contains("caf") && !n.contains(".bak") && n.ends_with(".txt")),
        "{names:?}"
    );
}

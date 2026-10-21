//! The legacy `gui open` argument tables (CLI-001…003, D-51), `infer_view_kind` (CLI-002), `gui_exe_candidates`,
//! `ViewKind::parse` and the `--folders` parser (§6.2.8).

mod common;

use std::ffi::OsString;
use std::path::PathBuf;

use hikasa_core::{Spec, WarningCode};
use hikasa_folder::FolderMode;
use hikasa_session::api::gui::{
    gui_exe_candidates, infer_view_kind, normalize_legacy_args, parse_open_args,
};
use hikasa_session::api::parse_folder_mode;
use hikasa_session::ipc::ViewKind;
use hikasa_session::{ApiError, Platform};

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

fn strings(list: &[OsString]) -> Vec<String> {
    list.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn normalize_table() {
    let unix = Platform::Linux;
    let windows = Platform::Windows;
    let rows: &[(&[&str], Platform, &[&str])] = &[
        (&["-ro", "a", "b"], unix, &["--ro", "a", "b"]),
        (&["/ro", "a", "b"], windows, &["--ro", "a", "b"]),
        (&["/ro", "a", "b"], unix, &["/ro", "a", "b"]),
        (&["--readonly"], unix, &["--readonly"].map(|_| "--ro")),
        (
            &["-RO1", "--LeftReadOnly", "-rro"],
            unix,
            &["--ro1", "--ro1", "--ro2"],
        ),
        (
            &["--lefttitle=x y", "-righttitle", "R"],
            unix,
            &["--title1=x y", "--title2", "R"],
        ),
        (&["--centertitle", "-x-"], unix, &["--title3", "-x-"]),
        (&["/outputtitle:o"], windows, &["/outputtitle:o"]),
        (&["/outputtitle=o"], windows, &["--title4=o"]),
        (&["--fileviewer=Text Compare"], unix, &["--fv=Text Compare"]),
        (
            &["-ignoreunimportant", "--ignore-unimportant"],
            unix,
            &["--iu", "--iu"],
        ),
        (&["-quickcompare=crc", "--qc"], unix, &["--qc=crc", "--qc"]),
        (
            &["-expandall", "/expandall"],
            windows,
            &["--expandall", "--expandall"],
        ),
        (
            &["-", "--", "-ro", "/ro"],
            windows,
            &["-", "--", "-ro", "/ro"],
        ),
        (
            &["--unknown", "-zz", "/zz"],
            windows,
            &["--unknown", "-zz", "/zz"],
        ),
        (
            &["-vcsleft=v", "/vcs4", "w"],
            windows,
            &["--vcs1=v", "--vcs4", "w"],
        ),
        (
            &["-q", "-h", "/?"],
            windows,
            &["--quiet", "--help", "--help"],
        ),
        (&["/tmp/a.c", "/tmp/b.c"], unix, &["/tmp/a.c", "/tmp/b.c"]),
    ];
    for (input, platform, expected) in rows {
        let out = normalize_legacy_args(&args(input), *platform);
        assert_eq!(strings(&out), *expected, "{input:?} on {platform:?}");
        // Idempotent.
        assert_eq!(normalize_legacy_args(&out, *platform), out, "{input:?}");
    }
}

#[test]
fn parse_specs_and_switches() {
    let parsed = parse_open_args(
        &args(&[
            "a.c",
            "-ro1",
            "--title1",
            "Left side",
            "--title2=R",
            "--wait",
            "--no-launch",
            "b.c",
            "--filters=*.c;-obj\\",
            "--expandall",
            "--format",
            "c-cpp",
            "--iu",
            "--stdin-name=x.py",
            "--fv=text-compare",
            "--json",
            "-q",
            "--no-color",
            "--solo",
        ]),
        Platform::Linux,
    )
    .unwrap();
    let request = &parsed.request;
    assert_eq!(strings(&request.specs), vec!["a.c", "b.c"]);
    assert_eq!(request.kind, Some(ViewKind::TextCompare));
    assert!(request.wait);
    let options = &request.options;
    assert_eq!(options.titles[0].as_deref(), Some("Left side"));
    assert_eq!(options.titles[1].as_deref(), Some("R"));
    assert_eq!(options.titles[2], None);
    assert!(options.read_only.left && !options.read_only.all && !options.read_only.right);
    assert_eq!(options.filters.as_deref(), Some("*.c;-obj\\"));
    assert!(options.expand_all && options.ignore_unimportant);
    assert_eq!(options.format.as_deref(), Some("c-cpp"));
    assert!(options.extra.is_empty());
    assert_eq!(parsed.stdin_name.as_deref(), Some("x.py"));
    assert!(parsed.no_launch && parsed.json && parsed.quiet && parsed.no_color && parsed.solo);
    assert!(!parsed.verbose && !parsed.help && parsed.warnings.is_empty());
}

#[test]
fn legacy_spellings_parse_like_canonical_ones() {
    let legacy = parse_open_args(
        &args(&[
            "/ro",
            "/lefttitle=x",
            "/fileviewer=Folder Compare",
            "C:\\a",
            "C:\\b",
        ]),
        Platform::Windows,
    )
    .unwrap();
    assert!(legacy.request.options.read_only.all);
    assert_eq!(legacy.request.options.titles[0].as_deref(), Some("x"));
    assert_eq!(legacy.request.kind, Some(ViewKind::FolderCompare));
    assert_eq!(strings(&legacy.request.specs), vec!["C:\\a", "C:\\b"]);
    // On Unix a leading slash is a path.
    let unix = parse_open_args(&args(&["/ro", "/x"]), Platform::Linux).unwrap();
    assert_eq!(strings(&unix.request.specs), vec!["/ro", "/x"]);
    assert!(!unix.request.options.read_only.all);
}

#[test]
fn stdin_clipboard_and_double_dash_are_specs() {
    let parsed = parse_open_args(
        &args(&["-", "clipboard://", "--", "-ro", "--wait"]),
        Platform::Linux,
    )
    .unwrap();
    assert_eq!(
        strings(&parsed.request.specs),
        vec!["-", "clipboard://", "-ro", "--wait"]
    );
    assert!(!parsed.request.wait);
}

#[test]
fn forwarded_switches_warn_switch_ignored() {
    let parsed = parse_open_args(
        &args(&[
            "--automerge",
            "--reviewconflicts",
            "-mergeoutput=out.txt",
            "--qc",
            "-center",
            "c.txt",
            "/favorleft",
            "--vcs2=v2",
        ]),
        Platform::Windows,
    )
    .unwrap();
    let extra = &parsed.request.options.extra;
    let pairs: Vec<(&str, &str)> = extra
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("automerge", ""),
            ("center", "c.txt"),
            ("favorleft", ""),
            ("mergeoutput", "out.txt"),
            ("qc", ""),
            ("reviewconflicts", ""),
            ("vcs2", "v2"),
        ]
    );
    assert_eq!(parsed.warnings.len(), 7);
    assert!(
        parsed
            .warnings
            .iter()
            .all(|w| w.code == WarningCode::SwitchIgnored)
    );
    assert!(parsed.request.specs.is_empty());
}

#[test]
fn gui_only_switches_and_values() {
    let parsed = parse_open_args(
        &args(&[
            "--config-dir",
            "/cfg",
            "--log=/l.txt",
            "--verbose",
            "--help",
        ]),
        Platform::Linux,
    )
    .unwrap();
    assert_eq!(parsed.config_dir, Some(PathBuf::from("/cfg")));
    assert_eq!(parsed.log, Some(PathBuf::from("/l.txt")));
    assert!(parsed.verbose && parsed.help);
}

#[test]
fn invalid_arguments() {
    for bad in [
        &["--bogus"][..],
        &["-zz", "a"],
        &["--ro=1"],
        &["--title1"],
        &["--fv=Spreadsheet Thing"],
        &["--wait=yes"],
    ] {
        assert!(
            matches!(
                parse_open_args(&args(bad), Platform::Linux),
                Err(ApiError::InvalidArgument(_))
            ),
            "{bad:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_specs_and_path_values_are_kept() {
    use std::os::unix::ffi::OsStringExt;
    let raw = OsString::from_vec(vec![b'a', 0xff, b'.', b'c']);
    let parsed = parse_open_args(
        &[raw.clone(), OsString::from("--log"), raw.clone()],
        Platform::Linux,
    )
    .unwrap();
    assert_eq!(parsed.request.specs, vec![raw.clone()]);
    assert_eq!(parsed.log, Some(PathBuf::from(raw.clone())));
    assert!(matches!(
        parse_open_args(&[OsString::from("--title1"), raw], Platform::Linux),
        Err(ApiError::InvalidArgument(_))
    ));
}

#[test]
fn view_kind_parse() {
    for (text, kind) in [
        ("Text Compare", ViewKind::TextCompare),
        ("text-compare", ViewKind::TextCompare),
        ("TEXT_COMPARE", ViewKind::TextCompare),
        ("Folder Compare", ViewKind::FolderCompare),
        ("folder sync", ViewKind::FolderSync),
        ("Registry Compare", ViewKind::RegistryCompare),
        ("Text Patch", ViewKind::TextPatch),
    ] {
        assert_eq!(ViewKind::parse(text), Some(kind), "{text}");
    }
    assert_eq!(ViewKind::parse("unknown"), None);
    assert_eq!(ViewKind::parse("Spreadsheet"), None);
    for kind in ViewKind::ALL {
        assert_eq!(ViewKind::parse(kind.as_str()), Some(kind));
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            format!("\"{}\"", kind.as_str())
        );
    }
}

#[test]
fn exe_candidates() {
    let unix = gui_exe_candidates(
        Some(std::path::Path::new("/opt/hk")),
        Some(std::ffi::OsStr::new("/usr/bin::/opt/hk:/bin")),
        Platform::Linux,
    );
    assert_eq!(
        unix,
        vec![
            PathBuf::from("/opt/hk").join("hikasa-gui"),
            PathBuf::from("/usr/bin").join("hikasa-gui"),
            PathBuf::from("/bin").join("hikasa-gui"),
        ]
    );
    let windows = gui_exe_candidates(
        Some(std::path::Path::new(r"C:\hk")),
        Some(std::ffi::OsStr::new(r#"C:\Windows;"C:\Program Files\x";;"#)),
        Platform::Windows,
    );
    assert_eq!(
        windows,
        vec![
            PathBuf::from(r"C:\hk").join("hikasa-gui.exe"),
            PathBuf::from(r"C:\Windows").join("hikasa-gui.exe"),
            PathBuf::from(r"C:\Program Files\x").join("hikasa-gui.exe"),
        ]
    );
    assert!(gui_exe_candidates(None, None, Platform::MacOs).is_empty());
}

#[test]
fn folder_modes() {
    assert_eq!(
        parse_folder_mode("always").unwrap(),
        FolderMode::AlwaysShowFolders
    );
    assert_eq!(
        parse_folder_mode("structure").unwrap(),
        FolderMode::CompareStructure
    );
    assert_eq!(
        parse_folder_mode("files-only").unwrap(),
        FolderMode::OnlyFiles
    );
    assert_eq!(
        parse_folder_mode("Files_Only").unwrap(),
        FolderMode::OnlyFiles
    );
    assert!(parse_folder_mode("all").is_err());
}

#[test]
fn infer_view_kind_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let work = tmp.path().join("work");
    let a = common::write(&work, "a.c", b"int a;\n");
    let b = common::write(&work, "b.c", b"int b;\n");
    let png = common::write(&work, "p.png", b"\x89PNG");
    let d1 = work.join("d1");
    let d2 = work.join("d2");
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();
    let spec = |p: &std::path::Path| service.parse_spec(p.as_os_str()).unwrap();
    assert_eq!(
        infer_view_kind(&service, &[spec(&d1), spec(&d2)], None).unwrap(),
        ViewKind::FolderCompare
    );
    assert_eq!(
        infer_view_kind(&service, &[spec(&a), spec(&b)], None).unwrap(),
        ViewKind::TextCompare
    );
    // Other kinds open in Text Compare until their views exist (a warning is logged).
    assert_eq!(
        infer_view_kind(&service, &[spec(&png), spec(&png)], None).unwrap(),
        ViewKind::TextCompare
    );
    // A missing side takes the other's kind.
    assert_eq!(
        infer_view_kind(&service, &[spec(&work.join("new.c")), spec(&b)], None).unwrap(),
        ViewKind::TextCompare
    );
    assert!(matches!(
        infer_view_kind(&service, &[spec(&d1), spec(&a)], None),
        Err(ApiError::InvalidArgument(_))
    ));
    assert!(matches!(
        infer_view_kind(&service, &[spec(&a)], None),
        Err(ApiError::InvalidArgument(_))
    ));
    assert!(matches!(
        infer_view_kind(&service, &[spec(&a), spec(&b), spec(&a)], None),
        Err(ApiError::InvalidArgument(_))
    ));
    assert!(matches!(
        infer_view_kind(
            &service,
            &[spec(&work.join("x")), spec(&work.join("y"))],
            None
        ),
        Err(ApiError::LoadFailed { .. })
    ));
    // A forced kind wins.
    assert_eq!(
        infer_view_kind(&service, &[spec(&a)], Some(ViewKind::TextEdit)).unwrap(),
        ViewKind::TextEdit
    );
    // Stdin uses the name hint for the format; the kind is TextCompare.
    let with_stdin = common::service_with(
        tmp.path(),
        Some(common::blob(b"x = 1\n", Some("s.py"))),
        std::sync::Arc::new(common::UpperRunner::default()),
    );
    assert_eq!(
        infer_view_kind(&with_stdin, &[Spec::Stdin, spec(&b)], None).unwrap(),
        ViewKind::TextCompare
    );
}

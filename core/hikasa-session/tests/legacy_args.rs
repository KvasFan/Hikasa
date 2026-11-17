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

/// A6.31 (CLI-003, D-190): `--sync` makes a Folder Sync view, and with `--fv` the last wins; `--nobackups` is an option of the
/// view. Neither is forwarded with `switch_ignored` any more.
#[test]
fn sync_and_nobackups_table() {
    let rows: &[(&[&str], Platform, Option<ViewKind>, bool)] = &[
        (
            &["--sync", "a", "b"],
            Platform::Linux,
            Some(ViewKind::FolderSync),
            false,
        ),
        (
            &["-sync", "a", "b"],
            Platform::Linux,
            Some(ViewKind::FolderSync),
            false,
        ),
        (
            &["/sync", "a", "b"],
            Platform::Windows,
            Some(ViewKind::FolderSync),
            false,
        ),
        (
            &["--SYNC"],
            Platform::Linux,
            Some(ViewKind::FolderSync),
            false,
        ),
        (
            &["--fv", "Text Compare", "--sync"],
            Platform::Linux,
            Some(ViewKind::FolderSync),
            false,
        ),
        (
            &["--sync", "--fv=text-compare"],
            Platform::Linux,
            Some(ViewKind::TextCompare),
            false,
        ),
        (
            &["--sync", "-fileviewer", "Folder Compare"],
            Platform::Linux,
            Some(ViewKind::FolderCompare),
            false,
        ),
        (&["--nobackups", "a", "b"], Platform::Linux, None, true),
        (
            &["/NoBackups", "--sync"],
            Platform::Windows,
            Some(ViewKind::FolderSync),
            true,
        ),
    ];
    for (line, platform, kind, no_backups) in rows {
        let parsed = parse_open_args(&args(line), *platform).unwrap();
        assert_eq!(parsed.request.kind, *kind, "{line:?}");
        assert_eq!(parsed.request.options.no_backups, *no_backups, "{line:?}");
        assert!(
            parsed.warnings.is_empty(),
            "{line:?}: {:?}",
            parsed.warnings
        );
        assert!(parsed.request.options.extra.is_empty(), "{line:?}");
    }
    assert!(parse_open_args(&args(&["--sync=yes"]), Platform::Linux).is_err());
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

/// A8.18 (CLI-002, CLI-003, D-228): `@script` in the shared parser. The first positional argument starting with `@`
/// is the script, every other positional argument (before or after it, and after `--`) one of its arguments, in order;
/// `@` alone is a usage error; `./@x` is a spec; a second `@word` is an argument. `--closescript` and `--silent` are
/// no longer `switch_ignored`: they apply to the script (`--closescript` without a script warns that it applies to a
/// script only; `--silent` without one stays for a settings package, D-127).
#[test]
fn at_script_table() {
    for platform in [Platform::Linux, Platform::Windows] {
        let parsed = parse_open_args(&args(&["@s.txt", "a", "b c"]), platform).unwrap();
        let script = parsed.script.as_ref().unwrap();
        assert_eq!(script.file, "s.txt");
        assert_eq!(strings(&script.args), ["a", "b c"]);
        assert!(parsed.request.specs.is_empty());
        assert!(parsed.warnings.is_empty());
        assert!(!parsed.silent && !parsed.closescript);
    }
    let parsed = parse_open_args(
        &args(&[
            "x",
            "@s.txt",
            "--wait",
            "-closescript",
            "--silent",
            "@y",
            "--",
            "-z",
        ]),
        Platform::Linux,
    )
    .unwrap();
    let script = parsed.script.unwrap();
    assert_eq!(script.file, "s.txt");
    assert_eq!(strings(&script.args), ["x", "@y", "-z"]);
    assert!(parsed.request.wait && parsed.closescript && parsed.silent);
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    assert!(
        parsed.request.options.extra.is_empty(),
        "nothing is forwarded"
    );
    // `@` alone.
    let err = parse_open_args(&args(&["@"]), Platform::Linux).unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
    assert!(err.to_string().contains("./@name"), "{err}");
    // A file whose name starts with `@`, and `@` after `--`, are specs.
    for list in [&["./@x", "b"][..], &["--", "@x", "b"]] {
        let parsed = parse_open_args(&args(list), Platform::Linux).unwrap();
        assert!(parsed.script.is_none(), "{list:?}");
        assert_eq!(parsed.request.specs.len(), 2);
    }
    // Without a script: `--closescript` warns, and `--silent` warns as in M1 (review F3) unless the one argument is a
    // settings package (D-127), which it imports without the wizard.
    let parsed = parse_open_args(
        &args(&["a", "b", "--closescript", "--silent"]),
        Platform::Linux,
    )
    .unwrap();
    assert!(parsed.closescript && parsed.silent && parsed.script.is_none());
    let codes: Vec<String> = parsed.warnings.iter().map(|w| w.message.clone()).collect();
    assert_eq!(
        codes,
        [
            "switch `--closescript` applies to a script (`@<file>`) only",
            "switch `--silent` applies to a script (`@<file>`) or a settings package only"
        ]
    );
    assert!(
        parsed
            .warnings
            .iter()
            .all(|w| w.code == WarningCode::SwitchIgnored)
    );
    for package in ["my.hikasa-pkg", "C.HIKASA-PKG"] {
        let parsed = parse_open_args(&args(&[package, "--silent"]), Platform::Linux).unwrap();
        assert!(parsed.silent);
        assert!(
            parsed.warnings.is_empty(),
            "{package}: {:?}",
            parsed.warnings
        );
    }
    // A script opens no view: each view switch it drops is `switch_ignored`, once (review F3).
    let parsed = parse_open_args(
        &args(&[
            "@s.txt",
            "l",
            "--ro",
            "--sync",
            "--nobackups",
            "-ro",
            "--wait",
        ]),
        Platform::Linux,
    )
    .unwrap();
    assert!(parsed.script.is_some() && parsed.request.wait);
    let codes: Vec<String> = parsed.warnings.iter().map(|w| w.message.clone()).collect();
    assert_eq!(
        codes,
        [
            "switch `--ro` applies to a view; a script (`@<file>`) opens none",
            "switch `--sync` applies to a view; a script (`@<file>`) opens none",
            "switch `--nobackups` applies to a view; a script (`@<file>`) opens none"
        ]
    );
    assert!(
        parsed
            .warnings
            .iter()
            .all(|w| w.code == WarningCode::SwitchIgnored)
    );
    // The usage names the form and the two switches.
    let usage = hikasa_session::api::open_usage();
    assert!(usage.contains("@<script>"), "{usage}");
    assert!(usage.contains("--closescript"), "{usage}");
    assert!(!usage.contains("forwarded with a warning): --automerge, --center, --mergeoutput, --force, --reviewconflicts, --favorleft, --favorright, --closescript"));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let raw = vec![
            OsString::from_vec(b"@s\xFF.txt".to_vec()),
            OsString::from("a"),
        ];
        let parsed = parse_open_args(&raw, Platform::Linux).unwrap();
        assert_eq!(
            parsed.script.unwrap().file,
            OsString::from_vec(b"s\xFF.txt".to_vec()),
            "the bytes of the name are kept"
        );
    }
}

/// A10.17 (CLI-003, D-278): `--edit`, `--savetarget`, `--ro*` and `--nobackups` into `OpenArgs` and `OpenOptions`. `--edit`
/// makes Text Edit views (with `--fv` or `--sync` the last wins, D-190); `--savetarget` keeps its value as typed (made
/// absolute by `resolve_open`); none of them leaves `switch_ignored` or `extra`.
#[test]
fn edit_savetarget_read_only_and_nobackups_table() {
    use hikasa_folder::ops::ReadOnlySides;
    struct Row {
        line: &'static [&'static str],
        platform: Platform,
        kind: Option<ViewKind>,
        specs: usize,
        save_target: Option<&'static str>,
        read_only: ReadOnlySides,
        no_backups: bool,
    }
    const NONE: ReadOnlySides = ReadOnlySides {
        left: false,
        right: false,
    };
    let row = |line, platform, kind, specs| Row {
        line,
        platform,
        kind,
        specs,
        save_target: None,
        read_only: NONE,
        no_backups: false,
    };
    let linux = Platform::Linux;
    let windows = Platform::Windows;
    let edit = Some(ViewKind::TextEdit);
    let both = ReadOnlySides {
        left: true,
        right: true,
    };
    let left = ReadOnlySides {
        left: true,
        right: false,
    };
    let right = ReadOnlySides {
        left: false,
        right: true,
    };
    let rows = vec![
        row(&["--edit", "a.txt"], linux, edit, 1),
        row(&["--edit", "a.txt", "b.txt"], linux, edit, 2),
        row(&["-EDIT", "a.txt"], linux, edit, 1),
        row(&["/edit", "a.txt", "b.txt"], windows, edit, 2),
        row(&["--fv=text-compare", "--edit", "a", "b"], linux, edit, 2),
        row(
            &["--edit", "--fv", "Text Compare", "a", "b"],
            linux,
            Some(ViewKind::TextCompare),
            2,
        ),
        row(&["--sync", "--edit", "a"], linux, edit, 1),
        row(
            &["--edit", "--sync", "a", "b"],
            linux,
            Some(ViewKind::FolderSync),
            2,
        ),
        Row {
            save_target: Some("out.txt"),
            ..row(&["--savetarget=out.txt", "a", "b"], linux, None, 2)
        },
        Row {
            save_target: Some("../o u t.txt"),
            ..row(
                &["--edit", "--savetarget", "../o u t.txt", "a"],
                linux,
                edit,
                1,
            )
        },
        Row {
            save_target: Some("C:\\x\\y.txt"),
            ..row(&["/savetarget=C:\\x\\y.txt", "a", "b"], windows, None, 2)
        },
        Row {
            save_target: Some("\\\\srv\\share\\y.txt"),
            ..row(
                &["-SaveTarget", "\\\\srv\\share\\y.txt", "a", "b"],
                windows,
                None,
                2,
            )
        },
        Row {
            read_only: both,
            ..row(&["--ro", "a", "b"], linux, None, 2)
        },
        Row {
            read_only: left,
            ..row(&["--ro1", "--edit", "a"], linux, edit, 1)
        },
        Row {
            read_only: right,
            ..row(&["--ro2", "a", "b"], linux, None, 2)
        },
        Row {
            read_only: right,
            ..row(&["/rightreadonly", "a", "b"], windows, None, 2)
        },
        Row {
            read_only: left,
            ..row(&["-lro", "a", "b"], linux, None, 2)
        },
        Row {
            no_backups: true,
            save_target: Some("t"),
            ..row(
                &["--nobackups", "--edit", "--savetarget=t", "a"],
                linux,
                edit,
                1,
            )
        },
    ];
    for r in rows {
        let parsed = parse_open_args(&args(r.line), r.platform).unwrap();
        let line = r.line;
        assert_eq!(parsed.request.kind, r.kind, "{line:?}");
        assert_eq!(parsed.request.specs.len(), r.specs, "{line:?}");
        let options = &parsed.request.options;
        assert_eq!(options.save_target.as_deref(), r.save_target, "{line:?}");
        assert_eq!(options.read_only.sides(), r.read_only, "{line:?}");
        assert_eq!(options.no_backups, r.no_backups, "{line:?}");
        assert!(
            parsed.warnings.is_empty(),
            "{line:?}: {:?}",
            parsed.warnings
        );
        assert!(options.extra.is_empty(), "{line:?}: {:?}", options.extra);
    }
    for bad in [
        &["--edit=1"][..],
        &["--savetarget"],
        &["a", "b", "--savetarget"],
    ] {
        assert!(
            matches!(
                parse_open_args(&args(bad), Platform::Linux),
                Err(ApiError::InvalidArgument(_))
            ),
            "{bad:?}"
        );
    }
    // With a script they apply to no view: `switch_ignored` each.
    let parsed = parse_open_args(
        &args(&["@s.txt", "--edit", "--savetarget=t", "--ro2"]),
        Platform::Linux,
    )
    .unwrap();
    let ignored: Vec<&str> = parsed
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::SwitchIgnored)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(ignored.len(), 3, "{ignored:?}");
    assert!(ignored.iter().any(|m| m.contains("--edit")), "{ignored:?}");
    assert!(
        ignored.iter().any(|m| m.contains("--savetarget")),
        "{ignored:?}"
    );
    // The usage lists both with their meaning, not among the forwarded switches.
    let usage = hikasa_session::api::open_usage();
    let forwarded = usage
        .lines()
        .find(|l| l.contains("forwarded with a warning"))
        .unwrap();
    for name in ["--edit", "--savetarget"] {
        assert!(!forwarded.contains(name), "{forwarded}");
        assert!(
            usage.lines().any(|l| l.trim_start().starts_with(name)),
            "{usage}"
        );
    }
}

/// A10.17 (D-278): `resolve_open` makes `--savetarget` absolute and canonical with the caller's cwd; a Text Edit view
/// opens one `OpenView` per spec, each with that one spec; a Text Compare one with both.
#[test]
fn resolve_open_makes_the_save_target_absolute_and_splits_text_edit_views() {
    use hikasa_session::ipc::Request;
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let work = tmp.path().join("work");
    common::write(&work, "a.txt", b"a\n");
    common::write(&work, "b.txt", b"b\n");
    let abs = |rel: &str| work.join(rel).to_string_lossy().into_owned();
    let parsed = parse_open_args(
        &args(&[
            "--edit",
            "a.txt",
            "b.txt",
            "--savetarget=out/new.txt",
            "--ro1",
            "--nobackups",
        ]),
        Platform::Linux,
    )
    .unwrap();
    let resolved = service.resolve_open(&parsed.request).unwrap();
    assert_eq!(resolved.kind, ViewKind::TextEdit);
    assert_eq!(resolved.specs, vec![abs("a.txt"), abs("b.txt")]);
    // The canonical spelling of the platform (`\` on Windows).
    let target = work
        .join("out")
        .join("new.txt")
        .to_string_lossy()
        .into_owned();
    assert_eq!(resolved.options.save_target, Some(target.clone()));
    service.check_open(&resolved).unwrap();
    let requests = resolved.requests(true);
    assert_eq!(requests.len(), 2);
    for (request, spec) in requests.iter().zip(["a.txt", "b.txt"]) {
        let Request::OpenView {
            kind,
            specs,
            options,
            wait,
        } = request
        else {
            panic!("{request:?}");
        };
        assert_eq!(*kind, ViewKind::TextEdit);
        assert_eq!(specs, &vec![abs(spec)]);
        assert_eq!(options.save_target, Some(target.clone()));
        assert!(options.read_only.left && options.no_backups && *wait);
    }
    // A Text Compare: one view, both specs; the save target is made absolute the same way.
    let parsed = parse_open_args(
        &args(&["a.txt", "b.txt", "--savetarget", "../t.txt"]),
        Platform::Linux,
    )
    .unwrap();
    let resolved = service.resolve_open(&parsed.request).unwrap();
    assert_eq!(resolved.kind, ViewKind::TextCompare);
    // Windows folds `..` (its canonical form); Unix keeps it, as a link can make it another folder.
    let up = if cfg!(windows) {
        tmp.path().join("t.txt").to_string_lossy().into_owned()
    } else {
        abs("../t.txt")
    };
    assert_eq!(resolved.options.save_target, Some(up));
    assert_eq!(resolved.requests(false).len(), 1);
    // Without the switch nothing is added.
    let parsed = parse_open_args(&args(&["a.txt", "b.txt"]), Platform::Linux).unwrap();
    assert_eq!(
        service
            .resolve_open(&parsed.request)
            .unwrap()
            .options
            .save_target,
        None
    );
    // `-` and `clipboard://` are not files to save to; a Text Edit view needs a file.
    for bad in [
        &["a.txt", "b.txt", "--savetarget=-"][..],
        &["a.txt", "b.txt", "--savetarget=clipboard://"],
        &["--edit"],
    ] {
        let parsed = parse_open_args(&args(bad), Platform::Linux).unwrap();
        assert!(
            matches!(
                service.resolve_open(&parsed.request),
                Err(ApiError::InvalidArgument(_))
            ),
            "{bad:?}"
        );
    }
    // A bad format of a Text Edit view is caught before the GUI is asked.
    let parsed = parse_open_args(
        &args(&["--edit", "a.txt", "--format=no-such"]),
        Platform::Linux,
    )
    .unwrap();
    let resolved = service.resolve_open(&parsed.request).unwrap();
    assert!(service.check_open(&resolved).is_err());
    // M2 W11b (D-313): the service's request of a Text Edit view carries the file and the forced format (the format
    // itself is `check_open`'s, above); two specs are one view each, never one request.
    match service
        .open_view_request(ViewKind::TextEdit, &[abs("a.txt")], &resolved.options)
        .unwrap()
    {
        hikasa_session::api::ViewRequest::TextEdit { request } => {
            assert_eq!(request.format.as_deref(), Some("no-such"));
        }
        other => panic!("{other:?}"),
    }
    let err = service
        .open_view_request(
            ViewKind::TextEdit,
            &[abs("a.txt"), abs("b.txt")],
            &resolved.options,
        )
        .unwrap_err();
    assert!(err.to_string().contains("text_edit"), "{err}");
}

/// A10.32 (pure tables, every platform): the spellings of `--savetarget` made absolute and canonical, with a Windows
/// context (drive, drive letter case, forward slashes, UNC, `\\?\` verbatim and verbatim UNC, `..`, `~`) and a Unix one.
#[test]
fn save_target_spellings_table() {
    use hikasa_core::{PathStyle, SpecContext};
    use hikasa_session::api::save_target_spec;
    let windows = SpecContext {
        style: PathStyle::Windows,
        cwd: Some(PathBuf::from("C:\\work dir")),
        home: Some(PathBuf::from("C:\\Users\\me")),
    };
    let unix = SpecContext {
        style: PathStyle::Unix,
        cwd: Some(PathBuf::from("/w x")),
        home: Some(PathBuf::from("/home/me")),
    };
    let rows: &[(&SpecContext, &str, &str)] = &[
        (&windows, "out.txt", "C:\\work dir\\out.txt"),
        (&windows, "sub\\..\\out.txt", "C:\\work dir\\out.txt"),
        (&windows, "..\\out.txt", "C:\\out.txt"),
        (&windows, "D:\\a\\b.txt", "D:\\a\\b.txt"),
        (&windows, "d:/a/b.txt", "D:\\a\\b.txt"),
        (
            &windows,
            "\\\\server\\share\\dir\\x.txt",
            "\\\\server\\share\\dir\\x.txt",
        ),
        (&windows, "//server/share/x.txt", "\\\\server\\share\\x.txt"),
        // Verbatim paths stay verbatim (their own namespace: no length limit, no `..`).
        (&windows, "\\\\?\\C:\\long\\x.txt", "\\\\?\\C:\\long\\x.txt"),
        (
            &windows,
            "\\\\?\\UNC\\server\\share\\x.txt",
            "\\\\?\\UNC\\server\\share\\x.txt",
        ),
        (&windows, "~\\x.txt", "C:\\Users\\me\\x.txt"),
        (&unix, "out.txt", "/w x/out.txt"),
        // On Unix `..` is kept (a link may make it mean another folder); `.` goes.
        (&unix, "../o.txt", "/w x/../o.txt"),
        (&unix, "/a/./b.txt", "/a/b.txt"),
        (&unix, "~/x.txt", "/home/me/x.txt"),
    ];
    for (ctx, raw, want) in rows {
        assert_eq!(
            save_target_spec(OsString::from(raw).as_os_str(), ctx).unwrap(),
            *want,
            "{raw}"
        );
    }
    for (ctx, raw) in [(&windows, "-"), (&unix, "-"), (&unix, "clipboard://")] {
        assert!(
            matches!(
                save_target_spec(OsString::from(raw).as_os_str(), ctx),
                Err(ApiError::InvalidArgument(_))
            ),
            "{raw}"
        );
    }
    // A relative target without a current folder is an error (never resolved against the GUI's).
    let absolute_only = SpecContext::absolute_only(PathStyle::Windows);
    assert!(save_target_spec(OsString::from("x.txt").as_os_str(), &absolute_only).is_err());
}

/// A10.17 (D-278): the IPC `OpenView` with and without `save_target` (golden `fixtures/ipc/edit_frames.jsonl`): the field
/// is left out of the wire when absent and an older frame without it decodes to `None`.
#[test]
fn open_view_frames_with_and_without_save_target_are_golden() {
    use hikasa_session::ipc::{Envelope, OpenOptions, ReadOnly, Request};
    let line = |id: u64, body: Request| serde_json::to_string(&Envelope { id, body }).unwrap();
    let edit = Request::OpenView {
        kind: ViewKind::TextEdit,
        specs: vec!["/w/a.txt".into()],
        options: OpenOptions {
            read_only: ReadOnly {
                all: false,
                left: true,
                right: false,
            },
            ..OpenOptions::default()
        },
        wait: true,
    };
    let compare = Request::OpenView {
        kind: ViewKind::TextCompare,
        specs: vec!["/w/a.txt".into(), "/w/b.txt".into()],
        options: OpenOptions {
            save_target: Some("/w/out.txt".into()),
            no_backups: true,
            ..OpenOptions::default()
        },
        wait: false,
    };
    let edit_with_target = Request::OpenView {
        kind: ViewKind::TextEdit,
        specs: vec!["C:\\w\\a.txt".into()],
        options: OpenOptions {
            save_target: Some("\\\\server\\share\\a.txt".into()),
            ..OpenOptions::default()
        },
        wait: false,
    };
    let lines = [line(1, edit), line(2, compare), line(3, edit_with_target)];
    assert!(!lines[0].contains("save_target"), "{}", lines[0]);
    assert!(
        lines[1].contains(r#""save_target":"/w/out.txt""#),
        "{}",
        lines[1]
    );
    let actual = lines.join("\n") + "\n";
    common::assert_golden(
        &common::fixtures("ipc").join("edit_frames.jsonl"),
        actual.as_bytes(),
    );
    let old: OpenOptions = serde_json::from_str(r#"{"titles":[null,null,null,null]}"#).unwrap();
    assert_eq!(old.save_target, None);
    let back: OpenOptions =
        serde_json::from_str(r#"{"save_target":"/w/out.txt","no_backups":true}"#).unwrap();
    assert_eq!(back.save_target.as_deref(), Some("/w/out.txt"));
}

/// §1.10.22 (D-278): `LiveOptions::from_open` takes `--ro*`, `--savetarget` and `--nobackups` of an `OpenView`; a Text Edit
/// view's outcome is 0 (no verdict) or 100 when its file failed to load.
#[test]
fn live_options_and_the_text_edit_outcome() {
    use hikasa_folder::ops::ReadOnlySides;
    use hikasa_session::api::{LiveOptions, ViewOutcome};
    use hikasa_session::ipc::{CloseReason, OpenOptions, ReadOnly};
    let options = OpenOptions {
        read_only: ReadOnly {
            all: false,
            left: false,
            right: true,
        },
        save_target: Some("/w/out.txt".into()),
        no_backups: true,
        ..OpenOptions::default()
    };
    assert_eq!(
        LiveOptions::from_open(&options),
        LiveOptions {
            read_only: ReadOnlySides {
                left: false,
                right: true,
            },
            save_target: Some("/w/out.txt".into()),
            no_backups: true,
            ..LiveOptions::default()
        }
    );
    let ro = |all, left, right| OpenOptions {
        read_only: ReadOnly { all, left, right },
        ..OpenOptions::default()
    };
    assert_eq!(
        LiveOptions::from_open(&ro(true, false, false)).read_only,
        ReadOnlySides {
            left: true,
            right: true
        }
    );
    assert_eq!(
        LiveOptions::from_open(&ro(false, true, false)).read_only,
        ReadOnlySides {
            left: true,
            right: false
        }
    );
    assert_eq!(
        LiveOptions::from_open(&OpenOptions::default()),
        LiveOptions::default()
    );
    let (result, code) = ViewOutcome::for_text_edit(true).close(CloseReason::User, true);
    assert_eq!(
        (
            code.code(),
            result.verdict,
            result.reason.as_str(),
            result.saved
        ),
        (0, None, "user", true)
    );
    let (result, code) = ViewOutcome::for_text_edit(false).close(CloseReason::User, false);
    assert_eq!((code.code(), result.reason.as_str()), (100, "load_failed"));
}

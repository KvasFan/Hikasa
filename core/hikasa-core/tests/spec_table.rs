//! The normative spec table of §2.1 (30 rows × the relevant styles), Secret handling and LocalPath checks.

use std::path::PathBuf;

use hikasa_core::spec::{
    HttpUrl, PathStyle, RemoteScheme, RemoteUrl, Secret, Spec, SpecContext, SpecError,
};
use pretty_assertions::assert_eq;

fn ctx(style: PathStyle, cwd: Option<&str>, home: Option<&str>) -> SpecContext {
    SpecContext {
        style,
        cwd: cwd.map(PathBuf::from),
        home: home.map(PathBuf::from),
    }
}

const BOTH: [PathStyle; 2] = [PathStyle::Unix, PathStyle::Windows];

/// Parses and checks the canonical text; also checks the round trip INV-SPEC-4.
fn canonical(input: &str, ctx: &SpecContext) -> String {
    let spec =
        Spec::parse(input, ctx).unwrap_or_else(|e| panic!("{input:?} ({:?}): {e}", ctx.style));
    let text = spec.canonical();
    assert_eq!(text, spec.to_string());
    let again = Spec::parse(&text, ctx).unwrap_or_else(|e| panic!("re-parse of {text:?}: {e}"));
    assert_eq!(again, spec.without_secrets(), "INV-SPEC-4 for {input:?}");
    text
}

fn error(input: &str, ctx: &SpecContext) -> SpecError {
    match Spec::parse(input, ctx) {
        Ok(spec) => panic!("{input:?} ({:?}) parsed as {spec:?}", ctx.style),
        Err(e) => e,
    }
}

#[test]
fn row_01_absolute_unix() {
    let spec = Spec::parse("/home/u/x", &ctx(PathStyle::Unix, None, None)).unwrap();
    assert!(matches!(&spec, Spec::Local(p) if p.as_path() == std::path::Path::new("/home/u/x")));
    assert_eq!(
        canonical("/home/u/x", &ctx(PathStyle::Unix, None, None)),
        "/home/u/x"
    );
}

#[test]
fn row_02_empty_and_dot_components() {
    assert_eq!(
        canonical("/home/u//x/./y/", &ctx(PathStyle::Unix, None, None)),
        "/home/u/x/y"
    );
}

#[test]
fn row_03_dotdot_is_kept() {
    assert_eq!(
        canonical("/a/../b", &ctx(PathStyle::Unix, None, None)),
        "/a/../b"
    );
}

#[test]
fn row_04_dotdot_under_the_root() {
    assert_eq!(canonical("/../x", &ctx(PathStyle::Unix, None, None)), "/x");
}

#[test]
fn row_05_relative_with_cwd() {
    assert_eq!(
        canonical("x/y", &ctx(PathStyle::Unix, Some("/w"), None)),
        "/w/x/y"
    );
}

#[test]
fn row_06_relative_without_cwd() {
    assert_eq!(
        error("x/y", &ctx(PathStyle::Unix, None, None)),
        SpecError::RelativeWithoutCwd { path: "x/y".into() }
    );
}

#[test]
fn row_07_tilde() {
    let c = ctx(PathStyle::Unix, None, Some("/home/u"));
    assert_eq!(canonical("~", &c), "/home/u");
    assert_eq!(canonical("~/x", &c), "/home/u/x");
    assert_eq!(
        error("~", &ctx(PathStyle::Unix, None, None)),
        SpecError::HomeUnknown
    );
}

#[test]
fn row_08_tilde_user() {
    assert_eq!(
        error("~bob/x", &ctx(PathStyle::Unix, None, Some("/home/u"))),
        SpecError::TildeUser {
            input: "~bob/x".into()
        }
    );
}

#[test]
fn row_09_backslash_is_a_normal_character_on_unix() {
    assert_eq!(
        canonical("a\\b", &ctx(PathStyle::Unix, Some("/w"), None)),
        "/w/a\\b"
    );
}

#[test]
fn row_10_stdin() {
    for style in BOTH {
        assert_eq!(
            Spec::parse("-", &ctx(style, None, None)).unwrap(),
            Spec::Stdin
        );
        assert_eq!(canonical("-", &ctx(style, None, None)), "-");
    }
}

#[test]
fn row_11_clipboard() {
    for style in BOTH {
        let c = ctx(style, None, None);
        assert_eq!(Spec::parse("CLIPBOARD://", &c).unwrap(), Spec::Clipboard);
        assert_eq!(canonical("CLIPBOARD://", &c), "clipboard://");
        assert!(matches!(
            error("clipboard://x", &c),
            SpecError::MalformedUrl { .. }
        ));
    }
}

#[test]
fn row_12_drive_paths() {
    let c = ctx(PathStyle::Windows, None, None);
    assert_eq!(canonical(r"C:\x\y", &c), r"C:\x\y");
    assert_eq!(canonical("c:/x/y", &c), r"C:\x\y");
}

#[test]
fn row_13_dotdot_is_resolved_on_windows() {
    assert_eq!(
        canonical(r"C:\a\..\b", &ctx(PathStyle::Windows, None, None)),
        r"C:\b"
    );
}

#[test]
fn row_14_unc() {
    let c = ctx(PathStyle::Windows, None, None);
    assert_eq!(canonical(r"\\srv\share\x", &c), r"\\srv\share\x");
    assert_eq!(canonical("//srv/share/x", &c), r"\\srv\share\x");
}

#[test]
fn row_15_unc_without_share() {
    assert_eq!(
        error(r"\\srv", &ctx(PathStyle::Windows, None, None)),
        SpecError::UncWithoutShare {
            path: r"\\srv".into()
        }
    );
}

#[test]
fn row_16_verbatim_is_unchanged() {
    assert_eq!(
        canonical(r"\\?\C:\x\..\y", &ctx(PathStyle::Windows, None, None)),
        r"\\?\C:\x\..\y"
    );
}

#[test]
fn row_17_device_path() {
    assert_eq!(
        error(r"\\.\pipe\x", &ctx(PathStyle::Windows, None, None)),
        SpecError::DevicePath {
            path: r"\\.\pipe\x".into()
        }
    );
}

#[test]
fn row_18_drive_relative_on_the_cwd_drive() {
    assert_eq!(
        canonical("C:x", &ctx(PathStyle::Windows, Some(r"C:\w"), None)),
        r"C:\w\x"
    );
    assert_eq!(
        canonical("c:x", &ctx(PathStyle::Windows, Some(r"C:\w"), None)),
        r"C:\w\x"
    );
}

#[test]
fn row_19_drive_relative_on_another_drive() {
    assert_eq!(
        error("C:x", &ctx(PathStyle::Windows, Some(r"D:\w"), None)),
        SpecError::DriveRelative { path: "C:x".into() }
    );
    assert_eq!(
        error("C:x", &ctx(PathStyle::Windows, None, None)),
        SpecError::DriveRelative { path: "C:x".into() }
    );
}

#[test]
fn row_20_rooted_path() {
    assert_eq!(
        canonical(r"\x", &ctx(PathStyle::Windows, Some(r"D:\w"), None)),
        r"D:\x"
    );
}

#[test]
fn row_21_one_letter_scheme_is_a_drive_on_windows() {
    assert_eq!(
        canonical("C://x", &ctx(PathStyle::Windows, None, None)),
        r"C:\x"
    );
}

#[test]
fn row_22_one_letter_scheme_on_unix() {
    assert_eq!(
        error("C://x", &ctx(PathStyle::Unix, Some("/w"), None)),
        SpecError::UnknownScheme { scheme: "C".into() }
    );
}

#[test]
fn row_23_remote_with_password() {
    for style in BOTH {
        let c = ctx(style, None, None);
        let spec = Spec::parse("ftp://user:pw@Host/p", &c).unwrap();
        let Spec::Remote(url) = &spec else {
            panic!("{spec:?}")
        };
        assert_eq!(url.scheme, RemoteScheme::Ftp);
        assert_eq!(url.user.as_deref(), Some("user"));
        assert_eq!(url.password.as_ref().map(|s| s.expose()), Some("pw"));
        assert_eq!(url.host, "host");
        assert_eq!(url.port, None);
        assert_eq!(url.path, "/p");
        assert_eq!(canonical("ftp://user:pw@Host/p", &c), "ftp://user@host/p");
    }
}

#[test]
fn row_24_remote_with_port() {
    for style in BOTH {
        let spec = Spec::parse("sftp://u@h:2222/p", &ctx(style, None, None)).unwrap();
        assert_eq!(
            spec,
            Spec::Remote(RemoteUrl {
                scheme: RemoteScheme::Sftp,
                user: Some("u".into()),
                password: None,
                host: "h".into(),
                port: Some(2222),
                path: "/p".into(),
            })
        );
        assert_eq!(
            canonical("sftp://u@h:2222/p", &ctx(style, None, None)),
            "sftp://u@h:2222/p"
        );
    }
}

#[test]
fn row_25_port_out_of_range() {
    for style in BOTH {
        assert!(matches!(
            error("sftp://u@h:99999/p", &ctx(style, None, None)),
            SpecError::MalformedUrl { reason, .. } if reason.contains("port")
        ));
    }
}

#[test]
fn row_26_s3_bucket() {
    for style in BOTH {
        let spec = Spec::parse("s3://bucket/prefix/", &ctx(style, None, None)).unwrap();
        let Spec::Remote(url) = &spec else {
            panic!("{spec:?}")
        };
        assert_eq!(
            (url.scheme, url.host.as_str(), url.path.as_str()),
            (RemoteScheme::S3, "bucket", "/prefix/")
        );
        assert_eq!(
            canonical("s3://bucket/prefix/", &ctx(style, None, None)),
            "s3://bucket/prefix/"
        );
    }
}

#[test]
fn row_27_profile() {
    for style in BOTH {
        let c = ctx(style, None, None);
        assert_eq!(
            Spec::parse("profile:My Prof?dir/f.txt", &c).unwrap(),
            Spec::Profile {
                name: "My Prof".into(),
                path: Some("dir/f.txt".into())
            }
        );
        assert_eq!(
            canonical("profile:My Prof?dir/f.txt", &c),
            "profile:My Prof?dir/f.txt"
        );
        assert_eq!(
            Spec::parse("PROFILE:x", &c).unwrap(),
            Spec::Profile {
                name: "x".into(),
                path: None
            }
        );
        assert_eq!(
            error("profile:?x", &c),
            SpecError::MalformedProfile {
                input: "profile:?x".into()
            }
        );
    }
}

#[test]
fn row_28_https() {
    for style in BOTH {
        let c = ctx(style, None, None);
        assert_eq!(
            Spec::parse("https://Example.com/a?b", &c).unwrap(),
            Spec::Http(HttpUrl {
                secure: true,
                user: None,
                password: None,
                host: "example.com".into(),
                port: None,
                rest: "/a?b".into()
            })
        );
        assert_eq!(
            canonical("https://Example.com/a?b", &c),
            "https://example.com/a?b"
        );
        assert_eq!(canonical("http://h:8080?q#f", &c), "http://h:8080?q#f");
    }
}

#[test]
fn row_29_unknown_scheme() {
    for style in BOTH {
        assert_eq!(
            error("foo://x", &ctx(style, None, None)),
            SpecError::UnknownScheme {
                scheme: "foo".into()
            }
        );
    }
}

#[test]
fn row_30_empty() {
    for style in BOTH {
        assert_eq!(error("", &ctx(style, None, None)), SpecError::Empty);
        assert_eq!(error("   ", &ctx(style, None, None)), SpecError::Empty);
        assert_eq!(error("a\0b", &ctx(style, None, None)), SpecError::Nul);
    }
}

#[test]
fn spaces_are_not_trimmed() {
    assert_eq!(
        canonical(" x ", &ctx(PathStyle::Unix, Some("/w"), None)),
        "/w/ x "
    );
}

#[test]
fn secrets_never_appear_in_debug_or_canonical_forms() {
    let spec = Spec::parse(
        "davs://alice:s3cr3t-Pw@files.example/x",
        &ctx(PathStyle::Unix, None, None),
    )
    .unwrap();
    assert!(!format!("{spec:?}").contains("s3cr3t"), "{spec:?}");
    assert!(format!("{spec:?}").contains("Secret(***)"));
    assert!(!spec.canonical().contains("s3cr3t"));
    assert!(!serde_json::to_string(&spec).unwrap().contains("s3cr3t"));
    assert!(spec.without_secrets() != spec);
    let Spec::Remote(url) = spec.without_secrets() else {
        panic!()
    };
    assert_eq!(url.password, None);
    let err = Spec::parse(
        "ftp://alice:s3cr3t-Pw@host:0/x",
        &ctx(PathStyle::Unix, None, None),
    )
    .unwrap_err();
    assert!(!err.to_string().contains("s3cr3t"), "{err}");
}

#[test]
fn every_remote_scheme_parses() {
    for scheme in RemoteScheme::ALL {
        let input = format!("{}://h/p", scheme.as_str().to_uppercase());
        let spec = Spec::parse(&input, &ctx(PathStyle::Unix, None, None)).unwrap();
        assert_eq!(spec.scheme_name(), scheme.as_str());
        assert_eq!(spec.canonical(), format!("{}://h/p", scheme.as_str()));
    }
}

#[test]
fn ipv6_hosts() {
    let c = ctx(PathStyle::Unix, None, None);
    assert_eq!(canonical("sftp://[::1]:22/p", &c), "sftp://[::1]:22/p");
    assert_eq!(canonical("ftp://[FE80::1]/", &c), "ftp://[fe80::1]/");
    for bad in [
        "ftp://[::1/p",
        "ftp://[]/p",
        "ftp://[::1]x/p",
        "ftp://:21/p",
        "ftp://h:/p",
        "ftp://user@/p",
    ] {
        assert!(
            matches!(error(bad, &c), SpecError::MalformedUrl { .. }),
            "{bad}"
        );
    }
}

/// D-05 / §2.1 step 5: the http(s) authority is parsed like a remote one. The user is kept and appears in the
/// canonical form; the password is a `Secret` that never appears in a canonical form, `Debug` or an error
/// message, and the canonical form round-trips (INV-SPEC-4).
#[test]
fn http_user_information_is_kept_and_the_password_is_secret() {
    for style in BOTH {
        let c = ctx(style, None, None);
        for (input, user, password, canonical_form) in [
            ("http://u@h/", Some("u"), None, "http://u@h/"),
            (
                "https://user:hunter2@Example.com:8443/a?b#f",
                Some("user"),
                Some("hunter2"),
                "https://user@example.com:8443/a?b#f",
            ),
            (
                "https://a@b:pw@h/x",
                Some("a@b"),
                Some("pw"),
                "https://a@b@h/x",
            ),
            ("http://:pw@h", Some(""), Some("pw"), "http://@h"),
            ("HTTP://U:@H?q", Some("U"), Some(""), "http://U@h?q"),
            ("https://h/x", None, None, "https://h/x"),
        ] {
            let spec = Spec::parse(input, &c).unwrap();
            let Spec::Http(url) = &spec else {
                panic!("{spec:?}")
            };
            assert_eq!(url.user.as_deref(), user, "{input}");
            assert_eq!(
                url.password.as_ref().map(Secret::expose),
                password,
                "{input}"
            );
            assert_eq!(spec.canonical(), canonical_form, "{input}");
            assert!(!format!("{spec:?}").contains("hunter2"), "{spec:?}");
            assert_eq!(
                Spec::parse(&spec.canonical(), &c).unwrap(),
                spec.without_secrets(),
                "INV-SPEC-4 for {input}"
            );
            let Spec::Http(stripped) = spec.without_secrets() else {
                panic!("{spec:?}")
            };
            assert_eq!(stripped.password, None);
            assert_eq!(stripped.user.as_deref(), user);
            assert!(!url.host.contains('@'));
        }
        for input in [
            "https://user:hun/ter2@host/x",
            "https://user:hunter2@host:0/x",
            "https://user:hunter2@[::1/x",
        ] {
            let err = error(input, &c);
            assert!(matches!(err, SpecError::MalformedUrl { .. }), "{err:?}");
            assert!(
                !err.to_string().contains("hun/ter2") && !err.to_string().contains("hunter2"),
                "{err}"
            );
        }
        assert!(matches!(
            error("http://user@/x", &c),
            SpecError::MalformedUrl {
                reason: "empty host",
                ..
            }
        ));
    }
}

/// §2.1 Windows steps 2–3 with a verbatim current or home directory (`std::fs::canonicalize` returns such paths
/// on Windows): the directory keeps its `\\?\` text, the user's relative part is still normalised (D-06,
/// INV-SPEC-1..3).
#[test]
fn verbatim_current_and_home_directories() {
    let c = ctx(
        PathStyle::Windows,
        Some(r"\\?\C:\w"),
        Some(r"\\?\C:\Users\u"),
    );
    for (input, expected) in [
        (r"..\other.txt", r"\\?\C:\other.txt"),
        (r"a\.\b\", r"\\?\C:\w\a\b"),
        (r"\a\..\b", r"\\?\C:\b"),
        ("C:../z", r"\\?\C:\z"),
        (r"~\a\..\b", r"\\?\C:\Users\u\b"),
        ("~", r"\\?\C:\Users\u"),
        (r"\\?\C:\x\..\y", r"\\?\C:\x\..\y"),
        (r"D:\x\..\y", r"D:\y"),
    ] {
        assert_eq!(canonical(input, &c), expected, "{input:?}");
    }
    let unc = ctx(PathStyle::Windows, Some(r"\\?\UNC\srv\share\w"), None);
    assert_eq!(canonical(r"..\..\x", &unc), r"\\?\UNC\srv\share\x");
    assert_eq!(canonical(r"\x", &unc), r"\\?\UNC\srv\share\x");
}

#[test]
fn error_messages_are_stable() {
    let cases = [
        (SpecError::Empty, "empty spec"),
        (SpecError::Nul, "spec contains a NUL character"),
        (
            SpecError::RelativeWithoutCwd { path: "x".into() },
            "relative path \"x\" needs a current directory",
        ),
        (SpecError::HomeUnknown, "'~' needs a home directory"),
        (
            SpecError::TildeUser { input: "~b".into() },
            "'~user' expansion is not supported: \"~b\"",
        ),
        (
            SpecError::UnknownScheme {
                scheme: "foo".into(),
            },
            "unknown URL scheme \"foo\"",
        ),
        (
            SpecError::MalformedUrl {
                input: "u".into(),
                reason: "invalid port",
            },
            "malformed URL \"u\": invalid port",
        ),
        (
            SpecError::MalformedProfile { input: "p".into() },
            "malformed profile spec \"p\"",
        ),
        (
            SpecError::DriveRelative { path: "C:x".into() },
            "drive-relative path \"C:x\" cannot be resolved",
        ),
        (
            SpecError::DevicePath { path: "d".into() },
            "device path \"d\" is not supported",
        ),
        (
            SpecError::UncWithoutShare { path: "s".into() },
            "UNC path \"s\" lacks a share name",
        ),
        (
            SpecError::InvalidComponent { name: "a/b".into() },
            "invalid file name component \"a/b\"",
        ),
        (
            SpecError::NotNormalized { path: "x".into() },
            "not an absolute normalised path: \"x\"",
        ),
    ];
    for (err, message) in cases {
        assert_eq!(err.to_string(), message);
    }
}

#[cfg(unix)]
#[test]
fn parsed_host_paths_are_valid_local_paths() {
    use hikasa_core::spec::LocalPath;

    let c = ctx(PathStyle::Unix, Some("/w x"), Some("/home/ü"));
    for input in ["a", "~/b//c/", "/x/../y", "../../z", ".", "/"] {
        let Spec::Local(path) = Spec::parse(input, &c).unwrap() else {
            panic!("{input}")
        };
        assert_eq!(
            LocalPath::new(path.as_path().to_path_buf()).unwrap(),
            path,
            "INV-SPEC-1..3 for {input:?}"
        );
    }
}

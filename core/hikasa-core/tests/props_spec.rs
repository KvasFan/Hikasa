//! Spec properties P-SPEC-1 (round trip, INV-SPEC-4) and P-SPEC-2 (idempotent normalisation), A1.3.

mod common;

use std::path::PathBuf;

use hikasa_core::spec::{PathStyle, Spec, SpecContext, normalize_local};
use proptest::prelude::*;

/// A path component: Unicode, spaces, dots, tildes; never a separator of the style.
fn component(style: PathStyle) -> impl Strategy<Value = String> {
    let pieces: Vec<&'static str> = match style {
        PathStyle::Unix => vec![
            "a", "B", " ", ".", "..", "~", "é", "漢", "x y", "\\", ":", "-", "#", "a.b",
        ],
        PathStyle::Windows => vec![
            "a", "B", " ", ".", "..", "~", "é", "漢", "x y", ":", "-", "#", "a.b",
        ],
    };
    proptest::collection::vec(prop::sample::select(pieces), 1..4).prop_map(|p| p.concat())
}

fn unix_local() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec!["", "/", "//", "~", "~/", "./", "../", "/../"]),
        proptest::collection::vec(
            (
                component(PathStyle::Unix),
                prop::sample::select(vec!["/", "//", "/./"]),
            ),
            0..5,
        ),
        any::<bool>(),
    )
        .prop_map(|(prefix, parts, trailing)| {
            let mut text = prefix.to_owned();
            for (k, (c, sep)) in parts.iter().enumerate() {
                if k > 0 {
                    text.push_str(sep);
                }
                text.push_str(c);
            }
            if trailing {
                text.push('/');
            }
            text
        })
}

fn windows_local() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec![
            "",
            "C:\\",
            "c:/",
            "D:\\",
            "\\\\srv\\share\\",
            "//srv/share/",
            "\\",
            "/",
            "C:",
            "c:",
            "D:",
            "~\\",
            "~/",
            "\\\\?\\C:\\",
            "..\\",
        ]),
        proptest::collection::vec(
            (
                component(PathStyle::Windows),
                prop::sample::select(vec!["\\", "/", "\\\\", "\\.\\"]),
            ),
            0..5,
        ),
        any::<bool>(),
    )
        .prop_map(|(prefix, parts, trailing)| {
            let mut text = prefix.to_owned();
            for (k, (c, sep)) in parts.iter().enumerate() {
                if k > 0 {
                    text.push_str(sep);
                }
                text.push_str(c);
            }
            if trailing {
                text.push('\\');
            }
            text
        })
}

/// Text that must never appear in a canonical form when used as a password.
const MARKER: &str = "SECRET";

fn url() -> impl Strategy<Value = String> {
    let scheme = prop::sample::select(vec![
        "ftp",
        "FTP",
        "ftps",
        "sftp",
        "dav",
        "Davs",
        "s3",
        "svn",
        "svn+http",
        "SVN+HTTPS",
        "svn+ssh",
        "mtp",
        "http",
        "HTTPS",
    ]);
    let user = proptest::option::of(prop::sample::select(vec![
        "u", "alice", "a@b", "", "x y", "é",
    ]));
    let password = proptest::option::of((0u16..1000).prop_map(|n| format!("{MARKER}{n}")));
    let host = prop::sample::select(vec![
        "h",
        "Host.Example",
        "srv-1",
        "[::1]",
        "[FE80::1]",
        "bucket",
        "ÜBER",
    ]);
    let port = proptest::option::of(prop::sample::select(vec![1u32, 21, 2222, 65535]));
    let rest = prop::sample::select(vec![
        "",
        "/",
        "/p",
        "//abs/path",
        "/a b/ü.txt",
        "/x?y",
        "/x#f",
        "?q",
        "#f",
        "/a@b:c",
    ]);
    (scheme, user, password, host, port, rest).prop_map(
        |(scheme, user, password, host, port, rest)| {
            let mut text = format!("{scheme}://");
            let is_http =
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
            // http(s) and remote URLs keep the user; the password is a secret (D-05, VFS-010).
            if let Some(user) = user {
                text.push_str(user);
                if let Some(password) = password {
                    text.push(':');
                    text.push_str(&password);
                }
                text.push('@');
            }
            text.push_str(host);
            if let Some(port) = port {
                text.push_str(&format!(":{port}"));
            }
            if !is_http && (rest.starts_with('?') || rest.starts_with('#')) {
                text.push('/');
            }
            text.push_str(rest);
            text
        },
    )
}

fn other_spec() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("-".to_owned()),
        prop::sample::select(vec!["clipboard://", "CLIPBOARD://", "Clipboard://"])
            .prop_map(str::to_owned),
        (
            prop::sample::select(vec!["profile:", "PROFILE:", "Profile:"]),
            prop::sample::select(vec!["P", "My Prof", "a:b", "é"]),
            proptest::option::of(prop::sample::select(vec!["", "dir/f.txt", "x?y", "a b"])),
        )
            .prop_map(|(prefix, name, path)| match path {
                Some(path) => format!("{prefix}{name}?{path}"),
                None => format!("{prefix}{name}"),
            }),
    ]
}

fn unix_ctx() -> SpecContext {
    SpecContext {
        style: PathStyle::Unix,
        cwd: Some(PathBuf::from("/w x/ü")),
        home: Some(PathBuf::from("/home/é")),
    }
}

fn windows_ctx() -> SpecContext {
    SpecContext {
        style: PathStyle::Windows,
        cwd: Some(PathBuf::from("C:\\w x\\ü")),
        home: Some(PathBuf::from("C:\\Users\\é")),
    }
}

/// The same directories as verbatim paths (as `std::fs::canonicalize` returns them on Windows).
fn windows_verbatim_ctx() -> SpecContext {
    SpecContext {
        style: PathStyle::Windows,
        cwd: Some(PathBuf::from("\\\\?\\C:\\w x\\ü")),
        home: Some(PathBuf::from("\\\\?\\C:\\Users\\é")),
    }
}

/// True when a Windows input is resolved against the current or home directory (it is not absolute).
fn uses_directories(input: &str) -> bool {
    let s = input.replace('/', "\\");
    let b = s.as_bytes();
    let drive_root = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
    !(s.starts_with("\\\\") || drive_root)
}

fn check_round_trip(input: &str, ctx: &SpecContext) -> Result<(), TestCaseError> {
    let Ok(spec) = Spec::parse(input, ctx) else {
        return Ok(());
    };
    let canonical = spec.canonical();
    prop_assert!(
        !canonical.contains(MARKER),
        "canonical form {:?} leaks a password",
        canonical
    );
    prop_assert!(
        !format!("{spec:?}").contains(MARKER),
        "Debug leaks a password"
    );
    let again = Spec::parse(&canonical, ctx);
    prop_assert!(
        again.is_ok(),
        "{:?} → {:?} does not re-parse: {:?}",
        input,
        canonical,
        again
    );
    prop_assert_eq!(
        again.unwrap(),
        spec.without_secrets(),
        "INV-SPEC-4 for {:?}",
        input
    );
    #[cfg(unix)]
    if ctx.style == PathStyle::HOST
        && let Spec::Local(path) = &spec
    {
        let checked = hikasa_core::spec::LocalPath::new(path.as_path().to_path_buf());
        prop_assert_eq!(checked.as_ref(), Ok(path), "INV-SPEC-1..3 for {:?}", input);
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props_spec.txt"))]

    /// P-SPEC-1: `parse(canonical(s)) == s.without_secrets()` over generated components (Unicode, spaces,
    /// both styles), for every input that parses.
    #[test]
    fn p_spec_1_round_trip(
        unix in unix_local(),
        windows in windows_local(),
        remote in url(),
        other in other_spec(),
    ) {
        let unix_ctx = unix_ctx();
        let windows_ctx = windows_ctx();
        check_round_trip(&unix, &unix_ctx)?;
        check_round_trip(&windows, &windows_ctx)?;
        check_round_trip(&windows, &windows_verbatim_ctx())?;
        for ctx in [&unix_ctx, &windows_ctx] {
            check_round_trip(&remote, ctx)?;
            check_round_trip(&other, ctx)?;
        }
        prop_assert!(Spec::parse(&other, &unix_ctx).is_ok(), "{:?}", other);
    }

    /// P-SPEC-2: `normalize_local` is idempotent (both styles, with and without cwd/home).
    #[test]
    fn p_spec_2_idempotent(
        unix in unix_local(),
        windows in windows_local(),
        with_dirs in any::<bool>(),
    ) {
        let (unix_cwd, unix_home) = if with_dirs { (Some("/w x/ü"), Some("/home/é")) } else { (None, None) };
        if let Ok(once) = normalize_local(&unix, PathStyle::Unix, unix_cwd, unix_home) {
            prop_assert_eq!(normalize_local(&once, PathStyle::Unix, unix_cwd, unix_home), Ok(once.clone()));
            prop_assert_eq!(normalize_local(&once, PathStyle::Unix, None, None), Ok(once.clone()));
            prop_assert!(once.starts_with('/'));
        }
        let (win_cwd, win_home) = if with_dirs { (Some("c:/w x/ü"), Some("C:\\Users\\é")) } else { (None, None) };
        if let Ok(once) = normalize_local(&windows, PathStyle::Windows, win_cwd, win_home) {
            prop_assert_eq!(normalize_local(&once, PathStyle::Windows, win_cwd, win_home), Ok(once.clone()));
            prop_assert_eq!(normalize_local(&once, PathStyle::Windows, None, None), Ok(once.clone()));
            prop_assert!(!once.contains('/') || once.starts_with("\\\\?\\"));
        }
    }

    /// P-SPEC-2 (verbatim directories): an input resolved against a verbatim (`\\?\…`) current or home
    /// directory is normalised exactly as against the plain directory, under the verbatim prefix; inputs that
    /// do not use the directories give the same result (§2.1 Windows steps 2–3, INV-SPEC-1..3).
    #[test]
    fn p_spec_2_verbatim_directories(windows in windows_local(), unc in any::<bool>()) {
        let (plain_cwd, verbatim_cwd, plain_root, verbatim_root) = if unc {
            ("\\\\srv\\share\\w x", "\\\\?\\UNC\\srv\\share\\w x", "\\\\srv\\share", "\\\\?\\UNC\\srv\\share")
        } else {
            ("C:\\w x\\ü", "\\\\?\\C:\\w x\\ü", "C:", "\\\\?\\C:")
        };
        let (plain_home, verbatim_home) = ("C:\\Users\\é", "\\\\?\\C:\\Users\\é");
        let plain = normalize_local(&windows, PathStyle::Windows, Some(plain_cwd), Some(plain_home));
        let verbatim = normalize_local(&windows, PathStyle::Windows, Some(verbatim_cwd), Some(verbatim_home));
        match (plain, verbatim) {
            (Ok(p), Ok(v)) => {
                let expected = if !uses_directories(&windows) {
                    p.clone()
                } else if windows.starts_with('~') {
                    format!("\\\\?\\{p}")
                } else {
                    let rest = p.strip_prefix(plain_root);
                    prop_assert!(rest.is_some(), "{:?} -> {:?}", windows, p);
                    format!("{verbatim_root}{}", rest.unwrap_or(""))
                };
                prop_assert_eq!(&v, &expected, "{:?}", windows);
                prop_assert_eq!(normalize_local(&v, PathStyle::Windows, None, None), Ok(v.clone()));
            }
            (Err(a), Err(b)) => prop_assert_eq!(a, b),
            (p, v) => prop_assert!(false, "{:?}: plain {:?}, verbatim {:?}", windows, p, v),
        }
    }
}

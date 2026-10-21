//! §2.22 configuration and cache folders, §2.23 endpoints (every row), FNV-1a vectors (§6.2.8).
//!
//! The Unix rows compare with Unix path literals, so they run on Unix; the Windows rows run everywhere.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hikasa_session::ipc::{Endpoint, EndpointAddress, MAX_SOCKET_PATH, fnv1a64};
use hikasa_session::{
    ConfigLocation, ConfigMode, EnvFacts, PORTABLE_MARKER, Platform, SessionError,
    default_config_dir, resolve_config_dir,
};

fn linux() -> EnvFacts {
    EnvFacts {
        cwd: Some("/w".into()),
        home: Some("/home/anna".into()),
        exe_dir: Some("/opt/hikasa".into()),
        uid: Some(1000),
        user: Some("anna".into()),
        ..EnvFacts::new(Platform::Linux, "/tmp".into())
    }
}

fn macos() -> EnvFacts {
    EnvFacts {
        home: Some("/Users/anna".into()),
        exe_dir: Some("/Applications/Hikasa.app/Contents/MacOS".into()),
        uid: Some(501),
        ..EnvFacts::new(Platform::MacOs, "/var/folders/xy/T".into())
    }
}

fn windows() -> EnvFacts {
    EnvFacts {
        cwd: Some(r"C:\work".into()),
        home: Some(r"C:\Users\Anna Lee".into()),
        exe_dir: Some(r"C:\Program Files\Hikasa".into()),
        appdata: Some(r"C:\Users\Anna Lee\AppData\Roaming".into()),
        local_appdata: Some(r"C:\Users\Anna Lee\AppData\Local".into()),
        user: Some("Anna Lee".into()),
        ..EnvFacts::new(
            Platform::Windows,
            r"C:\Users\Anna Lee\AppData\Local\Temp".into(),
        )
    }
}

fn never(_: &Path) -> bool {
    false
}

fn resolve(
    facts: &EnvFacts,
    explicit: Option<&str>,
    exists: &dyn Fn(&Path) -> bool,
) -> ConfigLocation {
    resolve_config_dir(facts, explicit.map(Path::new), exists).unwrap()
}

#[cfg(unix)]
#[test]
fn linux_user_default_xdg_and_home() {
    let loc = resolve(&linux(), None, &never);
    assert_eq!(loc.dir, PathBuf::from("/home/anna/.config/hikasa"));
    assert_eq!(loc.cache_dir, PathBuf::from("/home/anna/.cache/hikasa"));
    assert_eq!(loc.mode, ConfigMode::User);
    assert!(loc.is_default);

    let xdg = EnvFacts {
        xdg_config_home: Some("/xdg/config".into()),
        xdg_cache_home: Some("/xdg/cache".into()),
        ..linux()
    };
    let loc = resolve(&xdg, None, &never);
    assert_eq!(loc.dir, PathBuf::from("/xdg/config/hikasa"));
    assert_eq!(loc.cache_dir, PathBuf::from("/xdg/cache/hikasa"));
    assert!(loc.is_default);

    // Relative XDG values are ignored (XDG base directory spec).
    let relative = EnvFacts {
        xdg_config_home: Some("config".into()),
        xdg_cache_home: Some("cache".into()),
        ..linux()
    };
    let loc = resolve(&relative, None, &never);
    assert_eq!(loc.dir, PathBuf::from("/home/anna/.config/hikasa"));
    assert_eq!(loc.cache_dir, PathBuf::from("/home/anna/.cache/hikasa"));
}

#[cfg(unix)]
#[test]
fn other_unix_follows_linux() {
    let facts = EnvFacts {
        platform: Platform::OtherUnix,
        ..linux()
    };
    assert_eq!(
        default_config_dir(&facts),
        Some(PathBuf::from("/home/anna/.config/hikasa"))
    );
}

#[cfg(unix)]
#[test]
fn macos_user_default() {
    let loc = resolve(&macos(), None, &never);
    assert_eq!(
        loc.dir,
        Path::new("/Users/anna")
            .join("Library")
            .join("Application Support")
            .join("Hikasa")
    );
    assert_eq!(
        loc.cache_dir,
        Path::new("/Users/anna")
            .join("Library")
            .join("Caches")
            .join("Hikasa")
    );
    assert!(loc.is_default);
}

#[test]
fn windows_user_default() {
    let facts = windows();
    let loc = resolve(&facts, None, &never);
    assert_eq!(
        loc.dir,
        PathBuf::from(r"C:\Users\Anna Lee\AppData\Roaming").join("Hikasa")
    );
    assert_eq!(
        loc.cache_dir,
        PathBuf::from(r"C:\Users\Anna Lee\AppData\Local")
            .join("Hikasa")
            .join("cache")
    );
    assert!(loc.is_default);
}

#[cfg(unix)]
#[test]
fn explicit_and_portable_modes() {
    let loc = resolve(&linux(), Some("/data/hk"), &never);
    assert_eq!(loc.dir, PathBuf::from("/data/hk"));
    assert_eq!(loc.mode, ConfigMode::Explicit);
    assert_eq!(loc.cache_dir, PathBuf::from("/data/hk/cache"));
    assert!(!loc.is_default);

    // Relative --config-dir is made absolute against the captured cwd; `.` components are dropped.
    let loc = resolve(&linux(), Some("./cfg/./x"), &never);
    assert_eq!(loc.dir, PathBuf::from("/w/cfg/x"));

    // Explicitly naming the default folder keeps the default endpoint (is_default).
    let loc = resolve(&linux(), Some("/home/anna/.config/hikasa"), &never);
    assert_eq!(loc.mode, ConfigMode::Explicit);
    assert!(loc.is_default);

    // Portable: the marker next to the executable.
    let marker = PathBuf::from("/opt/hikasa").join(PORTABLE_MARKER);
    let exists = move |p: &Path| p == marker;
    let loc = resolve(&linux(), None, &exists);
    assert_eq!(loc.dir, PathBuf::from("/opt/hikasa"));
    assert_eq!(loc.mode, ConfigMode::Portable);
    assert_eq!(loc.cache_dir, PathBuf::from("/opt/hikasa/cache"));
    assert!(!loc.is_default);

    // --config-dir wins over the portable marker.
    let loc = resolve(&linux(), Some("/data/hk"), &|_| true);
    assert_eq!(loc.mode, ConfigMode::Explicit);

    // Windows explicit, relative against the Windows cwd.
    let loc = resolve(&windows(), Some(r"D:\cfg"), &never);
    assert_eq!(loc.dir, PathBuf::from(r"D:\cfg"));
    assert_eq!(loc.mode, ConfigMode::Explicit);
}

#[cfg(unix)]
#[test]
fn missing_pieces() {
    let no_cwd = EnvFacts {
        cwd: None,
        ..linux()
    };
    assert!(matches!(
        resolve_config_dir(&no_cwd, Some(Path::new("rel")), &never),
        Err(SessionError::RelativeConfigDir(p)) if p == Path::new("rel")
    ));
    let no_home = EnvFacts {
        home: None,
        ..linux()
    };
    assert!(matches!(
        resolve_config_dir(&no_home, None, &never),
        Err(SessionError::NoConfigDir)
    ));
    assert_eq!(default_config_dir(&no_home), None);
    // XDG alone is enough for the config folder; the cache falls back to the temp dir without a home.
    let xdg_only = EnvFacts {
        xdg_config_home: Some("/xdg".into()),
        ..no_home.clone()
    };
    let loc = resolve(&xdg_only, None, &never);
    assert_eq!(loc.dir, PathBuf::from("/xdg/hikasa"));
    assert_eq!(loc.cache_dir, PathBuf::from("/tmp/hikasa-cache"));
    let mac_no_home = EnvFacts {
        home: None,
        ..macos()
    };
    assert!(matches!(
        resolve_config_dir(&mac_no_home, None, &never),
        Err(SessionError::NoConfigDir)
    ));
    let win_no_appdata = EnvFacts {
        appdata: None,
        ..windows()
    };
    assert!(matches!(
        resolve_config_dir(&win_no_appdata, None, &never),
        Err(SessionError::NoConfigDir)
    ));
    // Explicit config dir without a default: not default, cache inside.
    let loc = resolve(&no_home, Some("/c"), &never);
    assert!(!loc.is_default);
    assert_eq!(loc.cache_dir, PathBuf::from("/c/cache"));
    // Windows user mode without LOCALAPPDATA: the temp fallback.
    let win_no_local = EnvFacts {
        local_appdata: None,
        ..windows()
    };
    let loc = resolve(&win_no_local, None, &never);
    assert_eq!(
        loc.cache_dir,
        PathBuf::from(r"C:\Users\Anna Lee\AppData\Local\Temp").join("hikasa-cache")
    );
}

fn h8(dir: &str) -> String {
    format!("{:016x}", fnv1a64(dir.as_bytes()))[..8].to_owned()
}

fn unix_socket(e: &Endpoint) -> &Path {
    match &e.address {
        EndpointAddress::UnixSocket(p) => p,
        EndpointAddress::NamedPipe(n) => panic!("pipe {n}"),
    }
}

#[test]
fn fnv1a64_known_vectors() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    const AT_COMPILE_TIME: u64 = fnv1a64(b"a");
    assert_eq!(AT_COMPILE_TIME, 0xaf63_dc4c_8601_ec8c);
}

#[cfg(unix)]
#[test]
fn endpoint_rows_of_2_23() {
    let runtime = EnvFacts {
        xdg_runtime_dir: Some("/run/user/1000".into()),
        ..linux()
    };
    // Row 1: Linux, default config, XDG_RUNTIME_DIR.
    let config = resolve(&runtime, None, &never);
    let e = Endpoint::for_config(&runtime, &config).unwrap();
    assert_eq!(unix_socket(&e), Path::new("/run/user/1000/hikasa.sock"));
    assert_eq!(
        e.lock_path,
        PathBuf::from("/run/user/1000/hikasa.sock.lock")
    );
    assert_eq!(e.own_dir, None);
    assert_eq!(e.display(), "/run/user/1000/hikasa.sock");

    // Row 2: --config-dir /data/hk.
    let config = resolve(&runtime, Some("/data/hk"), &never);
    let e = Endpoint::for_config(&runtime, &config).unwrap();
    let expected = format!("/run/user/1000/hikasa-{}.sock", h8("/data/hk"));
    assert_eq!(unix_socket(&e), Path::new(&expected));
    assert_eq!(e.lock_path, PathBuf::from(format!("{expected}.lock")));

    // Row 3: portable install, no XDG_RUNTIME_DIR, uid 1000 → Hikasa's own folder in the temp dir.
    let config = resolve(&linux(), None, &|p: &Path| p.ends_with(PORTABLE_MARKER));
    let e = Endpoint::for_config(&linux(), &config).unwrap();
    let expected = format!("/tmp/hikasa-1000/hikasa-{}.sock", h8("/opt/hikasa"));
    assert_eq!(unix_socket(&e), Path::new(&expected));
    assert_eq!(e.own_dir, Some(PathBuf::from("/tmp/hikasa-1000")));

    // Row 4: macOS default → $TMPDIR/hikasa.sock.
    let config = resolve(&macos(), None, &never);
    let e = Endpoint::for_config(&macos(), &config).unwrap();
    assert_eq!(unix_socket(&e), Path::new("/var/folders/xy/T/hikasa.sock"));
    assert_eq!(e.own_dir, None);

    // Row 5: Windows, user "Anna Lee", default.
    let config = resolve(&windows(), None, &never);
    let e = Endpoint::for_config(&windows(), &config).unwrap();
    assert_eq!(
        e.address,
        EndpointAddress::NamedPipe("hikasa-Anna_Lee".into())
    );
    assert_eq!(e.display(), r"\\.\pipe\hikasa-Anna_Lee");
    assert_eq!(
        e.lock_path,
        PathBuf::from(r"C:\Users\Anna Lee\AppData\Local\Temp").join("hikasa-Anna_Lee.lock")
    );
    // Windows with --config-dir: the suffix hashes the UTF-16LE code units.
    let config = resolve(&windows(), Some(r"D:\cfg"), &never);
    let e = Endpoint::for_config(&windows(), &config).unwrap();
    let utf16: Vec<u8> = r"D:\cfg"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let suffix = format!("{:016x}", fnv1a64(&utf16))[..8].to_owned();
    assert_eq!(
        e.address,
        EndpointAddress::NamedPipe(format!("hikasa-Anna_Lee-{suffix}"))
    );

    // Row 6: a Unix path longer than 100 bytes → `hk-<h16>.sock`. The hashed name (22 bytes) is longer than
    // `hikasa[-<h8>].sock` (11 or 20 bytes), so `<dir>/hk-<h16>.sock` is itself too long whenever the original
    // was, and the second fallback, Hikasa's own folder in /tmp, applies (§2.23 as written; see the report).
    for extra in [80, 90, 100] {
        let long_dir = format!("/run/user/{}", "d".repeat(extra));
        let long = EnvFacts {
            xdg_runtime_dir: Some(long_dir.clone().into()),
            ..linux()
        };
        let config = resolve(&long, None, &never);
        let e = Endpoint::for_config(&long, &config).unwrap();
        let original = format!("{long_dir}/hikasa.sock");
        if original.len() <= MAX_SOCKET_PATH {
            assert_eq!(unix_socket(&e), Path::new(&original));
            continue;
        }
        let hashed = format!("hk-{:016x}.sock", fnv1a64(original.as_bytes()));
        assert!(format!("{long_dir}/{hashed}").len() > MAX_SOCKET_PATH);
        let expected = format!("/tmp/hikasa-1000/{hashed}");
        assert_eq!(unix_socket(&e), Path::new(&expected));
        assert!(expected.len() <= MAX_SOCKET_PATH);
        assert_eq!(e.own_dir, Some(PathBuf::from("/tmp/hikasa-1000")));
        assert_eq!(e.lock_path, PathBuf::from(format!("{expected}.lock")));
    }

    // Row 7: HIKASA_IPC_ENDPOINT wins.
    let overridden = EnvFacts {
        ipc_endpoint_override: Some(OsString::from("/tmp/t/x.sock")),
        ..runtime.clone()
    };
    let config = resolve(&overridden, Some("/data/hk"), &never);
    let e = Endpoint::for_config(&overridden, &config).unwrap();
    assert_eq!(unix_socket(&e), Path::new("/tmp/t/x.sock"));
    assert_eq!(e.lock_path, PathBuf::from("/tmp/t/x.sock.lock"));
    assert_eq!(e.own_dir, None);
    let win_override = EnvFacts {
        ipc_endpoint_override: Some(OsString::from(r"\\.\pipe\hk-test")),
        ..windows()
    };
    let config = resolve(&win_override, None, &never);
    let e = Endpoint::for_config(&win_override, &config).unwrap();
    assert_eq!(e.address, EndpointAddress::NamedPipe("hk-test".into()));
}

#[cfg(unix)]
#[test]
fn endpoint_id_without_uid() {
    let no_uid = EnvFacts {
        uid: None,
        user: Some("a b/c".into()),
        ..linux()
    };
    let config = resolve(&no_uid, None, &never);
    let e = Endpoint::for_config(&no_uid, &config).unwrap();
    assert_eq!(unix_socket(&e), Path::new("/tmp/hikasa-a_b_c/hikasa.sock"));
    let nobody = EnvFacts {
        uid: None,
        user: None,
        ..linux()
    };
    let config = resolve(&nobody, None, &never);
    let e = Endpoint::for_config(&nobody, &config).unwrap();
    assert_eq!(unix_socket(&e), Path::new("/tmp/hikasa-user/hikasa.sock"));
    let win_nobody = EnvFacts {
        user: None,
        ..windows()
    };
    let config = resolve(&win_nobody, None, &never);
    let e = Endpoint::for_config(&win_nobody, &config).unwrap();
    assert_eq!(e.address, EndpointAddress::NamedPipe("hikasa-user".into()));
}

#[cfg(unix)]
#[test]
fn two_configurations_never_share_an_endpoint() {
    let facts = EnvFacts {
        xdg_runtime_dir: Some("/run/user/1000".into()),
        ..linux()
    };
    let a = Endpoint::for_config(&facts, &resolve(&facts, Some("/a"), &never)).unwrap();
    let b = Endpoint::for_config(&facts, &resolve(&facts, Some("/b"), &never)).unwrap();
    let d = Endpoint::for_config(&facts, &resolve(&facts, None, &never)).unwrap();
    assert_ne!(a, b);
    assert_ne!(a.lock_path, b.lock_path);
    assert_ne!(a, d);
    let again = Endpoint::for_config(&facts, &resolve(&facts, Some("/a/"), &never)).unwrap();
    assert_eq!(a, again, "the same configuration reaches the same endpoint");
}

/// The Windows rows, on every platform (the facts are pure values; paths are joined like the code joins them).
#[test]
fn windows_rows_everywhere() {
    let loc = resolve(&windows(), Some(r"D:\cfg"), &never);
    assert_eq!(loc.dir, PathBuf::from(r"D:\cfg"));
    assert_eq!(loc.mode, ConfigMode::Explicit);
    assert!(!loc.is_default);
    let win_no_appdata = EnvFacts {
        appdata: None,
        ..windows()
    };
    assert!(matches!(
        resolve_config_dir(&win_no_appdata, None, &never),
        Err(SessionError::NoConfigDir)
    ));
    let e = Endpoint::for_config(&windows(), &resolve(&windows(), None, &never)).unwrap();
    assert_eq!(
        e.address,
        EndpointAddress::NamedPipe("hikasa-Anna_Lee".into())
    );
    assert_eq!(e.display(), r"\\.\pipe\hikasa-Anna_Lee");
    let win_nobody = EnvFacts {
        user: None,
        ..windows()
    };
    let e = Endpoint::for_config(&win_nobody, &resolve(&win_nobody, None, &never)).unwrap();
    assert_eq!(e.address, EndpointAddress::NamedPipe("hikasa-user".into()));
}

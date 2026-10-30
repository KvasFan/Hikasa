//! [`Spec`]: the name of a file or folder on any back-end, its parser and its canonical form (§2.1; ARCH-020,
//! VFS-010; D-05, D-06).
//!
//! Parsing is *style-explicit* ([`PathStyle`]) and takes every ambient fact (current and home directory) from a
//! [`SpecContext`] that adapters capture once, so the Windows rules are unit-tested on Linux and no
//! environment variable is ever read here.
//!
//! ```
//! use hikasa_core::spec::{PathStyle, Spec, SpecContext};
//! use std::path::PathBuf;
//!
//! let ctx = SpecContext { style: PathStyle::Unix, cwd: Some(PathBuf::from("/w")), home: None };
//! assert_eq!(Spec::parse("x//y/./z", &ctx).unwrap().canonical(), "/w/x/y/z");
//! assert_eq!(Spec::parse("ftp://user:pw@Host/p", &ctx).unwrap().canonical(), "ftp://user@host/p");
//! assert_eq!(Spec::parse("-", &ctx).unwrap(), Spec::Stdin);
//! ```

mod error;
mod local;
mod url;

use std::ffi::OsStr;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use error::SpecError;
pub use local::{LocalPath, normalize_local};
pub use url::{HttpUrl, RemoteScheme, RemoteUrl, Secret};

use local::{Unit, os, starts_with_ascii_ci};

/// Syntax of local paths. Parsing is style-explicit, so Windows rules are unit-tested on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathStyle {
    /// `/`-separated, `~` expansion, `..` kept (D-06).
    Unix,
    /// Drive letters, UNC, `\\?\` verbatim, `..` resolved lexically.
    Windows,
}

impl PathStyle {
    /// Style of the compilation target.
    pub const HOST: PathStyle = if cfg!(windows) {
        PathStyle::Windows
    } else {
        PathStyle::Unix
    };
}

/// Ambient facts needed to resolve user-typed specs. Captured by adapters; core never reads them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecContext {
    /// Local path syntax.
    pub style: PathStyle,
    /// Absolute current directory for relative paths; `None` makes relative paths an error.
    pub cwd: Option<PathBuf>,
    /// Home directory for `~`; `None` makes `~` an error.
    pub home: Option<PathBuf>,
}

impl SpecContext {
    /// A context that accepts only absolute specs (used when deserialising persisted specs).
    pub const fn absolute_only(style: PathStyle) -> SpecContext {
        SpecContext {
            style,
            cwd: None,
            home: None,
        }
    }
}

/// Identifies a file or folder on any back-end (ARCH 3.1). `Display` is the canonical string (ARCH-020).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Spec {
    /// Local file system path.
    Local(LocalPath),
    /// Archive member (reserved: never produced by `parse` in M1; the M2 VFS resolver creates it).
    ArchiveMember {
        /// The archive.
        archive: Box<Spec>,
        /// `/`-separated member path inside the archive.
        member: String,
    },
    /// Snapshot member (reserved, P1).
    Snapshot {
        /// The `.hksnap` file.
        file: LocalPath,
        /// `/`-separated member path inside the snapshot.
        member: String,
    },
    /// Remote URL (parsed, unsupported in M1).
    Remote(RemoteUrl),
    /// `profile:Name[?path]` (parsed, unsupported in M1).
    Profile {
        /// Profile name (non-empty, without `?`).
        name: String,
        /// Path inside the profile's root, verbatim.
        path: Option<String>,
    },
    /// `http(s)://…` single file (parsed, unsupported in M1).
    Http(HttpUrl),
    /// `clipboard://`.
    Clipboard,
    /// `-` (standard input).
    Stdin,
}

const CLIPBOARD: &str = "clipboard://";
const PROFILE: &str = "profile:";

/// Result of the scheme step (§2.1 step 5) on the ASCII prefix of an input.
enum SchemeStep {
    /// Not a URL (or a Windows drive letter): continue with the local path.
    Local,
    /// `http` or `https`.
    Http { secure: bool },
    /// A recognised remote scheme.
    Remote(RemoteScheme),
    /// `scheme://` with an unknown scheme.
    Unknown(String),
}

fn scheme_step(prefix: &[u8], style: PathStyle) -> (SchemeStep, usize) {
    let Some(len) = url::scheme_len(prefix) else {
        return (SchemeStep::Local, 0);
    };
    let scheme = String::from_utf8_lossy(prefix.get(..len).unwrap_or(&[])).into_owned();
    if len == 1 && style == PathStyle::Windows {
        return (SchemeStep::Local, 0);
    }
    if scheme.eq_ignore_ascii_case("http") {
        return (SchemeStep::Http { secure: false }, len);
    }
    if scheme.eq_ignore_ascii_case("https") {
        return (SchemeStep::Http { secure: true }, len);
    }
    match RemoteScheme::from_scheme(&scheme) {
        Some(remote) => (SchemeStep::Remote(remote), len),
        None => (SchemeStep::Unknown(scheme), len),
    }
}

impl Spec {
    /// Parses a user-typed spec (§2.1).
    ///
    /// Order: NUL → [`SpecError::Nul`]; blank → [`SpecError::Empty`] (the input is otherwise **not** trimmed:
    /// leading and trailing spaces are legal in file names); `-` → [`Spec::Stdin`]; `clipboard://`
    /// (ASCII-case-insensitive) → [`Spec::Clipboard`]; `profile:` prefix → [`Spec::Profile`];
    /// `^[A-Za-z][A-Za-z0-9+.-]*://` → a URL (`http`/`https`, a [`RemoteScheme`], or
    /// [`SpecError::UnknownScheme`]; a one-letter scheme with [`PathStyle::Windows`] is a drive letter);
    /// anything else is a local path ([`normalize_local`]).
    pub fn parse(input: &str, ctx: &SpecContext) -> Result<Spec, SpecError> {
        if input.contains('\0') {
            return Err(SpecError::Nul);
        }
        if input.trim().is_empty() {
            return Err(SpecError::Empty);
        }
        if input == "-" {
            return Ok(Spec::Stdin);
        }
        let bytes = input.as_bytes();
        if starts_with_ascii_ci(bytes, CLIPBOARD.as_bytes()) {
            return if input.len() == CLIPBOARD.len() {
                Ok(Spec::Clipboard)
            } else {
                Err(SpecError::MalformedUrl {
                    input: url::redact(input),
                    reason: "clipboard:// takes no path",
                })
            };
        }
        if starts_with_ascii_ci(bytes, PROFILE.as_bytes()) {
            let rest = input.get(PROFILE.len()..).unwrap_or("");
            let (name, path) = match rest.split_once('?') {
                Some((name, path)) => (name, Some(path.to_owned())),
                None => (rest, None),
            };
            if name.is_empty() {
                return Err(SpecError::MalformedProfile {
                    input: input.to_owned(),
                });
            }
            return Ok(Spec::Profile {
                name: name.to_owned(),
                path,
            });
        }
        match scheme_step(bytes, ctx.style) {
            (SchemeStep::Local, _) => {}
            (SchemeStep::Http { secure }, len) => {
                return url::parse_http(input, len, secure).map(Spec::Http);
            }
            (SchemeStep::Remote(scheme), len) => {
                return url::parse_remote(input, len, scheme).map(Spec::Remote);
            }
            (SchemeStep::Unknown(scheme), _) => return Err(SpecError::UnknownScheme { scheme }),
        }
        local::normalize_host(
            OsStr::new(input),
            ctx.style,
            ctx.cwd.as_deref(),
            ctx.home.as_deref(),
        )
        .map(Spec::Local)
    }

    /// Parses a command-line argument; non-UTF-8 input is accepted for local paths only (a non-UTF-8 input
    /// that looks like a URL, `profile:` or `clipboard://` spec is rejected, never reinterpreted as a path).
    pub fn parse_os(input: &OsStr, ctx: &SpecContext) -> Result<Spec, SpecError> {
        if let Some(text) = input.to_str() {
            return Spec::parse(text, ctx);
        }
        let units = os::units(input);
        let ascii_prefix: Vec<u8> = units.iter().map_while(|&u| u.ascii()).collect();
        let lossy = input.to_string_lossy().into_owned();
        if units.iter().any(|&u| local::is(u, 0)) {
            return Err(SpecError::Nul);
        }
        if starts_with_ascii_ci(&units, CLIPBOARD.as_bytes()) {
            return Err(SpecError::MalformedUrl {
                input: url::redact(&lossy),
                reason: "not valid UTF-8",
            });
        }
        if starts_with_ascii_ci(&units, PROFILE.as_bytes()) {
            return Err(SpecError::MalformedProfile { input: lossy });
        }
        match scheme_step(&ascii_prefix, ctx.style) {
            (SchemeStep::Local, _) => {}
            (SchemeStep::Http { .. } | SchemeStep::Remote(_), _) => {
                return Err(SpecError::MalformedUrl {
                    input: url::redact(&lossy),
                    reason: "not valid UTF-8",
                });
            }
            (SchemeStep::Unknown(scheme), _) => return Err(SpecError::UnknownScheme { scheme }),
        }
        local::normalize_host(input, ctx.style, ctx.cwd.as_deref(), ctx.home.as_deref())
            .map(Spec::Local)
    }

    /// Canonical string (same as `Display`); never contains a password.
    pub fn canonical(&self) -> String {
        self.to_string()
    }

    /// True when `canonical()` lost information (non-UTF-8 local path). Lossy specs must not be persisted
    /// (serialising one is an error).
    pub fn is_lossy(&self) -> bool {
        match self {
            Spec::Local(path) => path.as_path().to_str().is_none(),
            Spec::ArchiveMember { archive, .. } => archive.is_lossy(),
            Spec::Snapshot { file, .. } => file.as_path().to_str().is_none(),
            Spec::Remote(_)
            | Spec::Profile { .. }
            | Spec::Http(_)
            | Spec::Clipboard
            | Spec::Stdin => false,
        }
    }

    /// Stable scheme name: `"local"`, `"archive"`, `"snapshot"`, `"ftp"`, … (every [`RemoteScheme::as_str`]),
    /// `"profile"`, `"http"`, `"https"`, `"clipboard"`, `"stdin"`.
    pub fn scheme_name(&self) -> &'static str {
        match self {
            Spec::Local(_) => "local",
            Spec::ArchiveMember { .. } => "archive",
            Spec::Snapshot { .. } => "snapshot",
            Spec::Remote(url) => url.scheme.as_str(),
            Spec::Profile { .. } => "profile",
            Spec::Http(url) => {
                if url.secure {
                    "https"
                } else {
                    "http"
                }
            }
            Spec::Clipboard => "clipboard",
            Spec::Stdin => "stdin",
        }
    }

    /// Final name component for format detection (`-` → `None`, `clipboard://` → `None`, a root → `None`).
    /// URL paths are not percent-decoded; the query and fragment of HTTP URLs are ignored.
    pub fn file_name(&self) -> Option<String> {
        fn last_segment(path: &str) -> Option<String> {
            let name = path.rsplit('/').next()?;
            (!name.is_empty()).then(|| name.to_owned())
        }
        match self {
            Spec::Local(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Spec::ArchiveMember { member, .. } => last_segment(member),
            Spec::Snapshot { file, member } => {
                if member.is_empty() {
                    file.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                } else {
                    last_segment(member)
                }
            }
            Spec::Remote(url) => last_segment(&url.path),
            Spec::Profile { path, .. } => path
                .as_deref()
                .and_then(|p| last_segment(&p.replace('\\', "/"))),
            Spec::Http(url) => {
                let path_end = url.rest.find(['?', '#']).unwrap_or(url.rest.len());
                last_segment(url.rest.get(..path_end).unwrap_or(""))
            }
            Spec::Clipboard | Spec::Stdin => None,
        }
    }

    /// The parent of a local path (`None` for a root); `None` for every other spec kind (M2 W3, additive;
    /// archive members follow in W5). Operations reach parent folders through this and `Vfs::child` only.
    pub fn parent(&self) -> Option<Spec> {
        match self {
            Spec::Local(path) => path.parent().map(Spec::Local),
            Spec::ArchiveMember { .. }
            | Spec::Snapshot { .. }
            | Spec::Remote(_)
            | Spec::Profile { .. }
            | Spec::Http(_)
            | Spec::Clipboard
            | Spec::Stdin => None,
        }
    }

    /// Returns a copy with every secret removed (`RemoteUrl::password = None`, `HttpUrl::password = None`).
    pub fn without_secrets(&self) -> Spec {
        match self {
            Spec::Remote(url) => Spec::Remote(RemoteUrl {
                password: None,
                ..url.clone()
            }),
            Spec::Http(url) => Spec::Http(HttpUrl {
                password: None,
                ..url.clone()
            }),
            Spec::ArchiveMember { archive, member } => Spec::ArchiveMember {
                archive: Box::new(archive.without_secrets()),
                member: member.clone(),
            },
            Spec::Local(_)
            | Spec::Snapshot { .. }
            | Spec::Profile { .. }
            | Spec::Clipboard
            | Spec::Stdin => self.clone(),
        }
    }
}

impl fmt::Display for Spec {
    /// The canonical form (§2.1): `Local` → the stored path (lossy for non-UTF-8 paths); `Stdin` → `-`;
    /// `Clipboard` → `clipboard://`; `Profile` → `profile:<name>[?<path>]`; `Remote` →
    /// `<scheme>://[<user>@]<host>[:<port>]<path>`; `Http` → `http[s]://[<user>@]<host>[:<port>]<rest>` (never a
    /// password); the reserved `ArchiveMember`/`Snapshot` → `<container>/<member>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Spec::Local(path) => f.write_str(&path.as_path().to_string_lossy()),
            Spec::ArchiveMember { archive, member } => write!(f, "{archive}/{member}"),
            Spec::Snapshot { file, member } => {
                write!(f, "{}/{member}", file.as_path().to_string_lossy())
            }
            Spec::Remote(url) => fmt::Display::fmt(url, f),
            Spec::Profile {
                name,
                path: Some(path),
            } => write!(f, "{PROFILE}{name}?{path}"),
            Spec::Profile { name, path: None } => write!(f, "{PROFILE}{name}"),
            Spec::Http(url) => fmt::Display::fmt(url, f),
            Spec::Clipboard => f.write_str(CLIPBOARD),
            Spec::Stdin => f.write_str("-"),
        }
    }
}

impl Serialize for Spec {
    /// The canonical string. A lossy spec (non-UTF-8 path) is an error: it must not be persisted.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.is_lossy() {
            return Err(serde::ser::Error::custom(
                "a spec with a non-UTF-8 path cannot be serialised",
            ));
        }
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Spec {
    /// [`Spec::parse`] with [`SpecContext::absolute_only`]`(`[`PathStyle::HOST`]`)`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Spec, D::Error> {
        let text = String::deserialize(deserializer)?;
        Spec::parse(&text, &SpecContext::absolute_only(PathStyle::HOST))
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_ctx() -> SpecContext {
        SpecContext {
            style: PathStyle::Unix,
            cwd: Some(PathBuf::from("/w")),
            home: Some(PathBuf::from("/home/u")),
        }
    }

    #[test]
    fn scheme_names() {
        let ctx = unix_ctx();
        let cases = [
            ("/x", "local"),
            ("ftp://h/p", "ftp"),
            ("svn+ssh://h/p", "svn+ssh"),
            ("profile:P", "profile"),
            ("http://h/", "http"),
            ("HTTPS://h/", "https"),
            ("clipboard://", "clipboard"),
            ("-", "stdin"),
        ];
        for (input, scheme) in cases {
            assert_eq!(
                Spec::parse(input, &ctx).unwrap().scheme_name(),
                scheme,
                "{input}"
            );
        }
        let archive = Spec::ArchiveMember {
            archive: Box::new(Spec::parse("/a.zip", &ctx).unwrap()),
            member: "x/y.txt".into(),
        };
        assert_eq!(archive.scheme_name(), "archive");
        assert_eq!(archive.canonical(), "/a.zip/x/y.txt");
        assert_eq!(archive.file_name().as_deref(), Some("y.txt"));
    }

    #[test]
    fn file_names() {
        let ctx = unix_ctx();
        let name = |s: &str| Spec::parse(s, &ctx).unwrap().file_name();
        assert_eq!(name("/a/b.c").as_deref(), Some("b.c"));
        assert_eq!(name("/"), None);
        assert_eq!(name("-"), None);
        assert_eq!(name("clipboard://"), None);
        assert_eq!(name("ftp://h/dir/f.txt").as_deref(), Some("f.txt"));
        assert_eq!(name("s3://bucket/prefix/"), None);
        assert_eq!(name("ftp://h"), None);
        assert_eq!(name("https://h/a/x.html?q=1#f").as_deref(), Some("x.html"));
        assert_eq!(name("profile:P?dir/f.txt").as_deref(), Some("f.txt"));
        assert_eq!(name("profile:P"), None);
    }

    #[test]
    fn non_utf8_specs() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let ctx = unix_ctx();
            let spec = Spec::parse_os(OsStr::from_bytes(b"dir/\xff.txt"), &ctx).unwrap();
            assert!(spec.is_lossy());
            assert_eq!(spec.canonical(), "/w/dir/\u{FFFD}.txt");
            assert!(
                serde_json::to_string(&spec).is_err(),
                "lossy specs are not serialised"
            );
            assert!(matches!(
                Spec::parse_os(OsStr::from_bytes(b"ftp://h/\xff"), &ctx),
                Err(SpecError::MalformedUrl { .. })
            ));
            assert!(matches!(
                Spec::parse_os(OsStr::from_bytes(b"foo://\xff"), &ctx),
                Err(SpecError::UnknownScheme { .. })
            ));
            assert!(matches!(
                Spec::parse_os(OsStr::from_bytes(b"profile:\xff"), &ctx),
                Err(SpecError::MalformedProfile { .. })
            ));
            assert_eq!(
                Spec::parse_os(OsStr::new("~/x"), &ctx).unwrap().canonical(),
                "/home/u/x"
            );
        }
    }

    #[test]
    fn serde_uses_canonical_strings() {
        let ctx = unix_ctx();
        let spec = Spec::parse("sftp://u:pw@h:22/p", &ctx).unwrap();
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, "\"sftp://u@h:22/p\"");
        let back: Spec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec.without_secrets());
        let http = Spec::parse("https://u:pw@h/x", &ctx).unwrap();
        assert_eq!(serde_json::to_string(&http).unwrap(), "\"https://u@h/x\"");
        let Spec::Http(url) = http.without_secrets() else {
            panic!("{http:?}")
        };
        assert_eq!((url.user.as_deref(), url.password), (Some("u"), None));
        assert!(
            serde_json::from_str::<Spec>("\"relative/path\"").is_err(),
            "persisted specs are absolute"
        );
    }

    #[test]
    fn malformed_url_messages_never_contain_passwords() {
        let err = Spec::parse("ftp://user:hunter2@host:0/p", &unix_ctx()).unwrap_err();
        assert!(!err.to_string().contains("hunter2"), "{err}");
        assert!(err.to_string().contains("user:***@host"), "{err}");
    }
}

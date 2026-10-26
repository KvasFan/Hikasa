//! Remote and HTTP URLs: [`RemoteScheme`], [`Secret`], [`RemoteUrl`], [`HttpUrl`] (§2.1 step 5, D-05).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::SpecError;
use crate::case;

/// Remote URL schemes recognised by the parser (resolving them is unsupported in M1, D-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteScheme {
    /// `ftp://`
    Ftp,
    /// `ftps://`
    Ftps,
    /// `sftp://`
    Sftp,
    /// `dav://` (WebDAV)
    Dav,
    /// `davs://` (WebDAV over TLS)
    Davs,
    /// `s3://` (S3-compatible object storage; the host is the bucket)
    S3,
    /// `svn://`
    Svn,
    /// `svn+http://`
    SvnHttp,
    /// `svn+https://`
    SvnHttps,
    /// `svn+ssh://`
    SvnSsh,
    /// `mtp://` (media devices)
    Mtp,
}

impl RemoteScheme {
    /// Every scheme, in declaration order.
    pub const ALL: [RemoteScheme; 11] = [
        RemoteScheme::Ftp,
        RemoteScheme::Ftps,
        RemoteScheme::Sftp,
        RemoteScheme::Dav,
        RemoteScheme::Davs,
        RemoteScheme::S3,
        RemoteScheme::Svn,
        RemoteScheme::SvnHttp,
        RemoteScheme::SvnHttps,
        RemoteScheme::SvnSsh,
        RemoteScheme::Mtp,
    ];

    /// Canonical lowercase scheme text (`"svn+https"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            RemoteScheme::Ftp => "ftp",
            RemoteScheme::Ftps => "ftps",
            RemoteScheme::Sftp => "sftp",
            RemoteScheme::Dav => "dav",
            RemoteScheme::Davs => "davs",
            RemoteScheme::S3 => "s3",
            RemoteScheme::Svn => "svn",
            RemoteScheme::SvnHttp => "svn+http",
            RemoteScheme::SvnHttps => "svn+https",
            RemoteScheme::SvnSsh => "svn+ssh",
            RemoteScheme::Mtp => "mtp",
        }
    }

    /// Case-insensitive lookup.
    pub fn from_scheme(s: &str) -> Option<RemoteScheme> {
        RemoteScheme::ALL
            .into_iter()
            .find(|scheme| scheme.as_str().eq_ignore_ascii_case(s))
    }
}

/// A credential taken from a URL. Never printed, never serialised; `Debug` shows `Secret(***)`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Secret(String);

impl Secret {
    /// Wraps a credential.
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// Reveals the value (only for back-ends; M1 never calls this).
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// `scheme://[user[:password]@]host[:port][/path]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteUrl {
    /// Scheme.
    pub scheme: RemoteScheme,
    /// User name (percent-decoding is not applied in M1; stored verbatim).
    pub user: Option<String>,
    /// Password; omitted from every canonical form (VFS-010).
    pub password: Option<Secret>,
    /// Lower-cased host (simple 1:1 folding, [`case::fold`]); an IPv6 literal keeps its brackets. For `s3://`
    /// the bucket.
    pub host: String,
    /// Explicit port.
    pub port: Option<u16>,
    /// Verbatim remainder starting at the first `'/'` after the authority (may be empty).
    pub path: String,
}

impl fmt::Display for RemoteUrl {
    /// `<scheme>://[<user>@]<host>[:<port>]<path>` — never the password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://", self.scheme.as_str())?;
        if let Some(user) = &self.user {
            write!(f, "{user}@")?;
        }
        f.write_str(&self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        f.write_str(&self.path)
    }
}

/// An `http(s)://[user[:password]@]host[:port][rest]` single-file URL (P1 back-end).
///
/// The user information is kept exactly like [`RemoteUrl`]'s (§2.1 step 5, D-05): the user verbatim, the
/// password as a [`Secret`] that no canonical form, `Debug` output or error message ever shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HttpUrl {
    /// `https` when true.
    pub secure: bool,
    /// User name (percent-decoding is not applied in M1; stored verbatim).
    pub user: Option<String>,
    /// Password; omitted from every canonical form (VFS-010).
    pub password: Option<Secret>,
    /// Lower-cased host (simple 1:1 folding, [`case::fold`]); an IPv6 literal keeps its brackets.
    pub host: String,
    /// Explicit port.
    pub port: Option<u16>,
    /// Verbatim path, query and fragment: everything from the first `/`, `?` or `#` after the authority (may be
    /// empty).
    pub rest: String,
}

impl fmt::Display for HttpUrl {
    /// `http[s]://[<user>@]<host>[:<port>]<rest>` — never the password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.secure { "https://" } else { "http://" })?;
        if let Some(user) = &self.user {
            write!(f, "{user}@")?;
        }
        f.write_str(&self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        f.write_str(&self.rest)
    }
}

/// Length of the scheme when `input` starts with `^[A-Za-z][A-Za-z0-9+.-]*://` (the `://` excluded).
pub(crate) fn scheme_len(input: &[u8]) -> Option<usize> {
    let first = *input.first()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let len = 1 + input
        .iter()
        .skip(1)
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
        .count();
    input.get(len..)?.starts_with(b"://").then_some(len)
}

/// `input` with the password of a `scheme://user:password@…` authority replaced by `***`, for error messages
/// (secrets never appear in messages, VFS-010).
///
/// The redaction is conservative, because it runs on input that failed to parse: when the authority (the text
/// up to the first `/`) holds user information, everything between its first `:` and its last `@` is masked;
/// otherwise a password may itself contain `/` (`ftp://user:pa/ss@host/p`), so everything between the first
/// `:` after `://` and the last `@` of the whole input is masked. Masking too much only shortens a message.
pub(crate) fn redact(input: &str) -> String {
    let Some(start) = input.find("://").map(|at| at + 3) else {
        return input.to_owned();
    };
    let after = input.get(start..).unwrap_or("");
    let authority_end = after.find('/').unwrap_or(after.len());
    let authority = after.get(..authority_end).unwrap_or("");
    let (colon, at) = match authority.rfind('@') {
        // `user[:password]@host` inside the authority: only a `:` before that `@` starts a password.
        Some(at) => match authority.get(..at).and_then(|info| info.find(':')) {
            Some(colon) => (colon, at),
            None => return input.to_owned(),
        },
        // No `@` in the authority: a `/` inside the password may have ended it early.
        None => match (after.find(':'), after.rfind('@')) {
            (Some(colon), Some(at)) if colon < at => (colon, at),
            _ => return input.to_owned(),
        },
    };
    let password_start = start + colon + 1;
    let password_end = start + at;
    let mut out = String::with_capacity(input.len());
    out.push_str(input.get(..password_start).unwrap_or(""));
    out.push_str("***");
    out.push_str(input.get(password_end..).unwrap_or(""));
    out
}

fn malformed(input: &str, reason: &'static str) -> SpecError {
    SpecError::MalformedUrl {
        input: redact(input),
        reason,
    }
}

/// Parses `host[:port]` (with `[v6]:port`); the host is folded to lower case and must be non-empty.
fn parse_host_port(input: &str, hostport: &str) -> Result<(String, Option<u16>), SpecError> {
    let (host, port) = if hostport.starts_with('[') {
        let close = hostport
            .find(']')
            .ok_or_else(|| malformed(input, "unclosed IPv6 literal"))?;
        let host = hostport.get(..=close).unwrap_or("");
        if host.len() <= 2 {
            return Err(malformed(input, "empty host"));
        }
        let rest = hostport.get(close + 1..).unwrap_or("");
        let port = if rest.is_empty() {
            None
        } else if let Some(port) = rest.strip_prefix(':') {
            Some(parse_port(input, port)?)
        } else {
            return Err(malformed(input, "unexpected text after an IPv6 literal"));
        };
        (host, port)
    } else {
        match hostport.find(':') {
            Some(colon) => (
                hostport.get(..colon).unwrap_or(""),
                Some(parse_port(input, hostport.get(colon + 1..).unwrap_or(""))?),
            ),
            None => (hostport, None),
        }
    };
    if host.is_empty() {
        return Err(malformed(input, "empty host"));
    }
    Ok((case::fold(host).into_owned(), port))
}

/// A port is 1–65535, written with ASCII digits only.
fn parse_port(input: &str, text: &str) -> Result<u16, SpecError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed(input, "invalid port"));
    }
    match text.parse::<u32>() {
        Ok(port @ 1..=65535) => u16::try_from(port).map_err(|_| malformed(input, "invalid port")),
        _ => Err(malformed(input, "port out of range (1-65535)")),
    }
}

/// `userinfo@hostport` split at the **last** `@`, and `user[:password]` split at the first `:` (§2.1 step 5).
fn split_authority(authority: &str) -> (Option<String>, Option<Secret>, &str) {
    let Some(at) = authority.rfind('@') else {
        return (None, None, authority);
    };
    let info = authority.get(..at).unwrap_or("");
    let hostport = authority.get(at + 1..).unwrap_or("");
    match info.find(':') {
        Some(colon) => (
            Some(info.get(..colon).unwrap_or("").to_owned()),
            Some(Secret(info.get(colon + 1..).unwrap_or("").to_owned())),
            hostport,
        ),
        None => (Some(info.to_owned()), None, hostport),
    }
}

/// Parses a remote URL whose scheme (`scheme_len` bytes) is `scheme`.
pub(crate) fn parse_remote(
    input: &str,
    scheme_len: usize,
    scheme: RemoteScheme,
) -> Result<RemoteUrl, SpecError> {
    let after = input.get(scheme_len + 3..).unwrap_or("");
    let authority_end = after.find('/').unwrap_or(after.len());
    let authority = after.get(..authority_end).unwrap_or("");
    let path = after.get(authority_end..).unwrap_or("");
    let (user, password, hostport) = split_authority(authority);
    let (host, port) = parse_host_port(input, hostport)?;
    Ok(RemoteUrl {
        scheme,
        user,
        password,
        host,
        port,
        path: path.to_owned(),
    })
}

/// Parses an `http(s)` URL whose scheme is `scheme_len` bytes long. The authority ends at the first `/`, `?` or
/// `#` and is parsed like a remote authority (§2.1 step 5): user information is split at the last `@`, user and
/// password at the first `:` (D-05: http(s) specs parse successfully, so specs written by later versions stay
/// readable).
pub(crate) fn parse_http(
    input: &str,
    scheme_len: usize,
    secure: bool,
) -> Result<HttpUrl, SpecError> {
    let after = input.get(scheme_len + 3..).unwrap_or("");
    let authority_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = after.get(..authority_end).unwrap_or("");
    let rest = after.get(authority_end..).unwrap_or("");
    let (user, password, hostport) = split_authority(authority);
    let (host, port) = parse_host_port(input, hostport)?;
    Ok(HttpUrl {
        secure,
        user,
        password,
        host,
        port,
        rest: rest.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_names() {
        for scheme in RemoteScheme::ALL {
            assert_eq!(RemoteScheme::from_scheme(scheme.as_str()), Some(scheme));
            assert_eq!(
                RemoteScheme::from_scheme(&scheme.as_str().to_uppercase()),
                Some(scheme)
            );
        }
        assert_eq!(RemoteScheme::from_scheme("http"), None);
        assert_eq!(
            serde_json::to_string(&RemoteScheme::SvnHttps).unwrap(),
            "\"svn_https\""
        );
    }

    #[test]
    fn scheme_prefix() {
        assert_eq!(scheme_len(b"ftp://x"), Some(3));
        assert_eq!(scheme_len(b"svn+ssh://x"), Some(7));
        assert_eq!(scheme_len(b"C://x"), Some(1));
        assert_eq!(scheme_len(b"1ftp://x"), None);
        assert_eq!(scheme_len(b"ftp:/x"), None);
        assert_eq!(scheme_len(b"f_p://x"), None);
        assert_eq!(scheme_len(b""), None);
    }

    #[test]
    fn redaction() {
        assert_eq!(redact("ftp://u:pw@h:99999/p"), "ftp://u:***@h:99999/p");
        assert_eq!(redact("ftp://u:p@w@h/p"), "ftp://u:***@h/p");
        assert_eq!(redact("ftp://u@h/p:x@y"), "ftp://u@h/p:x@y");
        assert_eq!(redact("ftp://u:pw@h/x@y"), "ftp://u:***@h/x@y");
        assert_eq!(redact("no url"), "no url");
        assert_eq!(redact("ftp://h:99999/p"), "ftp://h:99999/p");
        // A `/` inside the password ends the authority early; everything up to the last `@` is masked.
        assert_eq!(redact("ftp://user:pa/ss@host/p"), "ftp://user:***@host/p");
        assert_eq!(
            redact("https://user:hun/ter2@host/x"),
            "https://user:***@host/x"
        );
        assert_eq!(redact("ftp://u:pa/ss@h/x@y"), "ftp://u:***@y");
        assert_eq!(redact("https://u:pa?ss@h:0/"), "https://u:***@h:0/");
    }

    #[test]
    fn authority_split() {
        let (user, password, hostport) = split_authority("a@b:p:w@h:21");
        assert_eq!(user.as_deref(), Some("a@b"));
        assert_eq!(password.as_ref().map(Secret::expose), Some("p:w"));
        assert_eq!(hostport, "h:21");
        assert_eq!(split_authority("h").2, "h");
        assert_eq!(split_authority("@h").0.as_deref(), Some(""));
    }

    #[test]
    fn http_user_information_is_kept() {
        let url = parse_http("https://user:hunter2@Example.com:8443/a?b", 5, true).unwrap();
        assert_eq!(
            url,
            HttpUrl {
                secure: true,
                user: Some("user".into()),
                password: Some(Secret::new("hunter2")),
                host: "example.com".into(),
                port: Some(8443),
                rest: "/a?b".into()
            }
        );
        assert_eq!(url.to_string(), "https://user@example.com:8443/a?b");
        assert!(!format!("{url:?}").contains("hunter2"), "{url:?}");
        let url = parse_http("http://u@h?q", 4, false).unwrap();
        assert_eq!(
            (url.user.as_deref(), url.host.as_str(), url.rest.as_str()),
            (Some("u"), "h", "?q")
        );
        assert_eq!(url.password, None);
        let url = parse_http("http://a@b:p:w@h#f", 4, false).unwrap();
        assert_eq!(url.user.as_deref(), Some("a@b"));
        assert_eq!(url.password.as_ref().map(Secret::expose), Some("p:w"));
        assert_eq!(url.to_string(), "http://a@b@h#f");
        let err = parse_http("http://user:hunter2@h:0/", 4, false).unwrap_err();
        assert!(!err.to_string().contains("hunter2"), "{err}");
        assert!(
            parse_http("http://user@/x", 4, false).is_err(),
            "empty host"
        );
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn ports() {
        assert_eq!(parse_port("x", "1").unwrap(), 1);
        assert_eq!(parse_port("x", "65535").unwrap(), 65535);
        assert_eq!(parse_port("x", "0021").unwrap(), 21);
        for bad in [
            "",
            "0",
            "65536",
            "99999",
            "+1",
            "1a",
            "99999999999999999999",
        ] {
            assert!(parse_port("x", bad).is_err(), "{bad:?}");
        }
    }
}

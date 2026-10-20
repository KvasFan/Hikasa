//! [`LocalPath`] and the pure, style-explicit local-path normalisation of §2.1 (D-06).
//!
//! The algorithm works on *code units* ([`Unit`]): UTF-8 or raw bytes on Unix and in the `&str` API, UTF-16
//! units on Windows hosts. It only inspects and produces ASCII units (`/`, `\`, `.`, `~`, `:` and drive
//! letters), so valid UTF-8 stays valid UTF-8 and non-UTF-8 OS strings survive unchanged.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use super::{PathStyle, SpecError};

// ---------------------------------------------------------------------------------------------------------
// Code units
// ---------------------------------------------------------------------------------------------------------

/// A code unit of a path string.
pub(crate) trait Unit: Copy + Eq {
    /// The ASCII byte this unit encodes, if any.
    fn ascii(self) -> Option<u8>;
    /// The unit encoding ASCII byte `b`.
    fn from_ascii(b: u8) -> Self;
}

impl Unit for u8 {
    #[inline]
    fn ascii(self) -> Option<u8> {
        self.is_ascii().then_some(self)
    }
    #[inline]
    fn from_ascii(b: u8) -> u8 {
        b
    }
}

impl Unit for u16 {
    #[inline]
    fn ascii(self) -> Option<u8> {
        u8::try_from(self).ok().filter(u8::is_ascii)
    }
    #[inline]
    fn from_ascii(b: u8) -> u16 {
        u16::from(b)
    }
}

#[inline]
pub(crate) fn is<U: Unit>(u: U, b: u8) -> bool {
    u.ascii() == Some(b)
}

pub(crate) fn eq_ascii<U: Unit>(units: &[U], text: &[u8]) -> bool {
    units.len() == text.len() && units.iter().zip(text).all(|(&u, &b)| is(u, b))
}

pub(crate) fn starts_with_ascii<U: Unit>(units: &[U], text: &[u8]) -> bool {
    units
        .get(..text.len())
        .is_some_and(|head| eq_ascii(head, text))
}

/// ASCII-case-insensitive prefix test (`text` must be lower case).
pub(crate) fn starts_with_ascii_ci<U: Unit>(units: &[U], text: &[u8]) -> bool {
    units.get(..text.len()).is_some_and(|head| {
        head.iter()
            .zip(text)
            .all(|(&u, &b)| u.ascii().is_some_and(|a| a.to_ascii_lowercase() == b))
    })
}

fn push_ascii<U: Unit>(out: &mut Vec<U>, text: &[u8]) {
    out.extend(text.iter().map(|&b| U::from_ascii(b)));
}

fn first_is<U: Unit>(units: &[U], b: u8) -> bool {
    units.first().is_some_and(|&u| is(u, b))
}

/// Conversions between OS strings and host code units.
pub(crate) mod os {
    use std::ffi::{OsStr, OsString};

    #[cfg(unix)]
    pub(crate) type HostUnit = u8;
    #[cfg(windows)]
    pub(crate) type HostUnit = u16;
    #[cfg(not(any(unix, windows)))]
    pub(crate) type HostUnit = u8;

    /// The code units of `s`.
    #[cfg(unix)]
    pub(crate) fn units(s: &OsStr) -> Vec<HostUnit> {
        use std::os::unix::ffi::OsStrExt;
        s.as_bytes().to_vec()
    }
    /// The code units of `s`.
    #[cfg(windows)]
    pub(crate) fn units(s: &OsStr) -> Vec<HostUnit> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().collect()
    }
    /// The code units of `s`.
    #[cfg(not(any(unix, windows)))]
    pub(crate) fn units(s: &OsStr) -> Vec<HostUnit> {
        s.to_string_lossy().into_owned().into_bytes()
    }

    /// The OS string made of `units`.
    #[cfg(unix)]
    pub(crate) fn from_units(units: Vec<HostUnit>) -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(units)
    }
    /// The OS string made of `units`.
    #[cfg(windows)]
    pub(crate) fn from_units(units: Vec<HostUnit>) -> OsString {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&units)
    }
    /// The OS string made of `units`.
    #[cfg(not(any(unix, windows)))]
    pub(crate) fn from_units(units: Vec<HostUnit>) -> OsString {
        OsString::from(String::from_utf8_lossy(&units).into_owned())
    }
}

// ---------------------------------------------------------------------------------------------------------
// Normalisation
// ---------------------------------------------------------------------------------------------------------

/// Failure of [`normalize_units`]; mapped to [`SpecError`] by the callers, which know the texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NormError {
    Empty,
    Nul,
    RelativeWithoutCwd,
    HomeUnknown,
    TildeUser,
    DriveRelative,
    DevicePath,
    UncWithoutShare,
    /// The current directory is not an absolute path of the style.
    BadCwd,
    /// The home directory is not an absolute path of the style.
    BadHome,
}

impl NormError {
    /// The [`SpecError`] for this failure, given the lossy texts of the input, cwd and home.
    pub(crate) fn into_spec_error(
        self,
        input: String,
        cwd: Option<String>,
        home: Option<String>,
    ) -> SpecError {
        match self {
            NormError::Empty => SpecError::Empty,
            NormError::Nul => SpecError::Nul,
            NormError::RelativeWithoutCwd => SpecError::RelativeWithoutCwd { path: input },
            NormError::HomeUnknown => SpecError::HomeUnknown,
            NormError::TildeUser => SpecError::TildeUser { input },
            NormError::DriveRelative => SpecError::DriveRelative { path: input },
            NormError::DevicePath => SpecError::DevicePath { path: input },
            NormError::UncWithoutShare => SpecError::UncWithoutShare { path: input },
            NormError::BadCwd => SpecError::NotNormalized {
                path: cwd.unwrap_or_default(),
            },
            NormError::BadHome => SpecError::NotNormalized {
                path: home.unwrap_or_default(),
            },
        }
    }
}

/// The §2.1 normalisation on code units. `cwd` and `home` must be absolute paths of `style` (they are
/// normalised again, so they need not be normalised already).
pub(crate) fn normalize_units<U: Unit>(
    input: &[U],
    style: PathStyle,
    cwd: Option<&[U]>,
    home: Option<&[U]>,
) -> Result<Vec<U>, NormError> {
    let has_nul = |s: &[U]| s.iter().any(|&u| is(u, 0));
    if has_nul(input) || cwd.is_some_and(has_nul) || home.is_some_and(has_nul) {
        return Err(NormError::Nul);
    }
    if input
        .iter()
        .all(|&u| matches!(u.ascii(), Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)))
    {
        return Err(NormError::Empty);
    }
    match style {
        PathStyle::Unix => normalize_unix(input, cwd, home),
        PathStyle::Windows => normalize_windows(input, cwd, home),
    }
}

fn normalize_unix<U: Unit>(
    input: &[U],
    cwd: Option<&[U]>,
    home: Option<&[U]>,
) -> Result<Vec<U>, NormError> {
    // 1. `~` and `~/…`; `~name…` is not supported.
    let path: Vec<U> = if first_is(input, b'~') {
        let rest = input.get(1..).unwrap_or(&[]);
        if !(rest.is_empty() || first_is(rest, b'/')) {
            return Err(NormError::TildeUser);
        }
        let home = home.ok_or(NormError::HomeUnknown)?;
        if !first_is(home, b'/') {
            return Err(NormError::BadHome);
        }
        let mut joined = home.to_vec();
        joined.extend_from_slice(rest);
        joined
    } else if first_is(input, b'/') {
        input.to_vec()
    } else {
        // 2. Relative: prefixed with cwd + '/'.
        let cwd = cwd.ok_or(NormError::RelativeWithoutCwd)?;
        if !first_is(cwd, b'/') {
            return Err(NormError::BadCwd);
        }
        let mut joined = cwd.to_vec();
        push_ascii(&mut joined, b"/");
        joined.extend_from_slice(input);
        joined
    };
    // 3. Drop empty and `.` components; keep `..` except directly under the root (D-06).
    let mut out: Vec<U> = Vec::with_capacity(path.len());
    push_ascii(&mut out, b"/");
    let mut count = 0usize;
    for component in path.split(|&u| is(u, b'/')) {
        if component.is_empty() || eq_ascii(component, b".") {
            continue;
        }
        if count == 0 && eq_ascii(component, b"..") {
            continue;
        }
        if count > 0 {
            push_ascii(&mut out, b"/");
        }
        out.extend_from_slice(component);
        count += 1;
    }
    Ok(out)
}

/// Shape of a Windows path whose `/` were already replaced by `\`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WinKind {
    /// `\\?\…`
    Verbatim,
    /// `\\.\…`
    Device,
    /// `\\server` without a share.
    UncMissingShare,
    /// `\\server\share…`; `root_end` is the length of `\\server\share`.
    Unc { root_end: usize },
    /// `X:\…`; the letter is upper case.
    Drive { letter: u8 },
    /// `X:rest`
    DriveRelative { letter: u8 },
    /// `\rest`
    Rooted,
    /// Anything else.
    Relative,
}

fn classify_windows<U: Unit>(s: &[U]) -> WinKind {
    if starts_with_ascii(s, br"\\?\") {
        return WinKind::Verbatim;
    }
    if starts_with_ascii(s, br"\\.\") || eq_ascii(s, br"\\.") {
        return WinKind::Device;
    }
    if starts_with_ascii(s, br"\\") {
        let rest = s.get(2..).unwrap_or(&[]);
        let server_len = rest
            .iter()
            .position(|&u| is(u, b'\\'))
            .unwrap_or(rest.len());
        if server_len == 0 || server_len == rest.len() {
            return WinKind::UncMissingShare;
        }
        let after = rest.get(server_len + 1..).unwrap_or(&[]);
        let share_len = after
            .iter()
            .position(|&u| is(u, b'\\'))
            .unwrap_or(after.len());
        if share_len == 0 {
            return WinKind::UncMissingShare;
        }
        return WinKind::Unc {
            root_end: 2 + server_len + 1 + share_len,
        };
    }
    if let (Some(first), Some(&colon)) = (s.first().and_then(|u| u.ascii()), s.get(1))
        && first.is_ascii_alphabetic()
        && is(colon, b':')
    {
        let letter = first.to_ascii_uppercase();
        return if s.get(2).is_some_and(|&u| is(u, b'\\')) {
            WinKind::Drive { letter }
        } else {
            WinKind::DriveRelative { letter }
        };
    }
    if first_is(s, b'\\') {
        WinKind::Rooted
    } else {
        WinKind::Relative
    }
}

fn backslashed<U: Unit>(s: &[U]) -> Vec<U> {
    s.iter()
        .map(|&u| if is(u, b'/') { U::from_ascii(b'\\') } else { u })
        .collect()
}

/// Components of `tail` with empty and `.` components dropped and `..` resolved lexically (ignored at the root).
fn resolve_dots<U: Unit>(tail: &[U]) -> Vec<&[U]> {
    let mut stack: Vec<&[U]> = Vec::new();
    for component in tail.split(|&u| is(u, b'\\')) {
        if component.is_empty() || eq_ascii(component, b".") {
            continue;
        }
        if eq_ascii(component, b"..") {
            stack.pop();
            continue;
        }
        stack.push(component);
    }
    stack
}

fn normalize_windows<U: Unit>(
    input: &[U],
    cwd: Option<&[U]>,
    home: Option<&[U]>,
) -> Result<Vec<U>, NormError> {
    // 1. `/` → `\`; `~` expansion as on Unix: the rest is resolved onto the home directory exactly like a
    //    relative path onto the current directory (also when the home directory is a verbatim `\\?\` path).
    let s = backslashed(input);
    if first_is(&s, b'~') {
        let rest = s.get(1..).unwrap_or(&[]);
        if !(rest.is_empty() || first_is(rest, b'\\')) {
            return Err(NormError::TildeUser);
        }
        let home = home.ok_or(NormError::HomeUnknown)?;
        let home = windows_absolute(&backslashed(home)).map_err(|_| NormError::BadHome)?;
        return Ok(join_resolved(&home, rest));
    }
    resolve_windows(&s, cwd)
}

/// Step 3 for an input joined onto a directory: the components of `tail` are appended to the normalised
/// absolute path `base`, dropping empty and `.` components and resolving `..` lexically (pop; at the root of
/// `base`, ignore). The text of `base` itself is kept, so a verbatim (`\\?\…`) directory keeps its prefix while
/// the user's relative part is still normalised: `..\x` against `\\?\C:\w` is `\\?\C:\x`.
fn join_resolved<U: Unit>(base: &[U], tail: &[U]) -> Vec<U> {
    let root_len = windows_root_len(base).min(base.len());
    let (root, rest) = base.split_at(root_len);
    let rest = rest
        .strip_prefix(&[U::from_ascii(b'\\')][..])
        .unwrap_or(rest);
    let mut stack: Vec<&[U]> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split(|&u| is(u, b'\\')).collect()
    };
    // A trailing separator of a verbatim directory is not a component.
    if stack.last().is_some_and(|component| component.is_empty()) {
        stack.pop();
    }
    for component in tail.split(|&u| is(u, b'\\')) {
        if component.is_empty() || eq_ascii(component, b".") {
            continue;
        }
        if eq_ascii(component, b"..") {
            stack.pop();
            continue;
        }
        stack.push(component);
    }
    let root_has_separator = root.last().is_some_and(|&u| is(u, b'\\'));
    let mut out = Vec::with_capacity(base.len() + tail.len() + 1);
    out.extend_from_slice(root);
    for (k, component) in stack.iter().enumerate() {
        if k > 0 || !root_has_separator {
            push_ascii(&mut out, b"\\");
        }
        out.extend_from_slice(component);
    }
    out
}

/// Normalises an absolute Windows path (`/` already replaced); relative forms are errors.
fn windows_absolute<U: Unit>(s: &[U]) -> Result<Vec<U>, NormError> {
    resolve_windows(s, None)
}

/// The validated, normalised current directory.
fn windows_cwd<U: Unit>(cwd: Option<&[U]>, missing: NormError) -> Result<Vec<U>, NormError> {
    let cwd = cwd.ok_or(missing)?;
    windows_absolute(&backslashed(cwd)).map_err(|_| NormError::BadCwd)
}

/// 2.–3. of the Windows algorithm.
fn resolve_windows<U: Unit>(s: &[U], cwd: Option<&[U]>) -> Result<Vec<U>, NormError> {
    match classify_windows(s) {
        WinKind::Verbatim => Ok(s.to_vec()),
        WinKind::Device => Err(NormError::DevicePath),
        WinKind::UncMissingShare => Err(NormError::UncWithoutShare),
        WinKind::Unc { root_end } => {
            let mut out = s.get(..root_end).unwrap_or(s).to_vec();
            for component in resolve_dots(s.get(root_end..).unwrap_or(&[])) {
                push_ascii(&mut out, b"\\");
                out.extend_from_slice(component);
            }
            Ok(out)
        }
        WinKind::Drive { letter } => {
            let mut out = Vec::with_capacity(s.len());
            push_ascii(&mut out, &[letter, b':', b'\\']);
            for (k, component) in resolve_dots(s.get(3..).unwrap_or(&[]))
                .into_iter()
                .enumerate()
            {
                if k > 0 {
                    push_ascii(&mut out, b"\\");
                }
                out.extend_from_slice(component);
            }
            Ok(out)
        }
        // The joined forms are resolved component by component (never re-classified), so a verbatim current
        // directory does not turn the user's relative text into an unchanged verbatim path.
        WinKind::DriveRelative { letter } => {
            let cwd = windows_cwd(cwd, NormError::DriveRelative)?;
            if drive_letter(&cwd) != Some(letter) {
                return Err(NormError::DriveRelative);
            }
            Ok(join_resolved(&cwd, s.get(2..).unwrap_or(&[])))
        }
        WinKind::Rooted => {
            let cwd = windows_cwd(cwd, NormError::RelativeWithoutCwd)?;
            let root = cwd.get(..windows_root_len(&cwd)).unwrap_or(&cwd);
            Ok(join_resolved(root, s))
        }
        WinKind::Relative => {
            let cwd = windows_cwd(cwd, NormError::RelativeWithoutCwd)?;
            Ok(join_resolved(&cwd, s))
        }
    }
}

/// Upper-case drive letter of a normalised absolute Windows path (`X:\…` or `\\?\X:…`).
fn drive_letter<U: Unit>(s: &[U]) -> Option<u8> {
    let rest = if starts_with_ascii(s, br"\\?\") {
        s.get(4..)?
    } else {
        s
    };
    let letter = rest.first()?.ascii()?;
    (letter.is_ascii_alphabetic() && rest.get(1).is_some_and(|&u| is(u, b':')))
        .then(|| letter.to_ascii_uppercase())
}

/// Length of the root of a normalised absolute Windows path: the part that `..` cannot remove and that a rooted
/// `\rest` is appended to. A drive root includes its `\` (`C:\` = 3, `\\?\C:\` = 7); a UNC root is
/// `\\server\share` (or `\\?\UNC\server\share`); another verbatim root ends before its first `\`.
fn windows_root_len<U: Unit>(s: &[U]) -> usize {
    match classify_windows(s) {
        WinKind::Drive { .. } => 3,
        WinKind::Unc { root_end } => root_end,
        WinKind::Verbatim => {
            let rest = s.get(4..).unwrap_or(&[]);
            if rest.get(1).is_some_and(|&u| is(u, b':'))
                && rest
                    .first()
                    .and_then(|u| u.ascii())
                    .is_some_and(|b| b.is_ascii_alphabetic())
            {
                let has_sep = rest.get(2).is_some_and(|&u| is(u, b'\\'));
                4 + 2 + usize::from(has_sep)
            } else if starts_with_ascii_ci(rest, br"unc\") {
                let unc = rest.get(4..).unwrap_or(&[]);
                let server_len = unc.iter().position(|&u| is(u, b'\\')).unwrap_or(unc.len());
                let after = unc.get(server_len + 1..).unwrap_or(&[]);
                let share_len = after
                    .iter()
                    .position(|&u| is(u, b'\\'))
                    .unwrap_or(after.len());
                (8 + server_len + usize::from(server_len < unc.len()) + share_len).min(s.len())
            } else {
                4 + rest
                    .iter()
                    .position(|&u| is(u, b'\\'))
                    .unwrap_or(rest.len())
            }
        }
        WinKind::Device
        | WinKind::UncMissingShare
        | WinKind::DriveRelative { .. }
        | WinKind::Rooted
        | WinKind::Relative => 0,
    }
}

/// Length of the root (the part that has no parent) of a normalised absolute path of `style`.
pub(crate) fn root_len<U: Unit>(s: &[U], style: PathStyle) -> usize {
    match style {
        PathStyle::Unix => usize::from(first_is(s, b'/')),
        PathStyle::Windows => windows_root_len(s),
    }
}

/// The parent of a normalised absolute path of `style` (`None` for a root). On Unix a final `..` component is
/// kept, so the parent of `/a/..` is `/a/../..` (lexical removal is unsound across symlinks, D-06).
pub(crate) fn parent_units<U: Unit>(s: &[U], style: PathStyle) -> Option<Vec<U>> {
    let root = root_len(s, style);
    if s.len() <= root {
        return None;
    }
    let sep = match style {
        PathStyle::Unix => b'/',
        PathStyle::Windows => b'\\',
    };
    let last_sep = s.iter().rposition(|&u| is(u, sep));
    if style == PathStyle::Unix {
        let last = last_sep.map_or(s, |at| s.get(at + 1..).unwrap_or(&[]));
        if eq_ascii(last, b"..") {
            let mut out = s.to_vec();
            push_ascii(&mut out, b"/..");
            return Some(out);
        }
    }
    let end = match last_sep {
        Some(at) if at > root => at,
        _ => root,
    };
    Some(s.get(..end).unwrap_or(s).to_vec())
}

/// Public, `&str`-based form of the normalisation. See [`normalize_local`].
fn normalize_str(
    input: &str,
    style: PathStyle,
    cwd: Option<&str>,
    home: Option<&str>,
) -> Result<String, SpecError> {
    if input.contains('\0')
        || cwd.is_some_and(|c| c.contains('\0'))
        || home.is_some_and(|h| h.contains('\0'))
    {
        return Err(SpecError::Nul);
    }
    if input.trim().is_empty() {
        return Err(SpecError::Empty);
    }
    normalize_units(
        input.as_bytes(),
        style,
        cwd.map(str::as_bytes),
        home.map(str::as_bytes),
    )
    .map(|units| {
        String::from_utf8(units)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    })
    .map_err(|e| {
        e.into_spec_error(
            input.to_owned(),
            cwd.map(str::to_owned),
            home.map(str::to_owned),
        )
    })
}

/// Pure, style-explicit local-path normalisation (§2.1). `cwd`/`home` must already be normalised (they must
/// at least be absolute paths of `style`; otherwise the result is [`SpecError::NotNormalized`]).
///
/// **Unix:** `~` and `~/…` expand to `home` (`~name` is [`SpecError::TildeUser`]); a relative path is prefixed
/// with `cwd`; empty and `.` components are dropped; `..` is kept (lexical removal is unsound across symlinks,
/// D-06) except directly under the root (`/../x` → `/x`); no trailing `/` except for the root.
///
/// **Windows:** `/` becomes `\`; `\\?\…` is returned unchanged; `\\.\…` is [`SpecError::DevicePath`]; UNC
/// roots are `\\server\share`; drive letters are upper-cased; `X:rest` resolves against a `cwd` on drive `X`;
/// `\rest` against the root of `cwd`; `..` is resolved lexically; no trailing `\` except at `X:\`.
///
/// ```
/// use hikasa_core::spec::{normalize_local, PathStyle};
///
/// assert_eq!(normalize_local("/home/u//x/./y/", PathStyle::Unix, None, None).unwrap(), "/home/u/x/y");
/// assert_eq!(normalize_local("/a/../b", PathStyle::Unix, None, None).unwrap(), "/a/../b");
/// assert_eq!(normalize_local("c:/x/../y", PathStyle::Windows, None, None).unwrap(), r"C:\y");
/// ```
pub fn normalize_local(
    input: &str,
    style: PathStyle,
    cwd: Option<&str>,
    home: Option<&str>,
) -> Result<String, SpecError> {
    normalize_str(input, style, cwd, home)
}

// ---------------------------------------------------------------------------------------------------------
// LocalPath
// ---------------------------------------------------------------------------------------------------------

/// An absolute, normalised local path (invariants INV-SPEC-1..3, §2.1).
///
/// The methods follow [`PathStyle::HOST`]. A `LocalPath` produced by [`Spec::parse`](super::Spec::parse) with
/// a foreign style (Windows rules unit-tested on Linux) is only meaningful through its canonical text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalPath(PathBuf);

impl LocalPath {
    /// Validates `path` (absolute and already normalised for [`PathStyle::HOST`]).
    pub fn new(path: PathBuf) -> Result<LocalPath, SpecError> {
        let units = os::units(path.as_os_str());
        match normalize_units(&units, PathStyle::HOST, None, None) {
            Ok(normalized) if normalized == units => Ok(LocalPath(path)),
            Ok(_) => Err(SpecError::NotNormalized {
                path: lossy_os(path.as_os_str()),
            }),
            Err(NormError::Nul) => Err(SpecError::Nul),
            Err(NormError::DevicePath) => Err(SpecError::DevicePath {
                path: lossy_os(path.as_os_str()),
            }),
            Err(NormError::UncWithoutShare) => Err(SpecError::UncWithoutShare {
                path: lossy_os(path.as_os_str()),
            }),
            Err(
                NormError::Empty
                | NormError::RelativeWithoutCwd
                | NormError::HomeUnknown
                | NormError::TildeUser
                | NormError::DriveRelative
                | NormError::BadCwd
                | NormError::BadHome,
            ) => Err(SpecError::NotNormalized {
                path: lossy_os(path.as_os_str()),
            }),
        }
    }

    /// The path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Consumes `self`.
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    /// Joins one name component (no separator, not `.`/`..`, not empty, no NUL).
    pub fn join_name(&self, name: &OsStr) -> Result<LocalPath, SpecError> {
        let name_units = os::units(name);
        let separator = |u: os::HostUnit| {
            is(u, b'/') || (PathStyle::HOST == PathStyle::Windows && is(u, b'\\'))
        };
        if name_units.is_empty()
            || eq_ascii(&name_units, b".")
            || eq_ascii(&name_units, b"..")
            || name_units.iter().any(|&u| is(u, 0) || separator(u))
        {
            return Err(SpecError::InvalidComponent {
                name: name.to_string_lossy().into_owned(),
            });
        }
        let mut units = os::units(self.0.as_os_str());
        if !units.last().is_some_and(|&u| separator(u)) {
            let sep: &[u8] = if PathStyle::HOST == PathStyle::Windows {
                b"\\"
            } else {
                b"/"
            };
            push_ascii(&mut units, sep);
        }
        units.extend_from_slice(&name_units);
        Ok(LocalPath(PathBuf::from(os::from_units(units))))
    }

    /// The last component (`None` for a root).
    pub fn file_name(&self) -> Option<&OsStr> {
        match self.0.components().next_back()? {
            Component::Normal(name) => Some(name),
            Component::ParentDir => Some(OsStr::new("..")),
            Component::CurDir => Some(OsStr::new(".")),
            Component::RootDir | Component::Prefix(_) => None,
        }
    }

    /// The parent directory (`None` for a root). A final `..` component is kept on Unix, so the parent of
    /// `/a/..` is `/a/../..` (D-06).
    pub fn parent(&self) -> Option<LocalPath> {
        let units = os::units(self.0.as_os_str());
        parent_units(&units, PathStyle::HOST).map(|p| LocalPath(PathBuf::from(os::from_units(p))))
    }
}

/// Lossy text of an OS string (for messages).
pub(crate) fn lossy_os(s: &OsStr) -> String {
    s.to_string_lossy().into_owned()
}

/// Normalises host units of `input` against the context directories; the error carries lossy texts.
pub(crate) fn normalize_host(
    input: &OsStr,
    style: PathStyle,
    cwd: Option<&Path>,
    home: Option<&Path>,
) -> Result<LocalPath, SpecError> {
    let units = os::units(input);
    let cwd_units = cwd.map(|p| os::units(p.as_os_str()));
    let home_units = home.map(|p| os::units(p.as_os_str()));
    match normalize_units(&units, style, cwd_units.as_deref(), home_units.as_deref()) {
        Ok(normalized) => Ok(LocalPath(PathBuf::from(os::from_units(normalized)))),
        Err(e) => Err(e.into_spec_error(
            lossy_os(input),
            cwd.map(|p| lossy_os(p.as_os_str())),
            home.map(|p| lossy_os(p.as_os_str())),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(input: &str) -> Result<String, SpecError> {
        normalize_local(input, PathStyle::Unix, Some("/w"), Some("/home/u"))
    }

    fn win(input: &str, cwd: Option<&str>) -> Result<String, SpecError> {
        normalize_local(input, PathStyle::Windows, cwd, Some(r"C:\Users\u"))
    }

    #[test]
    fn unix_rules() {
        assert_eq!(unix("/").unwrap(), "/");
        assert_eq!(unix("//").unwrap(), "/");
        assert_eq!(unix("/..").unwrap(), "/");
        assert_eq!(unix("/../../x").unwrap(), "/x");
        assert_eq!(unix("/a/../../x").unwrap(), "/a/../../x");
        assert_eq!(unix(".").unwrap(), "/w");
        assert_eq!(unix("..").unwrap(), "/w/..");
        assert_eq!(unix("~").unwrap(), "/home/u");
        assert_eq!(unix("~/").unwrap(), "/home/u");
        assert_eq!(unix("a/~").unwrap(), "/w/a/~");
        assert_eq!(unix("~x"), Err(SpecError::TildeUser { input: "~x".into() }));
        assert_eq!(
            normalize_local("x", PathStyle::Unix, Some("rel"), None),
            Err(SpecError::NotNormalized { path: "rel".into() })
        );
        assert_eq!(
            normalize_local("~", PathStyle::Unix, None, Some("rel")),
            Err(SpecError::NotNormalized { path: "rel".into() })
        );
        assert_eq!(
            normalize_local("~", PathStyle::Unix, None, None),
            Err(SpecError::HomeUnknown)
        );
        assert_eq!(unix("a\0b"), Err(SpecError::Nul));
        assert_eq!(unix(" \t "), Err(SpecError::Empty));
        assert_eq!(unix(" x ").unwrap(), "/w/ x ");
    }

    #[test]
    fn windows_rules() {
        assert_eq!(win(r"C:\", None).unwrap(), r"C:\");
        assert_eq!(win(r"c:\..\..", None).unwrap(), r"C:\");
        assert_eq!(win(r"C:", Some(r"c:\w")).unwrap(), r"C:\w");
        assert_eq!(win(r"x\..\..\..\y", Some(r"D:\a\b")).unwrap(), r"D:\y");
        assert_eq!(win(r"\\srv\share", None).unwrap(), r"\\srv\share");
        assert_eq!(win(r"\\srv\share\", None).unwrap(), r"\\srv\share");
        assert_eq!(win(r"\\srv\share\a\..\..", None).unwrap(), r"\\srv\share");
        assert_eq!(
            win(r"\x", Some(r"\\srv\share\w")).unwrap(),
            r"\\srv\share\x"
        );
        assert_eq!(
            win(r"x", Some(r"\\srv\share\w")).unwrap(),
            r"\\srv\share\w\x"
        );
        assert_eq!(
            win(r"\\srv\", None),
            Err(SpecError::UncWithoutShare {
                path: r"\\srv\".into()
            })
        );
        assert_eq!(
            win(r"\\", None),
            Err(SpecError::UncWithoutShare { path: r"\\".into() })
        );
        assert_eq!(
            win(r"\\.", None),
            Err(SpecError::DevicePath {
                path: r"\\.".into()
            })
        );
        assert_eq!(win("~", None).unwrap(), r"C:\Users\u");
        assert_eq!(win("~/docs", None).unwrap(), r"C:\Users\u\docs");
        assert_eq!(
            win("~bob", None),
            Err(SpecError::TildeUser {
                input: "~bob".into()
            })
        );
        assert_eq!(
            win(r"C:x", None),
            Err(SpecError::DriveRelative { path: "C:x".into() })
        );
        assert_eq!(
            win(r"x", None),
            Err(SpecError::RelativeWithoutCwd { path: "x".into() })
        );
        assert_eq!(
            win(r"x", Some("relative")),
            Err(SpecError::NotNormalized {
                path: "relative".into()
            })
        );
        assert_eq!(win(r"\x", Some(r"\\?\C:\w")).unwrap(), r"\\?\C:\x");
        assert_eq!(win(r"C:x", Some(r"\\?\C:\w")).unwrap(), r"\\?\C:\w\x");
        assert_eq!(win(r"//?/C:/x/../y", None).unwrap(), r"\\?\C:\x\..\y");
        assert_eq!(win(r"1:\x", Some(r"C:\w")).unwrap(), r"C:\w\1:\x");
    }

    #[test]
    fn verbatim_directories_keep_their_prefix_but_not_the_users_dots() {
        let verbatim = Some(r"\\?\C:\w");
        for (input, expected) in [
            (r"..\other.txt", r"\\?\C:\other.txt"),
            (r"a\.\b\", r"\\?\C:\w\a\b"),
            (r"a\.\b\..\c", r"\\?\C:\w\a\c"),
            (r"a\\b", r"\\?\C:\w\a\b"),
            (r"\a\..\b", r"\\?\C:\b"),
            (r"\x", r"\\?\C:\x"),
            (r"C:..\z", r"\\?\C:\z"),
            (r"c:x", r"\\?\C:\w\x"),
            (r"..\..\..", r"\\?\C:\"),
            (".", r"\\?\C:\w"),
        ] {
            assert_eq!(win(input, verbatim).unwrap(), expected, "{input:?}");
            let plain = win(input, Some(r"C:\w")).unwrap();
            assert_eq!(format!(r"\\?\{plain}"), expected, "{input:?} as with C:\\w");
        }
        let unc = Some(r"\\?\UNC\srv\share\w");
        assert_eq!(win(r"..\x", unc).unwrap(), r"\\?\UNC\srv\share\x");
        assert_eq!(win(r"\x", unc).unwrap(), r"\\?\UNC\srv\share\x");
        assert_eq!(win(r"..\..\..", unc).unwrap(), r"\\?\UNC\srv\share");
        assert_eq!(win("x", Some(r"\\?\C:\w\")).unwrap(), r"\\?\C:\w\x");
        assert_eq!(win("x", Some(r"\\?\C:")).unwrap(), r"\\?\C:\x");
        assert_eq!(
            win(r"x\..\y", Some(r"\\?\Volume{1}\w")).unwrap(),
            r"\\?\Volume{1}\w\y"
        );
        assert_eq!(
            win(r"..\..\y", Some(r"\\?\Volume{1}\w")).unwrap(),
            r"\\?\Volume{1}\y"
        );
        // The verbatim directory's own text is kept as it is (Windows does not interpret it either).
        assert_eq!(
            win(r"..\z", Some(r"\\?\C:\x\..\y")).unwrap(),
            r"\\?\C:\x\..\z"
        );
        // `~` resolves onto a verbatim home directory like a relative path onto the current directory.
        assert_eq!(
            normalize_local(
                r"~\a\.\..\b\",
                PathStyle::Windows,
                None,
                Some(r"\\?\C:\Users\u")
            )
            .unwrap(),
            r"\\?\C:\Users\u\b"
        );
        // A verbatim input is still returned unchanged (§2.1 row 16).
        assert_eq!(win(r"\\?\C:\x\..\y", verbatim).unwrap(), r"\\?\C:\x\..\y");
    }

    #[test]
    fn roots_and_parents() {
        let parent = |s: &str, style| {
            parent_units(s.as_bytes(), style).map(|v| String::from_utf8(v).unwrap())
        };
        assert_eq!(parent("/", PathStyle::Unix), None);
        assert_eq!(parent("/a", PathStyle::Unix).as_deref(), Some("/"));
        assert_eq!(parent("/a/b", PathStyle::Unix).as_deref(), Some("/a"));
        assert_eq!(
            parent("/a/..", PathStyle::Unix).as_deref(),
            Some("/a/../..")
        );
        assert_eq!(parent(r"C:\", PathStyle::Windows), None);
        assert_eq!(parent(r"C:\a", PathStyle::Windows).as_deref(), Some(r"C:\"));
        assert_eq!(
            parent(r"C:\a\b", PathStyle::Windows).as_deref(),
            Some(r"C:\a")
        );
        assert_eq!(parent(r"\\srv\share", PathStyle::Windows), None);
        assert_eq!(
            parent(r"\\srv\share\x", PathStyle::Windows).as_deref(),
            Some(r"\\srv\share")
        );
        assert_eq!(parent(r"\\?\C:\", PathStyle::Windows), None);
        assert_eq!(
            parent(r"\\?\C:\x", PathStyle::Windows).as_deref(),
            Some(r"\\?\C:\")
        );
        assert_eq!(parent(r"\\?\UNC\srv\share", PathStyle::Windows), None);
        assert_eq!(
            parent(r"\\?\UNC\srv\share\x", PathStyle::Windows).as_deref(),
            Some(r"\\?\UNC\srv\share")
        );
        assert_eq!(
            parent(r"\\?\Volume{1}\x", PathStyle::Windows).as_deref(),
            Some(r"\\?\Volume{1}")
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_path_on_unix() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStrExt;

        let p = LocalPath::new(PathBuf::from("/a/b")).unwrap();
        assert_eq!(p.file_name(), Some(OsStr::new("b")));
        assert_eq!(p.parent().unwrap().as_path(), Path::new("/a"));
        assert_eq!(
            p.join_name(OsStr::new("c")).unwrap().as_path(),
            Path::new("/a/b/c")
        );
        let root = LocalPath::new(PathBuf::from("/")).unwrap();
        assert_eq!(root.file_name(), None);
        assert_eq!(root.parent(), None);
        assert_eq!(
            root.join_name(OsStr::new("x")).unwrap().as_path(),
            Path::new("/x")
        );
        assert_eq!(
            LocalPath::new(PathBuf::from("/a/..")).unwrap().file_name(),
            Some(OsStr::new(".."))
        );
        for bad in ["", ".", "..", "a/b", "nul\0"] {
            assert!(
                matches!(
                    p.join_name(OsStr::new(bad)),
                    Err(SpecError::InvalidComponent { .. })
                ),
                "{bad:?}"
            );
        }
        assert!(
            p.join_name(OsStr::new("a\\b")).is_ok(),
            "backslash is a normal character on Unix"
        );
        for bad in ["relative", "/a/", "/a//b", "/./a", "/../a", "~"] {
            assert!(
                matches!(
                    LocalPath::new(PathBuf::from(bad)),
                    Err(SpecError::NotNormalized { .. })
                ),
                "{bad:?}"
            );
        }
        let raw = OsStr::from_bytes(b"/tmp/\xff\xfe");
        let lossy = LocalPath::new(PathBuf::from(raw)).unwrap();
        assert_eq!(lossy.file_name(), Some(OsStr::from_bytes(b"\xff\xfe")));
        assert_eq!(os::from_units(b"/x".to_vec()), OsString::from("/x"));
    }
}

//! Member names (D-141; §1.5.7 rules 1 and 17): checked on the raw bytes, before anything else, so a hostile name
//! never reaches a tree, a cache path or a disk.

use std::borrow::Cow;

/// The longest member name accepted, in bytes.
pub(crate) const NAME_MAX: usize = 4096;

/// Why a name is refused (the stable reason text of the listing's `<refused-N>` rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A NUL byte.
    Nul,
    /// A leading `/` (also `\\srv\x` and `\x` of a zip).
    Absolute,
    /// A first component `X:`.
    Drive,
    /// A `..` component.
    Traversal,
    /// Longer than [`NAME_MAX`].
    TooLong,
    /// A `\` in a tar name.
    Backslash,
    /// A file below another file or a link (rule 2).
    Conflict,
    /// A hard link that names no earlier regular member (rule 6).
    HardLink,
    /// A file whose name reduces to nothing: it has nowhere to go.
    Empty,
}

impl Refusal {
    /// The reason text.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Refusal::Nul => "nul",
            Refusal::Absolute => "absolute",
            Refusal::Drive => "drive",
            Refusal::Traversal => "traversal",
            Refusal::TooLong => "too_long",
            Refusal::Backslash => "backslash",
            Refusal::Conflict => "conflict",
            Refusal::HardLink => "hard_link",
            Refusal::Empty => "empty",
        }
    }
}

/// How `\` is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameRules {
    /// `\` is `/` (legacy tools).
    Zip,
    /// `\` refuses the member.
    Tar,
}

/// Applies rule 1 to a raw name. `Ok` is the name with `/` separators and without `.` and empty components, as
/// bytes; empty when it reduces to nothing (`./`): that name only gives the root its metadata.
pub(crate) fn check_name(raw: &[u8], rules: NameRules) -> Result<Vec<u8>, Refusal> {
    if raw.len() > NAME_MAX {
        return Err(Refusal::TooLong);
    }
    if raw.contains(&0) {
        return Err(Refusal::Nul);
    }
    let name: Cow<'_, [u8]> = match rules {
        NameRules::Zip if raw.contains(&b'\\') => Cow::Owned(
            raw.iter()
                .map(|&b| if b == b'\\' { b'/' } else { b })
                .collect(),
        ),
        NameRules::Zip => Cow::Borrowed(raw),
        NameRules::Tar if raw.contains(&b'\\') => return Err(Refusal::Backslash),
        NameRules::Tar => Cow::Borrowed(raw),
    };
    if name.first() == Some(&b'/') {
        return Err(Refusal::Absolute);
    }
    let mut clean = Vec::with_capacity(name.len());
    for part in name.split(|&b| b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => return Err(Refusal::Traversal),
            part => {
                // The first component that stays: `./C:/x` is `C:/x` once reduced.
                if clean.is_empty() {
                    if let [letter, b':', ..] = *part
                        && letter.is_ascii_alphabetic()
                    {
                        return Err(Refusal::Drive);
                    }
                } else {
                    clean.push(b'/');
                }
                clean.extend_from_slice(part);
            }
        }
    }
    Ok(clean)
}

/// The same reduction on decoded text (`\` already mapped by the caller): drops `.` and empty components. The bytes
/// passed [`check_name`], so nothing else can appear.
pub(crate) fn reduce_text(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    for part in text.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if !clean.is_empty() {
            clean.push('/');
        }
        clean.push_str(part);
    }
    clean
}

/// The raw name in a form that can be shown: printable ASCII as it is, everything else escaped.
pub(crate) fn escape_name(raw: &[u8]) -> String {
    const SHOWN: usize = 256;
    let mut out = String::new();
    for chunk in raw.iter().take(SHOWN) {
        out.extend(chunk.escape_ascii().map(char::from));
    }
    if raw.len() > SHOWN {
        out.push_str("...");
    }
    out
}

/// Windows device names, compared case-insensitively before the first `.`.
const DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³", "CONIN$", "CONOUT$",
];

/// True when `name` is a Windows device name, in any case, also with an extension (`nul.txt`, `Com1.tar.gz`) or spaces
/// before it: Windows opens the device, not a file. The one list of them: session names ask here too.
pub fn windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    DEVICES
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem))
}

/// Rule 17: `Some(reason)` when `name` cannot be a file name on Windows (`< > : " | ? *`, control characters, a
/// trailing dot or space, a device name). Pure, so it is tested on every platform; `LocalVfs::child` calls it on
/// Windows.
pub fn windows_name_problem(name: &str) -> Option<&'static str> {
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control())
    {
        return Some("a character Windows does not allow");
    }
    if name.ends_with(['.', ' ']) {
        return Some("a trailing dot or space");
    }
    if windows_device_name(name) {
        return Some("a device name");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_1_table() {
        use Refusal as R;
        let long = vec![b'x'; NAME_MAX + 1];
        let fits = vec![b'x'; NAME_MAX];
        type Want<'a> = Result<&'a [u8], Refusal>;
        let rows: &[(&[u8], Want<'_>, Want<'_>)] = &[
            (b"../x", Err(R::Traversal), Err(R::Traversal)),
            (b"a/../../x", Err(R::Traversal), Err(R::Traversal)),
            (b"/etc/passwd", Err(R::Absolute), Err(R::Absolute)),
            (br"\x", Err(R::Absolute), Err(R::Backslash)),
            (br"\\srv\x", Err(R::Absolute), Err(R::Backslash)),
            (br"C:\x", Err(R::Drive), Err(R::Backslash)),
            (br".\C:\x", Err(R::Drive), Err(R::Backslash)),
            (b"./C:/x", Err(R::Drive), Err(R::Drive)),
            (b".//D:/x", Err(R::Drive), Err(R::Drive)),
            (b"a/C:/x", Ok(b"a/C:/x"), Ok(b"a/C:/x")),
            (b"C:/x", Err(R::Drive), Err(R::Drive)),
            (b"c:x", Err(R::Drive), Err(R::Drive)),
            (br"..\..\x", Err(R::Traversal), Err(R::Backslash)),
            (br"a\b", Ok(b"a/b"), Err(R::Backslash)),
            (b"a\0b", Err(R::Nul), Err(R::Nul)),
            (&long, Err(R::TooLong), Err(R::TooLong)),
            (&fits, Ok(&fits), Ok(&fits)),
            (b"./a//b", Ok(b"a/b"), Ok(b"a/b")),
            (b".", Ok(b""), Ok(b"")),
            (b"./", Ok(b""), Ok(b"")),
            (b"a/", Ok(b"a"), Ok(b"a")),
            (b"1:x", Ok(b"1:x"), Ok(b"1:x")),
            (b"a..b/..c", Ok(b"a..b/..c"), Ok(b"a..b/..c")),
        ];
        for (name, zip, tar) in rows {
            let show = String::from_utf8_lossy(&name[..name.len().min(20)]);
            let got = |rules| check_name(name, rules);
            let zip_got = got(NameRules::Zip);
            let tar_got = got(NameRules::Tar);
            assert_eq!(zip_got.as_deref().map_err(|e| *e), *zip, "zip {show}");
            assert_eq!(tar_got.as_deref().map_err(|e| *e), *tar, "tar {show}");
        }
    }

    #[test]
    fn rule_17_table() {
        for bad in [
            "a<b",
            "a>b",
            "a:b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "a\u{1}",
            "a.",
            "a ",
            "CON",
            "con",
            "nul.txt",
            "Com1.tar.gz",
            "LPT9",
            "com¹",
            "LPT³.x",
            "CONIN$",
            "conout$.log",
            "AUX .x",
        ] {
            assert!(windows_name_problem(bad).is_some(), "{bad:?}");
        }
        for good in [
            "a.b", "a b", ".a", "é", "a_b", "console", "COM0", "COM10", "x.con", "nul_", "LPT",
        ] {
            assert_eq!(windows_name_problem(good), None, "{good:?}");
        }
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_name(b"a\xffb\0"), "a\\xffb\\x00");
        assert!(escape_name(&[b'x'; 1000]).ends_with("..."));
        assert_eq!(reduce_text("./a//b/."), "a/b");
    }
}

//! The entry model (ARCH 3.2): [`Entry`], [`EntryKind`], [`FileName`], [`Attrs`], [`DosAttrs`],
//! [`UnixMeta`], [`FileId`].

use std::ffi::{OsStr, OsString};
use std::fmt;

use hikasa_core::time::FileTime;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use crate::error::VfsError;

/// Entry kind. `Ord` puts folders first (used for deterministic child ordering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A folder.
    Dir,
    /// A regular file.
    File,
    /// A symbolic link that was not followed.
    Symlink,
    /// Anything else (device, FIFO, socket, …).
    Other,
}

impl EntryKind {
    /// Every kind, in `Ord` order.
    pub const ALL: [EntryKind; 4] = [
        EntryKind::Dir,
        EntryKind::File,
        EntryKind::Symlink,
        EntryKind::Other,
    ];

    /// Stable snake_case name, identical to the serde form (`"dir"`, `"file"`, `"symlink"`, `"other"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            EntryKind::Dir => "dir",
            EntryKind::File => "file",
            EntryKind::Symlink => "symlink",
            EntryKind::Other => "other",
        }
    }
}

impl fmt::Display for EntryKind {
    /// Writes [`EntryKind::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A file name. `as_str()` is the UTF-8 form, lossy when needed, used for matching and display.
/// `raw` keeps the exact OS name when that conversion was lossy (INV-NAME-1: `raw.is_some()` ⇔ lossy).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileName {
    text: Box<str>,
    raw: Option<OsString>,
}

impl FileName {
    /// From an OS name (never fails; lossy names keep `raw`). The name is not validated: back-ends pass the
    /// names they read from directory records (and the whole path text for a root, which has no name).
    pub fn from_os(name: &OsStr) -> FileName {
        match name.to_str() {
            Some(text) => FileName {
                text: text.into(),
                raw: None,
            },
            None => FileName {
                text: name.to_string_lossy().into(),
                raw: Some(name.to_os_string()),
            },
        }
    }

    /// From UTF-8; rejects `""`, `"."`, `".."`, `'/'`, NUL (and `'\\'` on Windows) with
    /// [`VfsError::InvalidName`].
    pub fn new(name: &str) -> Result<FileName, VfsError> {
        let separator = |c: char| c == '/' || c == '\0' || (cfg!(windows) && c == '\\');
        if name.is_empty() || name == "." || name == ".." || name.contains(separator) {
            return Err(VfsError::InvalidName {
                name: name.to_owned(),
            });
        }
        Ok(FileName {
            text: name.into(),
            raw: None,
        })
    }

    /// A name whose exact bytes stay elsewhere (an archive holds them, D-141): `text` is lossy, and it is also the only
    /// form this name has, so `to_os_string` gives `text`.
    pub(crate) fn from_lossy_text(text: &str) -> FileName {
        FileName {
            text: text.into(),
            raw: Some(OsString::from(text)),
        }
    }

    /// UTF-8 (possibly lossy) text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Exact OS name.
    pub fn to_os_string(&self) -> OsString {
        match &self.raw {
            Some(raw) => raw.clone(),
            None => OsString::from(&*self.text),
        }
    }

    /// True when `as_str()` is lossy.
    pub fn is_lossy(&self) -> bool {
        self.raw.is_some()
    }

    /// Text after the last `'.'`, when that `'.'` is neither the first nor the last char (`".bashrc"` →
    /// `None`, `"a."` → `None`, `"a.b.c"` → `"c"`).
    pub fn extension(&self) -> Option<&str> {
        let dot = self.text.rfind('.')?;
        if dot == 0 || dot + 1 == self.text.len() {
            return None;
        }
        self.text.get(dot + 1..)
    }

    /// Name without `.extension` (the whole name when there is no extension).
    pub fn stem(&self) -> &str {
        match self.extension() {
            Some(ext) => self
                .text
                .get(..self.text.len() - ext.len() - 1)
                .unwrap_or(&self.text),
            None => &self.text,
        }
    }
}

impl fmt::Display for FileName {
    /// Writes the UTF-8 (possibly lossy) text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

bitflags::bitflags! {
    /// DOS/Windows attributes; values equal the Win32 `FILE_ATTRIBUTE_*` constants. Letters per D-30.
    /// No serde derive: the hand-written impls below serialise the canonical letter string (D-30).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct DosAttrs: u32 {
        /// R
        const READONLY = 0x0000_0001;
        /// H
        const HIDDEN = 0x0000_0002;
        /// S
        const SYSTEM = 0x0000_0004;
        /// D
        const DIRECTORY = 0x0000_0010;
        /// A
        const ARCHIVE = 0x0000_0020;
        /// N
        const NORMAL = 0x0000_0080;
        /// T
        const TEMPORARY = 0x0000_0100;
        /// Z
        const SPARSE = 0x0000_0200;
        /// L
        const REPARSE_POINT = 0x0000_0400;
        /// C
        const COMPRESSED = 0x0000_0800;
        /// O
        const OFFLINE = 0x0000_1000;
        /// I
        const NOT_CONTENT_INDEXED = 0x0000_2000;
        /// E
        const ENCRYPTED = 0x0000_4000;
        /// P
        const PINNED = 0x0008_0000;
        /// U
        const UNPINNED = 0x0010_0000;
        /// M
        const RECALL_ON_DATA_ACCESS = 0x0040_0000;
    }
}

/// The letters in canonical order `A C D E H I L M N O P R S T U Z` (FC-040, D-30).
const DOS_LETTERS: [(char, DosAttrs); 16] = [
    ('A', DosAttrs::ARCHIVE),
    ('C', DosAttrs::COMPRESSED),
    ('D', DosAttrs::DIRECTORY),
    ('E', DosAttrs::ENCRYPTED),
    ('H', DosAttrs::HIDDEN),
    ('I', DosAttrs::NOT_CONTENT_INDEXED),
    ('L', DosAttrs::REPARSE_POINT),
    ('M', DosAttrs::RECALL_ON_DATA_ACCESS),
    ('N', DosAttrs::NORMAL),
    ('O', DosAttrs::OFFLINE),
    ('P', DosAttrs::PINNED),
    ('R', DosAttrs::READONLY),
    ('S', DosAttrs::SYSTEM),
    ('T', DosAttrs::TEMPORARY),
    ('U', DosAttrs::UNPINNED),
    ('Z', DosAttrs::SPARSE),
];

impl DosAttrs {
    /// Letters in canonical order `A C D E H I L M N O P R S T U Z` (FC-040). Bits without a letter are not
    /// shown.
    pub fn letters(self) -> String {
        DOS_LETTERS
            .iter()
            .filter(|(_, flag)| self.contains(*flag))
            .map(|(letter, _)| *letter)
            .collect()
    }

    /// Parses letters (case-insensitive, any order, duplicates allowed); unknown letters are an error
    /// ([`VfsError::InvalidAttributes`]). The empty string is the empty set.
    pub fn parse_letters(s: &str) -> Result<DosAttrs, VfsError> {
        s.chars().try_fold(DosAttrs::empty(), |acc, c| {
            let upper = c.to_ascii_uppercase();
            DOS_LETTERS
                .iter()
                .find(|(letter, _)| *letter == upper)
                .map(|(_, flag)| acc | *flag)
                .ok_or_else(|| VfsError::InvalidAttributes {
                    input: s.to_owned(),
                })
        })
    }
}

impl Serialize for DosAttrs {
    /// The string `self.letters()`, e.g. `"HRS"`; the empty set is `""` (D-30).
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.letters())
    }
}

impl<'de> Deserialize<'de> for DosAttrs {
    /// [`DosAttrs::parse_letters`]; errors become serde errors.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DosAttrs, D::Error> {
        deserializer.deserialize_str(DosAttrsVisitor)
    }
}

struct DosAttrsVisitor;

impl Visitor<'_> for DosAttrsVisitor {
    type Value = DosAttrs;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DOS attribute letters such as \"HRS\"")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<DosAttrs, E> {
        DosAttrs::parse_letters(value).map_err(E::custom)
    }
}

/// Unix metadata (FC-024). `mode` holds permission bits only (`st_mode & 0o7777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnixMeta {
    /// Permission bits including setuid/setgid/sticky.
    pub mode: u32,
    /// Owner uid (names are not resolved in M1: std has no API for it).
    pub uid: u32,
    /// Group gid.
    pub gid: u32,
}

impl UnixMeta {
    /// `drwxr-xr-x` style string. The first char comes from `kind` (`d` folder, `-` file, `l` symlink, `?`
    /// other); the special bits show as `s`/`S` (setuid, setgid) and `t`/`T` (sticky), lower case when the
    /// execute bit under them is set.
    pub fn permissions_string(&self, kind: EntryKind) -> String {
        let mode = self.mode;
        let bit = |mask: u32, c: char| if mode & mask != 0 { c } else { '-' };
        let exec = |x: u32, special: u32, set: char, unset: char| match (
            mode & x != 0,
            mode & special != 0,
        ) {
            (true, true) => set,
            (false, true) => unset,
            (true, false) => 'x',
            (false, false) => '-',
        };
        let mut out = String::with_capacity(10);
        out.push(match kind {
            EntryKind::Dir => 'd',
            EntryKind::File => '-',
            EntryKind::Symlink => 'l',
            EntryKind::Other => '?',
        });
        out.push(bit(0o400, 'r'));
        out.push(bit(0o200, 'w'));
        out.push(exec(0o100, 0o4000, 's', 'S'));
        out.push(bit(0o040, 'r'));
        out.push(bit(0o020, 'w'));
        out.push(exec(0o010, 0o2000, 's', 'S'));
        out.push(bit(0o004, 'r'));
        out.push(bit(0o002, 'w'));
        out.push(exec(0o001, 0o1000, 't', 'T'));
        out
    }
}

/// Platform attributes of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Attrs {
    /// Back-end has no attributes.
    None,
    /// Windows.
    Dos {
        /// The attribute bits.
        attrs: DosAttrs,
    },
    /// Unix.
    Unix {
        /// Mode, owner and group.
        meta: UnixMeta,
    },
}

impl Attrs {
    /// The DOS attributes, when present.
    pub fn dos(&self) -> Option<DosAttrs> {
        match self {
            Attrs::Dos { attrs } => Some(*attrs),
            Attrs::None | Attrs::Unix { .. } => None,
        }
    }

    /// The Unix metadata, when present.
    pub fn unix(&self) -> Option<UnixMeta> {
        match self {
            Attrs::Unix { meta } => Some(*meta),
            Attrs::None | Attrs::Dos { .. } => None,
        }
    }
}

/// File-object identity for loop detection (Unix device + inode; `MemoryVfs` node ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId {
    /// Device.
    pub dev: u64,
    /// Inode.
    pub ino: u64,
}

/// Metadata of one entry (ARCH 3.2). Pure data; equality is field-wise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Name within its parent (for a root, which has no name, the root's path text).
    pub name: FileName,
    /// Kind (of the target when `followed`).
    pub kind: EntryKind,
    /// Files: byte length. Folders: 0. Unfollowed symlinks: byte length of the exact target (the raw OS bytes
    /// on Unix, UTF-8 on Windows), which equals `link_target.len()` unless the target is lossy.
    pub size: u64,
    /// Last modification time.
    pub mtime: Option<FileTime>,
    /// Creation (birth) time where available. NOT the POSIX status-change time.
    pub ctime: Option<FileTime>,
    /// Attributes.
    pub attrs: Attrs,
    /// Symlink target text (for unfollowed links, and for followed ones for display); UTF-8, lossy when the
    /// target is not (see `link_target_raw`).
    pub link_target: Option<String>,
    /// The exact OS target when `link_target` is lossy (`Some` only then, like `FileName`'s raw name,
    /// INV-NAME-1). Exact comparisons of link targets use [`Entry::link_target_os`].
    pub link_target_raw: Option<OsString>,
    /// True when this entry is a symlink that was followed (the other fields describe the target).
    pub followed: bool,
    /// CRC32 provided by the back-end, or computed earlier in this run.
    pub crc32: Option<u32>,
    /// Identity for loop detection (Unix and `MemoryVfs`; `None` elsewhere).
    pub file_id: Option<FileId>,
}

impl Entry {
    /// An entry with the given name and kind and no metadata: size 0, no times, `Attrs::None`, no link
    /// target, not followed, no CRC, no file id.
    pub fn new(name: FileName, kind: EntryKind) -> Entry {
        Entry {
            name,
            kind,
            size: 0,
            mtime: None,
            ctime: None,
            attrs: Attrs::None,
            link_target: None,
            link_target_raw: None,
            followed: false,
            crc32: None,
            file_id: None,
        }
    }

    /// The exact symlink target: `link_target_raw` when the text is lossy, else `link_target`. Two targets that
    /// differ only in bytes that are not UTF-8 have the same `link_target` text but different exact targets.
    pub fn link_target_os(&self) -> Option<OsString> {
        match (&self.link_target_raw, &self.link_target) {
            (Some(raw), _) => Some(raw.clone()),
            (None, Some(text)) => Some(OsString::from(text)),
            (None, None) => None,
        }
    }

    /// Sets `link_target` (UTF-8, lossy when needed) and `link_target_raw` (`Some` iff lossy) from an OS
    /// target.
    pub(crate) fn set_link_target(&mut self, target: &OsStr) {
        match target.to_str() {
            Some(text) => {
                self.link_target = Some(text.to_owned());
                self.link_target_raw = None;
            }
            None => {
                self.link_target = Some(target.to_string_lossy().into_owned());
                self.link_target_raw = Some(target.to_os_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> FileName {
        FileName::new(s).unwrap()
    }

    #[test]
    fn extension_and_stem() {
        let cases = [
            (".bashrc", None, ".bashrc"),
            ("a.", None, "a."),
            ("a.b.c", Some("c"), "a.b"),
            ("x", None, "x"),
            ("main.rs", Some("rs"), "main"),
            ("..a", Some("a"), "."),
            ("été.txt", Some("txt"), "été"),
        ];
        for (text, ext, stem) in cases {
            let n = name(text);
            assert_eq!(n.extension(), ext, "{text}");
            assert_eq!(n.stem(), stem, "{text}");
        }
    }

    #[test]
    fn new_rejects_invalid_names() {
        for bad in ["", ".", "..", "a/b", "a\0b"] {
            assert!(
                matches!(FileName::new(bad), Err(VfsError::InvalidName { .. })),
                "{bad:?}"
            );
        }
        assert_eq!(FileName::new("a\\b").is_ok(), !cfg!(windows));
        assert!(FileName::new("...").is_ok());
        assert!(FileName::new(" x ").is_ok());
    }

    #[test]
    fn utf8_names_are_not_lossy() {
        let n = FileName::from_os(OsStr::new("café"));
        assert!(!n.is_lossy());
        assert_eq!(n.as_str(), "café");
        assert_eq!(n.to_os_string(), OsString::from("café"));
        assert_eq!(n, name("café"));
    }

    #[cfg(unix)]
    #[test]
    fn lossy_names_keep_the_raw_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"a\xffb");
        let n = FileName::from_os(raw);
        assert!(n.is_lossy());
        assert_eq!(n.as_str(), "a\u{FFFD}b");
        assert_eq!(n.to_os_string(), raw.to_os_string());
    }

    #[test]
    fn dos_letters_round_trip() {
        let all = DosAttrs::all();
        assert_eq!(all.letters(), "ACDEHILMNOPRSTUZ");
        assert_eq!(DosAttrs::parse_letters("ACDEHILMNOPRSTUZ").unwrap(), all);
        let hrs = DosAttrs::HIDDEN | DosAttrs::READONLY | DosAttrs::SYSTEM;
        assert_eq!(hrs.letters(), "HRS");
        assert_eq!(DosAttrs::parse_letters("srhh").unwrap(), hrs);
        assert_eq!(DosAttrs::parse_letters("").unwrap(), DosAttrs::empty());
        assert_eq!(DosAttrs::empty().letters(), "");
        assert!(matches!(
            DosAttrs::parse_letters("HX"),
            Err(VfsError::InvalidAttributes { .. })
        ));
        for (letter, flag) in DOS_LETTERS {
            assert_eq!(flag.letters(), letter.to_string());
            assert_eq!(DosAttrs::parse_letters(&letter.to_string()).unwrap(), flag);
        }
        // Bits without a letter are not shown.
        assert_eq!(DosAttrs::from_bits_retain(0x8 | 0x2).letters(), "H");
    }

    #[test]
    fn permissions_table() {
        let cases = [
            (0o755, EntryKind::Dir, "drwxr-xr-x"),
            (0o644, EntryKind::File, "-rw-r--r--"),
            (0o777, EntryKind::Symlink, "lrwxrwxrwx"),
            (0o000, EntryKind::File, "----------"),
            (0o000, EntryKind::Other, "?---------"),
            (0o4755, EntryKind::File, "-rwsr-xr-x"),
            (0o4644, EntryKind::File, "-rwSr--r--"),
            (0o2755, EntryKind::File, "-rwxr-sr-x"),
            (0o2745, EntryKind::File, "-rwxr-Sr-x"),
            (0o1777, EntryKind::Dir, "drwxrwxrwt"),
            (0o1776, EntryKind::Dir, "drwxrwxrwT"),
            (0o7777, EntryKind::File, "-rwsrwsrwt"),
            (0o7000, EntryKind::File, "---S--S--T"),
            (0o421, EntryKind::File, "-r---w---x"),
        ];
        for (mode, kind, text) in cases {
            let meta = UnixMeta {
                mode,
                uid: 0,
                gid: 0,
            };
            assert_eq!(meta.permissions_string(kind), text, "{mode:o}");
        }
    }

    #[test]
    fn kinds_order_folders_first() {
        let mut kinds = [
            EntryKind::Other,
            EntryKind::Symlink,
            EntryKind::File,
            EntryKind::Dir,
        ];
        kinds.sort();
        assert_eq!(kinds, EntryKind::ALL);
        assert_eq!(EntryKind::Dir.to_string(), "dir");
    }

    #[test]
    fn attrs_accessors() {
        let dos = Attrs::Dos {
            attrs: DosAttrs::HIDDEN,
        };
        assert_eq!(dos.dos(), Some(DosAttrs::HIDDEN));
        assert_eq!(dos.unix(), None);
        let meta = UnixMeta {
            mode: 0o644,
            uid: 1,
            gid: 2,
        };
        assert_eq!(Attrs::Unix { meta }.unix(), Some(meta));
        assert_eq!(Attrs::None.dos(), None);
    }
}

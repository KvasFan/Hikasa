//! [`Capabilities`]: what a back-end can do at a given spec (ARCH-030).

bitflags::bitflags! {
    /// What a back-end can do at a given spec (ARCH-030).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Capabilities: u32 {
        /// `open_read`.
        const READ = 1 << 0;
        /// `open_write`.
        const WRITE = 1 << 1;
        /// `list`.
        const LIST = 1 << 2;
        /// Readers implement `Seek`.
        const SEEK = 1 << 3;
        /// `Entry::mtime` is provided.
        const MTIME = 1 << 4;
        /// `Entry::ctime` is provided.
        const CTIME = 1 << 5;
        /// `set_times` with a modification time works (M2 W3).
        const SET_TIMES = 1 << 6;
        /// `Attrs::Dos`.
        const DOS_ATTRS = 1 << 7;
        /// `Attrs::Unix` mode.
        const UNIX_MODE = 1 << 8;
        /// `Attrs::Unix` uid/gid.
        const UNIX_OWNER = 1 << 9;
        /// Symlinks are reported.
        const SYMLINKS = 1 << 10;
        /// `Entry::crc32` is provided without reading.
        const CRC_PROVIDED = 1 << 11;
        /// `local_path()` returns paths (mmap possible).
        const LOCAL_PATH = 1 << 12;
        /// Names are case-sensitive.
        const CASE_SENSITIVE = 1 << 13;
        /// `create_dir` (M2 W3).
        const MKDIR = 1 << 14;
        /// `remove` (M2 W3).
        const REMOVE = 1 << 15;
        /// `rename` (M2 W3).
        const RENAME = 1 << 16;
        /// `set_attrs` for the family the back-end reports (`DOS_ATTRS` or `UNIX_MODE`; M2 W3).
        const SET_ATTRS = 1 << 17;
        /// `move_to_trash` (M2 W3).
        const TRASH = 1 << 18;
        /// `create_symlink` (M2 W3).
        const CREATE_SYMLINKS = 1 << 19;
        /// `set_times` with a creation time (M2 W3).
        const SET_CREATED = 1 << 20;
    }
}

/// snake_case names in canonical bit order.
const NAMES: [(Capabilities, &str); 21] = [
    (Capabilities::READ, "read"),
    (Capabilities::WRITE, "write"),
    (Capabilities::LIST, "list"),
    (Capabilities::SEEK, "seek"),
    (Capabilities::MTIME, "mtime"),
    (Capabilities::CTIME, "ctime"),
    (Capabilities::SET_TIMES, "set_times"),
    (Capabilities::DOS_ATTRS, "dos_attrs"),
    (Capabilities::UNIX_MODE, "unix_mode"),
    (Capabilities::UNIX_OWNER, "unix_owner"),
    (Capabilities::SYMLINKS, "symlinks"),
    (Capabilities::CRC_PROVIDED, "crc_provided"),
    (Capabilities::LOCAL_PATH, "local_path"),
    (Capabilities::CASE_SENSITIVE, "case_sensitive"),
    (Capabilities::MKDIR, "mkdir"),
    (Capabilities::REMOVE, "remove"),
    (Capabilities::RENAME, "rename"),
    (Capabilities::SET_ATTRS, "set_attrs"),
    (Capabilities::TRASH, "trash"),
    (Capabilities::CREATE_SYMLINKS, "create_symlinks"),
    (Capabilities::SET_CREATED, "set_created"),
];

impl Capabilities {
    /// The set capabilities as snake_case names in canonical bit order (`["read", "list", …]`), the JSON form
    /// of bit sets (§4.1 rule 9).
    pub fn names(self) -> Vec<&'static str> {
        NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }

    /// Inverse of [`Capabilities::names`] for one name (`None` for unknown names).
    pub fn parse_name(name: &str) -> Option<Capabilities> {
        NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(flag, _)| *flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_bit_order() {
        assert_eq!(Capabilities::all().names().len(), 21);
        assert_eq!(
            (Capabilities::LIST | Capabilities::READ | Capabilities::CASE_SENSITIVE).names(),
            ["read", "list", "case_sensitive"]
        );
        for (flag, name) in NAMES {
            assert_eq!(Capabilities::parse_name(name), Some(flag));
            assert_eq!(flag.names(), [name]);
        }
        assert_eq!(Capabilities::parse_name("teleport"), None);
        let mut bits: Vec<u32> = NAMES.iter().map(|(flag, _)| flag.bits()).collect();
        let sorted = bits.clone();
        bits.sort_unstable();
        assert_eq!(bits, sorted, "canonical bit order");
    }
}

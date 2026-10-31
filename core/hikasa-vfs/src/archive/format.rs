//! [`ArchiveFormat`] and [`ArchiveMasks`] (D-136): a format is decided by the mask of the file name, never by content.

use hikasa_core::{CaseSensitivity, MaskError, MaskList, MaskOptions};
use serde::{Deserialize, Serialize};

use crate::caps::Capabilities;

/// The ten archive types of `archives.toml` (`ArchiveType` of the session maps one to one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    /// Zip and its relatives (`jar`, `war`, `ear`, `apk`).
    Zip,
    /// Plain tar.
    Tar,
    /// Gzip-compressed tar.
    TarGz,
    /// Bzip2-compressed tar.
    TarBz2,
    /// Xz-compressed tar.
    TarXz,
    /// Zstandard-compressed tar.
    TarZst,
    /// One gzip stream.
    Gz,
    /// One bzip2 stream.
    Bz2,
    /// One xz stream.
    Xz,
    /// One zstandard stream.
    Zst,
}

impl ArchiveFormat {
    /// Detection order, the same as `ArchiveType::ORDER`: zip, the tar family, then the single streams.
    pub const ORDER: [ArchiveFormat; 10] = [
        ArchiveFormat::Zip,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarBz2,
        ArchiveFormat::TarXz,
        ArchiveFormat::TarZst,
        ArchiveFormat::Gz,
        ArchiveFormat::Bz2,
        ArchiveFormat::Xz,
        ArchiveFormat::Zst,
    ];

    /// Zip and the five tar formats can be rewritten (D-63); a single stream cannot.
    pub const fn writable(self) -> bool {
        !self.is_single_stream()
    }

    /// `Gz`, `Bz2`, `Xz`, `Zst`: an archive of one member.
    pub const fn is_single_stream(self) -> bool {
        matches!(
            self,
            ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst
        )
    }

    /// The bits of §1.5.3 for W5a: no write bit in any format.
    pub(crate) fn capabilities(self) -> Capabilities {
        let common = Capabilities::READ
            | Capabilities::LIST
            | Capabilities::SEEK
            | Capabilities::MTIME
            | Capabilities::CASE_SENSITIVE;
        match self {
            ArchiveFormat::Zip => {
                common
                    | Capabilities::CRC_PROVIDED
                    | Capabilities::DOS_ATTRS
                    | Capabilities::UNIX_MODE
                    | Capabilities::SYMLINKS
            }
            ArchiveFormat::Tar
            | ArchiveFormat::TarGz
            | ArchiveFormat::TarBz2
            | ArchiveFormat::TarXz
            | ArchiveFormat::TarZst => {
                common | Capabilities::UNIX_MODE | Capabilities::UNIX_OWNER | Capabilities::SYMLINKS
            }
            ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
                common
            }
        }
    }
}

/// The name of the single member of a stream: the container's name without its last `.ext` (`x.txt.gz` → `x.txt`),
/// `<name>.data` for a name with no extension. The name stored in a gzip header is untrusted and ignored.
pub(crate) fn stream_member_name(container: &str) -> String {
    match container.rfind('.').and_then(|dot| container.get(..dot)) {
        Some(stem) if !matches!(stem, "" | "." | "..") && !container.ends_with('.') => {
            stem.to_owned()
        }
        _ => format!("{container}.data"),
    }
}

/// The compiled mask of each archive format, in [`ArchiveFormat::ORDER`]; an empty mask is left out, so that type
/// does not exist for resolution.
#[derive(Debug, Clone, Default)]
pub struct ArchiveMasks {
    lists: Vec<(ArchiveFormat, MaskList)>,
}

impl ArchiveMasks {
    /// Compiles `masks` (case-insensitive). The order of the argument does not matter.
    pub fn new(
        masks: impl IntoIterator<Item = (ArchiveFormat, String)>,
    ) -> Result<ArchiveMasks, MaskError> {
        let opts = MaskOptions {
            case: CaseSensitivity::Insensitive,
        };
        let mut lists = Vec::new();
        for (format, mask) in masks {
            if !mask.is_empty() {
                lists.push((format, MaskList::parse(&mask, opts)?));
            }
        }
        lists.sort_by_key(|(format, _)| ArchiveFormat::ORDER.iter().position(|f| f == format));
        Ok(ArchiveMasks { lists })
    }

    /// The first format in [`ArchiveFormat::ORDER`] whose mask matches `name`, case-insensitively (as
    /// `ArchiveTypes::detect`).
    pub fn detect(&self, name: &str) -> Option<ArchiveFormat> {
        self.lists
            .iter()
            .find(|(_, list)| list.matches_name(name))
            .map(|(format, _)| *format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> ArchiveMasks {
        ArchiveMasks::new([
            (ArchiveFormat::Gz, "*.gz".to_owned()),
            (ArchiveFormat::TarGz, "*.tar.gz;*.tgz".to_owned()),
            (ArchiveFormat::Zip, "*.zip;*.jar".to_owned()),
            (ArchiveFormat::Tar, String::new()),
        ])
        .unwrap()
    }

    #[test]
    fn detection_order_and_case() {
        let masks = defaults();
        assert_eq!(masks.detect("x.tar.gz"), Some(ArchiveFormat::TarGz));
        assert_eq!(masks.detect("X.TGZ"), Some(ArchiveFormat::TarGz));
        assert_eq!(masks.detect("x.gz"), Some(ArchiveFormat::Gz));
        assert_eq!(masks.detect("a.JAR"), Some(ArchiveFormat::Zip));
        assert_eq!(masks.detect("a.tar"), None, "an empty mask is left out");
        assert_eq!(masks.detect("a.txt"), None);
    }

    #[test]
    fn stream_member_names() {
        assert_eq!(stream_member_name("x.txt.gz"), "x.txt");
        assert_eq!(stream_member_name("noext"), "noext.data");
        assert_eq!(stream_member_name(".gz"), ".gz.data");
        assert_eq!(stream_member_name("a."), "a..data");
        assert_eq!(stream_member_name("..gz"), "..gz.data");
        assert_eq!(stream_member_name("...gz"), "...gz.data");
    }

    #[test]
    fn writable_and_streams() {
        let writable = ArchiveFormat::ORDER.iter().filter(|f| f.writable()).count();
        assert_eq!(writable, 6);
        for format in ArchiveFormat::ORDER {
            assert_eq!(format.writable(), !format.is_single_stream());
            assert!(!format.capabilities().contains(Capabilities::WRITE));
        }
    }
}

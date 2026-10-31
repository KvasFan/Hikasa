//! `archives.toml` (OPT-010): the mask of each archive type (empty: the type is disabled). A tar-family type
//! is tried before its single-stream type (`x.tar.gz` is `tar_gz`, not `gz`).

use hikasa_core::{CaseSensitivity, MaskList, MaskOptions};
use toml::Table;

use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KindRule, ValueKind};

settings_enum! {
    /// An archive type of `[types]`, in detection order.
    ArchiveType {
        Zip = "zip",
        Tar = "tar",
        TarGz = "tar_gz",
        TarBz2 = "tar_bz2",
        TarXz = "tar_xz",
        TarZst = "tar_zst",
        Gz = "gz",
        Bz2 = "bz2",
        Xz = "xz",
        Zst = "zst",
    } default Zip
}

settings_section! {
    /// `[types]`: one mask list per archive type.
    ArchiveMasks {
        /// Zip archives.
        zip: String = "*.zip;*.jar;*.war;*.ear;*.apk".to_owned(),
        /// Tar archives.
        tar: String = "*.tar".to_owned(),
        /// Gzip-compressed tar archives.
        tar_gz: String = "*.tar.gz;*.tgz".to_owned(),
        /// Bzip2-compressed tar archives.
        tar_bz2: String = "*.tar.bz2;*.tbz;*.tbz2".to_owned(),
        /// Xz-compressed tar archives.
        tar_xz: String = "*.tar.xz;*.txz".to_owned(),
        /// Zstandard-compressed tar archives.
        tar_zst: String = "*.tar.zst;*.tzst".to_owned(),
        /// A gzip stream.
        gz: String = "*.gz".to_owned(),
        /// A bzip2 stream.
        bz2: String = "*.bz2".to_owned(),
        /// An xz stream.
        xz: String = "*.xz".to_owned(),
        /// A zstandard stream.
        zst: String = "*.zst".to_owned(),
    }
}

settings_section! {
    /// `archives.toml`.
    ArchiveTypes {
        /// `[types]`.
        types: ArchiveMasks = ArchiveMasks::default(),
    }
}

impl ArchiveTypes {
    /// The mask list of `kind` (empty: disabled).
    pub fn mask(&self, kind: ArchiveType) -> &str {
        let t = &self.types;
        match kind {
            ArchiveType::Zip => &t.zip,
            ArchiveType::Tar => &t.tar,
            ArchiveType::TarGz => &t.tar_gz,
            ArchiveType::TarBz2 => &t.tar_bz2,
            ArchiveType::TarXz => &t.tar_xz,
            ArchiveType::TarZst => &t.tar_zst,
            ArchiveType::Gz => &t.gz,
            ArchiveType::Bz2 => &t.bz2,
            ArchiveType::Xz => &t.xz,
            ArchiveType::Zst => &t.zst,
        }
    }

    /// The archive type of a file name: the first type in [`ArchiveType::ORDER`] whose mask matches
    /// (case-insensitive).
    pub fn detect(&self, name: &str) -> Option<ArchiveType> {
        ArchiveType::ORDER.into_iter().find(|kind| {
            let mask = self.mask(*kind);
            !mask.is_empty()
                && MaskList::parse(
                    mask,
                    MaskOptions {
                        case: CaseSensitivity::Insensitive,
                    },
                )
                .is_ok_and(|list| list.matches_name(name))
        })
    }
}

impl ArchiveType {
    /// Detection order: zip, the tar family, then the single streams.
    pub const ORDER: [ArchiveType; 10] = [
        ArchiveType::Zip,
        ArchiveType::Tar,
        ArchiveType::TarGz,
        ArchiveType::TarBz2,
        ArchiveType::TarXz,
        ArchiveType::TarZst,
        ArchiveType::Gz,
        ArchiveType::Bz2,
        ArchiveType::Xz,
        ArchiveType::Zst,
    ];
}

impl SettingsSchema for ArchiveTypes {
    const FILE: FileId = FileId::Archives;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    /// Every mask parses.
    fn validate(&self) -> Vec<Problem> {
        super::validate_by_rules(self, &FileId::Archives)
    }
}

/// Kinds of the keys of `archives.toml`.
pub(crate) const RULES: &[KindRule] = &[KindRule {
    pattern: "types.*",
    kind: ValueKind::Mask,
    opt: Some("OPT-010"),
    priority: Some("P0"),
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_family_wins() {
        let types = ArchiveTypes::default();
        assert_eq!(types.detect("x.tar.gz"), Some(ArchiveType::TarGz));
        assert_eq!(types.detect("X.TGZ"), Some(ArchiveType::TarGz));
        assert_eq!(types.detect("x.gz"), Some(ArchiveType::Gz));
        assert_eq!(types.detect("a.jar"), Some(ArchiveType::Zip));
        assert_eq!(types.detect("a.txt"), None);
        let mut off = ArchiveTypes::default();
        off.types.tar_gz = String::new();
        assert_eq!(off.detect("x.tar.gz"), Some(ArchiveType::Gz));
        off.types.zip = "[".into();
        assert_eq!(off.validate()[0].key, "types.zip");
    }
}

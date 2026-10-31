//! `archives.toml` (OPT-010): the mask of each archive type (empty: the type is disabled). A tar-family type
//! is tried before its single-stream type (`x.tar.gz` is `tar_gz`, not `gz`).

use hikasa_core::{CaseSensitivity, MaskList, MaskOptions};
use hikasa_vfs::{ArchiveFormat, ArchiveLimits};
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
    /// `[limits]` (OPT-010; contract §1.5.7): what archives may cost. `ratio_floor` is not a key.
    ArchiveLimitsSection {
        /// Members of one archive.
        entries_max: u64 = 1_000_000,
        /// Decoded GiB of one pass, and of all reads of one archive.
        stream_max_gib: u64 = 64,
        /// GiB of the temp cache; also the largest member that spools.
        cache_max_gib: u64 = 8,
        /// Output / input past the floor; 0 turns the check off.
        ratio_max: u32 = 2000,
        /// Archives around a spec.
        nesting_max: u32 = 8,
        /// MiB of the text of one index: paths, link targets, the paths of folders no member names.
        paths_max_mib: u64 = 256,
        /// Folders above a member.
        depth_max: u32 = 256,
    }
}

settings_section! {
    /// `archives.toml`.
    ArchiveTypes {
        /// `[types]`.
        types: ArchiveMasks = ArchiveMasks::default(),
        /// `[limits]`.
        limits: ArchiveLimitsSection = ArchiveLimitsSection::default(),
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

    /// The masks as `hikasa-vfs` takes them. A mask that does not parse is left out (a normal load refuses it).
    pub fn masks(&self) -> hikasa_vfs::ArchiveMasks {
        let masks = ArchiveType::ORDER.into_iter().filter_map(|kind| {
            let mask = self.mask(kind);
            super::check_mask(mask)
                .is_ok()
                .then(|| (kind.format(), mask.to_owned()))
        });
        hikasa_vfs::ArchiveMasks::new(masks).unwrap_or_default()
    }

    /// The limits as `hikasa-vfs` takes them.
    pub fn limits(&self) -> ArchiveLimits {
        const GIB: u64 = 1 << 30;
        let l = &self.limits;
        ArchiveLimits {
            entries_max: l.entries_max,
            stream_max: l.stream_max_gib.saturating_mul(GIB),
            cache_max: l.cache_max_gib.saturating_mul(GIB),
            ratio_max: l.ratio_max,
            nesting_max: l.nesting_max,
            paths_max: l.paths_max_mib.saturating_mul(1 << 20),
            depth_max: l.depth_max,
            ..ArchiveLimits::default()
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

impl ArchiveType {
    /// The back-end's format of this type (one to one).
    pub const fn format(self) -> ArchiveFormat {
        match self {
            ArchiveType::Zip => ArchiveFormat::Zip,
            ArchiveType::Tar => ArchiveFormat::Tar,
            ArchiveType::TarGz => ArchiveFormat::TarGz,
            ArchiveType::TarBz2 => ArchiveFormat::TarBz2,
            ArchiveType::TarXz => ArchiveFormat::TarXz,
            ArchiveType::TarZst => ArchiveFormat::TarZst,
            ArchiveType::Gz => ArchiveFormat::Gz,
            ArchiveType::Bz2 => ArchiveFormat::Bz2,
            ArchiveType::Xz => ArchiveFormat::Xz,
            ArchiveType::Zst => ArchiveFormat::Zst,
        }
    }
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
pub(crate) const RULES: &[KindRule] = &[
    KindRule {
        pattern: "types.*",
        kind: ValueKind::Mask,
        opt: Some("OPT-010"),
        priority: Some("P0"),
    },
    limit("limits.entries_max", 1_000, 100_000_000),
    limit("limits.stream_max_gib", 1, 4_096),
    limit("limits.cache_max_gib", 1, 1_024),
    limit("limits.ratio_max", 0, 1_000_000),
    limit("limits.nesting_max", 1, 32),
    limit("limits.paths_max_mib", 1, 65_536),
    limit("limits.depth_max", 16, 2_048),
];

/// The rule of one limit of §1.5.7.
const fn limit(pattern: &'static str, min: i64, max: i64) -> KindRule {
    KindRule {
        pattern,
        kind: ValueKind::Int { min, max },
        opt: Some("OPT-010"),
        priority: Some("P0"),
    }
}

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
        assert_eq!(off.masks().detect("a.zip"), None, "a bad mask is left out");
        assert_eq!(off.masks().detect("x.tar"), Some(ArchiveFormat::Tar));
    }

    #[test]
    fn every_type_is_a_format_in_the_same_order() {
        let formats = ArchiveType::ORDER.map(ArchiveType::format);
        assert_eq!(formats, ArchiveFormat::ORDER);
        let types = ArchiveTypes::default();
        for name in ["a.zip", "A.TBZ2", "x.tar.zst", "x.zst", "x.txt"] {
            let via_settings = types.detect(name).map(ArchiveType::format);
            assert_eq!(types.masks().detect(name), via_settings, "{name}");
        }
    }

    #[test]
    fn limits_follow_the_table() {
        let types = ArchiveTypes::default();
        assert_eq!(types.limits(), ArchiveLimits::default());
        assert!(types.validate().is_empty());
        let mut low = ArchiveTypes::default();
        low.limits.entries_max = 999;
        low.limits.stream_max_gib = 0;
        low.limits.cache_max_gib = 2000;
        low.limits.ratio_max = 1_000_001;
        low.limits.nesting_max = 33;
        low.limits.paths_max_mib = 0;
        low.limits.depth_max = 15;
        let keys: Vec<_> = low.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(
            keys,
            [
                "limits.cache_max_gib",
                "limits.depth_max",
                "limits.entries_max",
                "limits.nesting_max",
                "limits.paths_max_mib",
                "limits.ratio_max",
                "limits.stream_max_gib"
            ]
        );
        low.limits.ratio_max = 0;
        assert_eq!(low.limits().ratio_max, 0);
    }
}

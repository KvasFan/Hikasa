//! [`ResolvedRules`]: the rules compiled for one scan (§3.3 step b): the name filter compiled with the resolved
//! case sensitivity, the date cutoff made absolute with the injected "now" (D-24), and the filter predicates
//! shared by the scanner and the invariant checks (INV-FOLDER-7).

use hikasa_core::{
    CaseSensitivity, FileTime, FolderVerdict, MaskOptions, NameFilter, RelPath, Side,
};
use hikasa_vfs::{Attrs, Capabilities, DosAttrs, EntryKind};

use crate::error::FolderError;
use crate::rules::{
    CaseMode, Cmp, DateCutoff, DateFilter, FolderRules, OtherFilters, SizeFilter, file_time,
};

/// Rules compiled for one scan.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedRules {
    /// The name filter (one for both sides: same lists, same case).
    pub(crate) filter: NameFilter,
    /// True when the name filter cannot exclude anything (no prefilter is passed).
    pub(crate) names_match_all: bool,
    /// Exclude SYSTEM+HIDDEN items.
    pub(crate) exclude_protected: bool,
    /// Size filter.
    pub(crate) size: Option<SizeFilter>,
    /// Date filter with its absolute cutoff.
    pub(crate) date: Option<(Cmp, FileTime)>,
}

/// `CaseMode::Auto` → sensitive iff both bases' back-ends report `CASE_SENSITIVE`.
pub(crate) fn resolve_case(mode: CaseMode, caps: [Capabilities; 2]) -> CaseSensitivity {
    match mode {
        CaseMode::Sensitive => CaseSensitivity::Sensitive,
        CaseMode::Insensitive => CaseSensitivity::Insensitive,
        CaseMode::Auto => {
            if caps
                .iter()
                .all(|c| c.contains(Capabilities::CASE_SENSITIVE))
            {
                CaseSensitivity::Sensitive
            } else {
                CaseSensitivity::Insensitive
            }
        }
    }
}

/// `DaysAgo{n}`: local midnight of today minus `n` days in `now`'s zone. A date before the representable range
/// resolves to the earliest representable midnight (every file is then newer).
pub(crate) fn days_ago_cutoff(now: &jiff::Zoned, days: u32) -> FileTime {
    let tz = now.time_zone().clone();
    let earliest = || {
        jiff::civil::Date::MIN
            .to_zoned(tz.clone())
            .map(|z| file_time(z.timestamp()))
            .unwrap_or_else(|_| file_time(jiff::Timestamp::MIN))
    };
    let Ok(span) = jiff::Span::new().try_days(i64::from(days)) else {
        return earliest();
    };
    match now.date().checked_sub(span) {
        Ok(date) => match date.to_zoned(tz.clone()) {
            Ok(midnight) => file_time(midnight.timestamp()),
            Err(_) => earliest(),
        },
        Err(_) => earliest(),
    }
}

/// The absolute cutoff of a date filter.
pub(crate) fn absolute_cutoff(filter: &DateFilter, now: &jiff::Zoned) -> FileTime {
    match filter.cutoff {
        DateCutoff::Absolute { at } => at,
        DateCutoff::DaysAgo { days } => days_ago_cutoff(now, days),
    }
}

impl ResolvedRules {
    /// Compiles `rules` for a scan with case `case` and "now" `now`.
    pub(crate) fn new(
        rules: &FolderRules,
        case: CaseSensitivity,
        now: &jiff::Zoned,
    ) -> Result<ResolvedRules, FolderError> {
        let filter = NameFilter::from_lists(&rules.filters.names, MaskOptions { case })?;
        let OtherFilters {
            exclude_protected,
            size,
            date,
        } = rules.filters.other;
        Ok(ResolvedRules {
            names_match_all: filter.is_match_all(),
            filter,
            exclude_protected,
            size,
            date: date.map(|d| (d.cmp, absolute_cutoff(&d, now))),
        })
    }

    /// Verdict of the folder named `name` inside `parent` (that side's path) whose verdict is `parent_verdict`.
    pub(crate) fn folder_verdict(
        &self,
        parent: &RelPath,
        name: &str,
        parent_verdict: FolderVerdict,
    ) -> FolderVerdict {
        if self.names_match_all {
            return if parent_verdict == FolderVerdict::Excluded {
                FolderVerdict::Excluded
            } else {
                FolderVerdict::Included
            };
        }
        self.filter.folder(parent, name, parent_verdict)
    }

    /// True when the name masks keep the entry (`Dir` → its folder verdict is not `Excluded`; any other kind →
    /// the file masks). This is the prefilter of D-45.
    pub(crate) fn names_accept(
        &self,
        parent: &RelPath,
        parent_verdict: FolderVerdict,
        name: &str,
        kind: EntryKind,
    ) -> bool {
        if self.names_match_all {
            return !matches!(
                parent_verdict,
                FolderVerdict::Excluded | FolderVerdict::PassThrough
            ) || kind == EntryKind::Dir && parent_verdict != FolderVerdict::Excluded;
        }
        match kind {
            EntryKind::Dir => {
                self.filter.folder(parent, name, parent_verdict) != FolderVerdict::Excluded
            }
            EntryKind::File | EntryKind::Symlink | EntryKind::Other => {
                self.filter.file(parent, name, parent_verdict)
            }
        }
    }

    /// True when the other filters (FC-053) hide a stat'ed entry: SYSTEM+HIDDEN with `exclude_protected` (any
    /// kind); for regular files, the size filter and the date filter (a missing mtime fails it).
    pub(crate) fn other_filtered(
        &self,
        kind: EntryKind,
        size: u64,
        mtime: Option<FileTime>,
        attrs: &Attrs,
    ) -> bool {
        if self.exclude_protected
            && let Attrs::Dos { attrs } = attrs
            && attrs.contains(DosAttrs::SYSTEM | DosAttrs::HIDDEN)
        {
            return true;
        }
        if kind != EntryKind::File {
            return false;
        }
        if let Some(filter) = self.size
            && !filter.cmp.holds(size, filter.bytes)
        {
            return true;
        }
        if let Some((cmp, cutoff)) = self.date {
            return match mtime {
                Some(mtime) => !cmp.holds(mtime, cutoff),
                None => true,
            };
        }
        false
    }

    /// True when a name filter can decline entries (a prefilter is worth passing).
    pub(crate) fn has_name_filter(&self) -> bool {
        !self.names_match_all
    }
}

/// The verdict chain of a `/`-separated folder path on one side, from the base (`Base`) down.
pub(crate) fn path_verdict(rules: &ResolvedRules, dir: &str) -> (RelPath, FolderVerdict) {
    let mut path = RelPath::root();
    let mut verdict = FolderVerdict::Base;
    for name in dir.split('/').filter(|n| !n.is_empty()) {
        verdict = rules.folder_verdict(&path, name, verdict);
        path = path.join(name).unwrap_or(path);
    }
    (path, verdict)
}

/// Index of a side in per-side arrays (Left 0, Right 1; Center/Output map to 0 and are never used).
pub(crate) const fn side_index(side: Side) -> usize {
    match side {
        Side::Right => 1,
        Side::Left | Side::Center | Side::Output => 0,
    }
}

/// The side of a per-side array index.
pub(crate) const fn side_of(index: usize) -> Side {
    if index == 0 { Side::Left } else { Side::Right }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::FolderRules;
    use hikasa_vfs::UnixMeta;

    /// 2026-10-20T15:30:00+02:00 in a central European zone built from POSIX rules (no system tz database, T1).
    fn now() -> jiff::Zoned {
        let cet = jiff::tz::TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        let now = "2026-10-20T13:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(cet);
        assert_eq!(now.offset(), jiff::tz::offset(2), "CEST on 2026-10-20");
        now
    }

    #[test]
    fn auto_case() {
        let s = Capabilities::CASE_SENSITIVE;
        let i = Capabilities::empty();
        assert_eq!(
            resolve_case(CaseMode::Auto, [s, s]),
            CaseSensitivity::Sensitive
        );
        assert_eq!(
            resolve_case(CaseMode::Auto, [s, i]),
            CaseSensitivity::Insensitive
        );
        assert_eq!(
            resolve_case(CaseMode::Sensitive, [i, i]),
            CaseSensitivity::Sensitive
        );
        assert_eq!(
            resolve_case(CaseMode::Insensitive, [s, s]),
            CaseSensitivity::Insensitive
        );
    }

    #[test]
    fn days_ago_is_relative_to_local_midnight() {
        let n = now();
        assert_eq!(
            days_ago_cutoff(&n, 0),
            FileTime::parse("2026-10-19T22:00:00Z").unwrap()
        );
        assert_eq!(
            days_ago_cutoff(&n, 30),
            FileTime::parse("2026-09-19T22:00:00Z").unwrap()
        );
        // Far in the past: the earliest representable midnight, no panic.
        let far = days_ago_cutoff(&n, u32::MAX);
        assert!(far < FileTime::parse("0001-01-01T00:00:00Z").unwrap());
    }

    #[test]
    fn other_filters() {
        let mut rules = FolderRules::default();
        rules.filters.other.size = Some(SizeFilter {
            cmp: Cmp::Less,
            bytes: 10,
        });
        rules.filters.other.date = Some(DateFilter {
            cmp: Cmp::Greater,
            cutoff: DateCutoff::DaysAgo { days: 1 },
        });
        let r = ResolvedRules::new(&rules, CaseSensitivity::Sensitive, &now()).unwrap();
        let recent = Some(FileTime::parse("2026-10-20T00:00:00Z").unwrap());
        let old = Some(FileTime::parse("2026-01-01T00:00:00Z").unwrap());
        let none = Attrs::None;
        assert!(!r.other_filtered(EntryKind::File, 5, recent, &none));
        assert!(
            r.other_filtered(EntryKind::File, 10, recent, &none),
            "not < 10"
        );
        assert!(r.other_filtered(EntryKind::File, 5, old, &none));
        assert!(
            r.other_filtered(EntryKind::File, 5, None, &none),
            "missing mtime"
        );
        assert!(
            !r.other_filtered(EntryKind::Dir, 50, old, &none),
            "files only"
        );
        assert!(
            !r.other_filtered(EntryKind::Symlink, 50, old, &none),
            "files only"
        );
        let protected = Attrs::Dos {
            attrs: DosAttrs::SYSTEM | DosAttrs::HIDDEN,
        };
        assert!(r.other_filtered(EntryKind::Dir, 0, None, &protected));
        let hidden = Attrs::Dos {
            attrs: DosAttrs::HIDDEN,
        };
        assert!(!r.other_filtered(EntryKind::Dir, 0, None, &hidden));
        let unix = Attrs::Unix {
            meta: UnixMeta {
                mode: 0,
                uid: 0,
                gid: 0,
            },
        };
        assert!(!r.other_filtered(EntryKind::Dir, 0, None, &unix));
        rules.filters.other.exclude_protected = false;
        let r = ResolvedRules::new(&rules, CaseSensitivity::Sensitive, &now()).unwrap();
        assert!(!r.other_filtered(EntryKind::Dir, 0, None, &protected));
    }

    #[test]
    fn name_predicates_match_the_filter() {
        let mut rules = FolderRules::default();
        let root = RelPath::root();
        let all = ResolvedRules::new(&rules, CaseSensitivity::Sensitive, &now()).unwrap();
        assert!(!all.has_name_filter());
        for v in [
            FolderVerdict::Base,
            FolderVerdict::Included,
            FolderVerdict::PassThrough,
            FolderVerdict::Excluded,
        ] {
            for kind in EntryKind::ALL {
                let expected = match kind {
                    EntryKind::Dir => all.filter.folder(&root, "x", v) != FolderVerdict::Excluded,
                    _ => all.filter.file(&root, "x", v),
                };
                assert_eq!(
                    all.names_accept(&root, v, "x", kind),
                    expected,
                    "{v:?} {kind:?}"
                );
            }
            assert_eq!(
                all.folder_verdict(&root, "x", v),
                all.filter.folder(&root, "x", v)
            );
        }
        rules.filters.names = NameFilter::parse(
            r"*.c;-obj\",
            MaskOptions {
                case: CaseSensitivity::Sensitive,
            },
        )
        .unwrap()
        .to_lists();
        let r = ResolvedRules::new(&rules, CaseSensitivity::Sensitive, &now()).unwrap();
        assert!(r.has_name_filter());
        assert!(r.names_accept(&root, FolderVerdict::Base, "a.c", EntryKind::File));
        assert!(!r.names_accept(&root, FolderVerdict::Base, "a.h", EntryKind::File));
        assert!(!r.names_accept(&root, FolderVerdict::Base, "obj", EntryKind::Dir));
        assert!(r.names_accept(&root, FolderVerdict::Base, "src", EntryKind::Dir));
        let (path, verdict) = path_verdict(&r, "src/obj/x");
        assert_eq!(path.as_str(), "src/obj/x");
        assert_eq!(verdict, FolderVerdict::Excluded);
        assert_eq!(path_verdict(&r, "").1, FolderVerdict::Base);
        assert_eq!(side_of(side_index(Side::Right)), Side::Right);
        assert_eq!(side_of(side_index(Side::Left)), Side::Left);
    }
}

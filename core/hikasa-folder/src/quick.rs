//! Quick tests (FC-020…FC-024; algorithm §2.6.1): pure, per pair, no file content needed.

use hikasa_core::case::eq_folded;
use hikasa_vfs::{Attrs, Entry, EntryKind};

use crate::align::folded_exact;
use crate::rules::Criteria;
use crate::time_cmp::{TimeRelation, compare_times};

bitflags::bitflags! {
    /// Which quick or content tests found a difference (detail for columns, JSON, reports).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct DiffFlags: u16 {
        /// File sizes differ (FC-020).
        const SIZE = 1;
        /// Modification times differ (FC-021).
        const TIME = 2;
        /// Names differ only in case (FC-022).
        const NAME_CASE = 4;
        /// Selected DOS attributes differ (FC-023).
        const ATTRIBUTES = 8;
        /// Unix permission bits differ (FC-024).
        const PERMISSIONS = 16;
        /// Owners differ (FC-024).
        const OWNER = 32;
        /// Groups differ (FC-024).
        const GROUP = 64;
        /// The content test found a difference (FC-025).
        const CONTENT = 128;
        /// Unfollowed symlinks point to different targets (exact OS targets).
        const LINK_TARGET = 256;
    }
}

/// JSON names of the diff flags in bit order (§4.1 rule 9).
const DIFF_NAMES: [(DiffFlags, &str); 9] = [
    (DiffFlags::SIZE, "size"),
    (DiffFlags::TIME, "time"),
    (DiffFlags::NAME_CASE, "name_case"),
    (DiffFlags::ATTRIBUTES, "attributes"),
    (DiffFlags::PERMISSIONS, "permissions"),
    (DiffFlags::OWNER, "owner"),
    (DiffFlags::GROUP, "group"),
    (DiffFlags::CONTENT, "content"),
    (DiffFlags::LINK_TARGET, "link_target"),
];

impl DiffFlags {
    /// The set flags as snake_case names in bit order (`["size", "time"]`), the JSON form of bit sets (§4.1
    /// rule 9). Additive API.
    pub fn names(self) -> Vec<&'static str> {
        DIFF_NAMES
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect()
    }
}

/// The two (different) exact names are equal after case folding. A name that is not valid Unicode is folded on
/// its exact OS form, never through its lossy text (§2.5.1), so two different non-UTF-8 names do not "differ only
/// in case".
fn differ_in_case_only(left: &Entry, right: &Entry) -> bool {
    if left.name.is_lossy() || right.name.is_lossy() {
        return folded_exact(&left.name) == folded_exact(&right.name);
    }
    eq_folded(left.name.as_str(), right.name.as_str())
}

/// Runs the quick tests on two present, stat'ed entries of the same kind (§2.6.1); name-only entries skip
/// every quick test, so the caller does not call this for them. Returns the differences found and the
/// timestamp relation (`None` when timestamps are not compared: the criterion is off or a time is missing).
///
/// A criterion whose data is missing on either side (no DOS attributes, no Unix metadata) is skipped; the
/// scanner reports the missing capability once per (criterion, side).
pub fn quick_tests(
    left: &Entry,
    right: &Entry,
    criteria: &Criteria,
) -> (DiffFlags, Option<TimeRelation>) {
    let mut diffs = DiffFlags::empty();
    let mut time = None;
    if criteria.size && left.kind == EntryKind::File && left.size != right.size {
        diffs |= DiffFlags::SIZE;
    }
    if criteria.timestamps.enabled
        && let (Some(l), Some(r)) = (left.mtime, right.mtime)
    {
        let relation = compare_times(l, r, &criteria.timestamps);
        if relation != TimeRelation::Same {
            diffs |= DiffFlags::TIME;
        }
        time = Some(relation);
    }
    if criteria.filename_case && left.name != right.name && differ_in_case_only(left, right) {
        diffs |= DiffFlags::NAME_CASE;
    }
    match (left.attrs, right.attrs) {
        (Attrs::Dos { attrs: l }, Attrs::Dos { attrs: r }) => {
            if !criteria.dos_attributes.is_empty()
                && !((l ^ r) & criteria.dos_attributes).is_empty()
            {
                diffs |= DiffFlags::ATTRIBUTES;
            }
        }
        (Attrs::Unix { meta: l }, Attrs::Unix { meta: r }) => {
            let unix = criteria.unix;
            if unix.permissions && l.mode != r.mode {
                diffs |= DiffFlags::PERMISSIONS;
            }
            if unix.owner && l.uid != r.uid {
                diffs |= DiffFlags::OWNER;
            }
            if unix.group && l.gid != r.gid {
                diffs |= DiffFlags::GROUP;
            }
        }
        _ => {}
    }
    if left.kind == EntryKind::Symlink
        && right.kind == EntryKind::Symlink
        && !left.followed
        && !right.followed
        && left.link_target_os() != right.link_target_os()
    {
        diffs |= DiffFlags::LINK_TARGET;
    }
    (diffs, time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{TimestampRule, UnixCriteria};
    use hikasa_core::FileTime;
    use hikasa_vfs::{DosAttrs, FileName, UnixMeta};

    fn file(name: &str, size: u64, mtime: Option<i64>) -> Entry {
        let mut e = Entry::new(FileName::new(name).unwrap(), EntryKind::File);
        e.size = size;
        e.mtime = mtime.map(|s| FileTime::from_unix(s, 0).unwrap());
        e
    }

    #[test]
    fn size_time_and_case() {
        let c = Criteria::default();
        let (d, t) = quick_tests(&file("a", 1, Some(10)), &file("a", 1, Some(11)), &c);
        assert_eq!((d, t), (DiffFlags::empty(), Some(TimeRelation::Same)));
        let (d, t) = quick_tests(&file("a", 1, Some(10)), &file("a", 2, Some(20)), &c);
        assert_eq!(d, DiffFlags::SIZE | DiffFlags::TIME);
        assert_eq!(t, Some(TimeRelation::RightNewer));
        // §2.4 row 13: disabled → not compared.
        let mut off = c.clone();
        off.timestamps = TimestampRule {
            enabled: false,
            ..TimestampRule::default()
        };
        assert_eq!(
            quick_tests(
                &file("a", 1, Some(12 * 3600)),
                &file("a", 1, Some(18 * 3600)),
                &off
            ),
            (DiffFlags::empty(), None)
        );
        // Missing mtime: not compared.
        assert_eq!(
            quick_tests(&file("a", 1, None), &file("a", 1, Some(20)), &c),
            (DiffFlags::empty(), None)
        );
        // Size applies to files only.
        let mut d1 = file("d", 0, None);
        d1.kind = EntryKind::Dir;
        let mut d2 = d1.clone();
        d2.size = 9;
        assert_eq!(quick_tests(&d1, &d2, &c).0, DiffFlags::empty());
        // Name case.
        let mut cased = c.clone();
        cased.filename_case = true;
        assert_eq!(
            quick_tests(&file("README", 1, None), &file("readme", 1, None), &cased).0,
            DiffFlags::NAME_CASE
        );
        assert_eq!(
            quick_tests(&file("README", 1, None), &file("readme", 1, None), &c).0,
            DiffFlags::empty()
        );
        assert_eq!(
            quick_tests(&file("x.txt", 1, None), &file("x.csv", 1, None), &cased).0,
            DiffFlags::empty(),
            "different names that are not case variants"
        );
    }

    #[test]
    fn attributes_and_unix() {
        let mut c = Criteria::default();
        let mut l = file("a", 1, None);
        let mut r = file("a", 1, None);
        l.attrs = Attrs::Dos {
            attrs: DosAttrs::HIDDEN | DosAttrs::ARCHIVE,
        };
        r.attrs = Attrs::Dos {
            attrs: DosAttrs::ARCHIVE,
        };
        assert_eq!(
            quick_tests(&l, &r, &c).0,
            DiffFlags::empty(),
            "off by default"
        );
        c.dos_attributes = DosAttrs::READONLY;
        assert_eq!(
            quick_tests(&l, &r, &c).0,
            DiffFlags::empty(),
            "not in the subset"
        );
        c.dos_attributes = DosAttrs::HIDDEN | DosAttrs::READONLY;
        assert_eq!(quick_tests(&l, &r, &c).0, DiffFlags::ATTRIBUTES);
        l.attrs = Attrs::Unix {
            meta: UnixMeta {
                mode: 0o644,
                uid: 1,
                gid: 2,
            },
        };
        assert_eq!(
            quick_tests(&l, &r, &c).0,
            DiffFlags::empty(),
            "mixed attrs: skipped"
        );
        r.attrs = Attrs::Unix {
            meta: UnixMeta {
                mode: 0o600,
                uid: 3,
                gid: 4,
            },
        };
        assert_eq!(quick_tests(&l, &r, &c).0, DiffFlags::empty());
        c.unix = UnixCriteria {
            permissions: true,
            owner: true,
            group: true,
        };
        assert_eq!(
            quick_tests(&l, &r, &c).0,
            DiffFlags::PERMISSIONS | DiffFlags::OWNER | DiffFlags::GROUP
        );
    }

    #[test]
    fn link_targets_use_the_exact_target() {
        let c = Criteria::default();
        let link = |target: &str| {
            let mut e = Entry::new(FileName::new("l").unwrap(), EntryKind::Symlink);
            e.link_target = Some(target.into());
            e.size = target.len() as u64;
            e
        };
        assert_eq!(
            quick_tests(&link("a"), &link("a"), &c).0,
            DiffFlags::empty()
        );
        assert_eq!(
            quick_tests(&link("a"), &link("b"), &c).0,
            DiffFlags::LINK_TARGET
        );
        // Same lossy text, different raw bytes.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let mut l = link("\u{FFFD}");
            let mut r = link("\u{FFFD}");
            l.link_target_raw = Some(std::ffi::OsStr::from_bytes(b"\xff").to_os_string());
            r.link_target_raw = Some(std::ffi::OsStr::from_bytes(b"\xfe").to_os_string());
            assert_eq!(quick_tests(&l, &r, &c).0, DiffFlags::LINK_TARGET);
        }
        // Followed links are compared as their targets.
        let mut a = link("x");
        let mut b = link("y");
        a.followed = true;
        b.followed = true;
        a.kind = EntryKind::File;
        b.kind = EntryKind::File;
        assert_eq!(quick_tests(&a, &b, &c).0, DiffFlags::empty());
    }

    #[test]
    fn names_in_bit_order() {
        assert_eq!(
            (DiffFlags::CONTENT | DiffFlags::SIZE | DiffFlags::LINK_TARGET).names(),
            ["size", "content", "link_target"]
        );
        assert_eq!(DiffFlags::all().names().len(), 9);
    }
}

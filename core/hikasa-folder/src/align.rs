//! Name alignment (FC-010, FC-013, FC-014; algorithm §2.5; D-20): [`align_names`] pairs the entries of one
//! folder pair, [`align_flat`] the files of two flattened trees; both return the pairs in canonical child order
//! (§2.5.4).

use std::borrow::Cow;
use std::cmp::Ordering;

use hikasa_core::case::{fold, nfc};
use hikasa_core::{Cancelled, CaseSensitivity, Pair};
use hikasa_vfs::{EntryKind, FileName};

/// What the aligner needs of one entry. Additive API (the contract names the functions, not their signatures).
#[derive(Debug, Clone, Copy)]
pub struct AlignInput<'a> {
    /// The entry name (exact OS name and its UTF-8 text).
    pub name: &'a FileName,
    /// Kind after symlink resolution (`Symlink` only for unfollowed links). Kinds never align with each other
    /// (D-20).
    pub kind: EntryKind,
    /// Flatten mode: the containing folder relative to the base (`""` = the base). Ignored by
    /// [`align_names`].
    pub dir: &'a str,
}

/// Alignment options (FC-010): the resolved case sensitivity and the two "align different …" options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlignOptions {
    /// Case sensitivity of names (after resolving `CaseMode::Auto`).
    pub case: CaseSensitivity,
    /// Align `x.txt` with `x.csv` (files only).
    pub align_extensions: bool,
    /// Align NFC with NFD names.
    pub align_normalization: bool,
}

/// "No index" marker of the internal index pairs.
pub(crate) const NO: u32 = u32::MAX;

/// Entries handled between two cancellation checks (§5.3: bounded work between checks).
const CHECK_EVERY: usize = 4096;

/// Kind rank (§2.5.4): Folder, File, Symlink, Other.
pub(crate) fn kind_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Dir => 0,
        EntryKind::File => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    }
}

/// Exact OS bytes of a name (borrowed unless the UTF-8 text is lossy).
pub(crate) fn exact_bytes(name: &FileName) -> Cow<'_, [u8]> {
    if name.is_lossy() {
        Cow::Owned(name.to_os_string().into_encoded_bytes())
    } else {
        Cow::Borrowed(name.as_str().as_bytes())
    }
}

/// The case-folded **exact** name (`k2` when case-insensitive, §2.5.1): the simple lower case of every valid
/// UTF-8 run of the exact OS bytes, with the invalid bytes kept as they are. A valid name folds its text; a lossy
/// name is never folded through its lossy text, so two different non-UTF-8 names (`a\xff`, `a\xfe`, both shown as
/// `a\u{FFFD}`) never get the same key. Borrows when nothing changes.
pub(crate) fn folded_exact(name: &FileName) -> Cow<'_, [u8]> {
    if !name.is_lossy() {
        return match fold(name.as_str()) {
            Cow::Borrowed(text) => Cow::Borrowed(text.as_bytes()),
            Cow::Owned(text) => Cow::Owned(text.into_bytes()),
        };
    }
    let raw = name.to_os_string().into_encoded_bytes();
    let mut out = Vec::with_capacity(raw.len());
    for chunk in raw.utf8_chunks() {
        out.extend_from_slice(fold(chunk.valid()).as_bytes());
        out.extend_from_slice(chunk.invalid());
    }
    Cow::Owned(out)
}

/// `fold(nfc(text))`, the primary key of the canonical child order (§2.5.4); borrows when nothing changes.
pub(crate) fn canonical_fold(text: &str) -> Cow<'_, str> {
    match nfc(text) {
        Cow::Borrowed(normal) => fold(normal),
        Cow::Owned(normal) => Cow::Owned(fold(&normal).into_owned()),
    }
}

/// `FileName::stem` semantics on bytes: the bytes before the last `.` when that `.` is neither the first nor the
/// last byte.
fn stem(s: &[u8]) -> &[u8] {
    match s.iter().rposition(|&b| b == b'.') {
        Some(dot) if dot > 0 && dot + 1 < s.len() => s.get(..dot).unwrap_or(s),
        _ => s,
    }
}

/// `k2`: the case-folded exact name when case-insensitive, else the exact name.
fn k2<'a>(input: &AlignInput<'a>, opts: &AlignOptions) -> Cow<'a, [u8]> {
    match opts.case {
        CaseSensitivity::Insensitive => folded_exact(input.name),
        CaseSensitivity::Sensitive => exact_bytes(input.name),
    }
}

/// `k3`: NFC of `k2` with `align_normalization`. A lossy name is not normalised (its key stays `k2`).
fn k3<'a>(input: &AlignInput<'a>, opts: &AlignOptions) -> Cow<'a, [u8]> {
    let key = k2(input, opts);
    if !opts.align_normalization || input.name.is_lossy() {
        return key;
    }
    match key {
        Cow::Borrowed(bytes) => match std::str::from_utf8(bytes).map(nfc) {
            Ok(Cow::Owned(normal)) => Cow::Owned(normal.into_bytes()),
            Ok(Cow::Borrowed(_)) | Err(_) => Cow::Borrowed(bytes),
        },
        Cow::Owned(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Cow::Owned(nfc(&text).into_owned().into_bytes()),
            Err(e) => Cow::Owned(e.into_bytes()),
        },
    }
}

/// `k4`: the stem of `k3` for files with `align_extensions` (a lossy name keeps `k3`).
fn k4<'a>(input: &AlignInput<'a>, opts: &AlignOptions) -> Cow<'a, [u8]> {
    let key = k3(input, opts);
    if input.kind != EntryKind::File || input.name.is_lossy() {
        return key;
    }
    match key {
        Cow::Borrowed(bytes) => Cow::Borrowed(stem(bytes)),
        Cow::Owned(bytes) => Cow::Owned(stem(&bytes).to_vec()),
    }
}

/// The tie-breaker inside a key group: `(dir, k1)` (§2.5.2, §2.5.3).
fn tie(a: &AlignInput<'_>, b: &AlignInput<'_>, flat: bool) -> Ordering {
    let dirs = if flat {
        a.dir.cmp(b.dir)
    } else {
        Ordering::Equal
    };
    dirs.then_with(|| exact_bytes(a.name).cmp(&exact_bytes(b.name)))
}

/// One level of §2.5.2: pairs the still unpaired entries whose `(class, key)` agree; within a group both lists are
/// sorted by `(dir, k1)` and paired by position. Paired indices leave `free`. `key` returns `None` for entries that
/// do not take part in the level.
fn level<'a, K: Ord>(
    inputs: [&'a [AlignInput<'a>]; 2],
    free: &mut [Vec<u32>; 2],
    pairs: &mut Vec<[u32; 2]>,
    flat: bool,
    key: impl Fn(&AlignInput<'a>) -> Option<K>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Cancelled> {
    if free[0].is_empty() || free[1].is_empty() {
        return Ok(());
    }
    let keyed = |side: usize| -> Result<Vec<(u8, K, u32)>, Cancelled> {
        let mut out = Vec::with_capacity(free[side].len());
        for (n, &i) in free[side].iter().enumerate() {
            if n % CHECK_EVERY == 0 && cancelled() {
                return Err(Cancelled);
            }
            if let Some(input) = inputs[side].get(i as usize)
                && let Some(k) = key(input)
            {
                out.push((kind_rank(input.kind), k, i));
            }
        }
        out.sort_by(|a, b| {
            (a.0, &a.1).cmp(&(b.0, &b.1)).then_with(|| {
                match (
                    inputs[side].get(a.2 as usize),
                    inputs[side].get(b.2 as usize),
                ) {
                    (Some(x), Some(y)) => tie(x, y, flat),
                    _ => Ordering::Equal,
                }
            })
        });
        Ok(out)
    };
    let l = keyed(0)?;
    let r = keyed(1)?;
    if cancelled() {
        return Err(Cancelled);
    }
    let mut paired = [vec![false; inputs[0].len()], vec![false; inputs[1].len()]];
    let (mut i, mut j) = (0, 0);
    while let (Some(a), Some(b)) = (l.get(i), r.get(j)) {
        match (a.0, &a.1).cmp(&(b.0, &b.1)) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                let same = |x: &(u8, K, u32)| x.0 == a.0 && x.1 == a.1;
                let i_end = i + l
                    .get(i..)
                    .map_or(0, |s| s.iter().take_while(|x| same(x)).count());
                let j_end = j + r
                    .get(j..)
                    .map_or(0, |s| s.iter().take_while(|x| same(x)).count());
                let group = (i_end - i).min(j_end - j);
                for k in 0..group {
                    if let (Some(x), Some(y)) = (l.get(i + k), r.get(j + k)) {
                        pairs.push([x.2, y.2]);
                        if let Some(flag) = paired[0].get_mut(x.2 as usize) {
                            *flag = true;
                        }
                        if let Some(flag) = paired[1].get_mut(y.2 as usize) {
                            *flag = true;
                        }
                    }
                }
                i = i_end;
                j = j_end;
            }
        }
    }
    for (side, list) in free.iter_mut().enumerate() {
        list.retain(|&k| !paired[side].get(k as usize).copied().unwrap_or(false));
    }
    Ok(())
}

/// The levels of §2.5.2 (with L0 first in flatten mode, §2.5.3) without the canonical sort: index pairs
/// `[left, right]` (`NO` = absent), paired entries first, then the leftovers of each side. `cancelled` is polled
/// between levels and every few thousand entries, so a huge folder never delays cancellation (§5.3).
pub(crate) fn align_pairs(
    left: &[AlignInput<'_>],
    right: &[AlignInput<'_>],
    opts: &AlignOptions,
    flat: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<[u32; 2]>, Cancelled> {
    let count = |len: usize| u32::try_from(len).unwrap_or(NO);
    let mut free = [
        (0..count(left.len())).collect::<Vec<u32>>(),
        (0..count(right.len())).collect::<Vec<u32>>(),
    ];
    let mut pairs = Vec::with_capacity(left.len().max(right.len()));
    let inputs = [left, right];
    if flat {
        // L0: exact (dir, k1).
        level(
            inputs,
            &mut free,
            &mut pairs,
            flat,
            |p| Some((p.dir, exact_bytes(p.name))),
            cancelled,
        )?;
    }
    // L1: exact OS name.
    level(
        inputs,
        &mut free,
        &mut pairs,
        flat,
        |p| Some(exact_bytes(p.name)),
        cancelled,
    )?;
    if opts.case == CaseSensitivity::Insensitive {
        level(
            inputs,
            &mut free,
            &mut pairs,
            flat,
            |p| Some(folded_exact(p.name)),
            cancelled,
        )?;
    }
    if opts.align_normalization {
        level(
            inputs,
            &mut free,
            &mut pairs,
            flat,
            |p| Some(k3(p, opts)),
            cancelled,
        )?;
    }
    if opts.align_extensions {
        level(
            inputs,
            &mut free,
            &mut pairs,
            flat,
            |p| Some(k4(p, opts)),
            cancelled,
        )?;
    }
    let [lfree, rfree] = free;
    pairs.extend(lfree.into_iter().map(|i| [i, NO]));
    pairs.extend(rfree.into_iter().map(|j| [NO, j]));
    Ok(pairs)
}

fn align_impl(
    left: &[AlignInput<'_>],
    right: &[AlignInput<'_>],
    opts: &AlignOptions,
    flat: bool,
) -> Vec<Pair<Option<usize>>> {
    let index = |i: u32| (i != NO).then_some(i as usize);
    let mut pairs: Vec<Pair<Option<usize>>> = align_pairs(left, right, opts, flat, &|| false)
        .unwrap_or_default()
        .into_iter()
        .map(|[l, r]| Pair::new(index(l), index(r)))
        .collect();
    sort_canonical(&mut pairs, left, right, flat);
    pairs
}

/// Sort key of a child in canonical order (§2.5.4): kind rank; `fold(nfc(representative name))`; the
/// representative's exact bytes; the right name's exact bytes; then (flatten only) the representative's and the
/// right side's folders. The representative is the left entry when present. Shared by the aligner, the scanner
/// and the INV-FOLDER-3 check, so that all three agree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ChildKey {
    class: u8,
    folded: String,
    repr: Vec<u8>,
    right: Option<Vec<u8>>,
    repr_dir: String,
    right_dir: Option<String>,
}

impl ChildKey {
    /// The key of a child of kind rank `class` whose representative has UTF-8 text `repr_text`, exact bytes
    /// `repr_exact` and folder `repr_dir`, and whose right entry (if any) has exact bytes and folder `right`.
    pub(crate) fn new(
        class: u8,
        repr_text: &str,
        repr_exact: &[u8],
        repr_dir: &str,
        right: Option<(&[u8], &str)>,
    ) -> ChildKey {
        ChildKey {
            class,
            folded: canonical_fold(repr_text).into_owned(),
            repr: repr_exact.to_vec(),
            right: right.map(|(exact, _)| exact.to_vec()),
            repr_dir: repr_dir.to_owned(),
            right_dir: right.map(|(_, dir)| dir.to_owned()),
        }
    }
}

/// Sorts pairs into canonical child order.
fn sort_canonical(
    pairs: &mut [Pair<Option<usize>>],
    left: &[AlignInput<'_>],
    right: &[AlignInput<'_>],
    flat: bool,
) {
    let dir = |p: &AlignInput<'_>| {
        if flat {
            p.dir.to_owned()
        } else {
            String::new()
        }
    };
    let mut keyed: Vec<(ChildKey, Pair<Option<usize>>)> = pairs
        .iter()
        .map(|pair| {
            let l = pair.left.and_then(|i| left.get(i));
            let r = pair.right.and_then(|j| right.get(j));
            let key = match l.or(r) {
                Some(repr) => {
                    let right_exact = r.map(|p| (exact_bytes(p.name), dir(p)));
                    ChildKey::new(
                        kind_rank(repr.kind),
                        repr.name.as_str(),
                        &exact_bytes(repr.name),
                        &dir(repr),
                        right_exact.as_ref().map(|(e, d)| (&e[..], d.as_str())),
                    )
                }
                None => ChildKey::new(0, "", &[], "", None),
            };
            (key, *pair)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    for (slot, (_, pair)) in pairs.iter_mut().zip(keyed) {
        *slot = pair;
    }
}

/// Aligns the entries of one folder pair (§2.5.2): levels L1 (exact name), L2 (case-folded, when
/// case-insensitive), L3 (NFC, with `align_normalization`), L4 (file stems, with `align_extensions`). Entries of
/// different kinds never align (D-20). A name that is not valid Unicode is folded on its exact OS form and takes no
/// part in L3/L4 beyond its folded key. Returns index pairs (`left[i]`, `right[j]`; `None` = orphan) in canonical
/// child order (§2.5.4).
pub fn align_names(
    left: &[AlignInput<'_>],
    right: &[AlignInput<'_>],
    opts: &AlignOptions,
) -> Vec<Pair<Option<usize>>> {
    align_impl(left, right, opts, false)
}

/// Aligns two flattened file lists (FC-014, §2.5.3): level L0 pairs exact `(dir, name)` first, then L1–L4 use
/// names only; within a group entries are sorted by `(dir, name)`. Canonical order as [`align_names`], with the
/// folders as final tie-breakers.
pub fn align_flat(
    left: &[AlignInput<'_>],
    right: &[AlignInput<'_>],
    opts: &AlignOptions,
) -> Vec<Pair<Option<usize>>> {
    align_impl(left, right, opts, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[(&str, EntryKind)]) -> Vec<FileName> {
        list.iter()
            .map(|(n, _)| FileName::new(n).unwrap())
            .collect()
    }

    fn run(
        left: &[(&str, EntryKind)],
        right: &[(&str, EntryKind)],
        opts: AlignOptions,
    ) -> Vec<(Option<String>, Option<String>)> {
        let ln = names(left);
        let rn = names(right);
        let li: Vec<AlignInput<'_>> = ln
            .iter()
            .zip(left)
            .map(|(name, (_, kind))| AlignInput {
                name,
                kind: *kind,
                dir: "",
            })
            .collect();
        let ri: Vec<AlignInput<'_>> = rn
            .iter()
            .zip(right)
            .map(|(name, (_, kind))| AlignInput {
                name,
                kind: *kind,
                dir: "",
            })
            .collect();
        align_names(&li, &ri, &opts)
            .into_iter()
            .map(|p| {
                (
                    p.left.map(|i| ln[i].as_str().to_owned()),
                    p.right.map(|j| rn[j].as_str().to_owned()),
                )
            })
            .collect()
    }

    const SENSITIVE: AlignOptions = AlignOptions {
        case: CaseSensitivity::Sensitive,
        align_extensions: false,
        align_normalization: false,
    };
    const INSENSITIVE: AlignOptions = AlignOptions {
        case: CaseSensitivity::Insensitive,
        align_extensions: false,
        align_normalization: false,
    };

    fn p(l: Option<&str>, r: Option<&str>) -> (Option<String>, Option<String>) {
        (l.map(str::to_owned), r.map(str::to_owned))
    }

    /// §2.5.2 table, rows 1–9.
    #[test]
    fn alignment_table() {
        use EntryKind::{Dir, File, Symlink};
        // 1
        assert_eq!(
            run(&[("a.txt", File)], &[("a.txt", File)], SENSITIVE),
            [p(Some("a.txt"), Some("a.txt"))]
        );
        // 2
        assert_eq!(
            run(&[("README", File)], &[("readme", File)], INSENSITIVE),
            [p(Some("README"), Some("readme"))]
        );
        // 3
        assert_eq!(
            run(&[("README", File)], &[("readme", File)], SENSITIVE),
            [p(Some("README"), None), p(None, Some("readme"))]
        );
        // 4
        assert_eq!(
            run(&[("A", File), ("a", File)], &[("a", File)], INSENSITIVE),
            [p(Some("A"), None), p(Some("a"), Some("a"))]
        );
        // 5
        let norm = AlignOptions {
            align_normalization: true,
            ..SENSITIVE
        };
        assert_eq!(
            run(&[("caf\u{e9}", File)], &[("cafe\u{301}", File)], norm),
            [p(Some("caf\u{e9}"), Some("cafe\u{301}"))]
        );
        // 6
        assert_eq!(
            run(&[("caf\u{e9}", File)], &[("cafe\u{301}", File)], SENSITIVE).len(),
            2
        );
        // 7
        let ext = AlignOptions {
            align_extensions: true,
            ..SENSITIVE
        };
        assert_eq!(
            run(&[("x.txt", File)], &[("x.csv", File), ("x.md", File)], ext),
            [p(None, Some("x.md")), p(Some("x.txt"), Some("x.csv"))]
        );
        // 8: folder first.
        assert_eq!(
            run(&[("x", File)], &[("x", Dir)], INSENSITIVE),
            [p(None, Some("x")), p(Some("x"), None)]
        );
        // 9
        assert_eq!(
            run(&[("l", Symlink)], &[("l", File)], SENSITIVE),
            [p(None, Some("l")), p(Some("l"), None)]
        );
    }

    #[test]
    fn extensions_apply_to_files_only() {
        use EntryKind::{Dir, File};
        let ext = AlignOptions {
            align_extensions: true,
            ..INSENSITIVE
        };
        assert_eq!(
            run(&[("d.a", Dir)], &[("d.b", Dir)], ext),
            [p(Some("d.a"), None), p(None, Some("d.b"))]
        );
        // Case + extension together; exact matches win first.
        assert_eq!(
            run(
                &[("A.TXT", File), ("a.csv", File)],
                &[("a.csv", File), ("a.md", File)],
                ext
            ),
            [
                p(Some("a.csv"), Some("a.csv")),
                p(Some("A.TXT"), Some("a.md"))
            ]
        );
        assert_eq!(stem(b".bashrc"), b".bashrc");
        assert_eq!(stem(b"a."), b"a.");
        assert_eq!(stem(b"a.b.c"), b"a.b");
    }

    #[test]
    fn canonical_order_is_by_folded_name() {
        use EntryKind::{Dir, File, Other, Symlink};
        let out = run(
            &[
                ("b", File),
                ("B", File),
                ("a", Other),
                ("z", Dir),
                ("c", Symlink),
            ],
            &[("A", File)],
            SENSITIVE,
        );
        assert_eq!(
            out,
            [
                p(Some("z"), None),
                p(None, Some("A")),
                p(Some("B"), None),
                p(Some("b"), None),
                p(Some("c"), None),
                p(Some("a"), None),
            ]
        );
    }

    /// §2.5.1 (amended): a name that is not valid Unicode folds its exact bytes, never its lossy text.
    #[cfg(unix)]
    #[test]
    fn lossy_names_fold_their_exact_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let name = |b: &[u8]| FileName::from_os(std::ffi::OsStr::from_bytes(b));
        let (upper, lower, other) = (name(b"A\xff"), name(b"a\xff"), name(b"a\xfe"));
        assert!(upper.is_lossy() && lower.is_lossy() && other.is_lossy());
        assert_eq!(lower.as_str(), other.as_str(), "the same lossy text");
        assert_eq!(folded_exact(&upper), folded_exact(&lower));
        assert_ne!(folded_exact(&lower), folded_exact(&other));
        assert_eq!(&*folded_exact(&name(b"\xffB\xfeC")), b"\xffb\xfec");
        let input = |n| AlignInput {
            name: n,
            kind: EntryKind::File,
            dir: "",
        };
        let out = align_names(
            &[input(&upper)],
            &[input(&other), input(&lower)],
            &INSENSITIVE,
        );
        assert_eq!(out.len(), 2);
        assert!(out.contains(&Pair::new(Some(0), Some(1))), "{out:?}");
        assert!(out.contains(&Pair::new(None, Some(0))), "{out:?}");
    }

    #[test]
    fn flatten_prefers_same_folder() {
        let n = FileName::new("x.txt").unwrap();
        let input = |dir| AlignInput {
            name: &n,
            kind: EntryKind::File,
            dir,
        };
        let left = [input("a"), input("b")];
        let right = [input("b"), input("c")];
        let out = align_flat(&left, &right, &SENSITIVE);
        // L0 pairs b/x.txt; L1 pairs the rest (a ↔ c).
        assert_eq!(
            out,
            [Pair::new(Some(0), Some(1)), Pair::new(Some(1), Some(0)),]
        );
        let out = align_flat(&left, &[input("b")], &SENSITIVE);
        assert_eq!(out, [Pair::new(Some(0), None), Pair::new(Some(1), Some(0))]);
    }
}

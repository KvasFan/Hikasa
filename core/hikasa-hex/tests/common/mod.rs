//! Helpers of the hex tests: seeded inputs with planted edits, documents, the naive oracles of §6.12 and goldens.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::sync::Arc;

use hikasa_core::CancelToken;
use hikasa_hex::{
    AlignMode, HexAlignOptions, HexAlignment, HexDocument, SectionKind, SegmentKind, SegmentStore,
};
use hikasa_vfs::{MmapPolicy, StreamOptions};
use proptest::test_runner::{Config, FileFailurePersistence};

/// Seeded xorshift64* (no `rand` crate).
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 8);
        while out.len() < len {
            out.extend_from_slice(&self.next().to_le_bytes());
        }
        out.truncate(len);
        out
    }
}

/// An edit of [`edited`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Insert { at: usize, len: usize },
    Delete { at: usize, len: usize },
    Replace { at: usize, len: usize },
}

/// `base` with `edits` applied in order (positions clamped to the current length), new bytes from `seed`.
pub fn edited(base: &[u8], edits: &[Edit], seed: u64) -> Vec<u8> {
    let mut rng = XorShift(seed | 1);
    let mut out = base.to_vec();
    for &e in edits {
        match e {
            Edit::Insert { at, len } => {
                let at = at.min(out.len());
                let new = rng.bytes(len);
                out.splice(at..at, new);
            }
            Edit::Delete { at, len } => {
                let at = at.min(out.len());
                let end = (at + len).min(out.len());
                out.drain(at..end);
            }
            Edit::Replace { at, len } => {
                let at = at.min(out.len());
                let end = (at + len).min(out.len());
                for b in &mut out[at..end] {
                    *b = !*b;
                }
            }
        }
    }
    out
}

/// `count` seeded edits (insertions, deletions, replacements of 1…`max_len` bytes) over a file of `len` bytes.
pub fn random_edits(len: usize, count: usize, max_len: usize, seed: u64) -> Vec<Edit> {
    let mut rng = XorShift(seed | 1);
    let mut edits: Vec<Edit> = (0..count)
        .map(|_| {
            let at = rng.below(len as u64) as usize;
            let n = 1 + rng.below(max_len as u64) as usize;
            match rng.below(3) {
                0 => Edit::Insert { at, len: n },
                1 => Edit::Delete { at, len: n },
                _ => Edit::Replace { at, len: n },
            }
        })
        .collect();
    // Back to front, so earlier positions stay meaningful.
    edits.sort_by_key(|e| match *e {
        Edit::Insert { at, .. } | Edit::Delete { at, .. } | Edit::Replace { at, .. } => {
            std::cmp::Reverse(at)
        }
    });
    edits
}

pub fn doc(name: &str, bytes: &[u8]) -> HexDocument {
    HexDocument::from_bytes(name, Arc::from(bytes))
}

/// Options of `mode` with `threads` threads (streamed, from memory: documents made by [`doc`]).
pub fn opts(mode: AlignMode, threads: usize) -> HexAlignOptions {
    HexAlignOptions {
        stream: StreamOptions {
            threads,
            ..StreamOptions::default()
        },
        ..HexAlignOptions::with_mode(mode)
    }
}

/// Mapped (`true`, every size) or streamed options.
pub fn stream(mapped: bool, threads: usize) -> StreamOptions {
    StreamOptions {
        buffer_size: 64 << 10,
        mmap: if mapped {
            MmapPolicy::Auto { min_len: 0 }
        } else {
            MmapPolicy::Never
        },
        threads,
    }
}

/// The column states of an alignment by a naive walk: 0 same, 1 changed, 2 left-only, 3 right-only.
pub fn naive_states(a: &[u8], b: &[u8], store: &SegmentStore) -> Vec<u8> {
    let mut states = Vec::with_capacity(store.columns() as usize);
    for s in store.iter() {
        for i in 0..s.len as usize {
            states.push(match s.kind {
                SegmentKind::Paired => u8::from(a[s.left as usize + i] != b[s.right as usize + i]),
                SegmentKind::LeftOnly => 2,
                SegmentKind::RightOnly => 3,
            });
        }
    }
    states
}

/// Naive sections over column states: `(start, end, non_same, kind)`, runs fewer than 8 same columns apart merged.
pub fn naive_sections(states: &[u8]) -> Vec<(u64, u64, u64, SectionKind)> {
    let mut out: Vec<(u64, u64, u64, u8)> = Vec::new();
    for (c, &s) in states.iter().enumerate() {
        if s == 0 {
            continue;
        }
        let bit = 1u8 << (s - 1);
        let c = c as u64;
        match out.last_mut() {
            Some(last) if c - last.1 < 8 => {
                last.1 = c + 1;
                last.2 += 1;
                last.3 |= bit;
            }
            _ => out.push((c, c + 1, 1, bit)),
        }
    }
    out.into_iter()
        .map(|(s, e, n, bits)| {
            let kind = match bits {
                1 => SectionKind::Changed,
                2 => SectionKind::LeftOnly,
                4 => SectionKind::RightOnly,
                _ => SectionKind::Mixed,
            };
            (s, e, n, kind)
        })
        .collect()
}

/// The sections of an alignment as `(start, end, non_same, kind)`.
pub fn sections_of(al: &HexAlignment) -> Vec<(u64, u64, u64, SectionKind)> {
    al.sections
        .iter(&al.segments)
        .map(|s| (s.columns.start, s.columns.end, s.non_same, s.kind))
        .collect()
}

/// P-HEXALIGN's checks of any alignment of `a` and `b`, against the naive walk: coverage, statistics,
/// sections, the thumbnail's sums.
pub fn check_alignment(a: &[u8], b: &[u8], al: &HexAlignment) -> Result<(), String> {
    al.segments.check()?;
    if (al.segments.left_len(), al.segments.right_len()) != (a.len() as u64, b.len() as u64) {
        return Err("lengths".into());
    }
    let states = naive_states(a, b, &al.segments);
    let count = |v: u8| states.iter().filter(|&&s| s == v).count() as u64;
    let st = al.stats;
    let want = (count(0), count(1), count(2), count(3));
    let got = (
        st.same_bytes,
        st.differing_bytes,
        st.left_only_bytes,
        st.right_only_bytes,
    );
    if want != got {
        return Err(format!("stats {got:?} != naive {want:?}"));
    }
    let naive = naive_sections(&states);
    if !al.sections.truncated() && sections_of(al) != naive {
        return Err(format!(
            "sections {:?} != naive {:?}",
            &sections_of(al)[..sections_of(al).len().min(8)],
            &naive[..naive.len().min(8)]
        ));
    }
    if st.sections != naive.len() as u64 {
        return Err(format!("section count {} != {}", st.sections, naive.len()));
    }
    let t = &al.thumbnail;
    let sum = |f: fn(&hikasa_hex::ThumbBucket) -> u32| {
        t.buckets.iter().map(|b| u64::from(f(b))).sum::<u64>()
    };
    let thumb = (
        sum(|b| b.same),
        sum(|b| b.different),
        sum(|b| b.left_only),
        sum(|b| b.right_only),
    );
    if thumb != want {
        return Err(format!("thumbnail {thumb:?} != {want:?}"));
    }
    Ok(())
}

/// A compact text dump of an alignment (goldens).
pub fn dump(al: &HexAlignment) -> String {
    let mut out = String::new();
    let st = al.stats;
    let _ = writeln!(
        out,
        "mode {} left {} right {} columns {}",
        al.mode,
        st.left_bytes,
        st.right_bytes,
        al.segments.columns()
    );
    let _ = writeln!(
        out,
        "stats same {} different {} left_only {} right_only {} sections {}",
        st.same_bytes, st.differing_bytes, st.left_only_bytes, st.right_only_bytes, st.sections
    );
    for s in al.segments.iter() {
        let _ = writeln!(
            out,
            "segment {} {} {} {}",
            s.kind.as_str(),
            s.left,
            s.right,
            s.len
        );
    }
    for s in al.sections.iter(&al.segments) {
        let _ = writeln!(
            out,
            "section {}..{} left {}..{} right {}..{} {} {}",
            s.columns.start,
            s.columns.end,
            s.left.start,
            s.left.end,
            s.right.start,
            s.right.end,
            s.kind.as_str(),
            s.non_same
        );
    }
    if al.sections.truncated() {
        let _ = writeln!(out, "sections truncated");
    }
    for w in &al.warnings {
        let _ = writeln!(out, "warning {} {}", w.code.as_str(), w.message);
    }
    out
}

/// Compares `actual` with `tests/fixtures/golden/<name>.txt` (`HIKASA_UPDATE_GOLDENS=1` rewrites it).
pub fn golden(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(format!("{name}.txt"));
    if std::env::var_os("HIKASA_UPDATE_GOLDENS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}; run with HIKASA_UPDATE_GOLDENS=1",
            path.display()
        )
    });
    pretty_assertions::assert_eq!(expected, actual, "golden {name}");
}

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise; failing seeds persisted.
pub fn config(regressions: &'static str) -> Config {
    config_with(regressions, 256)
}

/// [`config`] with another default case count: the properties whose cases align megabytes run fewer cases by
/// default, so a debug run stays within CI's timeout; the heavy runs set `PROPTEST_CASES` (10,000 for P-HEXALIGN).
pub fn config_with(regressions: &'static str, default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_cases);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// A cancelled token.
pub fn cancelled() -> CancelToken {
    let t = CancelToken::new();
    t.cancel();
    t
}

/// A spec of a local path.
pub fn lspec(path: &std::path::Path) -> hikasa_core::Spec {
    hikasa_core::Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
}

/// Every way of reading a pair the tests compare: (label, left, right, stream options). Files are written to a
/// temporary folder and opened through `LocalVfs` as `File` loads (mapped or streamed through the kept handle),
/// and as documents in memory; each with 1, 4 and 16 threads.
pub fn every_reading(
    a: &[u8],
    b: &[u8],
    check: &mut dyn FnMut(&str, &HexDocument, &HexDocument, StreamOptions),
) {
    let dir = tempfile::tempdir().unwrap();
    let (pa, pb) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&pa, a).unwrap();
    std::fs::write(&pb, b).unwrap();
    let vfs = hikasa_vfs::LocalVfs::default();
    let load = hikasa_hex::HexLoadOptions {
        unlocked_max: 0,
        ..hikasa_hex::HexLoadOptions::default()
    };
    let ctl = hikasa_core::Ctl::detached();
    let fa = HexDocument::open(&vfs, &lspec(&pa), &load, &ctl).unwrap();
    let fb = HexDocument::open(&vfs, &lspec(&pb), &load, &ctl).unwrap();
    let (ma, mb) = (doc("a", a), doc("b", b));
    for threads in [1, 4, 16] {
        check("mapped", &fa, &fb, stream(true, threads));
        check("streamed", &fa, &fb, stream(false, threads));
        check("memory", &ma, &mb, stream(true, threads));
    }
}

/// A back-end whose `stat` reports `size` for every file whatever it holds (procfs reports 0, sysfs 4096): the
/// files are the inner back-end's.
pub struct Pseudo<'a> {
    pub inner: &'a dyn hikasa_vfs::Vfs,
    pub size: u64,
}

impl hikasa_vfs::Vfs for Pseudo<'_> {
    fn capabilities(
        &self,
        at: &hikasa_core::Spec,
    ) -> Result<hikasa_vfs::Capabilities, hikasa_vfs::VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(
        &self,
        spec: &hikasa_core::Spec,
        follow: hikasa_vfs::FollowLinks,
    ) -> Result<hikasa_vfs::Entry, hikasa_vfs::VfsError> {
        let mut entry = self.inner.stat(spec, follow)?;
        if entry.kind == hikasa_vfs::EntryKind::File {
            entry.size = self.size;
        }
        Ok(entry)
    }
    fn list(
        &self,
        dir: &hikasa_core::Spec,
        opts: &hikasa_vfs::ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<hikasa_vfs::Listing, hikasa_vfs::VfsError> {
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(
        &self,
        spec: &hikasa_core::Spec,
    ) -> Result<Box<dyn hikasa_vfs::ReadSeek>, hikasa_vfs::VfsError> {
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &hikasa_core::Spec,
        opts: &hikasa_vfs::WriteOptions,
    ) -> Result<Box<dyn hikasa_vfs::AtomicWrite>, hikasa_vfs::VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(
        &self,
        dir: &hikasa_core::Spec,
        name: &hikasa_vfs::FileName,
    ) -> Result<hikasa_core::Spec, hikasa_vfs::VfsError> {
        self.inner.child(dir, name)
    }
}

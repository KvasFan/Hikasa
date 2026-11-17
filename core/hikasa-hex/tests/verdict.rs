//! A12.6 (D-331) and HX-009 at engine level: the verdict of every row of §1.12.8, the conversion rows with a
//! fake converter on `MemoryVfs`; no mode ever answers 12; a planted block-hash collision is still
//! `binary_different`; for pairs around the chunk edges, None, Fast and Complete give `compare_sources`' verdict.

mod common;

use std::path::PathBuf;

use common::{XorShift, doc, opts};
use hikasa_core::{CaseSensitivity, ContentKind, Ctl, ExitCode, FileTime, LocalPath, Spec};
use hikasa_hex::align::testing::align_blocks_with_hash;
use hikasa_hex::{
    AlignMode, Converted, HexConversion, HexDocument, HexError, HexLoadOptions, HexVerdict, align,
    converted_verdict, verdict,
};
use hikasa_vfs::{
    BinaryOutcome, FollowLinks, MemMeta, MemoryVfs, StreamOptions, VISIT_CHUNK, Vfs, VfsSource,
    compare_sources,
};

fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(host(path))).unwrap())
}

/// A load conversion that upper-cases ASCII letters into `/conv/<name>`.
struct Upper<'a> {
    vfs: &'a MemoryVfs,
}

impl HexConversion for Upper<'_> {
    fn convert(&self, vfs: &dyn Vfs, spec: &Spec, _ctl: &Ctl<'_>) -> Result<Converted, HexError> {
        let entry = vfs.stat(spec, FollowLinks::Yes).unwrap();
        let name = entry.name.as_str().to_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut vfs.open_read(spec).unwrap(), &mut bytes).unwrap();
        let out = format!("/conv/{name}");
        self.vfs
            .insert_file(&host(&out), bytes.to_ascii_uppercase(), MemMeta::default())
            .unwrap();
        Ok(Converted {
            spec: mspec(&out),
            keep: None,
        })
    }
}

const SAME: HexVerdict = HexVerdict {
    kind: ContentKind::BinarySame,
    exit_code: ExitCode::BinarySame,
};
const DIFFERENT: HexVerdict = HexVerdict {
    kind: ContentKind::BinaryDifferent,
    exit_code: ExitCode::BinaryDifferent,
};

#[test]
fn every_row_of_the_table() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let meta = || MemMeta {
        mtime: Some(FileTime::from_unix(1_700_000_000, 0).unwrap()),
        ..MemMeta::default()
    };
    for (name, content) in [
        ("lower", &b"hello world"[..]),
        ("lower2", b"hello world"),
        ("upper", b"HELLO WORLD"),
        ("other", b"hello there"),
    ] {
        vfs.insert_file(&host(&format!("/in/{name}")), content.to_vec(), meta())
            .unwrap();
    }
    let ctl = Ctl::detached();
    let load = HexLoadOptions::default();
    let upper = Upper { vfs: &vfs };
    let open = |name: &str, convert: bool| {
        let spec = mspec(&format!("/in/{name}"));
        if convert {
            HexDocument::open_converted(&vfs, &spec, &upper, &load, &ctl).unwrap()
        } else {
            HexDocument::open(&vfs, &spec, &load, &ctl).unwrap()
        }
    };
    let raw_same = |l: &str, r: &str| {
        let src = |n: &str| {
            let spec = mspec(&format!("/in/{n}"));
            let entry = vfs.stat(&spec, FollowLinks::Yes).unwrap();
            VfsSource::new(&vfs, spec, entry)
        };
        compare_sources(&src(l), &src(r), &StreamOptions::default(), &ctl).unwrap()
            == BinaryOutcome::Same
    };
    for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
        let o = opts(mode, 1);
        let al = |l: &HexDocument, r: &HexDocument| align(l, r, &o, &ctl).unwrap();
        // Bytes identical; bytes differ.
        assert_eq!(
            verdict(&al(&open("lower", false), &open("lower2", false))),
            SAME,
            "{mode}"
        );
        assert_eq!(
            verdict(&al(&open("lower", false), &open("upper", false))),
            DIFFERENT,
            "{mode}"
        );
        // A side converted: raw bytes identical → 1.
        let (l, r) = (open("lower", true), open("lower2", false));
        assert!(l.info().converted);
        assert_eq!(l.load(), hikasa_hex::HexLoad::TempCache);
        assert_eq!(
            converted_verdict(raw_same("lower", "lower2"), &al(&l, &r)),
            SAME,
            "{mode}"
        );
        // Converted outputs identical → 2.
        let (l, r) = (open("lower", true), open("upper", true));
        let v = converted_verdict(raw_same("lower", "upper"), &al(&l, &r));
        assert_eq!(
            (v.kind, v.exit_code),
            (ContentKind::RulesSame, ExitCode::RulesSame),
            "{mode}"
        );
        // Converted outputs differ → 13.
        let (l, r) = (open("lower", true), open("other", true));
        let v = converted_verdict(raw_same("lower", "other"), &al(&l, &r));
        assert_eq!(
            (v.kind, v.exit_code),
            (ContentKind::ImportantDifferences, ExitCode::RulesDifferent),
            "{mode}"
        );
    }
    assert_eq!(HexVerdict::ERROR.exit_code, ExitCode::UnknownError);
}

/// Every block hash collides (the hash is the length): the blocks pair, the bytes still decide.
#[test]
fn a_planted_collision_is_still_different() {
    let a = XorShift(8).bytes(300_000);
    let mut b = a.clone();
    b[123_456] ^= 0x10;
    b[200] ^= 0x01;
    for complete in [false, true] {
        let al = align_blocks_with_hash(
            &doc("a", &a),
            &doc("b", &b),
            &opts(AlignMode::Fast, 4),
            complete,
            |bytes| bytes.len() as u64,
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(verdict(&al), DIFFERENT);
        assert_eq!(al.stats.differing_bytes, 2);
    }
}

/// HX-009 at engine level: the verdict of every mode equals `compare_sources`' for pairs that are equal, a
/// prefix, or differ in one byte at 0, at a chunk edge ± 1 or at the last byte; never 12.
#[test]
fn every_mode_agrees_with_compare_sources() {
    let chunk = VISIT_CHUNK as usize;
    let base = XorShift(77).bytes(chunk + 3);
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (Vec::new(), Vec::new()),
        (vec![1], vec![1]),
        (vec![1], vec![2]),
        (vec![], vec![0]),
        (base.clone(), base.clone()),
        (base.clone(), base[..chunk].to_vec()),
        (base[..chunk - 1].to_vec(), base.clone()),
    ];
    for at in [0, chunk - 1, chunk, chunk + 1, chunk + 2] {
        let mut b = base.clone();
        b[at] ^= 0x80;
        pairs.push((base.clone(), b));
    }
    let ctl = Ctl::detached();
    for (k, (a, b)) in pairs.iter().enumerate() {
        let (da, db) = (doc("a", a), doc("b", b));
        let expected = match compare_sources(&da, &db, &StreamOptions::default(), &ctl).unwrap() {
            BinaryOutcome::Same => SAME,
            BinaryOutcome::Different { .. } => DIFFERENT,
        };
        for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
            let v = verdict(&align(&da, &db, &opts(mode, 4), &ctl).unwrap());
            assert_eq!(v, expected, "pair {k} {mode}");
            assert_ne!(v.exit_code, ExitCode::Similar);
        }
    }
}

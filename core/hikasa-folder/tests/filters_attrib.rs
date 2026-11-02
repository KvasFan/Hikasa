//! M2 W6a acceptance A6.7 (D-169, the attribute and Unix-type filters) and A6.8 (D-170, the keys of FC-012).

mod common;

use std::collections::BTreeMap;

use common::{compared, host, mspec, state_at};
use hikasa_core::{CaseSensitivity, CompareVerdict, FileTime, WarningCode};
use hikasa_folder::{
    AttribFilter, Change, CompareState, FolderComparison, FolderRules, FolderRulesPatch, Handling,
    NodeId, ScanStatus, UnixTypeFilter, UnixTypes, expanded_on_load,
};
use hikasa_vfs::{Attrs, Capabilities, DosAttrs, MemMeta, MemoryVfs, SpecialKind};

fn memory() -> MemoryVfs {
    MemoryVfs::new(CaseSensitivity::Sensitive)
}

fn meta(attrs: DosAttrs) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(1_000, 0),
        attrs: Some(Attrs::Dos { attrs }),
        ..MemMeta::default()
    }
}

fn bases(vfs: &MemoryVfs) {
    vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
    vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
}

fn state(cmp: &FolderComparison, path: &str) -> CompareState {
    state_at(&cmp.to_canonical(), path)
}

#[test]
fn a6_7_parsers() {
    let good = [
        ("+r", "+r"),
        ("-h", "-h"),
        ("+rh-s", "+hr-s"),
        ("-s+RH", "+hr-s"),
        ("+r+h", "+hr"),
    ];
    for (text, canonical) in good {
        let filter = AttribFilter::parse(text).unwrap();
        assert_eq!(filter.to_string(), canonical, "{text}");
        assert_eq!(AttribFilter::parse(canonical).unwrap(), filter);
    }
    let parsed = UnixTypeFilter::parse("+rl-p").unwrap();
    assert_eq!(parsed.include, UnixTypes::REGULAR | UnixTypes::LINK);
    assert_eq!(parsed.exclude, UnixTypes::FIFO);
    assert_eq!(parsed.to_string(), "+lr-p");
    assert_eq!(UnixTypeFilter::parse("-bcs").unwrap().to_string(), "-bcs");
    for bad in ["", "r", "+", "-", "+r-", "+r-r", "+q", "r+h", "+r h"] {
        let error = AttribFilter::parse(bad).unwrap_err();
        assert!(error.to_string().contains("filter"), "{bad}: {error}");
    }
    for bad in ["", "p", "+", "+d", "+p-p", "+x", "+R"] {
        assert!(UnixTypeFilter::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn a6_7_toml_is_sparse_and_round_trips() {
    let plain = FolderRules::default();
    let text = plain.to_toml();
    for key in [
        "attrib =",
        "unix_type",
        "auto_scan_subfolders",
        "expand_on_load",
        "expand_differences_only",
        "ignored",
        "format_overrides",
    ] {
        assert!(!text.contains(key), "{key} in {text}");
    }
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            attrib_filter: Some(Change::Set(AttribFilter::parse("+rh-s").unwrap())),
            unix_type_filter: Some(Change::Set(UnixTypeFilter::parse("-lp").unwrap())),
            auto_scan_subfolders: Some(false),
            expand_on_load: Some(true),
            ignored: Some(vec!["a".into(), "!a/b".into()]),
            format_overrides: Some(BTreeMap::from([("cpp".to_owned(), false)])),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let text = rules.to_toml();
    assert!(text.contains(r#"attrib = "+hr-s""#), "{text}");
    assert!(text.contains(r#"unix_type = "-lp""#), "{text}");
    let (back, warnings) = FolderRules::from_toml(&text).unwrap();
    assert_eq!(back, rules);
    assert!(warnings.is_empty(), "{warnings:?}");
    let off = "[filters.other]\nattrib = false\nunix_type = false\n";
    let (read, _) = FolderRules::from_toml(off).unwrap();
    assert_eq!(read.filters.other.attrib, None);
    assert_eq!(read.filters.other.unix_type, None);
    assert!(FolderRules::from_toml("[filters.other]\nattrib = \"+q\"\n").is_err());
    let mut changed = rules.clone();
    changed
        .apply(&FolderRulesPatch {
            attrib_filter: Some(Change::Clear),
            unix_type_filter: Some(Change::Clear),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    assert_eq!(changed.filters.other.attrib, None);
    assert_eq!(changed.filters.other.unix_type, None);
}

fn with_attrib(text: &str) -> FolderRules {
    let mut rules = FolderRules::default();
    rules.filters.other.attrib = Some(AttribFilter::parse(text).unwrap());
    rules
}

fn with_unix(text: &str) -> FolderRules {
    let mut rules = FolderRules::default();
    rules.filters.other.unix_type = Some(UnixTypeFilter::parse(text).unwrap());
    rules
}

#[test]
fn a6_7_attrib_on_files_not_on_folders() {
    let vfs = memory();
    bases(&vfs);
    let (r, h, s) = (DosAttrs::READONLY, DosAttrs::HIDDEN, DosAttrs::SYSTEM);
    for (name, attrs) in [
        ("plain", DosAttrs::empty()),
        ("ro", r),
        ("hidden", h),
        ("both", r | h),
        ("all", r | h | s),
    ] {
        for side in ["/l", "/r"] {
            vfs.insert_file(&host(&format!("{side}/{name}")), &b"x"[..], meta(attrs))
                .unwrap();
        }
    }
    vfs.insert_dir(&host("/l/d"), meta(h)).unwrap();
    vfs.insert_dir(&host("/r/d"), meta(h)).unwrap();
    let rows: [(&str, &[&str]); 3] = [
        ("+r", &["ro", "both", "all"]),
        ("-h", &["plain", "ro"]),
        ("+rh-s", &["both"]),
    ];
    for (text, kept) in rows {
        let mut rules = with_attrib(text);
        rules.filters.other.exclude_protected = false;
        let cmp = compared(&vfs, &rules);
        cmp.check_invariants().unwrap();
        for name in ["plain", "ro", "hidden", "both", "all"] {
            let expected = if kept.contains(&name) {
                CompareState::Same
            } else {
                CompareState::Filtered
            };
            assert_eq!(state(&cmp, name), expected, "{text} {name}");
        }
        assert_ne!(state(&cmp, "d"), CompareState::Filtered, "a folder stays");
    }
}

#[test]
fn a6_7_attrib_without_dos_attributes_has_no_effect() {
    let vfs = memory();
    bases(&vfs);
    vfs.insert_file(&host("/l/a"), &b"x"[..], MemMeta::default())
        .unwrap();
    vfs.insert_file(&host("/r/a"), &b"x"[..], MemMeta::default())
        .unwrap();
    let all = hikasa_vfs::Vfs::capabilities(&vfs, &mspec("/l")).unwrap();
    vfs.set_capabilities(all.difference(Capabilities::DOS_ATTRS));
    let cmp = compared(&vfs, &with_attrib("+r"));
    assert_ne!(state(&cmp, "a"), CompareState::Filtered);
    let warned: Vec<_> = cmp
        .read()
        .warnings()
        .iter()
        .filter(|w| w.code == WarningCode::CapabilityMissing)
        .filter(|w| w.message.contains("attribute filter"))
        .cloned()
        .collect();
    assert_eq!(warned.len(), 2, "one per side: {warned:?}");
}

fn special(kind: Option<SpecialKind>) -> MemMeta {
    MemMeta {
        special: kind,
        ..MemMeta::default()
    }
}

#[test]
fn a6_7_unix_type_every_letter() {
    let vfs = memory();
    bases(&vfs);
    let rows = [
        ("b", SpecialKind::Block),
        ("c", SpecialKind::Char),
        ("p", SpecialKind::Fifo),
        ("s", SpecialKind::Socket),
    ];
    for side in ["/l", "/r"] {
        for (name, kind) in rows {
            vfs.insert_other(&host(&format!("{side}/{name}")), special(Some(kind)))
                .unwrap();
        }
        vfs.insert_other(&host(&format!("{side}/none")), MemMeta::default())
            .unwrap();
        vfs.insert_file(&host(&format!("{side}/r")), &b"x"[..], MemMeta::default())
            .unwrap();
        vfs.insert_symlink(&host(&format!("{side}/l")), "r", MemMeta::default())
            .unwrap();
        vfs.insert_dir(&host(&format!("{side}/d")), MemMeta::default())
            .unwrap();
    }
    let names = ["b", "c", "p", "s", "r", "l", "none"];
    for letter in ["b", "c", "l", "p", "r", "s"] {
        let only = compared(&vfs, &with_unix(&format!("+{letter}")));
        let not = compared(&vfs, &with_unix(&format!("-{letter}")));
        only.check_invariants().unwrap();
        for name in names {
            let hidden = |cmp: &FolderComparison| state(cmp, name) == CompareState::Filtered;
            assert_eq!(hidden(&only), name != letter, "+{letter} {name}");
            assert_eq!(hidden(&not), name == letter, "-{letter} {name}");
        }
        assert_ne!(state(&only, "d"), CompareState::Filtered, "a folder stays");
    }
}

/// A link the comparison followed is its target's type.
#[test]
fn a6_7_a_followed_link_is_its_targets_type() {
    let vfs = memory();
    bases(&vfs);
    vfs.insert_file(&host("/l/t"), &b"x"[..], MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/l/k"), "t", MemMeta::default())
        .unwrap();
    let mut rules = with_unix("-l");
    rules.handling.follow_symlinks = true;
    let cmp = compared(&vfs, &rules);
    assert_eq!(state(&cmp, "k"), CompareState::LeftOrphan);
    let mut rules = with_unix("-r");
    rules.handling.follow_symlinks = true;
    let cmp = compared(&vfs, &rules);
    assert_eq!(state(&cmp, "k"), CompareState::Filtered);
}

/// A hidden side of a pair is teal (FC-054), and a folder pair still aligns.
#[test]
fn a6_7_a_hidden_side_keeps_its_pair() {
    let vfs = memory();
    bases(&vfs);
    vfs.insert_file(&host("/l/a"), &b"x"[..], meta(DosAttrs::READONLY))
        .unwrap();
    vfs.insert_file(&host("/r/a"), &b"x"[..], meta(DosAttrs::empty()))
        .unwrap();
    vfs.insert_dir(&host("/l/d"), meta(DosAttrs::READONLY))
        .unwrap();
    vfs.insert_dir(&host("/r/d"), meta(DosAttrs::empty()))
        .unwrap();
    let cmp = compared(&vfs, &with_attrib("+r"));
    cmp.check_invariants().unwrap();
    let read = cmp.read();
    let a = read.children(NodeId::ROOT).find(|&id| {
        read.node(id)
            .is_some_and(|view| view.representative().unwrap().name == "a")
    });
    let view = read.node(a.unwrap()).unwrap();
    assert!(!view.left.unwrap().filtered);
    assert!(view.right.unwrap().filtered);
    assert_eq!(read.children(NodeId::ROOT).len(), 2, "a pair each");
}

/// Real FIFOs and sockets are `p`, `s`.
#[cfg(unix)]
#[test]
fn a6_7_real_special_files() {
    use std::os::unix::net::UnixListener;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (left, right) = (root.join("l"), root.join("r"));
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    let _socket = UnixListener::bind(left.join("s")).unwrap();
    std::fs::write(left.join("f"), b"x").unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(left.join("p"))
        .status()
        .unwrap();
    assert!(status.success());
    let vfs = hikasa_vfs::LocalVfs::default();
    let spec = |path: &std::path::Path| {
        hikasa_core::Spec::Local(hikasa_core::LocalPath::new(path.to_path_buf()).unwrap())
    };
    let engines = common::engines(1 << 30);
    let run = |text: &str| {
        common::with_env(&vfs, &engines, &hikasa_core::CancelToken::new(), |env| {
            let model = hikasa_folder::FolderModel::new(spec(&left), spec(&right), with_unix(text))
                .unwrap();
            hikasa_folder::compare_into(&model, env).unwrap()
        })
    };
    let cmp = run("-ps");
    assert_eq!(state(&cmp, "p"), CompareState::Filtered);
    assert_eq!(state(&cmp, "s"), CompareState::Filtered);
    assert_eq!(state(&cmp, "f"), CompareState::LeftOrphan);
    let cmp = run("+p");
    assert_eq!(state(&cmp, "p"), CompareState::LeftOrphan);
    assert_eq!(state(&cmp, "s"), CompareState::Filtered);
}

/// On Windows the letters `b c p s` never match on disk: asserted, not skipped.
#[cfg(windows)]
#[test]
fn a6_7_letters_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("f");
    std::fs::write(&path, b"x").unwrap();
    let vfs = hikasa_vfs::LocalVfs::default();
    let spec = hikasa_core::Spec::Local(hikasa_core::LocalPath::new(path).unwrap());
    let entry = hikasa_vfs::Vfs::stat(&vfs, &spec, hikasa_vfs::FollowLinks::No).unwrap();
    assert_eq!(entry.special, None);
}

// ---- A6.8 FC-012 ---------------------------------------------------------------------------------------------

fn tree() -> MemoryVfs {
    let vfs = memory();
    for path in ["/l/pair/x", "/r/pair/x", "/l/orphan/sub/y"] {
        vfs.insert_file(&host(path), &b"x"[..], MemMeta::default())
            .unwrap();
    }
    vfs.insert_file(&host("/r/pair/inner/z"), &b"x"[..], MemMeta::default())
        .unwrap();
    vfs.insert_dir(&host("/l/pair/inner"), MemMeta::default())
        .unwrap();
    vfs
}

fn handling(auto: bool, orphans: bool) -> FolderRules {
    let mut rules = FolderRules::default();
    rules.handling.auto_scan_subfolders = auto;
    rules.handling.scan_orphan_folders = orphans;
    rules
}

fn scan_of(cmp: &FolderComparison, path: &str) -> ScanStatus {
    common::find(&cmp.to_canonical(), path).scan
}

#[test]
fn a6_8_the_table() {
    let vfs = tree();
    let all = compared(&vfs, &handling(true, true));
    assert_eq!(scan_of(&all, "pair"), ScanStatus::Scanned);
    assert_eq!(scan_of(&all, "pair/inner"), ScanStatus::Scanned);
    assert_eq!(scan_of(&all, "orphan"), ScanStatus::Scanned);
    assert_eq!(scan_of(&all, "orphan/sub"), ScanStatus::Scanned);
    assert_eq!(
        all.to_canonical(),
        compared(&vfs, &FolderRules::default()).to_canonical()
    );

    let pairs = compared(&vfs, &handling(true, false));
    assert_eq!(scan_of(&pairs, "pair"), ScanStatus::Scanned);
    assert_eq!(scan_of(&pairs, "orphan"), ScanStatus::NotScanned);
    assert_eq!(state(&pairs, "orphan"), CompareState::LeftOrphan);

    let orphans = compared(&vfs, &handling(false, true));
    orphans.check_invariants().unwrap();
    assert_eq!(scan_of(&orphans, "pair"), ScanStatus::NotScanned);
    assert_eq!(state(&orphans, "pair"), CompareState::Unknown);
    assert_eq!(scan_of(&orphans, "orphan"), ScanStatus::Scanned);
    assert_eq!(scan_of(&orphans, "orphan/sub"), ScanStatus::Scanned);

    let bases = compared(&vfs, &handling(false, false));
    bases.check_invariants().unwrap();
    assert_eq!(scan_of(&bases, "pair"), ScanStatus::NotScanned);
    assert_eq!(scan_of(&bases, "orphan"), ScanStatus::NotScanned);
    assert_eq!(state(&bases, "pair"), CompareState::Unknown);
    // A folder not scanned claims nothing: the verdict is incomplete, unless differences were found.
    let quiet = memory();
    quiet
        .insert_file(&host("/l/d/x"), &b"x"[..], MemMeta::default())
        .unwrap();
    quiet
        .insert_file(&host("/r/d/x"), &b"y"[..], MemMeta::default())
        .unwrap();
    let cmp = compared(&quiet, &handling(false, false));
    assert_eq!(cmp.verdict(false), CompareVerdict::Incomplete);
}

#[test]
fn a6_8_expanded_on_load() {
    let vfs = memory();
    for path in [
        "/l/same/x",
        "/r/same/x",
        "/l/diff/x",
        "/l/diff/deep/y",
        "/r/diff/deep/y",
    ] {
        vfs.insert_file(&host(path), &b"x"[..], MemMeta::default())
            .unwrap();
    }
    vfs.insert_dir(&host("/l/empty"), MemMeta::default())
        .unwrap();
    vfs.insert_dir(&host("/r/empty"), MemMeta::default())
        .unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let read = cmp.read();
    let paths = |handling: &Handling| -> Vec<String> {
        expanded_on_load(&read, handling)
            .into_iter()
            .map(|id| read.path(id).as_str().to_owned())
            .collect()
    };
    assert!(paths(&Handling::default()).is_empty());
    let open = Handling {
        expand_on_load: true,
        ..Handling::default()
    };
    assert_eq!(paths(&open), ["diff", "diff/deep", "same"]);
    let differences = Handling {
        expand_on_load: true,
        expand_differences_only: true,
        ..Handling::default()
    };
    assert_eq!(paths(&differences), ["diff"]);
    let flat = Handling {
        expand_on_load: true,
        flatten: true,
        ..Handling::default()
    };
    assert!(paths(&flat).is_empty());
}

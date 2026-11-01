//! A5.12: archive sides and FC-015 (D-147, D-151) over `ArchiveVfs` on `MemoryVfs`. Fixtures are small stored archives
//! written here; §1.5.11 modes, budget, filters, flatten, and the CRC of FC-025.

mod common;

use std::sync::Arc;

use common::{default_now, engines, mspec};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, NullProgress, Spec, WarningCode};
use hikasa_folder::{
    ArchiveHandling, CompareState, ContentEngines, FolderComparison, FolderEnv, FolderModel,
    FolderRules, NodeFlags, NodeId, NodeKind, NullObserver, compare_into,
};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, MemMeta, MemoryVfs, Vfs,
};

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip of stored `(name, data, stored crc, flags)`, each entry with a 0x5455 time.
fn zip_raw(entries: &[(&str, &[u8], u32, u16)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut extra = vec![0x55, 0x54, 5, 0, 1];
    extra.extend_from_slice(&1_790_000_000u32.to_le_bytes());
    for (name, data, crc, flags) in entries {
        let offset = out.len() as u32;
        let head = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&0x6000u16.to_le_bytes());
            out.extend_from_slice(&0x5cefu16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04");
        head(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&extra);
        out.extend_from_slice(data);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes());
        head(&mut central);
        central.extend_from_slice(&[0u8; 10]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&extra);
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let raw: Vec<_> = entries.iter().map(|(n, d)| (*n, *d, crc32(d), 0)).collect();
    zip_raw(&raw)
}

/// [`zip_of`] for byte names of nested archives.
fn zip_raw_with(entries: &[(&str, &Vec<u8>)]) -> Vec<u8> {
    let raw: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    zip_of(&raw)
}

fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        let mut block = [0u8; 512];
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[100..108].copy_from_slice(b"0000644\0");
        block[108..116].copy_from_slice(b"0000000\0");
        block[116..124].copy_from_slice(b"0000000\0");
        block[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        block[136..148].copy_from_slice(format!("{:011o}\0", 1_790_000_000).as_bytes());
        block[156] = b'0';
        block[257..263].copy_from_slice(b"ustar\0");
        block[263..265].copy_from_slice(b"00");
        block[148..156].fill(b' ');
        let sum: u32 = block.iter().map(|&b| u32::from(b)).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&block);
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(512) {
            out.push(0);
        }
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

struct World {
    mem: Arc<MemoryVfs>,
    vfs: ArchiveVfs,
    _dir: tempfile::TempDir,
    budget: u64,
}

fn world() -> World {
    let mem = Arc::new(MemoryVfs::new(CaseSensitivity::Sensitive));
    let dir = tempfile::tempdir().unwrap();
    let masks = ArchiveMasks::new([
        (ArchiveFormat::Zip, "*.zip".to_owned()),
        (ArchiveFormat::Tar, "*.tar".to_owned()),
    ])
    .unwrap();
    let config = ArchiveConfig {
        masks,
        limits: ArchiveLimits::default(),
        clock: Arc::new(default_now),
        cache_root: dir.path().to_path_buf(),
    };
    let vfs = ArchiveVfs::new(mem.clone() as Arc<dyn Vfs>, config);
    World {
        mem,
        vfs,
        _dir: dir,
        budget: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    }
}

impl World {
    fn put(&self, path: &str, bytes: &[u8]) {
        let meta = MemMeta {
            mtime: FileTime::from_unix(1_790_000_000, 0),
            ..MemMeta::default()
        };
        self.mem
            .insert_file(&common::host(path), bytes.to_vec(), meta)
            .unwrap();
    }

    fn compare(&self, left: Spec, right: Spec, rules: &FolderRules) -> FolderComparison {
        let contents: ContentEngines = engines(1 << 20);
        let now = default_now();
        let cancel = CancelToken::never();
        let progress = NullProgress;
        let env = FolderEnv {
            vfs: &self.vfs,
            content: &contents,
            pool: None,
            ctl: Ctl {
                cancel: &cancel,
                progress: &progress,
            },
            observer: &NullObserver,
            now: &now,
            content_budget: 1 << 30,
            archive_entries_max: self.budget,
        };
        let model = FolderModel::new(left, right, rules.clone()).unwrap();
        let cmp = compare_into(&model, &env).unwrap();
        cmp.check_invariants().unwrap();
        cmp
    }

    fn trees(&self, rules: &FolderRules) -> FolderComparison {
        self.compare(mspec("/l"), mspec("/r"), rules)
    }
}

fn rules(mode: ArchiveHandling) -> FolderRules {
    let mut rules = FolderRules::default();
    rules.handling.archives = mode;
    rules
}

/// `path kind state` of every node below the root, in tree order.
fn shape(cmp: &FolderComparison) -> Vec<String> {
    fn go(cmp: &FolderComparison, id: NodeId, prefix: &str, out: &mut Vec<String>) {
        let read = cmp.read();
        let children: Vec<NodeId> = read.children(id).collect();
        drop(read);
        for child in children {
            let read = cmp.read();
            let node = read.node(child).unwrap();
            let name = node.representative().unwrap().name.to_owned();
            let path = format!("{prefix}{name}");
            let state = node.state.as_str();
            out.push(format!("{path} {} {state}", node.kind.as_str()));
            drop(read);
            go(cmp, child, &format!("{path}/"), out);
        }
    }
    let mut out = Vec::new();
    go(cmp, NodeId::ROOT, "", &mut out);
    out
}

fn node(cmp: &FolderComparison, path: &str) -> NodeId {
    let read = cmp.read();
    let mut id = NodeId::ROOT;
    for name in path.split('/') {
        id = read
            .children(id)
            .find(|&c| {
                read.node(c)
                    .unwrap()
                    .representative()
                    .is_some_and(|e| e.name == name)
            })
            .unwrap_or_else(|| panic!("no {name} in {path}"));
    }
    id
}

fn flags(cmp: &FolderComparison, path: &str) -> NodeFlags {
    cmp.read().node(node(cmp, path)).unwrap().flags
}

const SAME: &str = "same";

#[test]
fn the_mode_table() {
    let w = world();
    let zip = zip_of(&[("x", b"one"), ("d/y", b"two")]);
    w.put("/l/a.zip", &zip);
    w.put("/r/a.zip", &zip);
    w.put("/l/plain", b"p");
    w.put("/r/plain", b"p");

    for mode in [ArchiveHandling::AsFiles, ArchiveHandling::OnceOpened] {
        let cmp = w.trees(&rules(mode));
        let flag = flags(&cmp, "a.zip");
        assert!(flag.contains(NodeFlags::ARCHIVE), "{mode:?}");
        let read = cmp.read();
        assert_eq!(
            read.node(node(&cmp, "a.zip")).unwrap().kind,
            NodeKind::File,
            "{mode:?}"
        );
        drop(read);
        assert_eq!(shape(&cmp).len(), 2, "{mode:?}");
    }
    assert_eq!(w.vfs.counters().index_passes, 0, "nothing was opened");

    let cmp = w.trees(&rules(ArchiveHandling::Always));
    let read = cmp.read();
    assert_eq!(
        read.node(node(&cmp, "a.zip")).unwrap().kind,
        NodeKind::Folder
    );
    drop(read);
    assert!(
        flags(&cmp, "a.zip").contains(NodeFlags::ARCHIVE),
        "the flag on open rows too"
    );
    let shown = shape(&cmp);
    assert!(
        shown.iter().any(|l| l.starts_with("a.zip/d/y file")),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .all(|l| l.ends_with(SAME) || l.contains("folder")),
        "{shown:?}"
    );
    // A member's spec is the archive's, on each side.
    let id = node(&cmp, "a.zip/d/y");
    let spec = cmp.read().spec(id, hikasa_core::Side::Left).unwrap();
    assert_eq!(spec, Spec::archive_member(mspec("/l/a.zip"), "d/y"));
}

#[test]
fn an_archive_against_a_folder_of_the_same_name_an_orphan_and_a_nested_one() {
    let w = world();
    w.put("/l/a.zip", &zip_of(&[("x", b"one")]));
    w.put("/r/a.zip/x", b"one");
    w.put("/l/only.zip", &zip_of(&[("o", b"1")]));
    let inner = tar_of(&[("deep", b"2")]);
    w.put("/l/n.zip", &zip_of(&[("b.tar", &inner)]));
    w.put("/r/n.zip", &zip_of(&[("b.tar", &inner)]));
    let cmp = w.trees(&rules(ArchiveHandling::Always));
    let shown = shape(&cmp);
    assert!(shown.contains(&"a.zip folder same".to_owned()), "{shown:?}");
    assert!(shown.contains(&"a.zip/x file same".to_owned()), "{shown:?}");
    assert!(
        shown.iter().any(|l| l.starts_with("only.zip/o file left")),
        "{shown:?}"
    );
    assert!(
        shown.contains(&"n.zip/b.tar/deep file same".to_owned()),
        "{shown:?}"
    );
    let id = node(&cmp, "n.zip/b.tar/deep");
    let spec = cmp.read().spec(id, hikasa_core::Side::Right).unwrap();
    let outer = Spec::archive_member(mspec("/r/n.zip"), "b.tar");
    assert_eq!(spec, Spec::archive_member(outer, "deep"));
}

#[test]
fn a_corrupt_archive_is_an_error_row_and_the_rest_completes() {
    let w = world();
    w.put("/l/bad.zip", b"this is text");
    w.put("/r/bad.zip", b"this is text");
    w.put("/l/ok.txt", b"1");
    w.put("/r/ok.txt", b"2");
    let cmp = w.trees(&rules(ArchiveHandling::Always));
    let read = cmp.read();
    let bad = read.node(node(&cmp, "bad.zip")).unwrap();
    assert_eq!(bad.scan, hikasa_folder::ScanStatus::Failed);
    assert!(bad.flags.contains(NodeFlags::LEFT_ERROR));
    assert!(!read.errors(node(&cmp, "bad.zip")).is_empty());
    drop(read);
    assert!(
        shape(&cmp).iter().any(|l| l.starts_with("ok.txt file")),
        "{:?}",
        shape(&cmp)
    );
}

#[test]
fn open_archives_by_a_path_on_the_left_on_the_right_nested() {
    let w = world();
    let inner = tar_of(&[("deep", b"2")]);
    w.put("/l/sub/a.zip", &zip_of(&[("x", b"1"), ("b.tar", &inner)]));
    w.put("/r/sub/a.zip", &zip_of(&[("x", b"1"), ("b.tar", &inner)]));
    w.put("/l/other.zip", &zip_of(&[("x", b"1")]));
    w.put("/r/third.zip", &zip_of(&[("x", b"1")]));
    let open = FolderRules {
        open_archives: vec![
            "sub/a.zip".into(),
            "sub/a.zip/b.tar".into(),
            "third.zip".into(),
        ],
        ..FolderRules::default()
    };
    let cmp = w.trees(&open);
    let shown = shape(&cmp);
    assert!(
        shown.contains(&"sub/a.zip/b.tar/deep file same".to_owned()),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .any(|l| l.starts_with("third.zip/x file right")),
        "{shown:?}"
    );
    assert!(
        shown.iter().any(|l| l.starts_with("other.zip file left")),
        "{shown:?}"
    );
}

#[test]
fn filter_masks_and_the_prefilter() {
    let w = world();
    let zip = zip_of(&[("keep.txt", b"1"), ("drop.bin", b"2")]);
    w.put("/l/a.zip", &zip);
    w.put("/r/a.zip", &zip);
    let mut mode = rules(ArchiveHandling::Always);
    // A file-include list: the opened archive passes as a folder, its members meet the mask.
    mode.apply(&hikasa_folder::FolderRulesPatch {
        name_filter: Some("*.txt".into()),
        ..Default::default()
    })
    .unwrap();
    let cmp = w.trees(&mode);
    assert!(
        cmp.read().node(node(&cmp, "a.zip")).unwrap().kind == NodeKind::Folder,
        "the name passes with kind Dir"
    );
    let read = cmp.read();
    let member = read.node(node(&cmp, "a.zip/drop.bin")).unwrap();
    assert!(member.flags.contains(NodeFlags::LEFT_FILTERED));
    drop(read);

    // A folder-exclude: the folder masks decide.
    let mut mode = rules(ArchiveHandling::Always);
    mode.apply(&hikasa_folder::FolderRulesPatch {
        name_filter: Some("-a.zip/".into()),
        ..Default::default()
    })
    .unwrap();
    let cmp = w.trees(&mode);
    assert!(
        shape(&cmp).iter().all(|l| !l.contains("keep.txt")),
        "{:?}",
        shape(&cmp)
    );

    // An archive that stays a file meets the file masks.
    let mut mode = rules(ArchiveHandling::AsFiles);
    mode.apply(&hikasa_folder::FolderRulesPatch {
        name_filter: Some("*.txt".into()),
        ..Default::default()
    })
    .unwrap();
    let cmp = w.trees(&mode);
    assert!(flags(&cmp, "a.zip").contains(NodeFlags::LEFT_FILTERED));
    assert!(flags(&cmp, "a.zip").contains(NodeFlags::ARCHIVE));
}

#[test]
fn flatten_descends_opened_archives() {
    let w = world();
    let zip = zip_of(&[("d/x", b"1")]);
    w.put("/l/a.zip", &zip);
    w.put("/r/a.zip", &zip);
    let mut mode = rules(ArchiveHandling::Always);
    mode.handling.flatten = true;
    let cmp = w.trees(&mode);
    let shown = shape(&cmp);
    assert_eq!(shown, ["x file same"], "{shown:?}");
    let id = node(&cmp, "x");
    let spec = cmp.read().spec(id, hikasa_core::Side::Left).unwrap();
    assert_eq!(spec, Spec::archive_member(mspec("/l/a.zip"), "d/x"));
}

/// Step 5: the budget binds inside one folder: an archive's entries are reserved from its index before it opens, so
/// neighbours in one listing cannot all open before anything was counted.
#[test]
fn the_budget_leaves_later_archives_files() {
    let mut w = world();
    w.budget = 4;
    let zip = zip_of(&[("1", b"1"), ("2", b"2"), ("3", b"3")]);
    for i in 0..5 {
        w.put(&format!("/l/a{i}.zip"), &zip);
        w.put(&format!("/r/a{i}.zip"), &zip);
    }
    let cmp = w.trees(&rules(ArchiveHandling::Always));
    let read = cmp.read();
    let warnings = read
        .warnings()
        .iter()
        .filter(|w| w.code == WarningCode::ArchiveScanLimit)
        .count();
    assert_eq!(warnings, 1);
    drop(read);
    let shown = shape(&cmp);
    let opened = shown.iter().filter(|l| l.contains("/")).count();
    assert!(opened <= 3, "one archive of 3 fits in 4: {shown:?}");
    assert!(
        shown.iter().any(|l| l.ends_with("zip file same")),
        "{shown:?}"
    );
}

/// The folders of one archive count once, by the index, however many listings walk them; a base counts first.
#[test]
fn the_budget_counts_bases_and_synthesized_folders() {
    let mut w = world();
    w.budget = 6;
    let tree = zip_of(&[("a/b/c", b"1"), ("a/b/d", b"2")]);
    let inner = zip_of(&[("x", b"1")]);
    let outer = zip_raw_with(&[("t.zip", &tree), ("u.zip", &inner)]);
    w.put("/base.zip", &outer);
    w.put("/empty/keep", b"k");
    // The base: 2 entries. `t.zip`: 4, with two folders that only exist through it. `u.zip` passes the budget.
    let root = Spec::archive_member(mspec("/base.zip"), "");
    let cmp = w.compare(root, mspec("/empty"), &rules(ArchiveHandling::Always));
    let shown = shape(&cmp);
    assert!(
        shown.iter().any(|l| l.starts_with("t.zip/a/b/c")),
        "{shown:?}"
    );
    assert!(shown.iter().any(|l| l.starts_with("u.zip ")), "{shown:?}");
    assert!(!shown.iter().any(|l| l.starts_with("u.zip/")), "{shown:?}");
}

/// A fan of nested archives multiplies by each level, and the budget binds it: 4^3 + 4^2 + 4 + 1 archives lie in a few
/// KiB, and the scan stops where the entries say.
#[test]
fn a_fan_of_nested_archives_stops_at_the_budget() {
    let mut w = world();
    w.budget = 40;
    let mut level = zip_of(&[("leaf.txt", b"x")]);
    for _ in 0..3 {
        let copies: Vec<(String, Vec<u8>)> = (0..4)
            .map(|i| (format!("n{i}.zip"), level.clone()))
            .collect();
        let entries: Vec<(&str, &Vec<u8>)> = copies.iter().map(|(n, d)| (n.as_str(), d)).collect();
        level = zip_raw_with(&entries);
    }
    w.put("/l/fan.zip", &level);
    w.put("/r/keep", b"k");
    let cmp = w.trees(&rules(ArchiveHandling::Always));
    let read = cmp.read();
    let warned = read
        .warnings()
        .iter()
        .filter(|w| w.code == WarningCode::ArchiveScanLimit)
        .count();
    assert_eq!(warned, 1);
    drop(read);
    let rows = shape(&cmp).len();
    assert!(
        rows <= 45,
        "{rows}: the entries of the budget, and the files that stayed"
    );
}

/// G4 at the start of a scan: a base inside an archive is indexed where the job's token reaches it.
#[test]
fn a_base_inside_an_archive_stops_with_the_job() {
    let mut w = world();
    w.budget = u64::MAX;
    w.put("/l.tar", &tar_of(&[("x", &vec![0u8; 256 << 20])]));
    w.put("/r/keep", b"k");
    let cancel = CancelToken::new();
    cancel.cancel();
    let contents: ContentEngines = engines(1 << 20);
    let now = default_now();
    let progress = NullProgress;
    let env = FolderEnv {
        vfs: &w.vfs,
        content: &contents,
        pool: None,
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &NullObserver,
        now: &now,
        content_budget: 1 << 30,
        archive_entries_max: w.budget,
    };
    let root = Spec::archive_member(mspec("/l.tar"), "");
    let model = FolderModel::new(root, mspec("/r"), FolderRules::default()).unwrap();
    let error = compare_into(&model, &env).unwrap_err();
    assert!(
        matches!(error, hikasa_folder::FolderError::Cancelled),
        "{error:?}"
    );
    assert_eq!(w.vfs.counters().index_passes, 0, "stopped before the pass");
}

/// FC-025: the zip's central value as it is, no member decoded.
#[test]
fn crc_of_zip_against_zip_decodes_nothing() {
    let w = world();
    let zip = zip_of(&[("x", b"one"), ("y", b"two")]);
    w.put("/l.zip", &zip);
    w.put("/r.zip", &zip);
    w.put("/d/x", b"one");
    w.put("/d/y", b"different");
    let mut crc = FolderRules::default();
    crc.apply(&hikasa_folder::FolderRulesPatch {
        content_method: Some(hikasa_core::ContentMethod::Crc),
        skip_if_quick_same: Some(false),
        override_quick: Some(true),
        ..Default::default()
    })
    .unwrap();
    let root = |name: &str| Spec::archive_member(mspec(name), "");
    let cmp = w.compare(root("/l.zip"), root("/r.zip"), &crc);
    assert!(
        shape(&cmp).iter().all(|l| l.ends_with(SAME)),
        "{:?}",
        shape(&cmp)
    );
    let counters = w.vfs.counters();
    assert_eq!((counters.member_decodes, counters.memory_reads), (0, 0));

    // Zip against local: only the local side is read.
    let cmp = w.compare(root("/l.zip"), mspec("/d"), &crc);
    let shown = shape(&cmp);
    assert!(shown.contains(&"x file same".to_owned()), "{shown:?}");
    assert!(!shown.contains(&"y file same".to_owned()), "{shown:?}");
    assert_eq!(w.vfs.counters().member_decodes, 0);
}

#[test]
fn crc_of_tar_against_tar_reuses_the_index() {
    let w = world();
    let tar = tar_of(&[("x", b"one")]);
    w.put("/l.tar", &tar);
    w.put("/r.tar", &tar);
    let mut crc = FolderRules::default();
    crc.apply(&hikasa_folder::FolderRulesPatch {
        content_method: Some(hikasa_core::ContentMethod::Crc),
        skip_if_quick_same: Some(false),
        ..Default::default()
    })
    .unwrap();
    let root = |name: &str| Spec::archive_member(mspec(name), "");
    let cmp = w.compare(root("/l.tar"), root("/r.tar"), &crc);
    assert_eq!(shape(&cmp), ["x file same"]);
    let counters = w.vfs.counters();
    assert_eq!(counters.index_passes, 2);
    assert_eq!(counters.range_reads, 0, "no content read");
}

/// A corrupt member with a good central CRC is "same" for the CRC test and an error for the binary one.
#[test]
fn a_corrupt_member_with_a_good_central_crc() {
    let w = world();
    let good = zip_of(&[("x", b"content")]);
    let bad = zip_raw(&[("x", b"CONTENT", crc32(b"content"), 0)]);
    w.put("/l.zip", &good);
    w.put("/r.zip", &bad);
    let root = |name: &str| Spec::archive_member(mspec(name), "");
    let with = |method| {
        let mut rules = FolderRules::default();
        rules
            .apply(&hikasa_folder::FolderRulesPatch {
                content_method: Some(method),
                skip_if_quick_same: Some(false),
                ..Default::default()
            })
            .unwrap();
        rules
    };
    let cmp = w.compare(
        root("/l.zip"),
        root("/r.zip"),
        &with(hikasa_core::ContentMethod::Crc),
    );
    assert_eq!(shape(&cmp), ["x file same"]);
    let cmp = w.compare(
        root("/l.zip"),
        root("/r.zip"),
        &with(hikasa_core::ContentMethod::Binary),
    );
    let shown = shape(&cmp);
    assert_ne!(shown, ["x file same"], "{shown:?}");
}

/// A5.9: an error row for encrypted members and a normal row for the plain one.
#[test]
fn encrypted_members_give_an_error_row() {
    let w = world();
    let zip = zip_raw(&[
        ("crypt", b"cipher", crc32(b"cipher"), 1),
        ("plain", b"plain", crc32(b"plain"), 0),
    ]);
    w.put("/l.zip", &zip);
    w.put("/r.zip", &zip);
    let mut binary = FolderRules::default();
    binary
        .apply(&hikasa_folder::FolderRulesPatch {
            content_method: Some(hikasa_core::ContentMethod::Binary),
            skip_if_quick_same: Some(false),
            ..Default::default()
        })
        .unwrap();
    let root = |name: &str| Spec::archive_member(mspec(name), "");
    let cmp = w.compare(root("/l.zip"), root("/r.zip"), &binary);
    let read = cmp.read();
    let crypt = read.node(node(&cmp, "crypt")).unwrap();
    assert_ne!(crypt.state, CompareState::Same, "{crypt:?}");
    let plain = read.node(node(&cmp, "plain")).unwrap();
    assert_eq!(plain.state, CompareState::Same);
    assert!(
        read.warnings()
            .iter()
            .any(|w| w.code == WarningCode::ArchiveEncrypted)
    );
}

/// `folder compare a.zip b/` and `a.zip b.tar` equal the comparison of the extracted trees.
#[test]
fn archives_equal_their_extracted_trees() {
    let w = world();
    let files: [(&str, &[u8]); 3] = [("x", b"one"), ("d/y", b"two"), ("d/e/z", b"three")];
    w.put("/a.zip", &zip_of(&files));
    w.put("/b.tar", &tar_of(&files));
    for (name, data) in files {
        w.put(&format!("/l/{name}"), data);
        w.put(&format!("/r/{name}"), data);
    }
    let root = |name: &str| Spec::archive_member(mspec(name), "");
    let mut crc = FolderRules::default();
    crc.apply(&hikasa_folder::FolderRulesPatch {
        content_method: Some(hikasa_core::ContentMethod::Crc),
        skip_if_quick_same: Some(false),
        ..Default::default()
    })
    .unwrap();
    let extracted = shape(&w.trees(&crc));
    assert_eq!(
        shape(&w.compare(root("/a.zip"), mspec("/r"), &crc)),
        extracted
    );
    assert_eq!(
        shape(&w.compare(root("/a.zip"), root("/b.tar"), &crc)),
        extracted
    );
}

#[test]
fn rules_round_trip_with_the_new_keys() {
    let mut rules = FolderRules::default();
    rules.handling.archives = ArchiveHandling::Always;
    rules.open_archives = vec!["a.zip".into(), "a.zip/b.tar".into()];
    let text = rules.to_toml();
    let (back, warnings) = FolderRules::from_toml(&text).unwrap();
    assert_eq!(back, rules);
    assert!(warnings.is_empty(), "{warnings:?}");
    let (sparse, warnings) =
        FolderRules::from_toml("schema_version = 1\n[handling]\narchives = \"as_files\"\n")
            .unwrap();
    assert_eq!(sparse.handling.archives, ArchiveHandling::AsFiles);
    assert!(sparse.open_archives.is_empty());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ArchiveHandling::default(), ArchiveHandling::OnceOpened);
}

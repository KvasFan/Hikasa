//! Folder properties P-FOLDER-1..8 (§6.2.6) on random tree pairs in `MemoryVfs` (case variants, NFC/NFD names,
//! extensions, kind mismatches, symlinks incl. loops and broken links, injected faults) under random rules.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use common::*;
use hikasa_core::{CancelToken, CaseSensitivity, CompareVerdict, Ctl, NullProgress};
use hikasa_folder::{
    CanonicalEntry, CanonicalError, CanonicalNode, Category, DisplayOptions, FolderComparison,
    FolderDisplayFilter, FolderEnv, FolderMode, FolderModel, FolderRules, FolderToggles, NodeId,
    NodeKind, ScanEvent, ScanObserver, TreeRead, VisibleRow, check_tree_invariants, compare_into,
};
use hikasa_vfs::{Fault, ListingOrder, MemoryVfs, VfsOp};
use proptest::prelude::*;

/// Paths of the generated entries of one side (for fault injection), in generation order.
fn gen_paths(nodes: &[GenNode], left: bool) -> Vec<String> {
    fn walk(nodes: &[GenNode], prefix: &str, left: bool, out: &mut Vec<String>) {
        for n in nodes {
            let present = if left { n.left_kind } else { n.right_kind };
            if present.is_none() {
                continue;
            }
            let path = format!("{prefix}/{}", NAMES[n.name]);
            out.push(path.clone());
            walk(&n.children, &path, left, out);
        }
    }
    let mut out = Vec::new();
    walk(nodes, if left { "/l" } else { "/r" }, left, &mut out);
    out
}

/// Faults: (left side?, path index, op 0 list / 1 stat / 2 open).
fn gen_faults() -> impl Strategy<Value = Vec<(bool, usize, u8)>> {
    proptest::collection::vec((any::<bool>(), 0usize..64, 0u8..3), 0..3)
}

fn inject(vfs: &MemoryVfs, nodes: &[GenNode], faults: &[(bool, usize, u8)]) {
    for &(left, index, op) in faults {
        let paths = gen_paths(nodes, left);
        if paths.is_empty() {
            continue;
        }
        let path = &paths[index % paths.len()];
        vfs.inject(Fault {
            path: host(path),
            op: match op {
                0 => VfsOp::List,
                1 => VfsOp::Stat,
                _ => VfsOp::Open,
            },
            kind: hikasa_core::ErrorKind::PermissionDenied,
        });
    }
}

fn fs_case(insensitive: bool) -> CaseSensitivity {
    if insensitive {
        CaseSensitivity::Insensitive
    } else {
        CaseSensitivity::Sensitive
    }
}

fn run(
    vfs: &MemoryVfs,
    rules: &FolderRules,
    pool: Option<&rayon::ThreadPool>,
    swapped: bool,
) -> FolderComparison {
    let engines = engines(16);
    let now = default_now();
    let cancel = CancelToken::new();
    run_on(
        vfs,
        rules,
        pool,
        &engines,
        &now,
        &hikasa_folder::NullObserver,
        &cancel,
        swapped,
    )
    .unwrap()
}

/// Rules with the time-zone offset (if any) moved to the other side.
fn swap_rules(rules: &FolderRules) -> FolderRules {
    let mut out = rules.clone();
    if let Some(offset) = &mut out.criteria.timestamps.offset {
        offset.side = offset.side.opposite();
    }
    out
}

/// Every display option combination: 13 presets plus three toggle sets × modes × iu × suppress.
fn all_display_options() -> Vec<DisplayOptions> {
    let mut filters: Vec<FolderDisplayFilter> = FolderDisplayFilter::PRESETS.to_vec();
    for bits in [
        FolderToggles::SAME | FolderToggles::DIFFERENCES,
        FolderToggles::LEFT_NEWER | FolderToggles::RIGHT_ORPHANS,
        FolderToggles::DIFFERENCES,
    ] {
        filters.push(FolderDisplayFilter::Toggles { bits: bits.bits() });
    }
    let mut out = Vec::new();
    for filter in filters {
        for folders in FolderMode::ALL {
            for ignore_unimportant in [false, true] {
                for suppress_filters in [false, true] {
                    out.push(DisplayOptions {
                        filter,
                        folders,
                        ignore_unimportant,
                        suppress_filters,
                    });
                }
            }
        }
    }
    out
}

/// The recursive reference of §2.7 written against the public API only.
fn reference_rows(read: &TreeRead<'_>, opts: &DisplayOptions) -> Vec<VisibleRow> {
    let toggles = opts.filter.toggles();
    let iu = opts.ignore_unimportant;
    let is_folder = |id: NodeId| read.node(id).is_some_and(|n| n.kind == NodeKind::Folder);
    fn has_visible_file(read: &TreeRead<'_>, id: NodeId, opts: &DisplayOptions) -> bool {
        read.children(id).any(|c| {
            if read.node(c).is_some_and(|n| n.kind == NodeKind::Folder) {
                has_visible_file(read, c, opts)
            } else {
                let cat = read.category(c, opts.ignore_unimportant);
                if cat == Category::Filtered {
                    opts.suppress_filters
                } else {
                    opts.filter.toggles().accepts(cat)
                }
            }
        })
    }
    fn visible(read: &TreeRead<'_>, id: NodeId, opts: &DisplayOptions) -> bool {
        let cat = read.category(id, opts.ignore_unimportant);
        if cat == Category::Filtered {
            return opts.suppress_filters;
        }
        if read.node(id).is_none_or(|n| n.kind != NodeKind::Folder) {
            return opts.filter.toggles().accepts(cat);
        }
        match opts.folders {
            FolderMode::AlwaysShowFolders => true,
            FolderMode::CompareStructure => {
                opts.filter.toggles().accepts(cat)
                    || read.children(id).any(|c| visible(read, c, opts))
            }
            FolderMode::OnlyFiles => has_visible_file(read, id, opts),
        }
    }
    fn walk(
        read: &TreeRead<'_>,
        id: NodeId,
        depth: u32,
        opts: &DisplayOptions,
        out: &mut Vec<VisibleRow>,
    ) {
        for c in read.children(id) {
            if visible(read, c, opts) {
                out.push(VisibleRow { node: c, depth });
                walk(read, c, depth + 1, opts, out);
            }
        }
    }
    let _ = (toggles, iu, is_folder(read.root()));
    let mut out = Vec::new();
    walk(read, read.root(), 0, opts, &mut out);
    out
}

// ----------------------------------------------------------------------------------------------------------
// Event replay (P-FOLDER-4) and per-event invariants (P-FOLDER-6)
// ----------------------------------------------------------------------------------------------------------

/// A node as seen at an event: its canonical fields and its child ids.
#[derive(Debug, Clone)]
struct Snap {
    node: CanonicalNode,
    children: Vec<NodeId>,
}

fn snapshot(read: &TreeRead<'_>, id: NodeId) -> Snap {
    let view = read.node(id).unwrap();
    let root = id == NodeId::ROOT;
    let entry = |e: &hikasa_folder::EntryView<'_>| CanonicalEntry {
        name: if root {
            String::new()
        } else {
            e.name.to_owned()
        },
        kind: e.kind,
        size: if root { 0 } else { e.size },
        dir: e.dir.map(str::to_owned),
    };
    Snap {
        node: CanonicalNode {
            name: if root {
                String::new()
            } else {
                view.representative()
                    .map(|e| e.name.to_owned())
                    .unwrap_or_default()
            },
            kind: view.kind,
            state: view.state,
            scan: view.scan,
            diffs: view.diffs.names().into_iter().map(str::to_owned).collect(),
            time: view.time,
            content: view.content.map(|c| c.kind),
            flags: view.flags.names().into_iter().map(str::to_owned).collect(),
            errors: read
                .errors(id)
                .iter()
                .map(|e| CanonicalError {
                    side: e.side,
                    kind: e.error.kind,
                })
                .collect(),
            left: view.left.as_ref().map(entry),
            right: view.right.as_ref().map(entry),
            children: Vec::new(),
        },
        children: read.children(id).collect(),
    }
}

struct Replay {
    model: Arc<FolderModel>,
    mirror: Mutex<BTreeMap<NodeId, Snap>>,
    events: Mutex<Vec<ScanEvent>>,
    listed: Mutex<BTreeSet<NodeId>>,
    order_violations: Mutex<Vec<String>>,
    /// P-FOLDER-6: check the invariants at every event.
    check: bool,
    invariant_failures: Mutex<Vec<String>>,
}

impl Replay {
    fn new(model: Arc<FolderModel>, check: bool) -> Replay {
        Replay {
            model,
            mirror: Mutex::new(BTreeMap::new()),
            events: Mutex::new(Vec::new()),
            listed: Mutex::new(BTreeSet::new()),
            order_violations: Mutex::new(Vec::new()),
            check,
            invariant_failures: Mutex::new(Vec::new()),
        }
    }

    fn take(&self, read: &TreeRead<'_>, id: NodeId, mirror: &mut BTreeMap<NodeId, Snap>) {
        mirror.insert(id, snapshot(read, id));
        for a in read.ancestors(id) {
            mirror.insert(a, snapshot(read, a));
        }
    }

    fn assemble(&self, id: NodeId) -> CanonicalNode {
        let mirror = self.mirror.lock().unwrap();
        fn build(mirror: &BTreeMap<NodeId, Snap>, id: NodeId) -> CanonicalNode {
            let snap = mirror
                .get(&id)
                .unwrap_or_else(|| panic!("node {id:?} never announced"));
            let mut node = snap.node.clone();
            node.children = snap.children.iter().map(|c| build(mirror, *c)).collect();
            node
        }
        build(&mirror, id)
    }
}

impl ScanObserver for Replay {
    fn on_event(&self, event: ScanEvent) {
        let read = self.model.read();
        let mut mirror = self.mirror.lock().unwrap();
        let mut listed = self.listed.lock().unwrap();
        match &event {
            ScanEvent::Started => {
                if !self.events.lock().unwrap().is_empty() {
                    self.order_violations
                        .lock()
                        .unwrap()
                        .push("Started is not first".into());
                }
                self.take(&read, NodeId::ROOT, &mut mirror);
            }
            ScanEvent::FolderListed { folder } => {
                let parent = read.node(*folder).and_then(|n| n.parent);
                if let Some(p) = parent
                    && !listed.contains(&p)
                {
                    self.order_violations
                        .lock()
                        .unwrap()
                        .push(format!("FolderListed {folder:?} before its parent"));
                }
                listed.insert(*folder);
                self.take(&read, *folder, &mut mirror);
                for c in read.children(*folder) {
                    mirror.insert(c, snapshot(&read, c));
                }
            }
            ScanEvent::NodesUpdated { nodes } => {
                assert!(nodes.len() <= 256);
                for id in nodes {
                    let parent = read.node(*id).and_then(|n| n.parent);
                    if parent.is_some_and(|p| !listed.contains(&p)) {
                        self.order_violations
                            .lock()
                            .unwrap()
                            .push(format!("{id:?} updated before its folder was listed"));
                    }
                    self.take(&read, *id, &mut mirror);
                }
            }
            ScanEvent::Finished { .. } => {}
        }
        if self.check
            && let Err(e) = check_tree_invariants(&read)
        {
            self.invariant_failures.lock().unwrap().push(e.to_string());
        }
        drop(listed);
        drop(mirror);
        drop(read);
        self.events.lock().unwrap().push(event);
    }
}

fn run_replay(
    vfs: &MemoryVfs,
    rules: &FolderRules,
    pool: Option<&rayon::ThreadPool>,
    check: bool,
) -> (FolderComparison, Replay) {
    let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
    let replay = Replay::new(Arc::clone(&model), check);
    let engines = engines(16);
    let now = default_now();
    let cancel = CancelToken::never();
    let progress = NullProgress;
    let env = FolderEnv {
        vfs,
        content: &engines,
        pool,
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &replay,
        now: &now,
        content_budget: 1 << 20,
    };
    let cmp = compare_into(&model, &env).unwrap();
    (cmp, replay)
}

/// Visible, unfiltered side entries `(side, dir/name)` under `opts`.
fn visible_entries(cmp: &FolderComparison, opts: &DisplayOptions) -> BTreeSet<(bool, String)> {
    let read = cmp.read();
    let mut rows = Vec::new();
    read.visible_rows(opts, &|_| true, &mut rows);
    let mut out = BTreeSet::new();
    for row in rows {
        let view = read.node(row.node).unwrap();
        for (left, entry) in [(true, view.left), (false, view.right)] {
            if let Some(e) = entry
                && !e.filtered
            {
                out.insert((left, format!("{}/{}", e.dir.unwrap_or(""), e.name)));
            }
        }
    }
    out
}

/// Visible paths under `opts`.
fn visible_paths(cmp: &FolderComparison, opts: &DisplayOptions) -> BTreeSet<String> {
    let read = cmp.read();
    let mut rows = Vec::new();
    read.visible_rows(opts, &|_| true, &mut rows);
    rows.iter()
        .map(|r| read.path(r.node).as_str().to_owned())
        .collect()
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_folder.txt"))]

    /// P-FOLDER-1: `check_invariants` holds on random tree pairs (with faults, on a pool).
    #[test]
    fn p_folder_1_invariants(
        nodes in gen_tree(),
        gr in gen_rules(),
        faults in gen_faults(),
        insensitive in proptest::bool::weighted(0.2),
        threads in prop_oneof![Just(0usize), Just(4usize)],
    ) {
        let vfs = materialize_gen(&nodes, fs_case(insensitive), gr.unix);
        inject(&vfs, &nodes, &faults);
        let pool = pool(threads);
        let cmp = run(&vfs, &gr.rules(true), pool.as_ref(), false);
        prop_assert!(cmp.is_complete());
        if let Err(e) = cmp.check_invariants() {
            return Err(TestCaseError::fail(e.to_string()));
        }
    }

    /// P-FOLDER-2: the canonical tree is identical across pools (None/1/8) × listing orders.
    #[test]
    fn p_folder_2_parallel_equals_sequential(
        nodes in gen_tree(),
        gr in gen_rules(),
        faults in gen_faults(),
        seed in any::<u64>(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        inject(&vfs, &nodes, &faults);
        let rules = gr.rules(true);
        let reference = run(&vfs, &rules, None, false);
        let canonical = reference.to_canonical();
        let pools = [pool(0), pool(1), pool(8)];
        for order in [ListingOrder::Sorted, ListingOrder::Reversed, ListingOrder::Shuffled { seed }] {
            vfs.set_listing_order(order);
            for p in &pools {
                let other = run(&vfs, &rules, p.as_ref(), false);
                prop_assert_eq!(&other.to_canonical(), &canonical);
                prop_assert_eq!(other.stats(false), reference.stats(false));
                let (a, b) = (other.read(), reference.read());
                prop_assert_eq!(a.warnings(), b.warnings());
            }
        }
    }

    /// P-FOLDER-3: swapping the sides mirrors the result (methods None/CRC/Binary).
    #[test]
    fn p_folder_3_swap_mirrors(
        nodes in gen_tree(),
        gr in gen_rules(),
        faults in gen_faults(),
        insensitive in proptest::bool::weighted(0.2),
    ) {
        let vfs = materialize_gen(&nodes, fs_case(insensitive), gr.unix);
        inject(&vfs, &nodes, &faults);
        let rules = gr.rules(false);
        let mut direct = mirror(&run(&vfs, &rules, None, false).to_canonical());
        let mut swapped = run(&vfs, &swap_rules(&rules), None, true).to_canonical();
        normalize_errors(&mut direct);
        normalize_errors(&mut swapped);
        prop_assert_eq!(swapped, direct);
    }

    /// P-FOLDER-4: replaying the events against the live model ends with exactly `to_canonical()` (INV-EVT-1..3).
    #[test]
    fn p_folder_4_event_replay(
        nodes in gen_tree(),
        gr in gen_rules(),
        faults in gen_faults(),
        threads in prop_oneof![Just(0usize), Just(4usize)],
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        inject(&vfs, &nodes, &faults);
        let pool = pool(threads);
        let (cmp, replay) = run_replay(&vfs, &gr.rules(true), pool.as_ref(), false);
        let events = replay.events.lock().unwrap().clone();
        prop_assert_eq!(events.first(), Some(&ScanEvent::Started));
        prop_assert_eq!(events.last(), Some(&ScanEvent::Finished { complete: true }));
        prop_assert!(replay.order_violations.lock().unwrap().is_empty(), "{:?}", replay.order_violations.lock().unwrap());
        let canonical = cmp.to_canonical();
        prop_assert_eq!(replay.assemble(NodeId::ROOT), canonical.root);
    }

    /// P-FOLDER-5: more exclusions never add visible nodes (display All; also Differences when no FC-054 pair can
    /// arise from extension alignment).
    #[test]
    fn p_folder_5_exclusions_never_add(
        nodes in gen_tree(),
        gr in gen_rules(),
        extra in prop_oneof![Just("-*.txt"), Just(r"-a\"), Just("-b*"), Just(r"-*;-d\")],
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        let rules = gr.rules(true);
        let mut more = rules.clone();
        let mut filter = FILTERS[usize::from(gr.filter)].to_owned();
        if !filter.is_empty() {
            filter.push(';');
        }
        filter.push_str(extra);
        more.apply(&hikasa_folder::FolderRulesPatch {
            name_filter: Some(filter),
            ..Default::default()
        }).unwrap();
        let base = run(&vfs, &rules, None, false);
        let fewer = run(&vfs, &more, None, false);
        if gr.flatten {
            // Flatten pairs files by name across folders, so an exclusion can change which files pair (and so a
            // node's path); compare the visible entries instead.
            for folders in FolderMode::ALL {
                let opts = DisplayOptions {
                    filter: FolderDisplayFilter::All,
                    folders,
                    ignore_unimportant: false,
                    suppress_filters: false,
                };
                let a = visible_entries(&base, &opts);
                let b = visible_entries(&fewer, &opts);
                prop_assert!(b.is_subset(&a), "{:?}: {:?}", opts, b.difference(&a).collect::<Vec<_>>());
            }
            return Ok(());
        }
        let mut filters = vec![FolderDisplayFilter::All];
        if !gr.align_extensions {
            filters.push(FolderDisplayFilter::Differences);
        }
        for filter in filters {
            for folders in FolderMode::ALL {
                for ignore_unimportant in [false, true] {
                    let opts = DisplayOptions { filter, folders, ignore_unimportant, suppress_filters: false };
                    let a = visible_paths(&base, &opts);
                    let b = visible_paths(&fewer, &opts);
                    prop_assert!(b.is_subset(&a), "{:?}: {:?} not in {:?}", opts, b.difference(&a).collect::<Vec<_>>(), a);
                }
            }
        }
    }

    /// P-FOLDER-6: counters and totals equal a recount after every applied batch (checked at every event on a
    /// pool), and the statistics equal a recount at the end.
    #[test]
    fn p_folder_6_counters_at_every_event(
        nodes in gen_tree(),
        gr in gen_rules(),
        faults in gen_faults(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        inject(&vfs, &nodes, &faults);
        let pool = pool(4);
        let (cmp, replay) = run_replay(&vfs, &gr.rules(true), pool.as_ref(), true);
        prop_assert!(replay.invariant_failures.lock().unwrap().is_empty(), "{:?}", replay.invariant_failures.lock().unwrap());
        if let Err(e) = cmp.check_invariants() {
            return Err(TestCaseError::fail(e.to_string()));
        }
        // Statistics equal a recount from the nodes.
        let read = cmp.read();
        let mut files = hikasa_folder::CategoryCounts::default();
        let mut folders = hikasa_folder::CategoryCounts::default();
        for (id, _) in read.preorder() {
            let cat = read.category(id, false);
            let counts = if read.node(id).unwrap().kind == NodeKind::Folder { &mut folders } else { &mut files };
            match cat {
                Category::Same => counts.same += 1,
                Category::LeftNewer => counts.left_newer += 1,
                Category::RightNewer => counts.right_newer += 1,
                Category::Different => counts.different += 1,
                Category::LeftOrphan => counts.left_orphan += 1,
                Category::RightOrphan => counts.right_orphan += 1,
                Category::Unknown => counts.unknown += 1,
                Category::Error => counts.error += 1,
                Category::Filtered => counts.filtered += 1,
            }
        }
        let stats = read.stats(false);
        prop_assert_eq!(stats.files, files);
        prop_assert_eq!(stats.folders, folders);
    }

    /// P-FOLDER-7: `visible_rows` equals a recursive reference for every filter, mode, iu and suppress value, on
    /// complete and partial (cancelled) trees.
    #[test]
    fn p_folder_7_visible_rows(
        nodes in gen_tree(),
        gr in gen_rules(),
        cancel_after in proptest::option::of(1usize..4),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        let rules = gr.rules(true);
        let engines = engines(16);
        let now = default_now();
        let cancel = CancelToken::new();
        let cmp = match cancel_after {
            Some(after) => {
                let observer = CancelAfter { after, seen: Default::default(), cancel: &cancel };
                run_on(&vfs, &rules, None, &engines, &now, &observer, &cancel, false).unwrap()
            }
            None => run(&vfs, &rules, None, false),
        };
        let read = cmp.read();
        for opts in all_display_options() {
            let mut rows = Vec::new();
            read.visible_rows(&opts, &|_| true, &mut rows);
            prop_assert_eq!(&rows, &reference_rows(&read, &opts), "{:?}", opts);
            // A collapsed folder hides its children only.
            let mut collapsed = Vec::new();
            read.visible_rows(&opts, &|_| false, &mut collapsed);
            let top: Vec<VisibleRow> = rows.iter().copied().filter(|r| r.depth == 0).collect();
            prop_assert_eq!(collapsed, top);
        }
    }

    /// P-FOLDER-8: ignore-unimportant monotonicity (INV-FOLDER-10) per node and for the verdict.
    #[test]
    fn p_folder_8_ignore_unimportant_monotone(
        nodes in gen_tree(),
        gr in gen_rules(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, gr.unix);
        let mut rules = gr.rules(true);
        rules.criteria.content.method = hikasa_core::ContentMethod::Rules;
        let cmp = run(&vfs, &rules, None, false);
        let read = cmp.read();
        for (id, _) in read.preorder() {
            let without = read.category(id, false);
            let with = read.category(id, true);
            prop_assert!(!with.is_difference() || without.is_difference(), "{:?}: {:?} / {:?}", read.path(id), without, with);
            if !without.is_difference() {
                prop_assert_eq!(with, without);
            }
        }
        if cmp.verdict(true) == CompareVerdict::Differences {
            prop_assert_eq!(cmp.verdict(false), CompareVerdict::Differences);
        }
        prop_assert_ne!(cmp.verdict(true), CompareVerdict::UnimportantOnly);
    }
}

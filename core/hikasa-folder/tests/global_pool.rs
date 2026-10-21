//! Global-pool trap (§5.1, T7): this test binary configures the rayon global pool with recognisable thread
//! names, then runs folder comparisons with explicit pools. The VFS wrapper and the content comparator record the
//! thread they run on; any work on a global-pool thread is an implicit use of the global pool and fails the test.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use hikasa_core::{
    CancelToken, ContentComparator, ContentMethod, ContentOutcome, ContentSource, Ctl,
};
use hikasa_folder::{ContentEngines, FolderRules, FolderRulesPatch, NullObserver};

static THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn note() {
    let name = std::thread::current().name().unwrap_or("").to_owned();
    THREADS.lock().unwrap().push(name);
}

/// The fake rules comparator, recording its thread.
struct Recording(FakeRules);

impl ContentComparator for Recording {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, hikasa_core::Cancelled> {
        note();
        self.0.compare(left, right, ctl)
    }
}

#[test]
fn scans_use_only_the_pool_they_are_given() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|i| format!("global-trap-{i}"))
        .build_global()
        .unwrap();
    let tree: TreeFile = toml::from_str(
        &std::fs::read_to_string(fixtures("trees").join("28_content_rules/tree.toml")).unwrap(),
    )
    .unwrap();
    let vfs = materialize(&tree);
    let counting = CountingVfs::new(&vfs);
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            content_method: Some(ContentMethod::Rules),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let engines = ContentEngines {
        rules: Some(Arc::new(Recording(FakeRules { limit: 16 }))),
        ..engines(16)
    };
    let now = default_now();
    let cancel = CancelToken::never();
    for threads in [1, 4] {
        let pool = pool(threads).unwrap();
        let cmp = run_on(
            &counting,
            &rules,
            Some(&pool),
            &engines,
            &now,
            &NullObserver,
            &cancel,
            false,
        )
        .unwrap();
        cmp.check_invariants().unwrap();
        let mut flat = rules.clone();
        flat.handling.flatten = true;
        run_on(
            &counting,
            &flat,
            Some(&pool),
            &engines,
            &now,
            &NullObserver,
            &cancel,
            false,
        )
        .unwrap();
    }
    let mut seen = THREADS.lock().unwrap().clone();
    seen.extend(counting.threads.lock().unwrap().iter().cloned());
    assert!(!seen.is_empty());
    let bad: Vec<&String> = seen
        .iter()
        .filter(|n| n.starts_with("global-trap"))
        .collect();
    assert!(bad.is_empty(), "work on the global pool: {bad:?}");
}

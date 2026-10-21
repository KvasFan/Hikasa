//! Global-pool trap (§5.1, T7): this test binary configures the rayon global pool with recognisable thread
//! names, then runs the parallel public API with explicit pools. Every callback records the thread it runs on;
//! any work on a global-pool thread is an implicit use of the global pool and fails the test.

use std::sync::Mutex;

use hikasa_core::cancel::CancelToken;
use hikasa_formats::{FormatId, FormatRegistry, Token, tokenize_all, tokenize_all_with_chunk};

static VIOLATIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record(_: u32, _: &str, _: &[Token], out: &mut Vec<u8>) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("");
    if name.starts_with("global-trap") {
        VIOLATIONS.lock().unwrap().push(name.to_owned());
    }
    out.push(1);
}

#[test]
fn parallel_apis_use_only_the_pool_they_are_given() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|i| format!("global-trap-{i}"))
        .build_global()
        .unwrap();
    let registry = FormatRegistry::builtin().unwrap();
    let c = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
    let lines: Vec<String> = (0..3000)
        .map(|k| {
            if k % 7 == 0 {
                "/* open".to_owned()
            } else {
                format!("int x{k} = {k}; */")
            }
        })
        .collect();
    let cancel = CancelToken::never();
    let sequential = tokenize_all(c.grammar(), &lines, None, &cancel, record).unwrap();
    for threads in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("explicit-{i}"))
            .build()
            .unwrap();
        let parallel =
            tokenize_all_with_chunk(c.grammar(), &lines, Some(&pool), &cancel, 64, record).unwrap();
        assert_eq!(parallel, sequential);
    }
    assert!(
        VIOLATIONS.lock().unwrap().is_empty(),
        "{:?}",
        VIOLATIONS.lock().unwrap()
    );
}

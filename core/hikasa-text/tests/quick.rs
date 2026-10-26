//! `RulesComparator` (§1.5.6, §2.19): the injected rules-based `ContentComparator` — D-15 effective format and
//! `format_mismatch`, D-11 byte comparison of non-text kinds, the D-25 size limit with `rules_size_limit`,
//! read failures, forced formats and enabled overrides; it agrees with `quick_compare_bytes`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::{
    CaseSensitivity, ContentComparator, ContentMethod, ContentResult, Ctl, ErrorKind, LocalPath,
    Spec, WarningCode,
};
use hikasa_formats::{EnabledOverrides, FormatId, FormatRegistry};
use hikasa_text::{
    DEFAULT_RULES_MAX_BYTES, LoadOptions, RulesComparator, TextPattern, TextRules, UnimportantItem,
    quick_compare_bytes,
};
use hikasa_vfs::{Fault, FollowLinks, MemMeta, MemoryVfs, Vfs, VfsOp, VfsSource};

fn spec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(common::host(path))).unwrap())
}

fn files() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let put = |p: &str, c: &[u8]| {
        vfs.insert_file(&common::host(p), c.to_vec(), MemMeta::default())
            .unwrap()
    };
    put("/l/a.c", b"int  x = 1; // one\n");
    put("/r/a.c", b"int x = 1; // two\n");
    put("/l/same.c", b"same\n");
    put("/r/same.c", b"same\n");
    put("/l/ws.txt", b"a  b\n");
    put("/r/ws.txt", b"a b\n");
    put("/l/p.png", b"\x89PNG\r\n\x1a\nxxxx");
    put("/r/p.png", b"\x89PNG\r\n\x1a\nyyyy");
    put("/l/x.py", b"x = 1   \n");
    put("/r/x.c", b"x = 1\n");
    put("/l/big.txt", &[b'a'; 64]);
    put("/r/big.txt", &[b'a'; 64]);
    put("/l/bad.txt", b"x\n");
    put("/r/bad.txt", b"x\n");
    vfs
}

fn source<'v>(vfs: &'v MemoryVfs, path: &str) -> VfsSource<'v> {
    let entry = vfs.stat(&spec(path), FollowLinks::No).unwrap();
    VfsSource::new(vfs, spec(path), entry)
}

fn comparator(rules: TextRules) -> RulesComparator {
    RulesComparator::new(
        Arc::new(FormatRegistry::builtin().unwrap()),
        rules,
        LoadOptions::default(),
    )
    .unwrap()
}

fn ws_off() -> TextRules {
    let mut rules = TextRules::default();
    rules
        .importance
        .apply_unimportant(&[UnimportantItem::Whitespace]);
    rules
}

#[test]
fn results_follow_the_rules() {
    let vfs = files();
    let ctl = Ctl::detached();
    let cmp = comparator(TextRules::default());
    assert_eq!(cmp.method(), ContentMethod::Rules);
    assert_eq!(cmp.size_limit(), Some(DEFAULT_RULES_MAX_BYTES));
    let run = |c: &RulesComparator, l: &str, r: &str| {
        c.compare(&source(&vfs, l), &source(&vfs, r), &ctl).unwrap()
    };
    let out = run(&cmp, "/l/same.c", "/r/same.c");
    assert_eq!(out.result, ContentResult::BinarySame);
    assert_eq!(out.method, ContentMethod::Rules);
    assert_eq!(
        run(&cmp, "/l/ws.txt", "/r/ws.txt").result,
        ContentResult::ImportantDifferences
    );
    let lenient = comparator(ws_off());
    assert_eq!(
        run(&lenient, "/l/ws.txt", "/r/ws.txt").result,
        ContentResult::UnimportantOnly
    );
    // Comments unimportant too: still unimportant only.
    let mut rules = ws_off();
    rules
        .importance
        .apply_unimportant(&[UnimportantItem::Element("Comment".into())]);
    assert_eq!(
        run(&comparator(rules), "/l/a.c", "/r/a.c").result,
        ContentResult::UnimportantOnly
    );
    // D-11: picture kinds are compared by bytes.
    let out = run(&cmp, "/l/p.png", "/r/p.png");
    assert_eq!(out.result, ContentResult::BinaryDifferent);
}

#[test]
fn d15_effective_format_and_mismatch_warning() {
    let vfs = files();
    let ctl = Ctl::detached();
    // Left x.py (Python: trailing whitespace unimportant), right x.c: Python wins, with a warning.
    let out = comparator(TextRules::default())
        .compare(&source(&vfs, "/l/x.py"), &source(&vfs, "/r/x.c"), &ctl)
        .unwrap();
    assert_eq!(out.result, ContentResult::UnimportantOnly);
    assert!(
        out.warnings
            .iter()
            .any(|w| w.code == WarningCode::FormatMismatch),
        "{:?}",
        out.warnings
    );
    // Forcing C makes the trailing blanks important again.
    let registry = Arc::new(FormatRegistry::builtin().unwrap());
    let c = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
    let forced = RulesComparator::new(
        Arc::clone(&registry),
        TextRules::default(),
        LoadOptions::default(),
    )
    .unwrap()
    .with_forced_format(c);
    let out = forced
        .compare(&source(&vfs, "/l/x.py"), &source(&vfs, "/r/x.c"), &ctl)
        .unwrap();
    assert_eq!(out.result, ContentResult::ImportantDifferences);
    assert!(out.warnings.is_empty());
    // Disabling Python through the overrides: Everything Else is detected for the left name.
    let mut overrides = EnabledOverrides::default();
    overrides.0.insert(FormatId::new("python").unwrap(), false);
    let out = comparator(TextRules::default())
        .with_overrides(overrides)
        .compare(&source(&vfs, "/l/x.py"), &source(&vfs, "/r/x.c"), &ctl)
        .unwrap();
    assert_eq!(out.result, ContentResult::ImportantDifferences);
}

#[test]
fn d25_size_limit_compares_by_bytes() {
    let vfs = files();
    let ctl = Ctl::detached();
    let small = comparator(TextRules::default()).with_max_bytes(10);
    assert_eq!(small.size_limit(), Some(10));
    let out = small
        .compare(
            &source(&vfs, "/l/big.txt"),
            &source(&vfs, "/r/big.txt"),
            &ctl,
        )
        .unwrap();
    assert_eq!(out.result, ContentResult::BinarySame);
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(out.warnings[0].code, WarningCode::RulesSizeLimit);
}

#[test]
fn read_failures_are_error_results() {
    let vfs = files();
    vfs.inject(Fault {
        path: common::host("/r/bad.txt"),
        op: VfsOp::Open,
        kind: ErrorKind::PermissionDenied,
    });
    let out = comparator(TextRules::default())
        .compare(
            &source(&vfs, "/l/bad.txt"),
            &source(&vfs, "/r/bad.txt"),
            &Ctl::detached(),
        )
        .unwrap();
    match out.result {
        ContentResult::Error { error } => {
            assert_eq!(error.kind, ErrorKind::PermissionDenied);
            assert_eq!(error.spec, Some(common::host("/r/bad.txt")));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn invalid_rules_are_rejected_up_front() {
    let mut rules = TextRules::default();
    rules
        .importance
        .unimportant
        .push(TextPattern::regex("(unclosed"));
    let err = RulesComparator::new(
        Arc::new(FormatRegistry::builtin().unwrap()),
        rules,
        LoadOptions::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("unimportant[0]"), "{err}");
}

#[test]
fn the_comparator_agrees_with_quick_compare_bytes() {
    let vfs = files();
    let registry = FormatRegistry::builtin().unwrap();
    let ctl = Ctl::detached();
    for rules in [TextRules::default(), ws_off()] {
        let cmp = comparator(rules.clone());
        for name in ["a.c", "same.c", "ws.txt"] {
            let (l, r) = (format!("/l/{name}"), format!("/r/{name}"));
            let via_trait = cmp
                .compare(&source(&vfs, &l), &source(&vfs, &r), &ctl)
                .unwrap()
                .result;
            let format = registry
                .detect(name, &EnabledOverrides::default())
                .unwrap()
                .format;
            let lb = read(&vfs, &l);
            let rb = read(&vfs, &r);
            let direct = quick_compare_bytes(
                (&lb, name),
                (&rb, name),
                &format,
                &rules,
                &LoadOptions::default(),
                &ctl,
            )
            .unwrap()
            .result;
            assert_eq!(via_trait, direct, "{name}");
        }
    }
}

fn read(vfs: &MemoryVfs, path: &str) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    vfs.open_read(&spec(path))
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    out
}

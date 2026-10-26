//! Gate G1's memory bound (§0.8, §5.7, §6.4), as an ignored test in its own binary (run in release:
//! `cargo test --release -p hikasa-text --test perf_rss -- --ignored --nocapture`, Linux only): the peak resident
//! set (`VmHWM` in `/proc/self/status`) of an end-to-end text compare of two 1,000,000-line C sources (the
//! general key path, which tokenises) must stay within
//! `3 × (L + R bytes) + 64 B × (L + R lines) + 32 MiB`. As in `Service::text_compare` (§3.2), the raw bytes are
//! dropped after `bytes_identical` and before the comparison.

#[path = "../benches/support/inputs.rs"]
mod inputs;

use std::sync::Arc;

use hikasa_core::Ctl;
use hikasa_formats::{FormatId, FormatRegistry};
use hikasa_text::{CompareInput, DocOrigin, LoadOptions, TextDocument, TextRules, compare};
use inputs::{c_pair, line_count};

/// `VmHWM` (peak resident set) in bytes.
fn vm_hwm() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    let kib: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kib * 1024
}

#[test]
#[ignore = "release-mode memory gate (Linux)"]
fn perf_rss_text_compare_1m() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let baseline = vm_hwm();
    let (left, right) = c_pair(1_000_000, 0x5EED_0001);
    let bytes = (left.len() + right.len()) as u64;
    let lines = (line_count(&left) + line_count(&right)) as u64;
    let budget = 3 * bytes + 64 * lines + (32 << 20);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let registry = FormatRegistry::builtin().unwrap();
    let format = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
    let load = LoadOptions::default();
    let doc = |b: &[u8]| TextDocument::from_bytes(b, &format, &load, DocOrigin::default()).unwrap();
    let (l, r) = pool.join(|| doc(&left), || doc(&right));
    let bytes_identical = left == right;
    drop((left, right));
    let after_load = vm_hwm();
    let input = CompareInput {
        left: Arc::new(l),
        right: Arc::new(r),
        format,
        rules: TextRules::default(),
        bytes_identical,
        force_tokenize: false,
    };
    let cmp = compare(input, Some(&pool), &Ctl::detached(), None).unwrap();
    let peak = vm_hwm();
    assert!(cmp.row_count() > 0);
    let mib = |b: u64| b as f64 / f64::from(1 << 20);
    println!(
        "gate G1 perf_rss: peak {:.1} MiB (baseline {:.1} MiB, after loading {:.1} MiB); budget {:.1} MiB for {} bytes and {} lines {}",
        mib(peak),
        mib(baseline),
        mib(after_load),
        mib(budget),
        bytes,
        lines,
        if peak <= budget { "PASS" } else { "FAIL" }
    );
    assert!(peak <= budget, "peak {peak} > budget {budget}");
}

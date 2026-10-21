//! Performance gates measured as ignored tests (run in release, one test at a time: `cargo test --release -p
//! hikasa-text --test gates -- --ignored --nocapture --test-threads=1`): G1 (1M-line text compare from bytes < 3 s), G5 (`row` and
//! `FilteredView::get` < 200 ns, `inline` p99 < 50 µs) and the text part of G4 (cancel at about 50 % of a G1
//! comparison returns within 100 ms; `g4_cancel_latency_adversarial`: the same on the adversarial inputs of the
//! wave-3 review — long-line closeness, 100 MiB lines, dense unimportant patterns, 256 MiB documents being
//! decoded, repetitive source code — plus their total times and peak memory). The criterion benches `text_compare`, `gui_enablers` and `cancel_latency`
//! measure the same things; these tests print one line per gate and fail when a gate is missed. Timing tests
//! read the clock, which only `src/` may not (§6.1 check 2).

#[path = "../benches/support/inputs.rs"]
mod inputs;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hikasa_core::{CancelToken, CaseSensitivity, Ctl, LocalPath, Side, Spec};
use hikasa_formats::{CompiledFormat, FormatId, FormatRegistry};
use hikasa_text::{
    AlgorithmChoice, CompareInput, DocOrigin, LineIdx, LoadOptions, RowIdx, TextComparison,
    TextDisplayFilter, TextDocument, TextError, TextPattern, TextRules, ViewOptions, compare,
    load_document, quick_compare_bytes,
};
use hikasa_vfs::{MemMeta, MemoryVfs};
use inputs::{XorShift, c_pair, line_count};

const LINES: usize = 1_000_000;

fn pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap()
}

fn format(id: &str) -> Arc<CompiledFormat> {
    FormatRegistry::builtin()
        .unwrap()
        .compiled(&FormatId::new(id).unwrap())
        .unwrap()
}

/// The latest phase reported (G4 prints where a cancel landed).
#[derive(Default)]
struct PhaseLog(std::sync::Mutex<Option<hikasa_core::Phase>>);

impl hikasa_core::ProgressSink for PhaseLog {
    fn report(&self, progress: hikasa_core::Progress) {
        *self.0.lock().unwrap() = Some(progress.phase);
    }
    fn due(&self, _phase: hikasa_core::Phase) -> bool {
        false
    }
}

/// End-to-end from bytes: both documents in parallel, then the comparison (raw bytes stay with the caller).
fn run(
    left: &[u8],
    right: &[u8],
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    pool: &rayon::ThreadPool,
    cancel: &CancelToken,
) -> Result<TextComparison, TextError> {
    run_logged(
        left,
        right,
        format,
        rules,
        pool,
        cancel,
        &PhaseLog::default(),
    )
}

fn run_logged(
    left: &[u8],
    right: &[u8],
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    pool: &rayon::ThreadPool,
    cancel: &CancelToken,
    log: &PhaseLog,
) -> Result<TextComparison, TextError> {
    let load = LoadOptions::default();
    let doc = |b: &[u8]| TextDocument::from_bytes(b, format, &load, DocOrigin::default());
    let (l, r) = pool.join(|| doc(left), || doc(right));
    let input = CompareInput {
        left: Arc::new(l?),
        right: Arc::new(r?),
        format: Arc::clone(format),
        rules: rules.clone(),
        bytes_identical: left == right,
        force_tokenize: false,
    };
    let ctl = Ctl {
        cancel,
        progress: log,
    };
    compare(input, Some(pool), &ctl, None)
}

/// The three G1 configurations: (name, format, rules).
fn configurations() -> Vec<(&'static str, Arc<CompiledFormat>, TextRules)> {
    let mut standard = TextRules::default();
    standard.alignment.algorithm = AlgorithmChoice::Standard;
    vec![
        (
            "text_compare_1m_auto",
            format("c-cpp"),
            TextRules::default(),
        ),
        ("text_compare_1m_standard", format("c-cpp"), standard),
        // Rust's preset has no unimportant or case-insensitive element: the identity fast path (D-53).
        (
            "text_compare_1m_identity",
            format("rust"),
            TextRules::default(),
        ),
    ]
}

#[test]
#[ignore = "release-mode performance gate"]
fn g1_text_compare_1m() {
    let (left, right) = c_pair(LINES, 0x5EED_0001);
    println!(
        "inputs: {} + {} bytes, {} + {} lines",
        left.len(),
        right.len(),
        line_count(&left),
        line_count(&right)
    );
    let pool = pool();
    let mut failed = Vec::new();
    for (name, format, rules) in configurations() {
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let start = Instant::now();
            let cmp = run(&left, &right, &format, &rules, &pool, &CancelToken::never()).unwrap();
            best = best.min(start.elapsed());
            assert!(cmp.stats().changed_rows + cmp.stats().left_only_rows > 0);
            if name.ends_with("identity") {
                assert!(cmp.used_identity_path());
            }
        }
        let pass = best < Duration::from_secs(3);
        println!(
            "gate G1 {name}: best of 3 {:.3} s (limit 3 s) {}",
            best.as_secs_f64(),
            if pass { "PASS" } else { "FAIL" }
        );
        if !pass {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
#[ignore = "release-mode performance gate"]
fn g5_gui_enablers() {
    let (left, right) = c_pair(LINES, 0x5EED_0001);
    let pool = pool();
    let cmp = run(
        &left,
        &right,
        &format("c-cpp"),
        &TextRules::default(),
        &pool,
        &CancelToken::never(),
    )
    .unwrap();
    drop((left, right));
    let rows = cmp.row_count();
    let mut rng = XorShift(7);
    let picks: Vec<u32> = (0..1_000_000)
        .map(|_| rng.below(u64::from(rows)) as u32)
        .collect();
    // row(): mean over 1M random rows.
    let start = Instant::now();
    let mut sink = 0u64;
    for &k in &picks {
        sink = sink.wrapping_add(u64::from(
            std::hint::black_box(cmp.row(RowIdx(std::hint::black_box(k)))).is_some(),
        ));
    }
    let row_ns = start.elapsed().as_nanos() as f64 / picks.len() as f64;
    // FilteredView::get(): mean over 1M random positions of the Differences and Context(3) views.
    let mut get_ns: f64 = 0.0;
    for filter in [
        TextDisplayFilter::Differences,
        TextDisplayFilter::Context { lines: 3 },
        TextDisplayFilter::All,
    ] {
        let view = cmp.view(&ViewOptions {
            filter,
            ignore_unimportant: false,
            expanded: vec![],
        });
        let len = view.len().max(1);
        let positions: Vec<u32> = picks.iter().map(|&k| k % len).collect();
        let start = Instant::now();
        for &p in &positions {
            sink = sink.wrapping_add(u64::from(
                std::hint::black_box(view.get(std::hint::black_box(p))).is_some(),
            ));
        }
        get_ns = get_ns.max(start.elapsed().as_nanos() as f64 / positions.len() as f64);
    }
    // inline(): p99 over every changed row (each call timed).
    let changed: Vec<u32> = (0..rows)
        .filter(|&k| cmp.inline(RowIdx(k)).is_some())
        .collect();
    let mut times: Vec<Duration> = changed
        .iter()
        .map(|&k| {
            let start = Instant::now();
            let inline = cmp.inline(RowIdx(k));
            let took = start.elapsed();
            sink = sink.wrapping_add(u64::from(inline.is_some()));
            took
        })
        .collect();
    times.sort();
    let p99 = times
        .get(times.len() * 99 / 100)
        .copied()
        .unwrap_or_default();
    assert!(sink > 0);
    let row_pass = row_ns < 200.0;
    let get_pass = get_ns < 200.0;
    let inline_pass = p99 < Duration::from_micros(50);
    println!(
        "gate G5 row: {row_ns:.1} ns (limit 200 ns) {}",
        if row_pass { "PASS" } else { "FAIL" }
    );
    println!(
        "gate G5 FilteredView::get: {get_ns:.1} ns (worst view; limit 200 ns) {}",
        if get_pass { "PASS" } else { "FAIL" }
    );
    println!(
        "gate G5 inline p99: {:.1} µs over {} changed rows (limit 50 µs) {}",
        p99.as_secs_f64() * 1e6,
        times.len(),
        if inline_pass { "PASS" } else { "FAIL" }
    );
    assert!(row_pass && get_pass && inline_pass);
}

#[test]
#[ignore = "release-mode performance gate"]
fn g4_cancel_latency() {
    let (left, right) = c_pair(LINES, 0x5EED_0001);
    let pool = pool();
    let mut worst_all = Duration::ZERO;
    for (name, format, rules) in configurations() {
        let start = Instant::now();
        run(&left, &right, &format, &rules, &pool, &CancelToken::never()).unwrap();
        let full = start.elapsed();
        let mut worst = Duration::ZERO;
        // Cancel at 10 %, 30 %, 50 %, 70 % and 90 % of a full run (G4 asks for 50 %; the others cover every phase).
        for percent in [10u32, 30, 50, 70, 90] {
            let cancel = CancelToken::new();
            let log = PhaseLog::default();
            let latency = thread::scope(|scope| {
                let worker = scope
                    .spawn(|| run_logged(&left, &right, &format, &rules, &pool, &cancel, &log));
                thread::sleep(full * percent / 100);
                let phase = *log.0.lock().unwrap();
                let at = Instant::now();
                cancel.cancel();
                let result = worker.join().unwrap();
                let took = at.elapsed();
                assert!(
                    matches!(result, Err(TextError::Cancelled)) || result.is_ok(),
                    "{name}: {result:?}"
                );
                println!(
                    "  {name}: cancel at {percent} % (phase {phase:?}): {:.2} ms",
                    took.as_secs_f64() * 1e3
                );
                took
            });
            worst = worst.max(latency);
        }
        println!(
            "gate G4 cancel_latency/{name}: worst of 5 {:.2} ms (limit 100 ms) {}",
            worst.as_secs_f64() * 1e3,
            if worst <= Duration::from_millis(100) {
                "PASS"
            } else {
                "FAIL"
            }
        );
        worst_all = worst_all.max(worst);
    }
    assert!(worst_all <= Duration::from_millis(100));
}

/// Peak resident memory of the process since the last reset, in MiB (Linux; 0 elsewhere).
fn peak_rss_mib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb / 1024)
}

/// Resets the peak resident memory counter (Linux `clear_refs` 5; ignored elsewhere).
fn reset_peak_rss() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// The 1-minute load average (the machine is shared; reported with every adversarial case).
fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_owned))
        .unwrap_or_default()
}

/// Runs `work` once uncancelled (total time, peak memory), then cancels it at 10 %, 50 % and 90 % of that time
/// and returns the worst latency from `cancel()` to the return. A cancelled run must be `Err(Cancelled)` or
/// (when it finished first) `Ok`.
fn adversarial_case(
    name: &str,
    work: &(dyn Fn(&CancelToken) -> Result<(), TextError> + Sync),
) -> Duration {
    reset_peak_rss();
    let start = Instant::now();
    work(&CancelToken::never()).unwrap();
    let full = start.elapsed();
    let peak = peak_rss_mib();
    let mut worst = Duration::ZERO;
    let mut parts = Vec::new();
    for percent in [10u32, 50, 90] {
        let cancel = CancelToken::new();
        let latency = thread::scope(|scope| {
            let worker = scope.spawn(|| work(&cancel));
            thread::sleep(full * percent / 100);
            let at = Instant::now();
            cancel.cancel();
            let result = worker.join().unwrap();
            let took = at.elapsed();
            assert!(
                matches!(result, Err(TextError::Cancelled) | Ok(())),
                "{name}: {result:?}"
            );
            took
        });
        parts.push(format!(
            "{percent} %: {:.1} ms",
            latency.as_secs_f64() * 1e3
        ));
        worst = worst.max(latency);
    }
    println!(
        "  {name}: full {:.0} ms, peak RSS {peak} MiB, cancel {} (load {})",
        full.as_secs_f64() * 1e3,
        parts.join(", "),
        load_average()
    );
    worst
}

/// `lines` lines of `chars` random lower-case letters.
fn random_lines(rng: &mut XorShift, lines: usize, chars: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines * (chars + 1));
    for _ in 0..lines {
        out.extend((0..chars).map(|_| b'a' + rng.below(26) as u8));
        out.push(b'\n');
    }
    out
}

/// `/l` and `/r` in a memory VFS.
fn vfs_pair(left: &[u8], right: &[u8]) -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_file("/l", left.to_vec(), MemMeta::default())
        .unwrap();
    vfs.insert_file("/r", right.to_vec(), MemMeta::default())
        .unwrap();
    vfs
}

/// The production path of a text compare: both documents through `load_document` (cancellable while reading,
/// detecting, decoding and building), in parallel, then `compare`.
fn compare_loaded(
    vfs: &MemoryVfs,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    pool: &rayon::ThreadPool,
    cancel: &CancelToken,
) -> Result<TextComparison, TextError> {
    let ctl = Ctl {
        cancel,
        progress: &hikasa_core::NullProgress,
    };
    let load = |p: &str| {
        let spec = Spec::Local(LocalPath::new(std::path::PathBuf::from(p)).unwrap());
        load_document(vfs, &spec, format, &LoadOptions::default(), None, &ctl)
    };
    let (l, r) = pool.join(|| load("/l"), || load("/r"));
    let ((l, lraw), (r, rraw)) = (l?, r?);
    // As a session should (§3.2): `bytes_identical` compared in 1 MiB steps with cancellation checks, and the
    // raw bytes freed off the calling thread (unmapping hundreds of MiB takes tens of milliseconds).
    let mut bytes_identical = lraw.len() == rraw.len();
    for (a, b) in lraw.chunks(1 << 20).zip(rraw.chunks(1 << 20)) {
        if !bytes_identical {
            break;
        }
        cancel.check()?;
        bytes_identical = a == b;
    }
    thread::spawn(move || drop((lraw, rraw)));
    let input = CompareInput {
        left: Arc::new(l),
        right: Arc::new(r),
        format: Arc::clone(format),
        rules: rules.clone(),
        bytes_identical,
        force_tokenize: false,
    };
    compare(input, Some(pool), &ctl, None)
}

fn compare_bytes(
    vfs: &MemoryVfs,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    pool: &rayon::ThreadPool,
    cancel: &CancelToken,
) -> Result<TextComparison, TextError> {
    compare_loaded(vfs, format, rules, pool, cancel)
}

#[test]
#[ignore = "release-mode performance gate"]
fn g4_cancel_latency_adversarial() {
    let pool = pool();
    let ee = Arc::new(CompiledFormat::everything_else());
    let c = format("c-cpp");
    let rules = TextRules::default();
    let mut worst = Duration::ZERO;
    let mut rng = XorShift(42);

    // Closeness on long, unrelated lines: the scorer's budget bounds the total, its samples bound one call.
    for (lines, chars) in [(64usize, 200_000usize), (2_000, 2_000), (2_000, 200)] {
        let vfs = vfs_pair(
            &random_lines(&mut rng, lines, chars),
            &random_lines(&mut rng, lines, chars),
        );
        let name = format!("closeness {lines} lines x {chars} chars");
        worst = worst.max(adversarial_case(&name, &|cancel| {
            compare_bytes(&vfs, &ee, &rules, &pool, cancel).map(|_| ())
        }));
    }

    // One 100 MiB line per side (identity path, C general path), plus inline() and spans() on it.
    let line = random_lines(&mut rng, 1, 100 << 20);
    let mut other = line.clone();
    let n = other.len();
    other[n - 2] = b'#';
    let vfs = vfs_pair(&line, &other);
    drop((line, other));
    for (fmt, label) in [(&ee, "identity"), (&c, "c-cpp")] {
        let name = format!("one 100 MiB line ({label})");
        worst = worst.max(adversarial_case(&name, &|cancel| {
            compare_bytes(&vfs, fmt, &rules, &pool, cancel).map(|_| ())
        }));
        let cmp = compare_bytes(&vfs, fmt, &rules, &pool, &CancelToken::never()).unwrap();
        let t = Instant::now();
        let inline = cmp.inline(RowIdx(0)).unwrap();
        let inline_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let spans = cmp.spans(Side::Left, LineIdx(0));
        println!(
            "  one 100 MiB line ({label}): inline {inline_ms:.1} ms (whole_line {}), spans {:.1} ms ({} spans)",
            inline.whole_line,
            t.elapsed().as_secs_f64() * 1e3,
            spans.len()
        );
    }
    drop(vfs);

    // A dense unimportant pattern on 50 MiB lines ('yxyx…', 26M matches), in compare and in the quick compare.
    let mut dense = TextRules::default();
    dense.importance.unimportant.push(TextPattern::plain("x"));
    let mut l = b"yx".repeat(25 << 20);
    let mut r = l.clone();
    let n = r.len();
    r[n - 2] = b'z';
    l.push(b'\n');
    r.push(b'\n');
    let vfs = vfs_pair(&l, &r);
    worst = worst.max(adversarial_case(
        "pattern on 50 MiB lines (compare)",
        &|cancel| compare_bytes(&vfs, &ee, &dense, &pool, cancel).map(|_| ()),
    ));
    drop(vfs);
    worst = worst.max(adversarial_case(
        "pattern on 50 MiB lines (quick)",
        &|cancel| {
            let ctl = Ctl {
                cancel,
                progress: &hikasa_core::NullProgress,
            };
            quick_compare_bytes(
                (&l, "l"),
                (&r, "r"),
                &ee,
                &dense,
                &LoadOptions::default(),
                &ctl,
            )
            .map(|_| ())
            .map_err(|_| TextError::Cancelled)
        },
    ));
    drop((l, r));

    // 256 MiB documents being decoded and built (load_document through a MemoryVfs).
    let text = "Caf\u{e9} cr\u{e8}me br\u{fb}l\u{e9}e, d\u{e9}j\u{e0} vu, na\u{ef}ve fa\u{e7}ade\n";
    for encoding in ["utf-8", "utf-16le", "windows-1252"] {
        let mut bytes = Vec::with_capacity(256 << 20);
        while bytes.len() < 256 << 20 {
            match encoding {
                "utf-16le" => {
                    if bytes.is_empty() {
                        bytes.extend_from_slice(&[0xFF, 0xFE]);
                    }
                    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
                }
                "windows-1252" => bytes.extend(text.chars().map(|ch| ch as u32 as u8)),
                _ => bytes.extend_from_slice(text.as_bytes()),
            }
        }
        let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
        vfs.insert_file("/w/big.txt", bytes, MemMeta::default())
            .unwrap();
        let spec = Spec::Local(LocalPath::new(std::path::PathBuf::from("/w/big.txt")).unwrap());
        let name = format!("load_document 256 MiB {encoding}");
        worst = worst.max(adversarial_case(&name, &|cancel| {
            let ctl = Ctl {
                cancel,
                progress: &hikasa_core::NullProgress,
            };
            load_document(&vfs, &spec, &ee, &LoadOptions::default(), None, &ctl).map(|_| ())
        }));
    }

    // Repetitive source code (D-36): Auto must not run an unbounded Histogram.
    let statements = |seed: u64| -> Vec<u8> {
        let mut rng = XorShift(seed);
        (0..200_000)
            .map(|_| {
                let s = rng.below(50);
                format!("    value_{s} = compute(value_{s}, {s});\n")
            })
            .collect::<String>()
            .into_bytes()
    };
    let vfs = vfs_pair(&statements(1), &statements(2));
    let full = {
        let t = Instant::now();
        let cmp = compare_bytes(&vfs, &c, &rules, &pool, &CancelToken::never()).unwrap();
        println!("  repetitive C 2 x 7 MiB: algorithm {}", cmp.algorithm());
        t.elapsed()
    };
    worst = worst.max(adversarial_case(
        "repetitive C 2 x 7 MiB (auto)",
        &|cancel| compare_bytes(&vfs, &c, &rules, &pool, cancel).map(|_| ()),
    ));
    let full_pass = full < Duration::from_secs(3);
    println!(
        "gate D-36 repetitive C 2 x 7 MiB: {:.0} ms (limit 3 s) {}",
        full.as_secs_f64() * 1e3,
        if full_pass { "PASS" } else { "FAIL" }
    );
    println!(
        "gate G4 cancel_latency/adversarial: worst {:.2} ms (limit 100 ms) {}",
        worst.as_secs_f64() * 1e3,
        if worst <= Duration::from_millis(100) {
            "PASS"
        } else {
            "FAIL"
        }
    );
    assert!(full_pass);
    assert!(worst <= Duration::from_millis(100));
}

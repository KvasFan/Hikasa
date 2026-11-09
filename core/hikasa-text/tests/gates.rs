//! Performance gates measured as ignored tests (run in release, one test at a time: `cargo test --release -p
//! hikasa-text --test gates -- --ignored --nocapture --test-threads=1 --skip g8_memory`, and G8 memory in a process of
//! its own: `… -- --ignored --nocapture --exact g8_memory`): G1 (1M-line text compare from bytes < 3 s), G5 (`row` and
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
        anchors: Vec::new(),
        right_format: None,
        ignored: Default::default(),
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

/// Current resident memory of the process in MiB (Linux `VmRSS`; 0 elsewhere).
fn rss_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map_or(0.0, |kb| kb as f64 / 1024.0)
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
        anchors: Vec::new(),
        right_format: None,
        ignored: Default::default(),
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

// ---------------------------------------------------------------------------------------------------------------
// G8 (W10a rows, §1.10.26): editing the G1 pair (c-cpp, Auto → Standard). Median of 20 edits at seeded random
// places; every recompare of the 20 must stay below 50 ms, `note_edit` below 1 ms.

use hikasa_core::Pair;
use hikasa_text::edit::{
    EditOptions, EditOutcome, EditRequest, Editability, EditableDocument, GroupHint, TextPos,
    copy_rows,
};
use hikasa_text::{LiveComparison, RegexLimits, RowState, compare_live};

/// The G1 pair as editable documents and their live comparison.
fn g8_setup() -> (EditableDocument, EditableDocument, LiveComparison) {
    let (left, right) = c_pair(LINES, 0x5EED_0001);
    let format = format("c-cpp");
    let load = LoadOptions::default();
    let doc = |b: &[u8]| TextDocument::from_bytes(b, &format, &load, DocOrigin::default()).unwrap();
    let (l, r) = (Arc::new(doc(&left)), Arc::new(doc(&right)));
    let pool = pool();
    let live = compare_live(
        CompareInput {
            left: l.clone(),
            right: r.clone(),
            format: format.clone(),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: None,
            ignored: Default::default(),
        },
        &RegexLimits::default(),
        Some(&pool),
        &Ctl::detached(),
        None,
    )
    .unwrap();
    let ed = |d: Arc<TextDocument>| {
        EditableDocument::new(d, EditOptions::default(), Editability::EDITABLE)
    };
    (ed(l), ed(r), live)
}

/// One measured edit: `note_edit`, then the recompare (job, run, install). Returns (note, recompare).
fn g8_measure(
    live: &mut LiveComparison,
    docs: (&EditableDocument, &EditableDocument),
    side: Side,
    out: &EditOutcome,
) -> (Duration, Duration, hikasa_text::PathCounters) {
    let doc = if side == Side::Left { docs.0 } else { docs.1 };
    let t = Instant::now();
    live.note_edit(side, doc, out);
    let note = t.elapsed();
    let t = Instant::now();
    let job = live.recompare_job(Pair::new(docs.0, docs.1));
    let done = job.run(None, &Ctl::detached()).unwrap();
    let path = done.path;
    live.install(done, Pair::new(docs.0, docs.1));
    (note, t.elapsed(), path)
}

/// Rows of `state` in the current comparison (left line holders), seeded picks.
fn g8_lines(live: &LiveComparison, state: RowState, rng: &mut XorShift, count: usize) -> Vec<u32> {
    let cmp = live.current();
    let mut picks = Vec::new();
    while picks.len() < count {
        let r = rng.below(u64::from(cmp.row_count())) as u32;
        let row = cmp.row(RowIdx(r)).unwrap();
        if row.state == state
            && let Some(l) = row.left
        {
            picks.push(l.0);
        }
    }
    picks
}

fn g8_report(
    name: &str,
    notes: &mut [Duration],
    recompares: &mut [Duration],
    limit_ms: f64,
) -> bool {
    notes.sort();
    recompares.sort();
    let worst = recompares.last().copied().unwrap_or_default();
    let pass = worst.as_secs_f64() * 1000.0 < limit_ms
        && notes.last().is_none_or(|n| n.as_secs_f64() < 0.001);
    println!(
        "gate G8 {name}: recompare median {:.2} ms, worst {:.2} ms (limit {limit_ms} ms each); note_edit median {:.3} ms, worst {:.3} ms (limit 1 ms); load {} {}",
        recompares[recompares.len() / 2].as_secs_f64() * 1000.0,
        worst.as_secs_f64() * 1000.0,
        notes[notes.len() / 2].as_secs_f64() * 1000.0,
        notes.last().unwrap().as_secs_f64() * 1000.0,
        load_average(),
        if pass { "PASS" } else { "FAIL" }
    );
    pass
}

fn type_char(doc: &mut EditableDocument, line: u32) -> EditOutcome {
    let len = doc.line_len(LineIdx(line));
    let at = TextPos::new(line, len / 2);
    let at = doc.clamp(at);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: "x".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap()
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_type_and_changed_lines() {
    let (mut left, right, mut live) = g8_setup();
    let mut rng = XorShift(0x6008_0001);
    let mut ok = true;
    for (name, state) in [
        ("type (same line)", RowState::Same),
        ("changed (changed line)", RowState::Changed),
    ] {
        let (mut notes, mut recs) = (Vec::new(), Vec::new());
        for line in g8_lines(&live, state, &mut rng, 20) {
            let out = type_char(&mut left, line);
            let (n, r, _) = g8_measure(&mut live, (&left, &right), Side::Left, &out);
            notes.push(n);
            recs.push(r);
        }
        ok &= g8_report(name, &mut notes, &mut recs, 50.0);
    }
    assert!(ok);
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_lines_split_delete_paste() {
    let (mut left, right, mut live) = g8_setup();
    let mut rng = XorShift(0x6008_0002);
    let mut ok = true;
    let paste: String = (0..100)
        .map(|k| format!("    pasted_{k} = {k};\n"))
        .collect();
    for name in ["enter", "delete line", "paste 100 lines"] {
        let (mut notes, mut recs) = (Vec::new(), Vec::new());
        let mut total = hikasa_text::PathCounters::default();
        for _ in 0..20 {
            let line = rng.below(u64::from(left.line_count() - 2)) as u32;
            let len = left.line_len(LineIdx(line));
            let req = match name {
                "enter" => {
                    let at = left.clamp(TextPos::new(line, len / 2));
                    EditRequest::Replace {
                        range: at..at,
                        text: "\n".into(),
                        keep_eols: false,
                    }
                }
                "delete line" => EditRequest::Replace {
                    range: TextPos::new(line, 0)..TextPos::new(line + 1, 0),
                    text: String::new(),
                    keep_eols: false,
                },
                _ => {
                    let at = TextPos::new(line, 0);
                    EditRequest::Replace {
                        range: at..at,
                        text: paste.clone(),
                        keep_eols: false,
                    }
                }
            };
            let out = left.apply(&req, GroupHint::None).unwrap();
            let (n, r, path) = g8_measure(&mut live, (&left, &right), Side::Left, &out);
            if r > Duration::from_millis(50) {
                println!(
                    "  slow {name} at line {line}: {:.1} ms, path {path:?}",
                    r.as_secs_f64() * 1000.0
                );
            }
            total.add(&path);
            notes.push(n);
            recs.push(r);
        }
        ok &= g8_report(name, &mut notes, &mut recs, 50.0);
        println!("  paths {total:?}");
    }
    assert!(ok);
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_copy_section() {
    let (left, mut right, mut live) = g8_setup();
    let mut rng = XorShift(0x6008_0003);
    let (mut notes, mut recs) = (Vec::new(), Vec::new());
    let cmp = live.current().clone();
    let sections: Vec<_> = cmp
        .sections()
        .iter()
        .filter(|s| s.rows.end - s.rows.start >= 3)
        .cloned()
        .collect();
    for _ in 0..20 {
        let s = &sections[rng.below(sections.len() as u64) as usize];
        let cmp = live.current().clone();
        let Some(id) = cmp.section_at(RowIdx(s.rows.start)) else {
            continue;
        };
        let rows = cmp.sections()[id.0 as usize].rows.clone();
        let rows = rows.start..rows.end.min(rows.start + 10);
        let edit = copy_rows(&cmp, rows.clone(), Side::Left, right.insert_eol()).unwrap();
        let out = right.apply(&edit.request, GroupHint::None).unwrap();
        let (n, r, path) = g8_measure(&mut live, (&left, &right), Side::Right, &out);
        if r > Duration::from_millis(50) {
            println!(
                "  slow copy at rows {rows:?}: {:.1} ms, path {path:?}",
                r.as_secs_f64() * 1000.0
            );
        }
        notes.push(n);
        recs.push(r);
    }
    assert!(g8_report(
        "copy section (≤ 10 rows) to right",
        &mut notes,
        &mut recs,
        50.0
    ));
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_undo() {
    let (mut left, right, mut live) = g8_setup();
    let original = live.current().clone();
    let mut rng = XorShift(0x6008_0004);
    let mut worst = Duration::ZERO;
    for k in 0..1000 {
        let line = rng.below(u64::from(left.line_count())) as u32;
        let out = type_char(&mut left, line);
        live.note_edit(Side::Left, &left, &out);
        if k % 50 == 49 {
            let t = Instant::now();
            let done = live
                .recompare_job(Pair::new(&left, &right))
                .run(None, &Ctl::detached())
                .unwrap();
            live.install(done, Pair::new(&left, &right));
            worst = worst.max(t.elapsed());
        }
    }
    let t = Instant::now();
    while let Some(out) = left.undo() {
        live.note_edit(Side::Left, &left, &out);
    }
    let done = live
        .recompare_job(Pair::new(&left, &right))
        .run(None, &Ctl::detached())
        .unwrap();
    let path = done.path;
    live.install(done, Pair::new(&left, &right));
    let undo = t.elapsed();
    // Equal to the original: runs, every row, sections, statistics and warnings.
    let cur = live.current();
    let all_rows = |c: &hikasa_text::TextComparison| -> Vec<hikasa_text::Row> {
        c.rows(0..c.row_count()).collect()
    };
    let same = cur.runs() == original.runs()
        && all_rows(cur) == all_rows(&original)
        && cur.sections() == original.sections()
        && cur.stats() == original.stats()
        && cur.warnings() == original.warnings();
    let pass = undo < Duration::from_secs(1) && same;
    println!(
        "gate G8 undo: 1,000 undos + note_edits + one recompare {:.3} s (limit 1 s), equal to the original (runs, rows, sections, statistics, warnings) {same}; recompare after 50 edits worst {:.1} ms; path {path:?}; load {} {}",
        undo.as_secs_f64(),
        worst.as_secs_f64() * 1000.0,
        load_average(),
        if pass { "PASS" } else { "FAIL" }
    );
    assert!(pass);
}

/// G8 memory (W10b note 67): the current resident size before and after 1,000 edits with a recompare after every 50.
/// **Run it in a process of its own** (`-- --ignored --exact g8_memory`, or nextest, which runs each test in its own
/// process): after other gates in the same process the allocator holds their freed memory and its arenas, and the
/// growth measures them, not the edits.
#[test]
#[ignore = "release-mode performance gate (a process of its own)"]
fn g8_memory() {
    let (mut left, right, mut live) = g8_setup();
    let mut rng = XorShift(0x6008_0004);
    // F3: the current resident size (VmRSS) before and after, and the peak of the phase alone (VmHWM reset).
    reset_peak_rss();
    let rss0 = rss_mib();
    // Also reported: the size after the first recompare, which frees the initial compare's result (the steady state).
    let mut warm = rss0;
    for k in 0..1000 {
        let line = rng.below(u64::from(left.line_count())) as u32;
        let out = type_char(&mut left, line);
        live.note_edit(Side::Left, &left, &out);
        if k % 50 == 49 {
            let done = live
                .recompare_job(Pair::new(&left, &right))
                .run(None, &Ctl::detached())
                .unwrap();
            live.install(done, Pair::new(&left, &right));
            if k == 49 {
                warm = rss_mib();
            }
        }
    }
    let rss1 = rss_mib();
    let peak = peak_rss_mib();
    let growth = rss1 - rss0;
    let mem_pass = growth < 10.0;
    println!(
        "gate G8 memory: current RSS {rss0:.1} -> {rss1:.1} MiB over 1,000 edits and their recompares, growth {growth:.1} MiB (limit 10 MiB); after the first recompare {warm:.1} MiB, growth from there {:.1} MiB over 950 edits; peak of the phase {peak} MiB; live data {:.1} MiB; load {} {}",
        rss1 - warm,
        live.keys().heap_bytes() as f64 / 1048576.0,
        load_average(),
        if mem_pass { "PASS" } else { "FAIL" }
    );
    assert!(mem_pass);
}

#[test]
#[ignore = "release-mode performance gate (reported)"]
fn g8_worst_prefix_edit() {
    let (mut left, right, mut live) = g8_setup();
    let (mut notes, mut recs) = (Vec::new(), Vec::new());
    let out = type_char(&mut left, 0);
    let (n, r, path) = g8_measure(&mut live, (&left, &right), Side::Left, &out);
    notes.push(n);
    recs.push(r);
    println!(
        "gate G8 worst (a character in the common prefix: the root re-runs): {:.1} ms, path {path:?} (reported)",
        r.as_secs_f64() * 1000.0
    );
}

#[test]
#[ignore = "measurement"]
fn g8_root_window_profile() {
    let (_, _, live) = g8_setup();
    let d = live.keys().derivation();
    let seg = &d.segments[0];
    let root = &seg.nodes[seg.root.unwrap() as usize];
    let (ml, mr) = root.stripped();
    let (na, nb) = (ml.end - ml.start, mr.end - mr.start);
    let s = 300i64;
    let mut hist = std::collections::BTreeMap::new();
    for &(x, y) in &root.chain {
        let (x, y) = (i64::from(x - ml.start), i64::from(y - mr.start));
        let ds = (x - y).abs();
        let de = ((i64::from(na) - x) - (i64::from(nb) - y)).abs();
        let class = match (ds <= s, de <= s) {
            (true, true) => format!("both sum<{}", ((s - ds + 1) + (s - de + 1)) / 64 * 64 + 64),
            (true, false) => format!("start ms<{}", (s - ds + 1) / 64 * 64 + 64),
            (false, true) => format!("end me<{}", (s - de + 1) / 64 * 64 + 64),
            _ => "out".into(),
        };
        *hist.entry(class).or_insert(0u64) += 1;
    }
    println!(
        "root: {} lines x {} lines, chain {}, candidates {}, diagonal runs {}, blocks {}, children {}",
        na,
        nb,
        root.chain.len(),
        root.candidates,
        root.diag.len(),
        root.blocks.len(),
        root.children.len()
    );
    for (k, v) in hist {
        println!("  {k}: {v}");
    }
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_note_edit_in_one_huge_section() {
    // F10: two unrelated 1,000,000-line files are one section; note_edit stays below 1 ms (the section changes
    // by difference).
    let n = 1_000_000u32;
    let text = |w: &str| -> Arc<TextDocument> {
        let t: String = (0..n).map(|k| format!("{w} {k}\n")).collect();
        Arc::new(TextDocument::from_text(&t))
    };
    let (l, r) = (text("left"), text("right"));
    let live = compare_live(
        CompareInput {
            left: l.clone(),
            right: r.clone(),
            format: Arc::new(CompiledFormat::everything_else()),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: None,
            ignored: Default::default(),
        },
        &RegexLimits::default(),
        Some(&pool()),
        &Ctl::detached(),
        None,
    )
    .unwrap();
    let mut live = live;
    let mut left = EditableDocument::new(l, EditOptions::default(), Editability::EDITABLE);
    let sections = live.current().sections().len();
    let mut rng = XorShift(0x6008_0010);
    let mut notes = Vec::new();
    for _ in 0..20 {
        let line = 1000 + rng.below(u64::from(n - 2000)) as u32;
        let out = type_char(&mut left, line);
        let t = Instant::now();
        live.note_edit(Side::Left, &left, &out);
        notes.push(t.elapsed());
    }
    notes.sort();
    let worst = notes.last().copied().unwrap_or_default();
    let pass = worst < Duration::from_millis(1);
    println!(
        "gate G8 note_edit in one {sections}-section pair of unrelated 1M-line files: median {:.3} ms, worst {:.3} ms (limit 1 ms); load {} {}",
        notes[notes.len() / 2].as_secs_f64() * 1000.0,
        worst.as_secs_f64() * 1000.0,
        load_average(),
        if pass { "PASS" } else { "FAIL" }
    );
    assert!(pass);
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_find() {
    // G8 find (§1.10.26): `Finder::all` of a plain word and of the regex `\b\w+_17\b` over the 1M-line left side of the
    // G1 pair: < 150 ms and < 400 ms (median of 5; every run printed).
    use hikasa_text::find::{FindQuery, Finder};
    let (left, _) = c_pair(LINES, 0x5EED_0001);
    let format = format("c-cpp");
    let doc = TextDocument::from_bytes(
        &left,
        &format,
        &LoadOptions::default(),
        DocOrigin::default(),
    )
    .unwrap();
    let doc = EditableDocument::new(Arc::new(doc), EditOptions::default(), Editability::EDITABLE);
    let mut pass = true;
    for (text, regex, limit_ms) in [("return", false, 150.0), (r"\b\w+_17\b", true, 400.0)] {
        let q = FindQuery {
            text: text.to_owned(),
            regex,
            match_case: true,
            whole_word: false,
        };
        let f = Finder::new(&q, "", &RegexLimits::default()).unwrap();
        let mut times = Vec::new();
        let mut hits = 0;
        for _ in 0..5 {
            let t = Instant::now();
            hits = f.all(&doc, u64::MAX, &Ctl::detached()).unwrap().len();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let mut sorted = times.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[2];
        let ok = median < limit_ms;
        pass &= ok;
        println!(
            "gate G8 find {text:?} ({}): {hits} hits, median {median:.1} ms, runs {times:.1?} (limit {limit_ms} ms); load {} {}",
            if regex { "regex" } else { "plain" },
            load_average(),
            if ok { "PASS" } else { "FAIL" }
        );
    }
    assert!(pass);
}

#[test]
#[ignore = "release-mode performance gate"]
fn g8_find_files() {
    // G8 find files (§1.10.26): a word over 10,000 small files (100 folders of 100 files of about 1 KiB, `MemoryVfs`,
    // warm: the second run counts) ≤ 1 s; the event stream identical with 1 and 8 threads.
    use hikasa_text::find::FindQuery;
    use hikasa_text::find::files::{FilesEvent, FilesQuery, FilesSink, find_in_files};
    struct Lines(Vec<String>);
    impl FilesSink for Lines {
        fn event(&mut self, e: FilesEvent<'_>) {
            self.0.push(format!("{e:?}"));
        }
    }
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let mut rng = XorShift(0x5EED_F11E);
    for d in 0..100 {
        for f in 0..100 {
            let mut text = String::new();
            for k in 0..24 {
                text.push_str(&inputs::c_line(&mut rng, (d * 100 + f) * 24 + k));
                text.push('\n');
            }
            let path = if cfg!(windows) {
                format!("C:\\g8\\d{d:03}\\f{f:03}.c")
            } else {
                format!("/g8/d{d:03}/f{f:03}.c")
            };
            vfs.insert_file(&path, text.into_bytes(), MemMeta::default())
                .unwrap();
        }
    }
    let root = if cfg!(windows) { "C:\\g8" } else { "/g8" };
    let root = Spec::Local(LocalPath::new(std::path::PathBuf::from(root)).unwrap());
    let q = FilesQuery::new(
        FindQuery {
            text: "return".into(),
            regex: false,
            match_case: true,
            whole_word: false,
        },
        vec![root],
    );
    let registry = FormatRegistry::builtin().unwrap();
    let mut streams = Vec::new();
    let mut pass = true;
    for threads in [8usize, 1] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let mut times = Vec::new();
        let mut lines = Lines(Vec::new());
        for _ in 0..2 {
            lines = Lines(Vec::new());
            let t = Instant::now();
            let summary = find_in_files(
                &vfs,
                &registry,
                &q,
                &mut lines,
                Some(&pool),
                &Ctl::detached(),
            )
            .unwrap();
            times.push(t.elapsed().as_secs_f64());
            assert_eq!(summary.files_searched, 10_000);
        }
        let warm = times[1];
        let ok = threads != 8 || warm <= 1.0;
        pass &= ok;
        println!(
            "gate G8 find files ({threads} threads): {} events, cold {:.3} s, warm {warm:.3} s (limit 1 s with the pool); load {} {}",
            lines.0.len(),
            times[0],
            load_average(),
            if ok { "PASS" } else { "FAIL" }
        );
        streams.push(lines.0);
    }
    let same = streams[0] == streams[1];
    println!("gate G8 find files: the same stream with 1 and 8 threads: {same}");
    assert!(pass && same);
}

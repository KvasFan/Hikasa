//! `tokenize_c_1m` (§6.4): tokenising a generated 1,000,000-line C file (≈ 48 bytes per line) with the
//! built-in C-family grammar, with a callback that builds a key-like byte string per line (element byte +
//! token text, as hikasa-text does).
//!
//! Gate: ≥ 50 MB/s per core (sequential run); parallel scaling ≥ 0.6 × cores up to 8 (8-thread pool versus
//! 1-thread pool); ≤ 4 allocations per 8192-line chunk. The gate report is printed once before the criterion
//! measurements; the allocation gate is asserted (it is deterministic), the timing gates are reported.
//!
//! The counting `#[global_allocator]` lives in this bench binary only (never in library code, §6.1).
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace
// lint is `deny`, so this bench binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use hikasa_core::cancel::CancelToken;
use hikasa_formats::{CompiledGrammar, FormatId, FormatRegistry, LineSource, Token, tokenize_all};

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; counting is a relaxed
// atomic increment that neither allocates nor re-enters the allocator.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }

    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc_zeroed` unchanged.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the
    // caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const LINES: usize = 1_000_000;
const CHUNK: usize = 8192;

/// Seeded xorshift64* (§6.1: no `rand` crate; inputs are never committed).
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const WORDS: &[&str] = &[
    "count", "index", "buffer", "result", "node", "value", "length", "offset", "flags", "state",
    "item", "ptr",
];

/// A C-like document of `lines` lines, about 48 bytes per line, with occasional multi-line block comments.
fn generate(lines: usize) -> String {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut out = String::with_capacity(lines * 50);
    let mut k = 0;
    while k < lines {
        let w1 = WORDS[rng.below(WORDS.len() as u64) as usize];
        let w2 = WORDS[rng.below(WORDS.len() as u64) as usize];
        let n = rng.below(1000);
        match rng.below(16) {
            0 if k + 3 < lines => {
                out.push_str("/* Update the ");
                out.push_str(w1);
                out.push_str(" before use.\n * See also ");
                out.push_str(w2);
                out.push_str(" and the notes in the header.\n */\n");
                k += 3;
                continue;
            }
            1 => out.push_str(&format!("#include \"{w1}_{n}.h\"\n")),
            2 => out.push_str(&format!("    if ({w1} > {n} && {w2} != NULL) {{\n")),
            3 => out.push_str("    }\n"),
            4 => out.push_str(&format!("    printf(\"{w1} = %d\\n\", {w2}_{n});\n")),
            5 => out.push_str(&format!(
                "static const char *{w1}_names[] = {{ \"{w2}\", 0 }};\n"
            )),
            6 => out.push_str(&format!("    {w1}[{n}] = 0x{n:04X}u; // reset the {w2}\n")),
            7 => out.push_str(&format!(
                "    for (int i = 0; i < {n}; i++) {w1} += {w2}[i];\n"
            )),
            8 => out.push_str(&format!("    return {w1}_{n}->{w2} * {n}.5f;\n")),
            _ => out.push_str(&format!(
                "    unsigned long {w1}_{n} = {w2}({n}, &{w1}); // {w2}\n"
            )),
        }
        k += 1;
    }
    out
}

/// Lines of a big text as borrowed slices (like the text document's line table).
struct Doc {
    text: String,
    starts: Vec<(usize, usize)>,
}

impl Doc {
    fn new(text: String) -> Doc {
        let mut starts = Vec::new();
        let mut s = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push((s, i));
                s = i + 1;
            }
        }
        if s < text.len() {
            starts.push((s, text.len()));
        }
        Doc { text, starts }
    }
}

impl LineSource for Doc {
    fn line_count(&self) -> u32 {
        self.starts.len() as u32
    }
    fn line(&self, idx: u32) -> Cow<'_, str> {
        let (a, b) = self.starts[idx as usize];
        Cow::Borrowed(&self.text[a..b])
    }
}

/// Key-like output: per token, the element id byte and the token text (≈ line length + one byte per token).
fn key(_line_no: u32, text: &str, tokens: &[Token], out: &mut Vec<u8>) {
    let bytes = text.as_bytes();
    out.reserve(text.len() + tokens.len());
    for t in tokens {
        out.push(t.element.0 as u8);
        out.extend_from_slice(&bytes[t.start as usize..t.end as usize]);
    }
}

fn grammar() -> CompiledGrammar {
    let registry = FormatRegistry::builtin().unwrap();
    let format = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
    format.grammar().clone()
}

fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
}

/// CPU time of the calling thread (Linux; `None` elsewhere): on a loaded machine it shows the per-core cost
/// better than wall-clock time.
fn thread_cpu() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/stat").ok()?;
    let fields: Vec<&str> = stat.get(stat.rfind(')')? + 2..)?.split(' ').collect();
    let ticks: u64 = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    Some(Duration::from_millis(ticks * 10))
}

fn best_of(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn gate_report(g: &CompiledGrammar, doc: &Doc) {
    let cancel = CancelToken::never();
    let bytes = doc.text.len() as f64;
    let chunks = doc.starts.len().div_ceil(CHUNK) as u64;
    let seq = best_of(3, || {
        tokenize_all(g, doc, None, &cancel, key).unwrap();
    });
    let mb_s = bytes / seq.as_secs_f64() / 1e6;
    let cpu = (0..3)
        .filter_map(|_| {
            let before = thread_cpu()?;
            tokenize_all(g, doc, None, &cancel, key).unwrap();
            Some(thread_cpu()? - before)
        })
        .min();
    let p1 = pool(1);
    let one = best_of(5, || {
        tokenize_all(g, doc, Some(&p1), &cancel, key).unwrap();
    });
    let cores = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    let pn = pool(cores);
    let many = best_of(5, || {
        tokenize_all(g, doc, Some(&pn), &cancel, key).unwrap();
    });
    let scaling = one.as_secs_f64() / many.as_secs_f64();
    let mut per_chunk = Vec::new();
    for (name, p) in [
        ("sequential", None),
        ("pool(1)", Some(&p1)),
        ("pool(n)", Some(&pn)),
    ] {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let out = tokenize_all(g, doc, p, &cancel, key).unwrap();
        let used = ALLOCATIONS.load(Ordering::Relaxed) - before;
        drop(out);
        per_chunk.push((name, used, used as f64 / chunks as f64));
    }
    println!(
        "tokenize_c_1m gate ({} lines, {:.1} MB, {chunks} chunks):",
        doc.starts.len(),
        bytes / 1e6
    );
    println!(
        "  sequential: {:>8.1} ms = {mb_s:.1} MB/s per core (gate >= 50)",
        seq.as_secs_f64() * 1e3
    );
    if let Some(cpu) = cpu {
        println!(
            "  sequential, thread CPU time: {:.0} ms = {:.1} MB/s",
            cpu.as_secs_f64() * 1e3,
            bytes / cpu.as_secs_f64().max(1e-3) / 1e6
        );
    }
    println!(
        "  pool(1) {:.1} ms, pool({cores}) {:.1} ms: scaling {scaling:.2}x (gate >= {:.1}x)",
        one.as_secs_f64() * 1e3,
        many.as_secs_f64() * 1e3,
        0.6 * cores as f64
    );
    for (name, used, ratio) in &per_chunk {
        println!("  allocations {name}: {used} = {ratio:.2} per chunk (gate <= 4)");
        assert!(*ratio <= 4.0, "allocation gate: {ratio:.2} per chunk");
    }
}

fn bench(c: &mut Criterion) {
    let g = grammar();
    let doc = Doc::new(generate(LINES));
    gate_report(&g, &doc);
    let cancel = CancelToken::never();
    let mut group = c.benchmark_group("tokenize_c_1m");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(doc.text.len() as u64));
    group.bench_function("sequential", |b| {
        b.iter(|| tokenize_all(&g, &doc, None, &cancel, key).unwrap())
    });
    let p8 = pool(8);
    group.bench_function("pool8", |b| {
        b.iter(|| tokenize_all(&g, &doc, Some(&p8), &cancel, key).unwrap())
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);

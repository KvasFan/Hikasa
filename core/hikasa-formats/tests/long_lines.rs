//! Long single lines (§2.10.5, §5.3, G4; wave-2 review): tokenising one line costs time linear in its length,
//! even for runs that a simple pattern or a regex item would re-examine from every position, and a cancel
//! returns within the latency budget while one multi-megabyte line is being tokenised.
//!
//! The adversarial lines are the review's probes: Python `x␣…␣y` (`[ \t]+$` inside a long run of blanks), Bash
//! `${a ${a …` (`\$\{[^}]*\}` without a closer), Rust `#[a #[a …` (`#!?\[[^\]]*\]` without a closer) and Visual
//! Basic `"rem" "rem" …` (a REM comment found inside every string and skipped). Debug builds assert a scaling
//! ratio (linear ≈ 4 for 4 × the input, quadratic ≈ 16); release builds also assert 1 MiB in well under 1 s.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_formats::{
    CompiledFormat, ElementName, FileFormat, FormatId, FormatKind, FormatRegistry, GrammarItem,
    ItemKind, LineState, RegexLimits, Token, Tokenizer, tokenize_all,
};

fn builtin(id: &str) -> Arc<CompiledFormat> {
    FormatRegistry::builtin()
        .unwrap()
        .compiled(&FormatId::new(id).unwrap())
        .unwrap()
}

/// The fastest of three runs (the machine may be busy).
fn time_line(c: &CompiledFormat, line: &str) -> Duration {
    let mut t = Tokenizer::new(c.grammar());
    let mut out = Vec::new();
    (0..3)
        .map(|_| {
            let start = Instant::now();
            t.tokenize_line(line, 0, LineState::Normal, &mut out);
            start.elapsed()
        })
        .min()
        .unwrap_or_default()
}

type Builder = fn(usize) -> String;

/// `(preset, line of about n bytes)`.
const CASES: [(&str, Builder); 4] = [
    ("python", |n| format!("x{}y", " ".repeat(n))),
    ("bash", |n| "${a ".repeat(n / 4)),
    ("rust", |n| "#[a ".repeat(n / 4)),
    ("visual-basic", |n| "\"rem\" ".repeat(n / 6)),
];

#[test]
fn adversarial_long_lines_tokenize_in_linear_time() {
    let mut report = Vec::new();
    for (id, build) in CASES {
        let c = builtin(id);
        let small = time_line(&c, &build(64 << 10));
        let big = time_line(&c, &build(256 << 10));
        let ratio = big.as_secs_f64() / small.as_secs_f64().max(1e-6);
        report.push(format!(
            "{id}: 64 KiB {small:?}, 256 KiB {big:?}, ratio {ratio:.1}"
        ));
        // Linear is about 4; quadratic would be about 16. The bound leaves room for a noisy machine, and tiny
        // timings (where the ratio is mostly noise) pass on the absolute bound.
        assert!(
            ratio < 9.0 || big < Duration::from_millis(20),
            "{id} is not linear: {report:#?}"
        );
        if !cfg!(debug_assertions) {
            let mib = time_line(&c, &build(1 << 20));
            report.push(format!("{id}: 1 MiB {mib:?}"));
            assert!(mib < Duration::from_millis(500), "{report:#?}");
        }
    }
    println!("{report:#?}");
}

/// A grammar whose single regex item hits the backtracking limit at every position: one long line then
/// takes a very long time (about one limited search per position), which is what a cancel must interrupt.
fn catastrophic() -> CompiledFormat {
    let mut f = FileFormat::new(FormatId::new("slow").unwrap(), "Slow", FormatKind::Text);
    f.grammar.push(GrammarItem {
        element: ElementName::new("X").unwrap(),
        kind: ItemKind::Basic {
            text: r"(a*)*\1b".into(),
            regex: true,
            match_case: true,
        },
    });
    let limits = RegexLimits {
        size_limit: 8 << 20,
        backtrack_limit: 10_000,
    };
    CompiledFormat::compile(Arc::new(f), &limits).unwrap()
}

/// Runs `tokenize_all` over one line, cancels after `after`, and returns the result and the time from the
/// cancel to the return.
fn cancel_after(
    c: &CompiledFormat,
    line: String,
    pool: Option<&rayon::ThreadPool>,
    after: Duration,
) -> (Result<(), Cancelled>, Duration) {
    let lines = vec![line];
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(after);
        trigger.cancel();
        Instant::now()
    });
    let f = |_: u32, _: &str, _: &[Token], _: &mut Vec<u8>| {};
    let result = tokenize_all(c.grammar(), &lines, pool, &cancel, f).map(|_| ());
    let returned = Instant::now();
    let cancelled_at = canceller.join().unwrap();
    (result, returned.saturating_duration_since(cancelled_at))
}

#[test]
fn a_cancel_interrupts_one_long_line() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    // The review's probe: one 1 MiB Python line (tokenised in a few milliseconds now; either way the call
    // returns within the budget).
    let python = builtin("python");
    let (_, latency) = cancel_after(
        &python,
        format!("x{}y", " ".repeat(1 << 20)),
        None,
        Duration::from_millis(20),
    );
    assert!(latency <= Duration::from_millis(100), "{latency:?}");
    // A line that would take minutes: the cancel is observed inside the line (before every regex engine call
    // and every 64 KiB of scanned text), sequentially and on a pool.
    let slow = catastrophic();
    for pool in [None, Some(&pool)] {
        let (result, latency) =
            cancel_after(&slow, "a".repeat(1 << 20), pool, Duration::from_millis(50));
        assert_eq!(result, Err(Cancelled), "pool: {}", pool.is_some());
        assert!(
            latency <= Duration::from_millis(100),
            "{latency:?}, pool: {}",
            pool.is_some()
        );
    }
}

#[test]
fn tokenize_line_cancellable_clears_the_output() {
    let slow = catastrophic();
    let mut t = Tokenizer::new(slow.grammar());
    let mut out = Vec::new();
    let cancel = CancelToken::new();
    cancel.cancel();
    let line = "a".repeat(1000);
    assert_eq!(
        t.tokenize_line_cancellable(&line, 0, LineState::Normal, &mut out, &cancel),
        Err(Cancelled)
    );
    assert!(out.is_empty());
    // Without a cancellation the result is the plain one.
    let o = t
        .tokenize_line_cancellable(
            "x ab",
            0,
            LineState::Normal,
            &mut out,
            &CancelToken::never(),
        )
        .unwrap();
    assert_eq!(o.end_state, LineState::Normal);
    let mut plain = Vec::new();
    Tokenizer::new(slow.grammar()).tokenize_line("x ab", 0, LineState::Normal, &mut plain);
    assert_eq!(out, plain);
    assert_eq!(out.len(), 2);
}

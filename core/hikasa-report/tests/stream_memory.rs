//! M2 W7 acceptance A7.10 (D-202, D-212): reports stream. A counting allocator measures the peak of live heap while a
//! report renders into a sink: 50,000 and 200,000 synthetic text rows in each new text layout (interleaved text and
//! HTML, with strike-out, and XML) stay within 4 MiB, the two sizes within 1 MiB of each other; 25,000 and 100,000
//! folder rows in folder side-by-side HTML and XML stay within 4 MiB + 32 bytes per visible row. A cancel at the n-th
//! MiB of a report written through [`CancelWriter`] stops with at most one MiB more written. The counting
//! `#[global_allocator]` lives in this test binary only (§6.1); one measurement runs at a time.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::inputs::compare_on;
use common::{compare_bytes, host, mspec};
use hikasa_core::{CancelToken, CaseSensitivity, FileTime};
use hikasa_folder::{FolderComparison, FolderRules};
use hikasa_report::{
    CANCEL_CHECK_BYTES, CancelWriter, HtmlStyle, ReportDefaults, ReportInput, ReportKind,
    ReportMeta, ReportRequest, ReportWords, render_report,
};
use hikasa_text::{TextComparison, TextRules};
use hikasa_vfs::{MemMeta, MemoryVfs};

/// Counts live heap bytes and their peak.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call goes to `System` with the same arguments; only the counters are added.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: forwarded to `System` with the caller's contract.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is forwarded unchanged.
        let block = unsafe { System.alloc(layout) };
        if !block.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        block
    }

    // SAFETY: forwarded to `System` with the caller's contract.
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is forwarded unchanged.
        unsafe { System.dealloc(block, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// One measurement at a time: the counters are global.
static MEASURE: Mutex<()> = Mutex::new(());

/// A sink that keeps nothing.
struct Sink(u64);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The peak of live heap above the live heap before `f`, and what `f` wrote.
fn peak_of(f: impl FnOnce(&mut Sink)) -> (usize, u64) {
    let mut sink = Sink(0);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    f(&mut sink);
    (PEAK.load(Ordering::Relaxed).saturating_sub(base), sink.0)
}

const MIB: usize = 1 << 20;

/// `rows` rows: mostly same lines, a change every 7th, an orphan every 13th, long-ish lines with markup.
fn text_pair(rows: usize) -> TextComparison {
    let mut left = String::with_capacity(rows * 40);
    let mut right = String::with_capacity(rows * 40);
    for i in 0..rows {
        let line = format!("line {i} <tag> & \"text\" with some words\n");
        if i % 13 == 5 {
            left.push_str(&line);
            continue;
        }
        left.push_str(&line);
        if i % 7 == 3 {
            right.push_str(&format!("line {i} <tag> & \"TEXT\" with other words\n"));
        } else {
            right.push_str(&line);
        }
    }
    compare_bytes(
        ("l.txt", left.as_bytes()),
        ("r.txt", right.as_bytes()),
        &TextRules::default(),
    )
}

fn render_text(
    cmp: &TextComparison,
    layout: &'static str,
    options: &[&str],
    output: &[&str],
    out: &mut dyn Write,
) {
    let words = ReportWords {
        layout: Some(layout.into()),
        options: options.iter().map(|s| (*s).to_owned()).collect(),
        output_options: output.iter().map(|s| (*s).to_owned()).collect(),
        title: None,
    };
    let req = ReportRequest::parse(ReportKind::Text, &words, &ReportDefaults::new(layout)).unwrap();
    let hikasa_report::ReportBody::Text(opts) = &req.body else {
        unreachable!()
    };
    let meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
    render_report(
        ReportInput::Text(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        out,
    )
    .unwrap();
}

/// The new text layouts: interleaved text (with strike-out), interleaved HTML, XML.
const TEXT_LAYOUTS: [(&str, &[&str], &[&str]); 4] = [
    ("interleaved", &[], &[]),
    (
        "interleaved",
        &["strikeout-left-diffs", "strikeout-right-diffs"],
        &[],
    ),
    ("interleaved", &["strikeout-left-diffs"], &["html-color"]),
    ("xml", &[], &[]),
];

#[test]
fn a7_10_text_layouts_stream() {
    let _guard = MEASURE.lock().unwrap();
    let small = text_pair(50_000);
    let large = text_pair(200_000);
    for (layout, options, output) in TEXT_LAYOUTS {
        let (peak_small, bytes_small) =
            peak_of(|sink| render_text(&small, layout, options, output, sink));
        let (peak_large, bytes_large) =
            peak_of(|sink| render_text(&large, layout, options, output, sink));
        eprintln!(
            "{layout} {options:?} {output:?}: 50k rows {peak_small} B peak ({bytes_small} B out), 200k rows {peak_large} B peak ({bytes_large} B out)"
        );
        assert!(
            bytes_large > 3 * 4 * MIB as u64,
            "the large report is far bigger than the bound: {bytes_large}"
        );
        assert!(
            peak_small <= 4 * MIB,
            "{layout} {options:?}: {peak_small} B at 50,000 rows"
        );
        assert!(
            peak_large <= 4 * MIB,
            "{layout} {options:?}: {peak_large} B at 200,000 rows"
        );
        assert!(
            peak_large.abs_diff(peak_small) <= MIB,
            "{layout} {options:?}: {peak_small} B and {peak_large} B: the peak grows with the rows"
        );
    }
}

/// `files` files in folders of 100, every third differing, on `MemoryVfs`.
fn folder_tree(files: usize) -> FolderComparison {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let meta = |secs| MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    };
    for i in 0..files {
        let path = format!("d{:04}/file &<{i}>.txt", i / 100);
        vfs.insert_file(&host(&format!("/l/{path}")), b"a".to_vec(), meta(1_000_000))
            .unwrap();
        let other: &[u8] = if i % 3 == 0 { b"bb" } else { b"a" };
        vfs.insert_file(
            &host(&format!("/r/{path}")),
            other.to_vec(),
            meta(1_000_000),
        )
        .unwrap();
    }
    compare_on(&vfs, mspec("/l"), mspec("/r"), &FolderRules::default())
}

fn render_folder(
    cmp: &FolderComparison,
    layout: &'static str,
    output: &[&str],
    out: &mut dyn Write,
) -> usize {
    let words = ReportWords {
        layout: Some(layout.into()),
        options: Vec::new(),
        output_options: output.iter().map(|s| (*s).to_owned()).collect(),
        title: None,
    };
    let req =
        ReportRequest::parse(ReportKind::Folder, &words, &ReportDefaults::new(layout)).unwrap();
    let hikasa_report::ReportBody::Folder { options, .. } = &req.body else {
        unreachable!()
    };
    let meta = ReportMeta::for_folder(cmp, options, None, "hikasa test");
    render_report(
        ReportInput::Folder(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        out,
    )
    .unwrap();
    let mut rows = Vec::new();
    cmp.read()
        .visible_rows(&options.display, &|_| true, &mut rows);
    rows.len()
}

#[test]
fn a7_10_folder_layouts_stream() {
    let _guard = MEASURE.lock().unwrap();
    let small = folder_tree(25_000);
    let large = folder_tree(100_000);
    for (layout, output) in [("side-by-side", &["html-color"][..]), ("xml", &[][..])] {
        for cmp in [&small, &large] {
            let mut visible = 0;
            let (peak, bytes) = peak_of(|sink| visible = render_folder(cmp, layout, output, sink));
            let bound = 4 * MIB + 32 * visible;
            eprintln!("{layout}: {visible} rows, {peak} B peak (bound {bound}), {bytes} B out");
            assert!(
                bytes > (4 * MIB) as u64,
                "the report is bigger than the bound"
            );
            assert!(peak <= bound, "{layout}: {peak} B for {visible} rows");
        }
    }
}

/// A cancel at the n-th MiB of a report written through `CancelWriter`: at most one MiB more is written.
#[test]
fn a7_10_cancel_stops_within_a_mib() {
    let _guard = MEASURE.lock().unwrap();
    let cmp = text_pair(200_000);
    for n in [1u64, 3, 5] {
        let token = CancelToken::new();
        /// A sink that cancels when it holds `at` bytes.
        struct Trip<'a> {
            got: u64,
            at: u64,
            token: &'a CancelToken,
        }
        impl Write for Trip<'_> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.got += buf.len() as u64;
                if self.got >= self.at {
                    self.token.cancel();
                }
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let trip = Trip {
            got: 0,
            at: n * CANCEL_CHECK_BYTES,
            token: &token,
        };
        let mut writer = CancelWriter::new(trip, &token);
        let words = ReportWords {
            layout: Some("interleaved".into()),
            ..ReportWords::default()
        };
        let req = ReportRequest::parse(
            ReportKind::Text,
            &words,
            &ReportDefaults::new("interleaved"),
        )
        .unwrap();
        let hikasa_report::ReportBody::Text(opts) = &req.body else {
            unreachable!()
        };
        let meta = ReportMeta::for_text(&cmp, opts, None, "hikasa test");
        let err = render_report(
            ReportInput::Text(&cmp),
            &req,
            &meta,
            &HtmlStyle::default(),
            None,
            &mut writer,
        )
        .unwrap_err();
        assert!(err.is_cancelled(), "{err}");
        assert_eq!(
            hikasa_core::Classify::kind(&err),
            hikasa_core::ErrorKind::Cancelled
        );
        let written = writer.written();
        assert!(written >= n * CANCEL_CHECK_BYTES, "{written}");
        assert!(
            written <= (n + 1) * CANCEL_CHECK_BYTES,
            "cancel at {n} MiB, {written} B written"
        );
    }
}

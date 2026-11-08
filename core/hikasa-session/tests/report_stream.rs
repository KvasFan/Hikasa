//! M2 W7b acceptance A7.14 (D-202): a file target streams. A counting allocator measures the peak of live heap while
//! `Service::report_execute` (and `Service::write_output_with` directly) writes a report of 50,000 rows to a file on
//! disk, for every text layout and for folder side-by-side HTML and XML: the report is never held, so the peak stays
//! under 4 MiB whatever the layout. The counting `#[global_allocator]` lives in this test binary only (§6.1); one
//! measurement runs at a time.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{job, os, service, write};
use hikasa_core::JobKind;
use hikasa_folder::NullObserver;
use hikasa_report::{ReportKind, ReportWords};
use hikasa_session::api::{
    FolderCompareRequest, ReportFlags, ReportSource, ReportTarget, TextCompareRequest,
};

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

/// The bound of every row.
const BOUND: usize = 4 << 20;

/// The peak of live heap above the live heap before `f`.
fn peak_of(f: impl FnOnce()) -> usize {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    f();
    PEAK.load(Ordering::Relaxed).saturating_sub(base)
}

/// A7.14: every text layout streams into a file target.
#[test]
fn a7_14_text_reports_stream_into_a_file() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let service = service(root);
    let body = |side: &str| -> String {
        (0..50_000)
            .map(|i| {
                if i % 7 == 0 {
                    format!("{side} line {i} with some text\n")
                } else {
                    format!("line {i} with some text\n")
                }
            })
            .collect()
    };
    let left = write(root, "l.txt", body("left").as_bytes());
    let right = write(root, "r.txt", body("right").as_bytes());
    let request = TextCompareRequest {
        left: os(&left),
        right: os(&right),
        ..TextCompareRequest::default()
    };
    let resp = service
        .text_compare(&request, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    let out = root.join("out.report");
    let target = ReportTarget::File(out.clone().into_os_string());
    for (layout, html) in [
        ("unified", false),
        ("side-by-side", false),
        ("side-by-side", true),
        ("summary", true),
        ("interleaved", false),
        ("interleaved", true),
        ("patch", false),
        ("statistics", false),
        ("xml", false),
    ] {
        let flags = ReportFlags {
            words: ReportWords {
                layout: Some(layout.to_owned()),
                ..ReportWords::default()
            },
            html: html.then_some(false),
            ..ReportFlags::default()
        };
        let mut req = service
            .report_request(ReportKind::Text, "side-by-side", &flags, &target)
            .unwrap();
        let control = job(&service, JobKind::Report);
        let plan = service
            .report_plan(ReportSource::Text(&resp), &mut req, &target, None, &control)
            .unwrap();
        let mut bytes = 0;
        let peak = peak_of(|| {
            bytes = service
                .report_execute(ReportSource::Text(&resp), &req, plan, None, &control)
                .unwrap()
                .bytes;
        });
        assert_eq!(std::fs::metadata(&out).unwrap().len(), bytes, "{layout}");
        assert!(
            peak <= BOUND,
            "{layout} (html {html}): {peak} B live at the peak for {bytes} B written"
        );
        eprintln!("{layout} html={html}: peak {peak} B, {bytes} B written");
    }
    // `write_output_with` directly: the renderer writes into the target's stream.
    let flags = ReportFlags {
        words: ReportWords {
            layout: Some("interleaved".into()),
            ..ReportWords::default()
        },
        ..ReportFlags::default()
    };
    let req = service
        .report_request(ReportKind::Text, "side-by-side", &flags, &target)
        .unwrap();
    let meta = hikasa_report::ReportMeta::for_text(
        &resp.comparison,
        match &req.body {
            hikasa_report::ReportBody::Text(opts) => opts,
            hikasa_report::ReportBody::Folder { .. } => unreachable!(),
        },
        None,
        "hikasa test",
    );
    let control = job(&service, JobKind::Report);
    let peak = peak_of(|| {
        service
            .write_output_with(out.as_os_str(), &control, &mut |w| {
                hikasa_report::render_report(
                    hikasa_report::ReportInput::Text(&resp.comparison),
                    &req,
                    &meta,
                    &hikasa_report::HtmlStyle::default(),
                    None,
                    w,
                )
                .map(drop)
                .map_err(hikasa_session::ApiError::Report)
            })
            .unwrap();
    });
    assert!(peak <= BOUND, "write_output_with: {peak} B");
}

/// A7.14: folder side-by-side HTML and XML stream into a file target.
#[test]
fn a7_14_folder_reports_stream_into_a_file() {
    let _one = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for i in 0..3_000 {
        write(root, &format!("l/d{}/f{i}.txt", i % 30), b"left\n");
        write(
            root,
            &format!("r/d{}/f{i}.txt", i % 30),
            if i % 3 == 0 { b"right!\n" } else { b"left\n" },
        );
    }
    let service = service(root);
    let request = FolderCompareRequest {
        left: os(&root.join("l")),
        right: os(&root.join("r")),
        ..FolderCompareRequest::default()
    };
    let control = job(&service, JobKind::FolderCompare);
    let prepared = service.folder_prepare_in(&request, &control).unwrap();
    let response = service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    let source = ReportSource::Folder {
        request: &request,
        prepared: &prepared,
        response: &response,
    };
    let out = root.join("out.report");
    let target = ReportTarget::File(out.clone().into_os_string());
    for (layout, html) in [("side-by-side", true), ("xml", false)] {
        let flags = ReportFlags {
            words: ReportWords {
                layout: Some(layout.to_owned()),
                options: vec!["display-all".into()],
                ..ReportWords::default()
            },
            html: html.then_some(false),
            ..ReportFlags::default()
        };
        let mut req = service
            .report_request(ReportKind::Folder, "side-by-side", &flags, &target)
            .unwrap();
        let control = job(&service, JobKind::Report);
        let plan = service
            .report_plan(source, &mut req, &target, None, &control)
            .unwrap();
        let mut bytes = 0;
        let peak = peak_of(|| {
            bytes = service
                .report_execute(source, &req, plan, None, &control)
                .unwrap()
                .bytes;
        });
        // 3,030 visible rows: the bound of §1.7.17 (4 MiB + 32 B a row) holds with room.
        assert!(
            peak <= BOUND + 32 * 3_030,
            "{layout}: {peak} B for {bytes} B written"
        );
        // The report (about 1 MB) is never held: the peak stays under half of it.
        assert!(
            (peak as u64) < bytes / 2,
            "{layout}: {peak} B live for {bytes} B written: the report is held"
        );
        eprintln!("folder {layout}: peak {peak} B, {bytes} B written");
    }
}

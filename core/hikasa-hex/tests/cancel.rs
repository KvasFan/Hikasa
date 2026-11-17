//! A12.11 (G4, untimed share): every long operation cancelled midway returns `Cancelled`, never a partial result
//! — Fast, Complete, the None scan and find, mapped and streamed. The timed rows (≤ 100 ms on a 256 MiB pair) are
//! in `tests/gates.rs`.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};

use common::{Edit, XorShift, edited};
use hikasa_core::{CancelToken, Ctl, Phase, Progress, ProgressSink};
use hikasa_hex::{AlignMode, HexAlignOptions, HexFinder, HexPattern, align};
use hikasa_vfs::VISIT_CHUNK;

/// Cancels `token` at the `after`-th progress report.
struct CancelAt<'a> {
    token: &'a CancelToken,
    seen: AtomicU64,
    after: u64,
}

impl ProgressSink for CancelAt<'_> {
    fn report(&self, _progress: Progress) {
        if self.seen.fetch_add(1, Ordering::Relaxed) + 1 == self.after {
            self.token.cancel();
        }
    }
    fn due(&self, _phase: Phase) -> bool {
        true
    }
}

#[test]
fn long_operations_cancel_midway() {
    let n = 3 * VISIT_CHUNK as usize + 1000;
    let a = XorShift(1).bytes(n);
    let b = edited(
        &a,
        &[
            Edit::Insert { at: n / 2, len: 10 },
            Edit::Replace { at: 100, len: 50 },
        ],
        3,
    );
    common::every_reading(&a, &b, &mut |label, l, r, stream| {
        // A one-pass operation (None, find) sees a cancel only before its last window is claimed: with one thread a
        // window is 2 chunks of the 3 here; with more threads the whole range is one window.
        let one_pass = stream.threads == 1;
        for mode in [AlignMode::None, AlignMode::Fast, AlignMode::Complete] {
            if mode == AlignMode::None && !one_pass {
                continue;
            }
            let token = CancelToken::new();
            let sink = CancelAt {
                token: &token,
                seen: AtomicU64::new(0),
                after: 1,
            };
            let ctl = Ctl {
                cancel: &token,
                progress: &sink,
            };
            let o = HexAlignOptions {
                stream,
                ..HexAlignOptions::with_mode(mode)
            };
            let result = align(l, r, &o, &ctl);
            assert!(
                matches!(&result, Err(e) if e.is_cancelled()),
                "{label} {} {mode}: {:?}",
                stream.threads,
                result.map(|al| al.stats)
            );
        }
        if !one_pass {
            return;
        }
        let token = CancelToken::new();
        let sink = CancelAt {
            token: &token,
            seen: AtomicU64::new(0),
            after: 1,
        };
        let ctl = Ctl {
            cancel: &token,
            progress: &sink,
        };
        let f = HexFinder::new(&HexPattern::Bytes {
            bytes: vec![Some(1), Some(2), Some(3)],
        })
        .unwrap();
        let result = f.all(l, u64::MAX - 1, &stream, &ctl);
        assert!(
            matches!(&result, Err(e) if e.is_cancelled()),
            "{label} {} find: {:?} after {} reports",
            stream.threads,
            result.as_ref().map(|h| h.0.len()),
            sink.seen.load(Ordering::Relaxed)
        );
    });
}

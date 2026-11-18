//! A12.18 (G4, untimed share): a picture diff, a new geometry and a full render cancelled midway return
//! `Cancelled`, never a partial result. The timed rows (≤ 100 ms) are in `tests/gates.rs`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use common::{THREADS, in_pool, noise};
use hikasa_core::{CancelToken, Ctl, Phase, Progress, ProgressSink};
use hikasa_picture::{
    DiffMode, Geometry, Pane, PictureColors, PictureError, PictureOptions, compare_pictures,
};

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
fn diff_geometry_and_render_cancel_midway() {
    let left = noise(300, 700, 1);
    let right = noise(310, 690, 2);
    let options = PictureOptions {
        tolerance: 3,
        geometry: Geometry {
            offset: (5, -4),
            ..Geometry::default()
        },
    };
    let base = compare_pictures(
        Arc::clone(&left),
        Arc::clone(&right),
        &options,
        &Ctl::detached(),
    )
    .unwrap();
    for threads in THREADS {
        for after in [1, 3] {
            let run = |op: &(dyn Fn(&Ctl<'_>) -> Result<(), PictureError> + Sync)| {
                let token = CancelToken::new();
                let sink = CancelAt {
                    token: &token,
                    seen: AtomicU64::new(0),
                    after,
                };
                let ctl = Ctl {
                    cancel: &token,
                    progress: &sink,
                };
                let result = in_pool(threads, || op(&ctl));
                assert!(
                    token.is_cancelled(),
                    "{threads} threads: the operation reported no progress"
                );
                assert!(
                    matches!(result, Err(PictureError::Cancelled)),
                    "{threads} threads, after {after}: {result:?}"
                );
            };
            run(&|ctl| {
                compare_pictures(Arc::clone(&left), Arc::clone(&right), &options, ctl).map(drop)
            });
            run(&|ctl| {
                base.with_geometry(
                    Geometry {
                        offset: (0, 9),
                        ..Geometry::default()
                    },
                    ctl,
                )
                .map(drop)
            });
            run(&|ctl| {
                base.render(
                    Pane::Difference,
                    DiffMode::Range,
                    &PictureColors::default(),
                    true,
                    ctl,
                )
                .map(drop)
            });
        }
    }
}

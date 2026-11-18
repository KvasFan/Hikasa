//! The option and geometry jobs of the Picture Compare view (PX-003, PX-004; §1.13.9, §1.13.12, D-361, D-364):
//! `with_options` (a tolerance) and `with_geometry` (offset, orientations, Auto Scale) — named only here in `gui/` —
//! run latest-wins: at most one runs; a change while it runs is remembered and the next job starts when it lands
//! (W11's `Recompare` pattern), so a drag of 60 moves a second always ends on the last offset. Also the offset drag's
//! arithmetic (sub-pixel carry) and the transforms of the current picture. Toolkit-free.

use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_picture::{Geometry, MAX_OFFSET, Orientation, Pane, PictureComparison, PictureOptions};
use hikasa_session::api::PictureLoaded;
use hikasa_session::{ApiError, JobControl, Service};

use super::{Calls, Gate};
use crate::job::{Job, Wake};

/// What a landed option or geometry job brings: the comparison and, when it is all same and the view did not know
/// yet, whether the inputs hold the same bytes (D-339).
pub struct Refined {
    /// The comparison.
    pub comparison: Arc<PictureComparison>,
    /// `picture_bytes_identical`, asked once.
    pub identical: Option<bool>,
}

/// Which call a job made (the counters of A13.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefineKind {
    /// `with_options` (the tolerance changed, maybe the geometry too).
    Options,
    /// `with_geometry` (only the geometry changed).
    Geometry,
}

/// The latest-wins runner of option and geometry jobs.
#[derive(Default)]
pub struct Refine {
    running: Option<(Job<Refined>, PictureOptions)>,
    queued: bool,
}

impl Refine {
    /// True while a job runs.
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// True while the job's result is being published.
    pub fn settling(&self) -> bool {
        self.running.as_ref().is_some_and(|(j, _)| j.settling())
    }

    /// The options of the running job.
    pub fn running_options(&self) -> Option<PictureOptions> {
        self.running.as_ref().map(|(_, o)| *o)
    }

    /// The running job (the job indicator).
    pub fn job(&self) -> Option<&Job<Refined>> {
        self.running.as_ref().map(|(j, _)| j)
    }

    /// Cancels the running job (the view closes or reloads) and forgets the queued change.
    pub fn cancel(&mut self) {
        if let Some((job, _)) = self.running.take() {
            job.cancel();
        }
        self.queued = false;
    }

    /// Asks for `wanted` from `base`: starts a job now, or remembers the change while one runs. Returns the kind of
    /// a job started.
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        service: &Service,
        wake: &Wake,
        calls: &Calls,
        loaded: &Arc<PictureLoaded>,
        base: &Arc<PictureComparison>,
        wanted: PictureOptions,
        know_identity: bool,
    ) -> Option<RefineKind> {
        if self.running.is_some() {
            self.queued = true;
            return None;
        }
        if *base.options() == wanted {
            return None;
        }
        let kind = if base.options().tolerance == wanted.tolerance {
            RefineKind::Geometry
        } else {
            RefineKind::Options
        };
        match kind {
            RefineKind::Options => calls
                .counters
                .options
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            RefineKind::Geometry => calls
                .counters
                .geometry
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        };
        let base = Arc::clone(base);
        let loaded = Arc::clone(loaded);
        let gate = calls.gate.clone();
        let counters = Arc::clone(&calls.counters);
        let title = match kind {
            RefineKind::Options => "compare the pictures again",
            RefineKind::Geometry => "place the pictures",
        };
        let job = Job::spawn(
            service,
            JobKind::PictureCompare,
            title.into(),
            wake,
            move |svc, job| {
                if let Some(g) = &gate {
                    g.pass(match kind {
                        RefineKind::Options => super::GateKind::Options,
                        RefineKind::Geometry => super::GateKind::Geometry,
                    });
                }
                let now = counters
                    .running
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                counters
                    .max_running
                    .fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                let _done = Running(&counters);
                let ctl = job.ctl();
                let compared = svc.pool().install(|| match kind {
                    RefineKind::Options => base.with_options(wanted, &ctl),
                    RefineKind::Geometry => base.with_geometry(wanted.geometry, &ctl),
                });
                let comparison = Arc::new(compared.map_err(|e| {
                    if e.is_cancelled() {
                        ApiError::Cancelled
                    } else {
                        ApiError::Picture(e, None)
                    }
                })?);
                let identical = identity(svc, job, &loaded, &comparison, know_identity, &counters)?;
                Ok(Refined {
                    comparison,
                    identical,
                })
            },
        );
        self.running = Some((job, wanted));
        self.queued = false;
        Some(kind)
    }

    /// The landed job, once; `(result, a change waits)`.
    pub fn take(&mut self) -> Option<(Result<Refined, ApiError>, bool)> {
        let result = self.running.as_ref()?.0.try_result()?;
        self.running = None;
        Some((result, std::mem::take(&mut self.queued)))
    }
}

/// Counts a running option or geometry job down when it ends.
struct Running<'a>(&'a super::JobCounters);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0
            .running
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// `picture_bytes_identical` when the comparison is all same and the view does not know it yet.
pub(super) fn identity(
    svc: &Service,
    job: &JobControl,
    loaded: &PictureLoaded,
    comparison: &PictureComparison,
    known: bool,
    counters: &super::JobCounters,
) -> Result<Option<bool>, ApiError> {
    if known || !comparison.stats().all_same() {
        return Ok(None);
    }
    counters
        .identity
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    svc.picture_bytes_identical(loaded, job).map(Some)
}

/// The offset drag of the difference pane (§1.13.12): the offset at its start and the sub-pixel rest carried to the
/// next move.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetDrag {
    /// The offset when the drag started (Esc returns to it).
    pub start: (i32, i32),
    /// The rest of the moves in area pixels not applied yet.
    pub carry: (f64, f64),
}

impl OffsetDrag {
    /// A drag from `offset`.
    pub fn new(offset: (i32, i32)) -> OffsetDrag {
        OffsetDrag {
            start: offset,
            carry: (0.0, 0.0),
        }
    }

    /// A move of `delta` points at `scale` (physical pixels per area pixel) and `ppp`: the whole area pixels to add to
    /// the offset (`Δpoints × ppp / z`), the rest carried.
    pub fn moved(&mut self, delta: (f64, f64), scale: f64, ppp: f64) -> (i64, i64) {
        let k = ppp / scale;
        let x = self.carry.0 + delta.0 * k;
        let y = self.carry.1 + delta.1 * k;
        let (wx, wy) = (x.trunc(), y.trunc());
        self.carry = (x - wx, y - wy);
        (wx as i64, wy as i64)
    }
}

/// `offset + (dx, dy)` within ±2²⁰ (D-340); the second value is true when a component stopped at the limit.
pub fn add_offset(offset: (i32, i32), d: (i64, i64)) -> ((i32, i32), bool) {
    let max = i64::from(MAX_OFFSET);
    let one = |v: i32, d: i64| {
        let n = i64::from(v) + d;
        (n.clamp(-max, max) as i32, !(-max..=max).contains(&n))
    };
    let (x, sx) = one(offset.0, d.0);
    let (y, sy) = one(offset.1, d.1);
    ((x, y), sx || sy)
}

/// A transform of the current picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// 90° clockwise.
    RotateRight,
    /// 90° counterclockwise.
    RotateLeft,
    /// Mirror about the vertical axis.
    FlipHorz,
    /// Mirror about the horizontal axis.
    FlipVert,
}

impl Transform {
    /// The orientation of the transform.
    pub fn orientation(self) -> Orientation {
        match self {
            Transform::RotateRight => Orientation::Cw90,
            Transform::RotateLeft => Orientation::Ccw90,
            Transform::FlipHorz => Orientation::FlipH,
            Transform::FlipVert => Orientation::FlipV,
        }
    }
}

/// The geometry after `transform` of the focused pane's picture (the difference pane: the right one, **open
/// default**), composed with `Orientation::then`.
pub fn transformed(geometry: Geometry, focus: Pane, transform: Transform) -> Geometry {
    let mut g = geometry;
    let side = match focus {
        Pane::Left => &mut g.orientation.left,
        Pane::Right | Pane::Difference => &mut g.orientation.right,
    };
    *side = side.then(transform.orientation());
    g
}

/// A gate a test closes to hold job bodies of a kind.
pub(super) fn pass(gate: &Option<Arc<Gate>>, kind: super::GateKind) {
    if let Some(g) = gate {
        g.pass(kind);
    }
}

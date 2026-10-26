//! Crate-internal helpers: chunked parallelism on the caller's pool only (T2, T7), monotone progress reports
//! from worker threads (§5.4), and the release of large buffers off the calling thread when a cancellation
//! unwinds (§5.3).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::progress::{Ctl, Phase, Progress, Unit};
use rayon::prelude::*;

/// Runs `f(k)` for `k in 0..n` — on `pool` when given (never on the global pool, T7), else on the current
/// thread — and collects the results in index order, so the output never depends on the pool (T2). The first
/// `Err(Cancelled)` in index order wins.
pub(crate) fn par_map<T, F>(
    pool: Option<&rayon::ThreadPool>,
    n: usize,
    f: F,
) -> Result<Vec<T>, Cancelled>
where
    T: Send,
    F: Fn(usize) -> Result<T, Cancelled> + Sync + Send,
{
    match pool {
        None => (0..n).map(f).collect(),
        Some(pool) => pool.install(|| (0..n).into_par_iter().map(&f).collect()),
    }
}

/// Runs `f(k, chunk)` over the `size`-element chunks of `items` (on `pool` when given), in place.
pub(crate) fn par_chunks_mut<T, F>(
    pool: Option<&rayon::ThreadPool>,
    items: &mut [T],
    size: usize,
    f: F,
) -> Result<(), Cancelled>
where
    T: Send,
    F: Fn(usize, &mut [T]) -> Result<(), Cancelled> + Sync + Send,
{
    let size = size.max(1);
    match pool {
        None => items
            .chunks_mut(size)
            .enumerate()
            .try_for_each(|(k, chunk)| f(k, chunk)),
        Some(pool) => pool.install(|| {
            items
                .par_chunks_mut(size)
                .enumerate()
                .try_for_each(|(k, chunk)| f(k, chunk))
        }),
    }
}

/// Drops `value` on a short-lived background thread (inline when no thread can be started). A cancellation must
/// return within 100 ms (§5.3), and unmapping hundreds of MiB of buffers takes tens of milliseconds (more under
/// memory pressure), so the cancellation paths hand their large buffers here. The thread only frees memory; it
/// touches no shared state and decides nothing (T6).
pub(crate) fn release_later<T: Send + 'static>(value: T) {
    // When no thread can be started, `spawn` drops the closure, and the value in it, right here.
    let _ = std::thread::Builder::new()
        .name("hikasa-text-release".into())
        .spawn(move || drop(value));
}

/// Owns a large value and, when it is dropped after its cancellation token fired, releases it with
/// [`release_later`] instead of freeing it on the calling thread. Read and take the value through the public
/// field.
pub(crate) struct ReleaseOnCancel<'c, T: Default + Send + 'static> {
    /// The value.
    pub(crate) value: T,
    cancel: &'c CancelToken,
}

impl<'c, T: Default + Send + 'static> ReleaseOnCancel<'c, T> {
    /// Guards `value`.
    pub(crate) fn new(value: T, cancel: &'c CancelToken) -> Self {
        ReleaseOnCancel { value, cancel }
    }
}

impl<T: Default + Send + 'static> Drop for ReleaseOnCancel<'_, T> {
    fn drop(&mut self) {
        if self.cancel.is_cancelled() {
            release_later(std::mem::take(&mut self.value));
        }
    }
}

/// Progress of one phase fed from several threads: `add` counts work done anywhere and forwards a report only
/// when the sink is due, under a lock that keeps the forwarded `done` values increasing (§5.4: `done` never
/// decreases within a phase). Callers add in batches (every 1,024 lines or 4,096 rows), so the lock is taken
/// rarely.
pub(crate) struct PhaseProgress<'c> {
    ctl: &'c Ctl<'c>,
    phase: Phase,
    unit: Unit,
    total: u64,
    done: AtomicU64,
    reported: Mutex<u64>,
}

impl<'c> PhaseProgress<'c> {
    /// Starts `phase` (an unconditional report of `0 / total`).
    pub(crate) fn start(
        ctl: &'c Ctl<'c>,
        phase: Phase,
        unit: Unit,
        total: u64,
    ) -> PhaseProgress<'c> {
        ctl.progress_now(Progress {
            phase,
            done: 0,
            total: Some(total),
            unit,
            current: None,
        });
        PhaseProgress {
            ctl,
            phase,
            unit,
            total,
            done: AtomicU64::new(0),
            reported: Mutex::new(0),
        }
    }

    /// Adds `n` units of work.
    pub(crate) fn add(&self, n: u64) {
        let done = (self.done.fetch_add(n, Ordering::Relaxed) + n).min(self.total);
        self.report(done);
    }

    /// Sets the absolute progress (single-threaded callers such as the alignment sink).
    pub(crate) fn set(&self, done: u64) {
        let done = done.min(self.total);
        self.done.fetch_max(done, Ordering::Relaxed);
        self.report(done);
    }

    fn report(&self, done: u64) {
        if !self.ctl.progress.due(self.phase) {
            return;
        }
        let mut reported = self
            .reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if done > *reported {
            *reported = done;
            self.ctl.progress.report(Progress {
                phase: self.phase,
                done,
                total: Some(self.total),
                unit: self.unit,
                current: None,
            });
        }
    }

    /// Ends the phase (an unconditional report of `total / total`).
    pub(crate) fn finish(&self) {
        let mut reported = self
            .reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *reported = self.total;
        self.ctl.progress_now(Progress {
            phase: self.phase,
            done: self.total,
            total: Some(self.total),
            unit: self.unit,
            current: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::cancel::CancelToken;

    #[test]
    fn par_map_is_ordered_for_any_pool() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap();
        let seq = par_map(None, 100, |k| Ok(k * 2)).unwrap();
        let par = par_map(Some(&pool), 100, |k| Ok(k * 2)).unwrap();
        assert_eq!(seq, par);
        assert_eq!(
            par_map(Some(&pool), 10, |k| if k == 7 {
                Err(Cancelled)
            } else {
                Ok(k)
            }),
            Err(Cancelled)
        );
        let mut items = vec![0u32; 1000];
        par_chunks_mut(Some(&pool), &mut items, 64, |k, chunk| {
            chunk.iter_mut().for_each(|x| *x = k as u32);
            Ok(())
        })
        .unwrap();
        assert_eq!(items[999], 15);
    }

    /// A value guarded by `ReleaseOnCancel` is dropped in place normally and on another thread after a
    /// cancellation.
    #[test]
    fn large_values_are_released_off_thread_after_cancellation() {
        #[derive(Default)]
        struct Probe(Option<std::sync::mpsc::Sender<std::thread::ThreadId>>);
        impl Drop for Probe {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(std::thread::current().id());
                }
            }
        }
        let here = std::thread::current().id();
        let (tx, rx) = std::sync::mpsc::channel();
        let never = CancelToken::never();
        drop(ReleaseOnCancel::new(Probe(Some(tx.clone())), &never));
        assert_eq!(rx.recv().unwrap(), here);
        let cancel = CancelToken::new();
        cancel.cancel();
        drop(ReleaseOnCancel::new(Probe(Some(tx)), &cancel));
        assert_ne!(rx.recv().unwrap(), here);
    }

    #[test]
    fn progress_is_monotone() {
        let seen = Mutex::new(Vec::new());
        let sink = |p: Progress| seen.lock().unwrap().push(p.done);
        let cancel = CancelToken::never();
        let ctl = Ctl {
            cancel: &cancel,
            progress: &sink,
        };
        let p = PhaseProgress::start(&ctl, Phase::Tokenizing, Unit::Lines, 10);
        p.add(3);
        p.set(2);
        p.add(30);
        p.finish();
        let seen = seen.into_inner().unwrap();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&10));
    }
}

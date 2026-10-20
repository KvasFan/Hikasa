//! Cooperative cancellation (ARCH-003, §5.3).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// [`CancelInner::verified`] before any check: no epoch is known to be clear.
const UNVERIFIED: u64 = u64::MAX;

/// Shared state of a live token.
///
/// A token is cancelled when its own flag or the flag of any ancestor is set (INV-CANCEL-1/2). Walking the
/// ancestors on every check would cost O(depth), so every tree of tokens shares an *epoch*: the number of
/// effective `cancel()` calls anywhere in the tree. A token remembers an epoch at which none of its ancestors
/// was cancelled (`verified`); while the tree's epoch still has that value, no ancestor can have been cancelled
/// since, and the check is O(1). A walk happens only after a cancellation in the tree, once per token.
struct CancelInner {
    /// Set by `cancel()` on this token (or a clone of it).
    cancelled: AtomicBool,
    /// The parent's state; kept alive so that cancelling any ancestor still reaches this token.
    parent: Option<Arc<CancelInner>>,
    /// Effective cancellations in this token's tree (shared by the whole tree).
    epoch: Arc<AtomicU64>,
    /// An epoch at which no ancestor of this token was cancelled ([`UNVERIFIED`] if none is known).
    verified: AtomicU64,
}

impl CancelInner {
    fn root() -> CancelInner {
        CancelInner {
            cancelled: AtomicBool::new(false),
            parent: None,
            epoch: Arc::new(AtomicU64::new(0)),
            // A root has no ancestors: it is clear at the initial epoch.
            verified: AtomicU64::new(0),
        }
    }

    fn child_of(parent: &Arc<CancelInner>) -> CancelInner {
        // Load the epoch first (Acquire): every cancellation counted in it is visible to the loads below. When
        // the parent itself is live and its ancestors are known clear at this epoch, so are the child's.
        let epoch = parent.epoch.load(Ordering::Acquire);
        let clear = !parent.cancelled.load(Ordering::Acquire)
            && parent.verified.load(Ordering::Relaxed) == epoch;
        CancelInner {
            cancelled: AtomicBool::new(false),
            parent: Some(Arc::clone(parent)),
            epoch: Arc::clone(&parent.epoch),
            verified: AtomicU64::new(if clear { epoch } else { UNVERIFIED }),
        }
    }

    fn cancel(&self) {
        // The flag is set before the epoch moves (Release): a check that sees the new epoch sees the flag.
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn is_cancelled(&self) -> bool {
        let epoch = self.epoch.load(Ordering::Acquire);
        if self.cancelled.load(Ordering::Acquire) {
            return true;
        }
        if self.verified.load(Ordering::Relaxed) == epoch {
            return false;
        }
        let mut node = self.parent.as_deref();
        while let Some(ancestor) = node {
            if ancestor.cancelled.load(Ordering::Acquire) {
                return true;
            }
            node = ancestor.parent.as_deref();
        }
        self.verified.store(epoch, Ordering::Relaxed);
        false
    }
}

impl Drop for CancelInner {
    /// Unlinks the ancestor chain iteratively: dropping a token at the end of a long `child()` chain must not
    /// recurse once per level (that would overflow the stack and abort the process, T9).
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(node) = next {
            next = match Arc::into_inner(node) {
                // The last reference: take its parent before it is dropped (its own drop then has nothing to do).
                Some(mut inner) => inner.parent.take(),
                None => None,
            };
        }
    }
}

impl fmt::Debug for CancelInner {
    /// Shows this token's own state only (never walks the ancestors, which may be many).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelInner")
            .field("cancelled", &self.cancelled.load(Ordering::Relaxed))
            .field("has_parent", &self.parent.is_some())
            .finish_non_exhaustive()
    }
}

/// Cooperative cancellation. Cloning shares the flag; `child()` creates a token that is cancelled when its
/// parent is (INV-CANCEL-1), while cancelling a child never cancels the parent (INV-CANCEL-2).
///
/// ```
/// use hikasa_core::cancel::CancelToken;
///
/// let job = CancelToken::new();
/// let step = job.child();
/// step.cancel();
/// assert!(step.is_cancelled() && !job.is_cancelled());
/// let other = job.child();
/// job.cancel();
/// assert!(other.is_cancelled());
/// ```
#[derive(Debug, Clone)]
pub struct CancelToken {
    /// `None` = never cancelled (const-constructible).
    inner: Option<Arc<CancelInner>>,
}

impl CancelToken {
    /// A new, live token.
    // `Default` is deliberately not implemented: "a default token" would be ambiguous between `new()` and
    // `never()`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> CancelToken {
        CancelToken {
            inner: Some(Arc::new(CancelInner::root())),
        }
    }

    /// A token that is never cancelled (tests, quick helpers). `cancel()` on it does nothing.
    pub const fn never() -> CancelToken {
        CancelToken { inner: None }
    }

    /// Requests cancellation (idempotent; Release ordering).
    pub fn cancel(&self) {
        if let Some(inner) = &self.inner {
            inner.cancel();
        }
    }

    /// True if this token or any ancestor was cancelled (Acquire ordering). O(1) unless a token of the same tree
    /// was cancelled since this token's last check; then the ancestors are walked once.
    pub fn is_cancelled(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.is_cancelled())
    }

    /// `Err(Cancelled)` when cancelled, for `?` use at check points.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// A child token: cancelled when `self` (or any ancestor) is; cancelling it leaves `self` untouched.
    pub fn child(&self) -> CancelToken {
        let inner = match &self.inner {
            Some(parent) => CancelInner::child_of(parent),
            // The child of a token that is never cancelled is a new, independent root.
            None => CancelInner::root(),
        };
        CancelToken {
            inner: Some(Arc::new(inner)),
        }
    }
}

/// Marker error: the operation observed cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("operation cancelled")]
pub struct Cancelled;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_and_child() {
        let parent = CancelToken::new();
        let child = parent.child();
        let grandchild = child.child();
        assert!(!grandchild.is_cancelled());
        child.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled(), "INV-CANCEL-1 through two levels");
        assert!(!parent.is_cancelled(), "INV-CANCEL-2");
        let sibling = parent.child();
        assert!(!sibling.is_cancelled());
        parent.cancel();
        assert!(sibling.is_cancelled());
        assert_eq!(sibling.check(), Err(Cancelled));
    }

    #[test]
    fn clones_share_the_flag_and_never_stays_live() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert_eq!(clone.check(), Ok(()));
        token.cancel();
        token.cancel();
        assert!(clone.is_cancelled());
        let never = CancelToken::never();
        never.cancel();
        assert!(!never.is_cancelled());
        let child = never.child();
        assert!(!child.is_cancelled());
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!never.is_cancelled());
    }

    #[test]
    fn tokens_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CancelToken>();
    }

    /// A chain built by `t = t.child()` in a loop (e.g. a retry loop): dropping it must not recurse per level.
    /// The thread has the default 2 MiB stack of worker threads; the old recursive drop aborted at this depth.
    #[test]
    fn deep_chains_drop_and_check_without_recursion() {
        let handle = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(|| {
                let root = CancelToken::new();
                let mut leaf = root.child();
                for _ in 0..300_000 {
                    leaf = leaf.child();
                    assert!(!leaf.is_cancelled());
                }
                assert!(format!("{leaf:?}").contains("cancelled: false"));
                let sibling = root.child();
                sibling.cancel();
                assert!(
                    !leaf.is_cancelled(),
                    "a sibling's cancellation does not reach the chain"
                );
                root.cancel();
                assert!(leaf.is_cancelled(), "INV-CANCEL-1 through 300,000 levels");
                drop(root);
                drop(leaf);
                true
            })
            .unwrap();
        assert!(handle.join().unwrap());
    }

    #[test]
    fn checks_after_a_cancellation_elsewhere_in_the_tree_stay_correct() {
        let root = CancelToken::new();
        let a = root.child();
        let a1 = a.child();
        let b = root.child();
        assert!(!a1.is_cancelled() && !b.is_cancelled());
        b.cancel();
        assert!(
            !a1.is_cancelled(),
            "the epoch moved, the walk finds no cancelled ancestor"
        );
        assert!(!a1.is_cancelled(), "now cached at the new epoch");
        let a2 = a1.child();
        assert!(!a2.is_cancelled());
        a.cancel();
        assert!(a1.is_cancelled() && a2.is_cancelled());
        assert!(!root.is_cancelled() && !CancelToken::never().is_cancelled());
        // A child created after its parent was cancelled is cancelled from the start.
        assert!(a.child().is_cancelled());
        assert!(a1.child().child().is_cancelled());
    }

    #[test]
    fn concurrent_cancellation_is_observed() {
        for _ in 0..200 {
            let root = CancelToken::new();
            let mut leaf = root.child();
            for _ in 0..16 {
                leaf = leaf.child();
            }
            std::thread::scope(|scope| {
                let watcher = scope.spawn(|| {
                    while !leaf.is_cancelled() {
                        std::hint::spin_loop();
                    }
                });
                let sibling = root.child();
                scope.spawn(move || sibling.cancel());
                root.cancel();
                watcher.join().unwrap();
            });
        }
    }
}

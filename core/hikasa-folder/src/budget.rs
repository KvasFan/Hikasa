//! [`ByteBudget`]: the shared memory budget of the content tests of one scan (D-25, §5.2, INV-BUDGET-1).

use std::time::Duration;

use hikasa_core::{CancelToken, Cancelled};
use parking_lot::{Condvar, Mutex};

/// Wait slice between two cancellation checks while bytes are not available (§5.3).
const WAIT_SLICE: Duration = Duration::from_millis(10);

/// A counting semaphore over bytes (parking_lot `Mutex` + `Condvar`), shared by all content tests of one scan.
#[derive(Debug)]
pub struct ByteBudget {
    capacity: u64,
    available: Mutex<u64>,
    released: Condvar,
}

impl ByteBudget {
    /// A budget of `capacity` bytes (≥ 1; 0 is taken as 1).
    pub fn new(capacity: u64) -> ByteBudget {
        let capacity = capacity.max(1);
        ByteBudget {
            capacity,
            available: Mutex::new(capacity),
            released: Condvar::new(),
        }
    }

    /// Takes `min(bytes, capacity)` bytes, blocking while they are not available. Waits in 10 ms slices
    /// (`Condvar::wait_for`) to observe `cancel` (returns `Err(Cancelled)`). The guard returns the bytes on drop.
    pub fn acquire(&self, bytes: u64, cancel: &CancelToken) -> Result<BudgetGuard<'_>, Cancelled> {
        let want = bytes.min(self.capacity);
        let mut available = self.available.lock();
        loop {
            cancel.check()?;
            if *available >= want {
                *available -= want;
                return Ok(BudgetGuard {
                    budget: self,
                    bytes: want,
                });
            }
            self.released.wait_for(&mut available, WAIT_SLICE);
        }
    }

    /// The capacity in bytes. Additive API.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Bytes not held by any guard right now. Additive API (tests and diagnostics).
    pub fn available(&self) -> u64 {
        *self.available.lock()
    }
}

/// Returns its bytes to the budget on drop.
#[derive(Debug)]
pub struct BudgetGuard<'a> {
    budget: &'a ByteBudget,
    bytes: u64,
}

impl BudgetGuard<'_> {
    /// Bytes held by this guard. Additive API.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for BudgetGuard<'_> {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let mut available = self.budget.available.lock();
        *available = available
            .saturating_add(self.bytes)
            .min(self.budget.capacity);
        drop(available);
        self.budget.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn larger_than_capacity_takes_everything() {
        let budget = ByteBudget::new(100);
        let never = CancelToken::never();
        let g = budget.acquire(1_000, &never).unwrap();
        assert_eq!(g.bytes(), 100);
        assert_eq!(budget.available(), 0);
        drop(g);
        assert_eq!(budget.available(), 100);
        let zero = budget.acquire(0, &never).unwrap();
        assert_eq!(zero.bytes(), 0);
        assert_eq!(ByteBudget::new(0).capacity(), 1);
    }

    /// INV-BUDGET-1: never more than the capacity in flight.
    #[test]
    fn never_more_than_capacity_in_flight() {
        let budget = Arc::new(ByteBudget::new(1_000));
        let in_flight = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let budget = Arc::clone(&budget);
                let in_flight = Arc::clone(&in_flight);
                let peak = Arc::clone(&peak);
                std::thread::spawn(move || {
                    let never = CancelToken::never();
                    for i in 0..200u64 {
                        let g = budget.acquire((t * 97 + i * 31) % 700 + 1, &never).unwrap();
                        let now = in_flight.fetch_add(g.bytes(), Ordering::SeqCst) + g.bytes();
                        peak.fetch_max(now, Ordering::SeqCst);
                        std::hint::spin_loop();
                        in_flight.fetch_sub(g.bytes(), Ordering::SeqCst);
                        drop(g);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 1_000);
        assert_eq!(budget.available(), 1_000);
    }
}

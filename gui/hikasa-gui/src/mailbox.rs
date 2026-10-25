//! The UI thread's mailbox: a FIFO that any thread posts to and the UI awaits (contract §5.1: IPC handlers and
//! job threads never block, they post to the event loop and return), and the [`Ticker`] that wakes the UI at
//! frame rate while jobs run (`JobHandle::try_result` is polled per frame, §1.10).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;

/// A cross-thread FIFO whose receiving end is a future (the Dioxus event loop awaits it).
pub struct Mailbox<T> {
    inner: Arc<Inner<T>>,
}

struct Inner<T> {
    queue: Mutex<VecDeque<T>>,
    waker: Mutex<Option<Waker>>,
}

impl<T> std::fmt::Debug for Mailbox<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailbox")
            .field("queued", &self.inner.queue.lock().len())
            .finish()
    }
}

impl<T> Clone for Mailbox<T> {
    fn clone(&self) -> Mailbox<T> {
        Mailbox {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Default for Mailbox<T> {
    fn default() -> Mailbox<T> {
        Mailbox::new()
    }
}

impl<T> Mailbox<T> {
    /// An empty mailbox.
    pub fn new() -> Mailbox<T> {
        Mailbox {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                waker: Mutex::new(None),
            }),
        }
    }

    /// Appends a message and wakes the receiver (never blocks for longer than the queue lock).
    pub fn post(&self, msg: T) {
        self.inner.queue.lock().push_back(msg);
        let waker = self.inner.waker.lock().take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// The next message (waits until one is posted).
    pub fn next(&self) -> Next<'_, T> {
        Next { mailbox: self }
    }

    /// Every message posted so far (headless use and tests).
    pub fn drain(&self) -> Vec<T> {
        self.inner.queue.lock().drain(..).collect()
    }

    /// Number of queued messages.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.queue.lock().len()
    }

    /// True when no message is queued.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The future of [`Mailbox::next`].
pub struct Next<'a, T> {
    mailbox: &'a Mailbox<T>,
}

impl<T> Future for Next<'_, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let inner = &self.mailbox.inner;
        if let Some(msg) = inner.queue.lock().pop_front() {
            return Poll::Ready(msg);
        }
        *inner.waker.lock() = Some(cx.waker().clone());
        // A message posted between the first check and storing the waker found no waker: look again.
        match inner.queue.lock().pop_front() {
            Some(msg) => Poll::Ready(msg),
            None => Poll::Pending,
        }
    }
}

/// Posts one "tick" message per frame interval while [`Ticker::set_busy`] is true, never more than one
/// unhandled tick at a time (ticks coalesce when the UI is slow). Idle, the thread parks.
pub struct Ticker {
    shared: Arc<TickerShared>,
    thread: Option<JoinHandle<()>>,
}

struct TickerShared {
    busy: AtomicBool,
    pending: AtomicBool,
    stop: AtomicBool,
    interval_ms: AtomicUsize,
}

impl Ticker {
    /// Starts the ticker thread; `post` delivers one tick.
    pub fn start(interval: Duration, post: impl Fn() + Send + 'static) -> Ticker {
        let shared = Arc::new(TickerShared {
            busy: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            interval_ms: AtomicUsize::new(interval.as_millis().max(1) as usize),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("hikasa-gui-ticker".into())
            .spawn(move || {
                while !worker.stop.load(Ordering::Acquire) {
                    if !worker.busy.load(Ordering::Acquire) {
                        thread::park();
                        continue;
                    }
                    if !worker.pending.swap(true, Ordering::AcqRel) {
                        post();
                    }
                    let ms = worker.interval_ms.load(Ordering::Relaxed) as u64;
                    thread::sleep(Duration::from_millis(ms));
                }
            })
            .ok();
        Ticker { shared, thread }
    }

    /// Turns frame ticks on or off.
    pub fn set_busy(&self, busy: bool) {
        let was = self.shared.busy.swap(busy, Ordering::AcqRel);
        if busy
            && !was
            && let Some(thread) = &self.thread
        {
            thread.thread().unpark();
        }
    }

    /// Called by the UI when it handles a tick (allows the next one).
    pub fn ticked(&self) {
        self.shared.pending.store(false, Ordering::Release);
    }

    /// True while ticks are on.
    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::task::Wake;

    struct Flag(AtomicBool);
    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn mailbox_is_fifo_and_wakes_the_receiver() {
        let mailbox = Mailbox::new();
        let flag = Arc::new(Flag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&flag));
        let mut cx = Context::from_waker(&waker);
        let mut next = mailbox.next();
        assert!(Pin::new(&mut next).poll(&mut cx).is_pending());
        let other = mailbox.clone();
        thread::spawn(move || {
            other.post(1);
            other.post(2);
        })
        .join()
        .unwrap();
        assert!(flag.0.load(Ordering::SeqCst), "post wakes the receiver");
        assert_eq!(Pin::new(&mut next).poll(&mut cx), Poll::Ready(1));
        assert_eq!(mailbox.drain(), vec![2]);
        assert!(mailbox.is_empty());
    }

    #[test]
    fn ticker_ticks_only_while_busy_and_coalesces() {
        let count = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&count);
        let ticker = Ticker::start(Duration::from_millis(1), move || {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        thread::sleep(Duration::from_millis(20));
        assert_eq!(count.load(Ordering::SeqCst), 0, "idle: no tick");
        ticker.set_busy(true);
        assert!(ticker.is_busy());
        thread::sleep(Duration::from_millis(30));
        // Never acknowledged: exactly one tick is outstanding.
        assert_eq!(count.load(Ordering::SeqCst), 1);
        ticker.ticked();
        thread::sleep(Duration::from_millis(30));
        assert_eq!(count.load(Ordering::SeqCst), 2);
        ticker.set_busy(false);
        ticker.ticked();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}

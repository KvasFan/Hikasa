//! The UI thread's mailbox: a FIFO that any thread posts to and the UI drains once per frame (contract §5.1: IPC
//! handlers and job threads never block, they post to the event loop and return). Every post calls the
//! mailbox's *notifier*, which the window sets to `egui::Context::request_repaint`, so the UI wakes up on demand
//! instead of polling.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;

/// Called after every post (from the posting thread; it must not block).
pub type Notifier = Arc<dyn Fn() + Send + Sync>;

/// A cross-thread FIFO with a wake-up callback.
pub struct Mailbox<T> {
    inner: Arc<Inner<T>>,
}

struct Inner<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    notify: Mutex<Option<Notifier>>,
}

impl<T> std::fmt::Debug for Mailbox<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailbox")
            .field("queued", &self.inner.rx.len())
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
    /// An empty mailbox without a notifier.
    pub fn new() -> Mailbox<T> {
        let (tx, rx) = unbounded();
        Mailbox {
            inner: Arc::new(Inner {
                tx,
                rx,
                notify: Mutex::new(None),
            }),
        }
    }

    /// Installs the wake-up callback. Messages posted before it was installed are not lost: the callback is
    /// called once at once when any is queued.
    pub fn set_notifier(&self, notify: Notifier) {
        *self.inner.notify.lock() = Some(Arc::clone(&notify));
        if !self.inner.rx.is_empty() {
            notify();
        }
    }

    /// Appends a message and wakes the receiver (never blocks: the channel is unbounded).
    pub fn post(&self, msg: T) {
        // The receiver lives as long as the mailbox, so sending cannot fail.
        let _ = self.inner.tx.send(msg);
        let notify = self.inner.notify.lock().clone();
        if let Some(notify) = notify {
            notify();
        }
    }

    /// Every message posted so far, in order.
    pub fn drain(&self) -> Vec<T> {
        self.inner.rx.try_iter().collect()
    }

    /// Number of queued messages.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.rx.len()
    }

    /// True when no message is queued.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    #[test]
    fn mailbox_is_fifo_and_notifies_on_every_post() {
        let mailbox = Mailbox::new();
        let woken = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&woken);
        mailbox.set_notifier(Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(woken.load(Ordering::SeqCst), 0, "nothing queued yet");
        let other = mailbox.clone();
        thread::spawn(move || {
            other.post(1);
            other.post(2);
        })
        .join()
        .unwrap();
        assert_eq!(woken.load(Ordering::SeqCst), 2, "one wake-up per post");
        assert_eq!(mailbox.len(), 2);
        assert_eq!(mailbox.drain(), vec![1, 2]);
        assert!(mailbox.is_empty());
    }

    #[test]
    fn a_late_notifier_still_sees_earlier_posts() {
        let mailbox = Mailbox::new();
        mailbox.post("early");
        let woken = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&woken);
        mailbox.set_notifier(Arc::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(
            woken.load(Ordering::SeqCst),
            1,
            "woken for the queued message"
        );
        assert_eq!(mailbox.drain(), vec!["early"]);
        // Without a notifier nothing happens beyond queueing.
        let quiet: Mailbox<u8> = Mailbox::new();
        quiet.post(7);
        assert_eq!(quiet.drain(), vec![7]);
    }
}

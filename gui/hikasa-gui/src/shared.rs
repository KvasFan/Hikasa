//! State shared by the UI thread and the IPC handler threads: the list of open comparison views (`ListViews`)
//! and the [`ViewWaiter`]s of `gui open --wait` (contract §1.10, §4.4.3 rules 2 and 5).
//!
//! The IPC handler never blocks on the UI (§5.1): it registers a view here and posts the work to the UI
//! mailbox. Whoever closes a view takes its slot, computes the D-23 outcome with `ViewOutcome::close` and
//! notifies the waiter.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::ExitCode;
use hikasa_session::ipc::{ViewId, ViewInfo, ViewResult, ViewWaiter};
use parking_lot::Mutex;

/// The last component of a spec (`/w/src/a.c` → `a.c`, `/w/dir/` → `dir`; a root stays as it is).
pub fn spec_name(spec: &str) -> &str {
    let trimmed = spec.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(spec)
}

/// The one title of a comparison view: the tab caption and the `ListViews` title (§4.4, stable from the
/// moment the view is registered). Each side is its title (`--title1`/`--title2`) or the spec's last component.
pub fn view_title(titles: [Option<&str>; 2], specs: [&str; 2]) -> String {
    let side = |k: usize| titles[k].unwrap_or_else(|| spec_name(specs[k]));
    format!("{} ⇄ {}", side(0), side(1))
}

/// One registered view.
#[derive(Debug)]
struct Slot {
    info: ViewInfo,
    waiter: Option<ViewWaiter>,
}

/// The registry of comparison views.
#[derive(Debug)]
pub struct Shared {
    next: AtomicU64,
    views: Mutex<BTreeMap<ViewId, Slot>>,
}

impl Default for Shared {
    fn default() -> Shared {
        Shared::new()
    }
}

impl Shared {
    /// An empty registry; view ids start at 1.
    pub fn new() -> Shared {
        Shared {
            next: AtomicU64::new(1),
            views: Mutex::new(BTreeMap::new()),
        }
    }

    /// A fresh view id (unique per process).
    pub fn allocate(&self) -> ViewId {
        ViewId(self.next.fetch_add(1, Ordering::SeqCst))
    }

    /// Registers a view (with the waiter of `gui open --wait`, if any).
    pub fn register(&self, info: ViewInfo, waiter: Option<ViewWaiter>) {
        self.views.lock().insert(info.view, Slot { info, waiter });
    }

    /// True when `view` is registered.
    pub fn contains(&self, view: ViewId) -> bool {
        self.views.lock().contains_key(&view)
    }

    /// The registered views in id order (`ListViews`).
    pub fn list(&self) -> Vec<ViewInfo> {
        self.views.lock().values().map(|s| s.info.clone()).collect()
    }

    /// Replaces the title of a view (shown by `gui list`).
    pub fn set_title(&self, view: ViewId, title: String) {
        if let Some(slot) = self.views.lock().get_mut(&view) {
            slot.info.title = title;
        }
    }

    /// Removes a view and reports its result to its waiter (exactly once; never blocks, §1.8.7 item 13).
    /// Returns false when the view was not registered.
    pub fn close(&self, view: ViewId, result: ViewResult, code: ExitCode) -> bool {
        let slot = self.views.lock().remove(&view);
        match slot {
            Some(slot) => {
                if let Some(waiter) = slot.waiter {
                    waiter.view_closed(view, result, code);
                }
                true
            }
            None => false,
        }
    }

    /// True when a waiter waits for `view`.
    #[cfg(test)]
    pub fn is_waited(&self, view: ViewId) -> bool {
        self.views
            .lock()
            .get(&view)
            .is_some_and(|slot| slot.waiter.is_some())
    }

    /// Number of registered views.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.views.lock().len()
    }

    /// True when no view is registered.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_session::ipc::{CloseReason, ViewKind};

    fn info(view: ViewId) -> ViewInfo {
        ViewInfo {
            view,
            kind: ViewKind::TextCompare,
            title: "a | b".into(),
            specs: vec!["/a".into(), "/b".into()],
        }
    }

    #[test]
    fn registry_lists_renames_and_closes_once() {
        let shared = Shared::new();
        let a = shared.allocate();
        let b = shared.allocate();
        assert_ne!(a, b);
        shared.register(info(b), None);
        shared.register(info(a), None);
        assert_eq!(
            shared.list().iter().map(|v| v.view).collect::<Vec<_>>(),
            [a, b]
        );
        shared.set_title(a, "x".into());
        assert_eq!(shared.list()[0].title, "x");
        assert!(!shared.is_waited(a));
        let result = ViewResult::new(None, None, false, CloseReason::User);
        assert!(shared.close(a, result.clone(), ExitCode::Success));
        assert!(!shared.close(a, result, ExitCode::Success));
        assert!(shared.contains(b) && !shared.contains(a));
        assert_eq!(shared.len(), 1);
    }
}

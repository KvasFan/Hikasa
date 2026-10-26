//! Incremental scan events (ARCH-004, ARCH-082; INV-EVT-1..3): [`ScanEvent`], [`ScanObserver`],
//! [`NullObserver`], [`ChannelObserver`].

use crate::model::NodeId;

/// Incremental scan events (ARCH-004). They carry ids only; consumers read details from the model.
/// Ordering guarantees INV-EVT-1..3 (Appendix B): `Started` first and `Finished` last; `FolderListed{F}` precedes
/// every event naming a child of `F`; every event is sent after the mutation it reports is visible in the model.
///
/// A mutation also changes the counters (and possibly the state) of every ancestor of the nodes it names; those
/// ancestors are not listed. A consumer that mirrors the tree re-reads the named nodes, their ancestors, and the
/// children of a listed folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    /// The base folders were loaded (`TreeRead::bases` is set); the root exists.
    Started,
    /// A folder pair was listed and aligned; its children now exist as one contiguous block, with their quick
    /// test states.
    FolderListed {
        /// The listed folder.
        folder: NodeId,
    },
    /// Nodes whose state, content result or flags changed since they were last announced (deduplicated batch
    /// of at most 256 ids; the counters of their ancestors changed too).
    NodesUpdated {
        /// The changed nodes.
        nodes: Vec<NodeId>,
    },
    /// The scan ended (`complete == false` after cancellation). Always the last event.
    Finished {
        /// False after cancellation.
        complete: bool,
    },
}

/// Receives events from worker threads, after the mutation they report is visible in the model. Must not block
/// for long (use [`ChannelObserver`]).
pub trait ScanObserver: Send + Sync {
    /// One event.
    fn on_event(&self, event: ScanEvent);
    /// False for observers that ignore events; the scanner then builds no event at all (no allocation on the
    /// CLI path). Default true.
    fn wants_events(&self) -> bool {
        true
    }
}

/// Discards events (CLI); `wants_events()` is false.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullObserver;

impl ScanObserver for NullObserver {
    fn on_event(&self, _event: ScanEvent) {}

    fn wants_events(&self) -> bool {
        false
    }
}

/// Forwards events to a bounded crossbeam channel. The scanner then applies backpressure, so a slow GUI slows
/// the scan instead of growing memory (ARCH-082).
#[derive(Debug, Clone)]
pub struct ChannelObserver {
    tx: crossbeam_channel::Sender<ScanEvent>,
}

impl ChannelObserver {
    /// Creates the observer and its receiver (`capacity` ≥ 1; 0 is taken as 1).
    pub fn bounded(capacity: usize) -> (ChannelObserver, crossbeam_channel::Receiver<ScanEvent>) {
        let (tx, rx) = crossbeam_channel::bounded(capacity.max(1));
        (ChannelObserver { tx }, rx)
    }
}

impl ScanObserver for ChannelObserver {
    /// Sends the event, blocking while the channel is full; a disconnected receiver drops events silently.
    fn on_event(&self, event: ScanEvent) {
        let _ = self.tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observers() {
        assert!(!NullObserver.wants_events());
        NullObserver.on_event(ScanEvent::Started);
        let (obs, rx) = ChannelObserver::bounded(0);
        assert!(obs.wants_events());
        obs.on_event(ScanEvent::Started);
        assert_eq!(rx.recv().unwrap(), ScanEvent::Started);
        drop(rx);
        // Disconnected: dropped silently, no panic, no block.
        obs.on_event(ScanEvent::Finished { complete: true });
    }
}

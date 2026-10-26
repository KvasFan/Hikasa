//! Folder Compare engine of Hikasa: rules (FC-020…FC-027) with the script `criteria` vocabulary (D-57), filters
//! (FC-050…FC-054), name alignment (FC-010), symlink handling and flatten (FC-013, FC-014), and a parallel,
//! cancellable scanner (FC-016) that fills a **live [`FolderModel`]**.
//!
//! The model is readable during the scan (ARCH-004, ARCH-082): per-folder counters make state, colour,
//! visibility, statistics and verdict O(1) per folder on partial trees too (FC-030, FC-055…FC-057, FC-064,
//! D-55). Id-only [`ScanEvent`]s tell observers what changed. When the scan ends, the model is frozen into a
//! [`FolderComparison`]. Content tests go through the injected [`hikasa_core::ContentComparator`] objects
//! ([`ContentEngines`]) under a shared [`ByteBudget`] (D-25); the crate does not depend on hikasa-text.
//!
//! ```
//! use std::path::PathBuf;
//! use hikasa_core::{CaseSensitivity, CompareVerdict, Ctl, LocalPath, Spec};
//! use hikasa_folder::{
//!     compare_folders, ContentEngines, FolderEnv, FolderRules, NullObserver, CompareState,
//! };
//! use hikasa_vfs::{MemMeta, MemoryVfs, StreamOptions};
//!
//! # fn path(p: &str) -> String { if cfg!(windows) { format!("C:{}", p.replace('/', "\\")) } else { p.to_owned() } }
//! let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
//! vfs.insert_file(&path("/l/a.txt"), b"one".to_vec(), MemMeta::default())?;
//! vfs.insert_file(&path("/r/a.txt"), b"two!".to_vec(), MemMeta::default())?;
//! vfs.insert_file(&path("/l/only.txt"), b"x".to_vec(), MemMeta::default())?;
//! let spec = |p: &str| Spec::Local(LocalPath::new(PathBuf::from(path(p))).unwrap());
//!
//! let engines = ContentEngines::with_stream_options(StreamOptions::default(), None);
//! let now = jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC);
//! let env = FolderEnv {
//!     vfs: &vfs,
//!     content: &engines,
//!     pool: None,
//!     ctl: Ctl::detached(),
//!     observer: &NullObserver,
//!     now: &now,
//!     content_budget: 1 << 30,
//! };
//! let cmp = compare_folders(&spec("/l"), &spec("/r"), &FolderRules::default(), &env)?;
//! let read = cmp.read();
//! let states: Vec<CompareState> = read.children(read.root()).map(|id| read.node(id).unwrap().state).collect();
//! assert_eq!(states, [CompareState::Different, CompareState::LeftOrphan]);
//! assert_eq!(cmp.verdict(false), CompareVerdict::Differences);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Additions to the M1 contract
//!
//! Additive public API beyond §1.6 (no signature of §1.6 changes; recorded in contract §1.6.8):
//! [`FolderError::Schema`] (a rules file from a newer version, classified `unsupported`);
//! [`FolderRules::validate`]; [`Cmp::holds`]; [`Change::into_option`]; the signatures of [`align_names`] /
//! [`align_flat`] with [`AlignInput`] and [`AlignOptions`]; [`DiffFlags::names`], [`NodeFlags::names`] and the
//! side helpers [`NodeFlags::filtered`] / [`NodeFlags::stat_skipped`] / [`NodeFlags::error`];
//! `as_str`/`ALL`/`mirrored`/`category`/`index`/`state` helpers on [`CompareState`], [`Category`],
//! [`NodeKind`], [`ScanStatus`], [`TimeRelation`]; [`NodeKind::of`] and [`NodeKind::entry_kind`];
//! [`Category::is_difference`]; [`state::base_state`], [`state::pair_state`], [`state::state_if_rules_same`],
//! [`state::file_category`] and [`state::OVERRIDABLE`] (the pure §2.6.2/§2.6.3 functions);
//! [`CategoryCounts::plus`], [`Aggregate::both`]; [`ContentCounts::get`]; [`ByteBudget::capacity`],
//! [`ByteBudget::available`], [`BudgetGuard::bytes`]; [`ContentEngines::with_stream_options`] (the caller
//! chooses `StreamOptions::threads`) and [`ContentEngines::get`]; [`FolderToggles::atom`],
//! [`FolderToggles::accepts`], [`FolderDisplayFilter::PRESETS`], [`FolderMode::ALL`], [`VisibleTree::is_empty`];
//! [`NodeView::side`], [`NodeView::representative`], [`NodeView::is_folder`]; [`TreeRead::is_empty`],
//! [`TreeRead::rules`], [`TreeRead::ancestors`]; [`check_tree_invariants`] (INV-FOLDER-1..10 on a live model,
//! used by P-FOLDER-6); the layout of [`CanonicalTree`] ([`CanonicalNode`], [`CanonicalEntry`],
//! [`CanonicalError`]); [`MAX_FOLLOW_DEPTH`]; `Debug` for [`FolderModel`], [`TreeRead`], [`FolderEnv`],
//! [`ContentEngines`], [`ChannelObserver`], and `Clone` for [`ChannelObserver`]; the bench-only
//! `FolderModel::reserved_bytes` (`#[doc(hidden)]`).
//!
//! Specification: `docs/design/m1-contracts.md` §1.6 and the algorithms of §2.3–§2.7; user documentation:
//! `docs/04-folder-views.md`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod align;
mod budget;
mod counts;
mod display;
mod error;
mod events;
mod model;
mod quick;
mod resolve;
mod rules;
mod scan;
pub mod state;
mod stats;
mod time_cmp;

pub use align::{AlignInput, AlignOptions, align_flat, align_names};
pub use budget::{BudgetGuard, ByteBudget};
pub use counts::{Aggregate, Aggregates, CategoryCounts, SideTotals};
pub use display::{
    ColourClass, DisplayOptions, FolderDisplayFilter, FolderMode, FolderToggles, VisibleRow,
    VisibleTree, visible_tree,
};
pub use error::FolderError;
pub use events::{ChannelObserver, NullObserver, ScanEvent, ScanObserver};
pub use model::{
    BaseInfo, CanonicalEntry, CanonicalError, CanonicalNode, CanonicalTree, ContentSummary,
    EntryView, FolderComparison, FolderModel, NodeError, NodeFlags, NodeId, NodeKind, NodeView,
    ScanStatus, TreeRead, check_tree_invariants,
};
pub use quick::{DiffFlags, quick_tests};
pub use rules::{
    CaseMode, Change, Cmp, ContentRule, Criteria, CriteriaSpec, DateCutoff, DateFilter,
    FOLDER_RULES_SCHEMA_VERSION, FilterRules, FolderRules, FolderRulesPatch, Handling,
    NameAlignment, OtherFilters, SizeFilter, TimestampRule, TzOffset, UnixCriteria,
};
pub use scan::{ContentEngines, FolderEnv, MAX_FOLLOW_DEPTH, compare_folders, compare_into};
pub use state::{Category, CompareState};
pub use stats::{ContentCounts, FolderStats};
pub use time_cmp::{TimeRelation, compare_times};

/// Re-exports of the `hikasa_vfs` types used in this crate's API (§0.4 rule 8: no extra manifest edges for
/// consumers such as hikasa-report).
pub use hikasa_vfs::{
    Attrs, Capabilities, DosAttrs, EntryKind, FollowLinks, UnixMeta, Vfs, VfsError,
};

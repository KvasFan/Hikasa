//! Sessions (M2 W2; contract §1.2.1–§1.2.4, D-80…D-84): names and references, the session file, the store
//! (named sessions in folders, defaults per type, auto-saved sessions), scope resolution and close prompts.
//!
//! | Module | Content |
//! |---|---|
//! | [`name`] | [`SessionName`], [`SessionRef`], [`AutoId`] (D-80) |
//! | [`file`](mod@file) | [`SessionType`], [`Session`], [`SessionRules`], [`SessionDoc`] (D-81) |
//! | [`store`] | [`SessionStore`], [`SessionTree`] |
//! | [`autosave`] | [`AutoGroup`], [`auto_group`] (D-83) |
//! | [`scope`] | [`Scope`], [`effective_rules`], [`apply_scope`] (D-82) |
//! | [`close`] | [`close_action`], [`CloseAction`], [`Origin`] (D-84) |
//! | [`view_state`] | the `view` and `display` tables of a session as the GUI fills them (D-108, SES-010) |

pub mod autosave;
pub mod close;
pub mod file;
pub mod name;
pub mod scope;
pub mod store;
pub mod view_state;

pub use autosave::{AutoGroup, auto_group};
pub use close::{CloseAction, Origin, close_action, is_dirty};
pub use file::{
    Override, OverrideScope, ParentLink, SESSION_SCHEMA_VERSION, Session, SessionDoc, SessionRules,
    SessionType, Specs,
};
pub use name::{AutoId, MAX_NAME_CHARS, SessionName, SessionRef, parse_folder};
pub use scope::{Scope, ScopeWrites, apply_scope, effective_rules};
pub use store::{AutoEntry, KeyEdits, NamedEntry, SessionStore, SessionTree, label};
pub use view_state::{
    OpaqueTable, apply_folder_display, apply_text_display, folder_display_table, layout_table,
    text_display_table, view_bool,
};

//! The session layer of Hikasa: the service API used identically by the CLI and the GUI (ARCH-013), the
//! configuration and cache folders (SES-050), program options (docs/15), format-registry loading (SES-051,
//! FF-004), atomic configuration writes (SES-053), job control and job spawning, the shared legacy `gui open`
//! argument parser (D-51), and the IPC protocol with its client and server (ARCH-090…092).
//!
//! It is the only core crate that may log (`tracing`, T8), and its [`host`] module is the only place in the core
//! crates that reads the process environment and the system clock (T1). Only adapters (`cli/`, `gui/`) call
//! the host capture functions; everything else receives [`EnvFacts`] and a [`Clock`] as values.
//!
//! | Module | Content |
//! |---|---|
//! | [`host`] | host facts and clocks (adapter-only capture) |
//! | [`paths`] | the configuration and cache folders (§2.22) |
//! | [`options`] | `options.toml` (D-25, D-31) |
//! | [`formats`] | [`FormatStore`]: built-ins + user formats + `order.toml` (§2.12.4, D-21) |
//! | [`jobs`] | [`JobControl`], [`JobHandle`], [`run_guarded`] (§5.6) |
//! | [`api`] | [`api::Service`] and its requests, responses, DTOs (§4) and the `gui open` parser |
//! | [`ipc`] | protocol v1, framing, endpoint, server and client (§4.4, §2.23) |
//! | [`commands`] | the command catalogue of every view type (ARCH-070; M2 W0) |
//! | [`help`] | the help topics of `help/` and their Markdown reader (M2 W0) |
//! | [`settings`] | the settings store: every program-owned settings file, keys, change detection (M2 W1) |
//! | [`sessions`] | session files, the session store, auto-save, scopes, close prompts (M2 W2) |
//! | [`package`] | settings packages: export, import plans, restore factory defaults (M2 W2) |
//! | [`workspaces`] | workspace files and their restore (M2 W2) |
//!
//! Specification: `docs/design/m1-contracts.md` §1.8, §2.21–§2.23, §3, §4, §5; `docs/design/m2-contracts.md`
//! (W0: catalogue and help; W1: settings store).
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod api;
mod atomic;
pub mod commands;
mod error;
pub mod formats;
pub mod help;
pub mod host;
pub mod ipc;
pub mod jobs;
pub mod options;
pub mod package;
pub mod paths;
pub mod sessions;
pub mod settings;
pub mod workspaces;

pub use api::{ApiError, ApiOp, HIKASA_VERSION, Service, ServiceInit, StdinSource};
pub use atomic::{TEMP_NAME_PREFIX_MAX, write_atomic};
pub use error::SessionError;
pub use formats::FormatStore;
pub use host::{Clock, EnvFacts, FixedClock, Platform};
pub use host::{SystemClock, capture_env};
pub use jobs::{JobControl, JobHandle, run_guarded};
pub use options::{OPTION_SECTIONS, OptionsLoad, ProgramOptions, load_options};
pub use paths::{
    ConfigLocation, ConfigMode, PORTABLE_MARKER, default_config_dir, resolve_config_dir,
};

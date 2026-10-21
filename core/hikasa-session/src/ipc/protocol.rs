//! The IPC protocol v1 (§4.4, ARCH-091, D-46): [`Envelope`], [`Request`], [`Response`], [`ViewKind`],
//! [`ViewId`], [`OpenOptions`], [`ReadOnly`], [`ViewResult`], [`CloseReason`], [`ServerInfo`], [`ViewInfo`],
//! [`IpcErrorCode`].
//!
//! Forward compatibility (D-46): `Request`, `Response` and `ViewKind` decode unknown `type`/kind values to their
//! `Unknown` variant; frames are decoded in two phases (`serde_json::Value`, then `id`, then the body, see
//! [`super::framing`]); error codes and view results travel as plain strings; unknown fields are ignored.

use std::collections::BTreeMap;

use hikasa_core::{CompareVerdict, ContentKind};
use hikasa_folder::FolderRulesPatch;
use hikasa_text::TextRulesPatch;
use serde::{Deserialize, Serialize};

/// A framed message: request id + flattened body. The id is chosen by the client and strictly increases per
/// connection; responses (and ViewClosed) echo it. Serialised in declaration order: `id`, then `type`, then the
/// body's fields (the `#[serde(flatten)]` struct form). Decoded in two phases (§4.4.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Request id (0 only in `Error{protocol}` replies to frames without a usable id).
    pub id: u64,
    /// The message.
    #[serde(flatten)]
    pub body: T,
}

/// Client → server. Unknown `type` values decode to `Unknown` (D-46).
// The variant sizes follow the wire shape of §4.4.2 (`OpenView` carries its options inline); requests are few and
// short-lived, so the size difference does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Handshake (always the first message).
    Hello {
        /// Client identity (`"hikasa 0.1.0"`).
        client: String,
        /// Newest protocol version the client speaks.
        version: u32,
        /// Oldest protocol version the client accepts.
        min_version: u32,
    },
    /// Opens a view.
    OpenView {
        /// View type.
        kind: ViewKind,
        /// Canonical, absolute spec strings (`-` and `clipboard://` never appear, D-49).
        specs: Vec<String>,
        /// View options.
        #[serde(default)]
        options: OpenOptions,
        /// Send a `ViewClosed` when the view closes (CLI-G01).
        #[serde(default)]
        wait: bool,
    },
    /// Loads a named session (M1: `Error{unsupported_request}`).
    LoadSession {
        /// Session name.
        name: String,
        /// Wait for the session's view to close.
        #[serde(default)]
        wait: bool,
    },
    /// Loads a named workspace (M1: `Error{unsupported_request}`).
    LoadWorkspace {
        /// Workspace name.
        name: String,
    },
    /// Runs a script (M1: `Error{unsupported_request}`).
    RunScript {
        /// Script path.
        path: String,
        /// Script arguments.
        #[serde(default)]
        args: Vec<String>,
    },
    /// Lists the open views.
    ListViews,
    /// Closes a view.
    CloseView {
        /// The view.
        view: ViewId,
    },
    /// Liveness check.
    Ping,
    /// Asks the GUI to quit.
    Shutdown,
    /// Any `type` this build does not know. Never sent.
    #[serde(other)]
    Unknown,
}

impl Request {
    /// The wire name of the request's `type` (`"open_view"`, …; `"unknown"` for [`Request::Unknown`]).
    pub const fn type_name(&self) -> &'static str {
        match self {
            Request::Hello { .. } => "hello",
            Request::OpenView { .. } => "open_view",
            Request::LoadSession { .. } => "load_session",
            Request::LoadWorkspace { .. } => "load_workspace",
            Request::RunScript { .. } => "run_script",
            Request::ListViews => "list_views",
            Request::CloseView { .. } => "close_view",
            Request::Ping => "ping",
            Request::Shutdown => "shutdown",
            Request::Unknown => "unknown",
        }
    }
}

/// Server → client. Unknown `type` values decode to `Unknown`, which clients treat as a protocol error for the
/// pending request (D-46).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Handshake reply with the negotiated version.
    Hello {
        /// Server identity (`"hikasa-gui 0.1.0"`).
        server: String,
        /// Negotiated protocol version.
        version: u32,
    },
    /// A view was opened.
    ViewOpened {
        /// The view.
        view: ViewId,
    },
    /// A waited-for view closed (sent once, after the immediate response, with the `OpenView` request's id).
    ViewClosed {
        /// The view.
        view: ViewId,
        /// Its result.
        result: ViewResult,
        /// Exit code for the CLI (authoritative, D-23).
        exit_code: u8,
    },
    /// The open views.
    Views {
        /// One entry per view.
        views: Vec<ViewInfo>,
    },
    /// Success without data.
    Ok,
    /// Reply to `Ping`.
    Pong,
    /// `code` is an `IpcErrorCode` name; clients accept codes they do not know.
    Error {
        /// Error code (wire string).
        code: String,
        /// Human-readable message.
        message: String,
    },
    /// Any `type` this build does not know. Never sent.
    #[serde(other)]
    Unknown,
}

impl Response {
    /// An `Error` response with a code of this build.
    pub fn error(code: IpcErrorCode, message: impl Into<String>) -> Response {
        Response::Error {
            code: code.as_str().to_owned(),
            message: message.into(),
        }
    }
}

/// View/session types (00-overview §2). M1 GUI serves `folder_compare` and `text_compare`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    /// Folder Compare.
    FolderCompare,
    /// Folder Merge.
    FolderMerge,
    /// Folder Sync.
    FolderSync,
    /// Text Compare.
    TextCompare,
    /// Text Merge.
    TextMerge,
    /// Text Edit.
    TextEdit,
    /// Text Patch.
    TextPatch,
    /// Table Compare.
    TableCompare,
    /// Hex Compare.
    HexCompare,
    /// Picture Compare.
    PictureCompare,
    /// Media Compare.
    MediaCompare,
    /// Version Compare.
    VersionCompare,
    /// Registry Compare.
    RegistryCompare,
    /// A kind added by a newer peer; the server answers `Error{unsupported_view}`.
    #[serde(other)]
    Unknown,
}

impl ViewKind {
    /// Every known kind, in declaration order (without `Unknown`).
    pub const ALL: [ViewKind; 13] = [
        ViewKind::FolderCompare,
        ViewKind::FolderMerge,
        ViewKind::FolderSync,
        ViewKind::TextCompare,
        ViewKind::TextMerge,
        ViewKind::TextEdit,
        ViewKind::TextPatch,
        ViewKind::TableCompare,
        ViewKind::HexCompare,
        ViewKind::PictureCompare,
        ViewKind::MediaCompare,
        ViewKind::VersionCompare,
        ViewKind::RegistryCompare,
    ];

    /// Stable snake_case name, identical to the serde form (`"text_compare"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            ViewKind::FolderCompare => "folder_compare",
            ViewKind::FolderMerge => "folder_merge",
            ViewKind::FolderSync => "folder_sync",
            ViewKind::TextCompare => "text_compare",
            ViewKind::TextMerge => "text_merge",
            ViewKind::TextEdit => "text_edit",
            ViewKind::TextPatch => "text_patch",
            ViewKind::TableCompare => "table_compare",
            ViewKind::HexCompare => "hex_compare",
            ViewKind::PictureCompare => "picture_compare",
            ViewKind::MediaCompare => "media_compare",
            ViewKind::VersionCompare => "version_compare",
            ViewKind::RegistryCompare => "registry_compare",
            ViewKind::Unknown => "unknown",
        }
    }

    /// Parses `--fv` values: "Text Compare", "text-compare", "text_compare", … (ASCII-case-insensitive).
    pub fn parse(s: &str) -> Option<ViewKind> {
        let normalized: String = s
            .trim()
            .chars()
            .map(|c| match c {
                ' ' | '-' => '_',
                c => c.to_ascii_lowercase(),
            })
            .collect();
        ViewKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == normalized)
    }

    /// True for the kinds the M1 GUI serves (`folder_compare`, `text_compare`; §4.4.3 rule 5).
    pub const fn is_served_in_m1(self) -> bool {
        matches!(self, ViewKind::FolderCompare | ViewKind::TextCompare)
    }
}

/// View id (unique per GUI process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ViewId(pub u64);

/// Options of `OpenView` (CLI-003 subset for M1, plus the `--gui` rules of D-50). Every field defaults when
/// absent; unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenOptions {
    /// Pane titles: left, right, center, output (`--title1..4`).
    pub titles: [Option<String>; 4],
    /// Read-only switches (`--ro`, `--ro1`, `--ro2`).
    pub read_only: ReadOnly,
    /// Name filter for the initial folder comparison (`--filters`).
    pub filters: Option<String>,
    /// `--expandall`.
    pub expand_all: bool,
    /// Forced format id or name.
    pub format: Option<String>,
    /// `--ignore-unimportant`.
    pub ignore_unimportant: bool,
    /// Absolute path of a text rules file (`--rules-file` with `--gui`).
    pub text_rules_file: Option<String>,
    /// Text rule overrides (`--gui`).
    pub text_rules: Option<TextRulesPatch>,
    /// Absolute path of a folder rules file.
    pub folder_rules_file: Option<String>,
    /// `--criteria` text.
    pub criteria: Option<String>,
    /// Folder rule overrides (`--gui`).
    pub folder_rules: Option<FolderRulesPatch>,
    /// Recognised CLI-003 switches that M1 does not implement, forwarded verbatim (name → value, "" for flags).
    pub extra: BTreeMap<String, String>,
}

/// Read-only switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadOnly {
    /// `--ro`: every pane.
    pub all: bool,
    /// `--ro1`: the left pane.
    pub left: bool,
    /// `--ro2`: the right pane.
    pub right: bool,
}

/// Why a view closed (sent as its snake_case name inside `ViewResult::reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// The user closed it.
    User,
    /// The GUI shut down.
    Shutdown,
    /// The view could not load its inputs.
    LoadFailed,
}

impl CloseReason {
    /// Every reason, in declaration order.
    pub const ALL: [CloseReason; 3] = [
        CloseReason::User,
        CloseReason::Shutdown,
        CloseReason::LoadFailed,
    ];

    /// Stable snake_case name (`"user"`, `"shutdown"`, `"load_failed"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            CloseReason::User => "user",
            CloseReason::Shutdown => "shutdown",
            CloseReason::LoadFailed => "load_failed",
        }
    }

    /// Inverse of `as_str` (`None` for a reason added by a newer peer).
    pub fn parse(s: &str) -> Option<CloseReason> {
        CloseReason::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

/// Result reported with `ViewClosed`. Plain strings, so that values added later never break an older peer
/// (D-46); `ViewClosed::exit_code` is authoritative for the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewResult {
    /// File views: `ContentKind` name at close time ("unimportant_only", …).
    pub verdict: Option<String>,
    /// Folder views: `CompareVerdict` name at close time.
    pub folder_verdict: Option<String>,
    /// Whether edits were saved.
    pub saved: bool,
    /// `CloseReason` name ("user", "shutdown", "load_failed"; unknown values allowed).
    pub reason: String,
}

impl ViewResult {
    /// Builds the strings from typed values.
    pub fn new(
        verdict: Option<ContentKind>,
        folder_verdict: Option<CompareVerdict>,
        saved: bool,
        reason: CloseReason,
    ) -> ViewResult {
        ViewResult {
            verdict: verdict.map(|k| k.as_str().to_owned()),
            folder_verdict: folder_verdict.map(|v| v.as_str().to_owned()),
            saved,
            reason: reason.as_str().to_owned(),
        }
    }

    /// `ContentKind::parse(verdict)`.
    pub fn verdict_kind(&self) -> Option<ContentKind> {
        self.verdict.as_deref().and_then(ContentKind::parse)
    }

    /// `CompareVerdict::parse(folder_verdict)` (additive).
    pub fn folder_verdict_kind(&self) -> Option<CompareVerdict> {
        self.folder_verdict
            .as_deref()
            .and_then(CompareVerdict::parse)
    }

    /// `CloseReason::parse(reason)` (additive).
    pub fn close_reason(&self) -> Option<CloseReason> {
        CloseReason::parse(&self.reason)
    }
}

/// `ListViews` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewInfo {
    /// The view.
    pub view: ViewId,
    /// Its kind.
    pub kind: ViewKind,
    /// Its title.
    pub title: String,
    /// Its specs.
    pub specs: Vec<String>,
}

/// Server identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server identity (`"hikasa-gui 0.1.0"`).
    pub server: String,
    /// Protocol version (the negotiated one, on the client side).
    pub version: u32,
}

/// Error codes produced by this build (sent as strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IpcErrorCode {
    /// Malformed frame or message sequence.
    Protocol,
    /// No common protocol version.
    IncompatibleVersion,
    /// A request type this build does not serve.
    UnsupportedRequest,
    /// A view kind this build does not serve.
    UnsupportedView,
    /// A known request whose fields do not decode or make no sense.
    InvalidRequest,
    /// An unknown view id.
    NotFound,
    /// A bug in the server.
    Internal,
}

impl IpcErrorCode {
    /// Every code, in declaration order.
    pub const ALL: [IpcErrorCode; 7] = [
        IpcErrorCode::Protocol,
        IpcErrorCode::IncompatibleVersion,
        IpcErrorCode::UnsupportedRequest,
        IpcErrorCode::UnsupportedView,
        IpcErrorCode::InvalidRequest,
        IpcErrorCode::NotFound,
        IpcErrorCode::Internal,
    ];

    /// snake_case wire name ("unsupported_request", …).
    pub const fn as_str(self) -> &'static str {
        match self {
            IpcErrorCode::Protocol => "protocol",
            IpcErrorCode::IncompatibleVersion => "incompatible_version",
            IpcErrorCode::UnsupportedRequest => "unsupported_request",
            IpcErrorCode::UnsupportedView => "unsupported_view",
            IpcErrorCode::InvalidRequest => "invalid_request",
            IpcErrorCode::NotFound => "not_found",
            IpcErrorCode::Internal => "internal",
        }
    }

    /// Inverse of `as_str` (`None` for a code of a newer peer).
    pub fn parse(s: &str) -> Option<IpcErrorCode> {
        IpcErrorCode::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

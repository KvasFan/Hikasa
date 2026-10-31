//! The JSON envelope and the DTOs of the session-owned result shapes (§4.1, §4.2, §4.3.1, §4.3.4).
//!
//! Serialisation rules (§4.1): field order is declaration order; enums are snake_case strings; maps are
//! `BTreeMap`; absent optional values are `null` with the key always present; arrays are never `null`;
//! integers only; CRC32 as 8 lowercase hex digits; timestamps RFC 3339 UTC; specs canonical. The text and folder
//! comparison shapes (§4.3.2, §4.3.3) are owned by hikasa-report (`TextCompareJson`, `FolderCompareJson`).
//! No DTO here contains a `HashMap` (architecture check 6).

use std::collections::BTreeMap;
use std::io::Write;

use hikasa_core::{ContentResult, ErrorReport, ExitCode, Warning};
use hikasa_formats::{
    Conversion, ElementInfo, EncodingSetting, FileFormat, FilenameEncoding, GrammarItem, ItemKind,
    LineState, TableSettings, TextSettings,
};
use hikasa_report::json::{DetectReasonJson, FormatJson};
use hikasa_vfs::{Attrs, Capabilities, Entry};
use serde::Serialize;

use super::responses::QuickResponse;
use super::{ApiError, ApiOp, HIKASA_VERSION};
use crate::ipc::{PROTOCOL_VERSION, ServerInfo, ViewInfo, ViewResult};

/// `hikasa quick --json` (§4.3.1).
pub const QUICK_SCHEMA: &str = "hikasa.quick.v1";
/// `hikasa text compare --json` (§4.3.2).
pub const TEXT_COMPARE_SCHEMA: &str = "hikasa.text_compare.v1";
/// `hikasa folder compare --json` (§4.3.3).
pub const FOLDER_COMPARE_SCHEMA: &str = "hikasa.folder_compare.v1";
/// `hikasa format list --json`.
pub const FORMAT_LIST_SCHEMA: &str = "hikasa.format_list.v1";
/// `hikasa format show --json`.
pub const FORMAT_SHOW_SCHEMA: &str = "hikasa.format_show.v1";
/// `hikasa format detect --json`.
pub const FORMAT_DETECT_SCHEMA: &str = "hikasa.format_detect.v1";
/// `hikasa format tokens --json`.
pub const FORMAT_TOKENS_SCHEMA: &str = "hikasa.format_tokens.v1";
/// `hikasa vfs ls --json`.
pub const VFS_LIST_SCHEMA: &str = "hikasa.vfs_list.v1";
/// `hikasa vfs stat --json`.
pub const VFS_STAT_SCHEMA: &str = "hikasa.vfs_stat.v1";
/// `hikasa vfs cat --json` (D-32).
pub const VFS_CAT_SCHEMA: &str = "hikasa.vfs_cat.v1";
/// `hikasa gui open --json` (and `--gui --json`).
pub const GUI_OPEN_SCHEMA: &str = "hikasa.gui_open.v1";
/// `hikasa gui ping --json`.
pub const GUI_PING_SCHEMA: &str = "hikasa.gui_ping.v1";
/// `hikasa gui list --json`.
pub const GUI_LIST_SCHEMA: &str = "hikasa.gui_list.v1";
/// `hikasa gui close --json`.
pub const GUI_CLOSE_SCHEMA: &str = "hikasa.gui_close.v1";
/// `hikasa gui quit --json`.
pub const GUI_QUIT_SCHEMA: &str = "hikasa.gui_quit.v1";
/// `hikasa version --json`.
pub const VERSION_SCHEMA: &str = "hikasa.version.v1";
/// `hikasa completions <shell> --json` (`{ "shell", "script" }`). Added in the wave-4 review.
pub const COMPLETIONS_SCHEMA: &str = "hikasa.completions.v1";
/// A command line that does not parse, with `--json` anywhere in it (`result` is `null`, `error.kind` is
/// `invalid_input`, exit 100, D-35). Added in the wave-4 review.
pub const USAGE_ERROR_SCHEMA: &str = "hikasa.usage_error.v1";
/// `hikasa help [topic] --json` and `hikasa help --list --json` (M2 W0).
pub const HELP_SCHEMA: &str = "hikasa.help.v1";
/// `hikasa settings get --json` (M2 W1).
pub const SETTINGS_GET_SCHEMA: &str = "hikasa.settings_get.v1";
/// `hikasa settings set|unset --json` (M2 W1).
pub const SETTINGS_SET_SCHEMA: &str = "hikasa.settings_set.v1";
/// `hikasa settings list --json` (M2 W1).
pub const SETTINGS_LIST_SCHEMA: &str = "hikasa.settings_list.v1";
/// `hikasa settings restore-defaults --json` (M2 W1, D-77).
pub const SETTINGS_RESTORE_SCHEMA: &str = "hikasa.settings_restore.v1";
/// `hikasa format new|edit|delete|import|enable|disable|move --json` (M2 W1, D-75).
pub const FORMAT_WRITE_SCHEMA: &str = "hikasa.format_write.v1";
/// `hikasa format export --json` (M2 W1).
pub const FORMAT_EXPORT_SCHEMA: &str = "hikasa.format_export.v1";
/// `hikasa format restore --json` (M2 W1).
pub const FORMAT_RESTORE_SCHEMA: &str = "hikasa.format_restore.v1";

/// `hikasa session list` (M2 W2).
pub const SESSION_LIST_SCHEMA: &str = "hikasa.session_list.v1";
/// `hikasa session show` (M2 W2).
pub const SESSION_SHOW_SCHEMA: &str = "hikasa.session_show.v1";
/// The `hikasa session` writes (M2 W2).
pub const SESSION_WRITE_SCHEMA: &str = "hikasa.session_write.v1";
/// `hikasa workspace list` (M2 W2).
pub const WORKSPACE_LIST_SCHEMA: &str = "hikasa.workspace_list.v1";
/// `hikasa workspace show` (M2 W2).
pub const WORKSPACE_SHOW_SCHEMA: &str = "hikasa.workspace_show.v1";
/// The `hikasa workspace` writes (M2 W2).
pub const WORKSPACE_WRITE_SCHEMA: &str = "hikasa.workspace_write.v1";
/// `hikasa settings export` (M2 W2).
pub const SETTINGS_EXPORT_SCHEMA: &str = "hikasa.settings_export.v1";
/// `hikasa settings import` (M2 W2).
pub const SETTINGS_IMPORT_SCHEMA: &str = "hikasa.settings_import.v1";

/// `hikasa vfs cp|mv|rm|mkdir|touch` (M2 W3, D-102).
pub const VFS_OP_SCHEMA: &str = "hikasa.vfs_op.v1";

/// Every schema id of this build, in a fixed order (`hikasa.version.v1`'s `schemas`; new schemas are appended).
pub const SCHEMAS: [&str; 35] = [
    QUICK_SCHEMA,
    TEXT_COMPARE_SCHEMA,
    FOLDER_COMPARE_SCHEMA,
    FORMAT_LIST_SCHEMA,
    FORMAT_SHOW_SCHEMA,
    FORMAT_DETECT_SCHEMA,
    FORMAT_TOKENS_SCHEMA,
    VFS_LIST_SCHEMA,
    VFS_STAT_SCHEMA,
    VFS_CAT_SCHEMA,
    GUI_OPEN_SCHEMA,
    GUI_PING_SCHEMA,
    GUI_LIST_SCHEMA,
    GUI_CLOSE_SCHEMA,
    GUI_QUIT_SCHEMA,
    VERSION_SCHEMA,
    COMPLETIONS_SCHEMA,
    USAGE_ERROR_SCHEMA,
    HELP_SCHEMA,
    SETTINGS_GET_SCHEMA,
    SETTINGS_SET_SCHEMA,
    SETTINGS_LIST_SCHEMA,
    SETTINGS_RESTORE_SCHEMA,
    FORMAT_WRITE_SCHEMA,
    FORMAT_EXPORT_SCHEMA,
    FORMAT_RESTORE_SCHEMA,
    SESSION_LIST_SCHEMA,
    SESSION_SHOW_SCHEMA,
    SESSION_WRITE_SCHEMA,
    WORKSPACE_LIST_SCHEMA,
    WORKSPACE_SHOW_SCHEMA,
    WORKSPACE_WRITE_SCHEMA,
    SETTINGS_EXPORT_SCHEMA,
    SETTINGS_IMPORT_SCHEMA,
    VFS_OP_SCHEMA,
];

/// Every `--json` document (owned by hikasa_session::api::dto).
#[derive(Debug, Serialize)]
pub struct JsonEnvelope<T: Serialize> {
    /// `hikasa.<command>.v<major>`.
    pub schema: &'static str,
    /// Version of the producing build.
    pub hikasa_version: &'static str,
    /// False when the command failed (`result` is then `null` and `error` is set).
    pub ok: bool,
    /// The process exit code.
    pub exit_code: u8,
    /// The result (`null` on error).
    pub result: Option<T>,
    /// Warnings.
    pub warnings: Vec<Warning>,
    /// The error (`null` on success).
    pub error: Option<ErrorReport>,
}

impl<T: Serialize> JsonEnvelope<T> {
    /// A successful document.
    pub fn success(
        schema: &'static str,
        exit_code: ExitCode,
        result: T,
        warnings: Vec<Warning>,
    ) -> JsonEnvelope<T> {
        JsonEnvelope {
            schema,
            hikasa_version: HIKASA_VERSION,
            ok: true,
            exit_code: exit_code.code(),
            result: Some(result),
            warnings,
            error: None,
        }
    }

    /// A successful document whose exit code is a raw byte (`gui open --wait`: a newer GUI may send a code this
    /// build does not know, D-23). Additive (wave-4 review).
    pub fn success_raw(
        schema: &'static str,
        exit_code: u8,
        result: T,
        warnings: Vec<Warning>,
    ) -> JsonEnvelope<T> {
        JsonEnvelope {
            schema,
            hikasa_version: HIKASA_VERSION,
            ok: true,
            exit_code,
            result: Some(result),
            warnings,
            error: None,
        }
    }

    /// A failed document: `exit_code = error.exit_code(op)`, `result = null`, `error = error.report()`.
    pub fn failure(
        schema: &'static str,
        error: &ApiError,
        op: ApiOp,
        warnings: Vec<Warning>,
    ) -> JsonEnvelope<T> {
        JsonEnvelope {
            schema,
            hikasa_version: HIKASA_VERSION,
            ok: false,
            exit_code: error.exit_code(op).code(),
            result: None,
            warnings,
            error: Some(error.report()),
        }
    }

    /// Writes the document pretty-printed with 2-space indentation and a final `\n` (§4.1 rule 1).
    pub fn write_to(&self, out: &mut dyn Write) -> std::io::Result<()> {
        write_json(out, self)
    }
}

impl JsonEnvelope<()> {
    /// The document of a usage error (`hikasa.usage_error.v1`, CLI invariant 2): `ok: false`, exit code 100
    /// (D-35), `result: null`, `error: { "kind": "invalid_input", "message", "spec": null, "causes": [] }`.
    /// Additive (wave-4 review).
    pub fn usage_error(message: &str) -> JsonEnvelope<()> {
        JsonEnvelope {
            schema: USAGE_ERROR_SCHEMA,
            hikasa_version: HIKASA_VERSION,
            ok: false,
            exit_code: ExitCode::UnknownError.code(),
            result: None,
            warnings: Vec::new(),
            error: Some(ErrorReport {
                kind: hikasa_core::ErrorKind::InvalidInput,
                message: message.to_owned(),
                spec: None,
                causes: Vec::new(),
            }),
        }
    }
}

/// `hikasa.completions.v1`: the completion script inside the envelope (CLI-000: every subcommand supports
/// `--json`). Additive (wave-4 review).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompletionsDto {
    /// The shell (`bash`, `elvish`, `fish`, `powershell`, `zsh`).
    pub shell: String,
    /// The script.
    pub script: String,
}

/// `hikasa.help.v1`: one topic (`hikasa help [topic]`) or the topic list (`hikasa help --list`); the other part
/// is `null` / empty (§4.1 rule 3: arrays are never `null`). M2 W0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpDto {
    /// The topic shown (`null` with `--list`).
    pub topic: Option<HelpTopicDto>,
    /// Every topic, in contents order (empty unless `--list`).
    pub topics: Vec<HelpListItemDto>,
}

/// One topic of `hikasa.help.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpTopicDto {
    /// Id (`text-compare`).
    pub id: String,
    /// Title (the `#` heading).
    pub title: String,
    /// The Markdown source.
    pub markdown: String,
    /// The topics it links to.
    pub links: Vec<String>,
}

/// One entry of `hikasa help --list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelpListItemDto {
    /// Id.
    pub id: String,
    /// Title.
    pub title: String,
}

impl HelpDto {
    /// The document of one topic.
    pub fn topic(topic: &crate::help::HelpTopic) -> HelpDto {
        HelpDto {
            topic: Some(HelpTopicDto {
                id: topic.id.to_owned(),
                title: topic.title(),
                markdown: topic.source.to_owned(),
                links: topic.links(),
            }),
            topics: Vec::new(),
        }
    }

    /// The document of `--list`.
    pub fn list() -> HelpDto {
        HelpDto {
            topic: None,
            topics: crate::help::topics()
                .iter()
                .map(|t| HelpListItemDto {
                    id: t.id.to_owned(),
                    title: t.title(),
                })
                .collect(),
        }
    }
}

/// Pretty JSON that also escapes what `serde_json` leaves as it is and a terminal or an editor would act on: DEL and
/// the C1 controls (U+009B is a CSI), the line and paragraph separators and the bidirectional controls (U+202E shows
/// `txt.exe` as `exe.txt`). Names reach `--json` from hostile archives. A parser gets the same text back, so this
/// costs a consumer nothing (§1.5.13).
struct JsonEscapes(serde_json::ser::PrettyFormatter<'static>);

/// True for the characters of [`JsonEscapes`].
fn json_escaped(c: char) -> bool {
    matches!(
        c,
        '\u{7f}'..='\u{9f}'
            | '\u{61c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

macro_rules! pretty {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name<W: ?Sized + Write>(&mut self, out: &mut W $(, $arg: $ty)*) -> std::io::Result<()> {
            self.0.$name(out $(, $arg)*)
        })*
    };
}

impl serde_json::ser::Formatter for JsonEscapes {
    pretty! {
        begin_array();
        end_array();
        begin_array_value(first: bool);
        end_array_value();
        begin_object();
        end_object();
        begin_object_key(first: bool);
        begin_object_value();
        end_object_value();
    }

    fn write_string_fragment<W: ?Sized + Write>(
        &mut self,
        out: &mut W,
        fragment: &str,
    ) -> std::io::Result<()> {
        let mut start = 0;
        for (at, c) in fragment.char_indices().filter(|(_, c)| json_escaped(*c)) {
            out.write_all(fragment.get(start..at).unwrap_or_default().as_bytes())?;
            write!(out, "\\u{:04x}", u32::from(c))?;
            start = at + c.len_utf8();
        }
        out.write_all(fragment.get(start..).unwrap_or_default().as_bytes())
    }
}

/// Writes `value` as pretty JSON (2-space indentation) followed by `\n` (§4.1 rule 1).
pub fn write_json<T: Serialize + ?Sized>(out: &mut dyn Write, value: &T) -> std::io::Result<()> {
    // A value that cannot be serialised writes nothing; a large document is not held in memory twice (G6).
    serde_json::to_writer(std::io::sink(), value).map_err(std::io::Error::other)?;
    let mut out = std::io::BufWriter::with_capacity(1 << 16, out);
    let pretty = serde_json::ser::PrettyFormatter::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, JsonEscapes(pretty));
    value
        .serialize(&mut serializer)
        .map_err(std::io::Error::from)?;
    out.write_all(b"\n")?;
    out.flush()
}

/// `hikasa.quick.v1` (§4.3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuickDto {
    /// Left spec.
    pub left: String,
    /// Right spec.
    pub right: String,
    /// `size`, `crc`, `binary` or `rules`.
    pub method: &'static str,
    /// `{"result": "<content kind>"}` (plus `error` for `error`).
    pub result: ContentResult,
    /// The request's ignore-unimportant flag.
    pub ignore_unimportant: bool,
    /// Effective format (`null` for the size, CRC and binary methods).
    pub format: Option<FormatJson>,
}

impl QuickDto {
    /// The DTO of a quick response (`ignore_unimportant` is the response's, i.e. the request's, flag).
    /// Changed in the wave-4 review (the flag was a second argument).
    pub fn new(resp: &QuickResponse) -> QuickDto {
        QuickDto {
            left: resp.left.canonical(),
            right: resp.right.canonical(),
            method: resp.method.as_str(),
            result: resp.outcome.result.clone(),
            ignore_unimportant: resp.ignore_unimportant,
            format: resp.format.as_ref().map(|f| f.json()),
        }
    }
}

/// `hikasa.format_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatListDto {
    /// The configuration folder.
    pub config_dir: String,
    /// The registry in order.
    pub formats: Vec<FormatListItemDto>,
}

/// One registry entry of `format list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatListItemDto {
    /// 1-based position (detection order).
    pub position: u32,
    /// Id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Kind.
    pub kind: &'static str,
    /// Mask (`;`-separated).
    pub mask: String,
    /// Globally enabled.
    pub enabled: bool,
    /// `built_in` or `user`.
    pub origin: &'static str,
    /// The user file (`null` for built-ins).
    pub path: Option<String>,
    /// Shadowing status (§2.12.3).
    pub status: ShadowStatusDto,
}

/// `{"status": "visible"|"shadowed"|"manual_only"|"disabled", "by": [ids]}` (`by` is empty unless shadowed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShadowStatusDto {
    /// Status name.
    pub status: &'static str,
    /// The covering entries (shadowed only; else `[]`).
    pub by: Vec<String>,
}

/// `hikasa.format_show.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatShowDto {
    /// Id.
    pub id: String,
    /// `built_in` or `user`.
    pub origin: &'static str,
    /// The user file (`null` for built-ins).
    pub path: Option<String>,
    /// Globally enabled.
    pub enabled: bool,
    /// The full format (every section present, §1.4.8 item 10).
    pub format: FileFormatDto,
    /// The compile error of the format, when it does not compile (`null` otherwise).
    pub compile_error: Option<ErrorReport>,
}

/// A `FileFormat` with every section present (§4.1 rule 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileFormatDto {
    /// Schema version of the format file.
    pub schema_version: u32,
    /// Id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Kind.
    pub kind: &'static str,
    /// Mask.
    pub mask: String,
    /// Conversion.
    pub conversion: ConversionDto,
    /// Encoding.
    pub encoding: EncodingSettingDto,
    /// Text settings.
    pub text: TextSettingsDto,
    /// Element settings by name (`{}` when none).
    pub elements: BTreeMap<String, ElementInfoDto>,
    /// Grammar items in order (`[]` when none).
    pub grammar: Vec<GrammarItemDto>,
    /// Line weights (`[]` when none).
    pub line_weight: Vec<LineWeightDto>,
    /// Table settings (`null` unless set).
    pub table: Option<TableSettingsDto>,
}

/// `{"method": "none"|"sorted"|"external", "load", "save", "filename_encoding", "timeout_secs"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversionDto {
    /// Method.
    pub method: &'static str,
    /// External: load command line.
    pub load: Option<String>,
    /// External: save command line.
    pub save: Option<String>,
    /// External: `unicode` or `ansi`.
    pub filename_encoding: Option<&'static str>,
    /// External: timeout in seconds (0 = none).
    pub timeout_secs: Option<u32>,
}

/// `{"mode": "auto"|"fixed", "fallback", "label"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EncodingSettingDto {
    /// Mode.
    pub mode: &'static str,
    /// Auto: the fallback label.
    pub fallback: Option<String>,
    /// Fixed: the label.
    pub label: Option<String>,
}

/// Text settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextSettingsDto {
    /// Tab stop.
    pub tab_size: u8,
    /// Insert spaces instead of tabs.
    pub tab_as_spaces: bool,
    /// Characters-per-line limit.
    pub chars_per_line_limit: Option<u32>,
    /// Lines are independent.
    pub lines_independent: bool,
    /// Column-based data.
    pub column_based: bool,
    /// Ctrl-Z ends the file.
    pub ctrl_z_eof: bool,
    /// Save: trim trailing whitespace.
    pub save_trim_trailing_ws: bool,
    /// Save: leading spaces to tabs.
    pub save_leading_spaces_to_tabs: bool,
    /// Extra word characters.
    pub word_chars: String,
}

/// Element settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ElementInfoDto {
    /// Case changes are important.
    pub case_sensitive: bool,
    /// Default importance.
    pub important: bool,
}

/// A grammar item with every field of its kind (§1.4.8 item 10); `element` and `kind` come first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GrammarItemDto {
    /// `basic`.
    Basic {
        /// Element.
        element: String,
        /// `"basic"`.
        kind: &'static str,
        /// Text or regex.
        text: String,
        /// Regex.
        regex: bool,
        /// Case-sensitive.
        match_case: bool,
    },
    /// `delimited`.
    Delimited {
        /// Element.
        element: String,
        /// `"delimited"`.
        kind: &'static str,
        /// Opening delimiter.
        from: String,
        /// Closing delimiter (`null` = end of line).
        to: Option<String>,
        /// Escape character.
        escape: Option<String>,
        /// Stop at the end of the line.
        stop_at_eol: bool,
        /// Regex.
        regex: bool,
        /// Case-sensitive.
        match_case: bool,
    },
    /// `list`.
    List {
        /// Element.
        element: String,
        /// `"list"`.
        kind: &'static str,
        /// Tokens.
        tokens: Vec<String>,
        /// Regex.
        regex: bool,
        /// Case-sensitive.
        match_case: bool,
    },
    /// `columns`.
    Columns {
        /// Element.
        element: String,
        /// `"columns"`.
        kind: &'static str,
        /// First column (1-based).
        from_column: u32,
        /// Last column (`null` = end of line).
        to_column: Option<u32>,
    },
    /// `lines`.
    Lines {
        /// Element.
        element: String,
        /// `"lines"`.
        kind: &'static str,
        /// Heading text or regex.
        heading: String,
        /// Lines covered after the heading.
        next_lines: u32,
        /// Line 1 also starts the item.
        or_line_one: bool,
        /// Regex.
        regex: bool,
        /// Case-sensitive.
        match_case: bool,
    },
}

/// A line weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineWeightDto {
    /// Text or regex.
    pub pattern: String,
    /// Regex.
    pub regex: bool,
    /// Case-sensitive.
    pub match_case: bool,
    /// Priority 1..=4.
    pub priority: u8,
}

/// Table settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableSettingsDto {
    /// Field delimiter.
    pub delimiter: String,
    /// Text qualifier.
    pub qualifier: Option<String>,
    /// Consecutive delimiters count as one.
    pub consecutive_as_one: bool,
}

impl FileFormatDto {
    /// The full shape of a format definition.
    pub fn new(format: &FileFormat) -> FileFormatDto {
        FileFormatDto {
            schema_version: format.schema_version,
            id: format.id.as_str().to_owned(),
            name: format.name.clone(),
            description: format.description.clone(),
            kind: format.kind.as_str(),
            mask: format.mask.clone(),
            conversion: conversion_dto(&format.conversion),
            encoding: encoding_dto(&format.encoding),
            text: text_settings_dto(&format.text),
            elements: format
                .elements
                .iter()
                .map(|(name, info)| (name.as_str().to_owned(), element_dto(*info)))
                .collect(),
            grammar: format.grammar.iter().map(grammar_dto).collect(),
            line_weight: format
                .line_weights
                .iter()
                .map(|w| LineWeightDto {
                    pattern: w.pattern.clone(),
                    regex: w.regex,
                    match_case: w.match_case,
                    priority: w.priority,
                })
                .collect(),
            table: format.table.as_ref().map(table_dto),
        }
    }
}

fn conversion_dto(conversion: &Conversion) -> ConversionDto {
    match conversion {
        Conversion::None | Conversion::Sorted => ConversionDto {
            method: if matches!(conversion, Conversion::None) {
                "none"
            } else {
                "sorted"
            },
            load: None,
            save: None,
            filename_encoding: None,
            timeout_secs: None,
        },
        Conversion::External(x) => ConversionDto {
            method: "external",
            load: Some(x.load.clone()),
            save: x.save.clone(),
            filename_encoding: Some(match x.filename_encoding {
                FilenameEncoding::Unicode => "unicode",
                FilenameEncoding::Ansi => "ansi",
            }),
            timeout_secs: Some(x.timeout_secs),
        },
    }
}

fn encoding_dto(encoding: &EncodingSetting) -> EncodingSettingDto {
    match encoding {
        EncodingSetting::Auto { fallback } => EncodingSettingDto {
            mode: "auto",
            fallback: fallback.clone(),
            label: None,
        },
        EncodingSetting::Fixed { label } => EncodingSettingDto {
            mode: "fixed",
            fallback: None,
            label: Some(label.clone()),
        },
    }
}

fn text_settings_dto(text: &TextSettings) -> TextSettingsDto {
    TextSettingsDto {
        tab_size: text.tab_size,
        tab_as_spaces: text.tab_as_spaces,
        chars_per_line_limit: text.chars_per_line_limit,
        lines_independent: text.lines_independent,
        column_based: text.column_based,
        ctrl_z_eof: text.ctrl_z_eof,
        save_trim_trailing_ws: text.save_trim_trailing_ws,
        save_leading_spaces_to_tabs: text.save_leading_spaces_to_tabs,
        word_chars: text.word_chars.clone(),
    }
}

fn element_dto(info: ElementInfo) -> ElementInfoDto {
    ElementInfoDto {
        case_sensitive: info.case_sensitive,
        important: info.important,
    }
}

fn grammar_dto(item: &GrammarItem) -> GrammarItemDto {
    let element = item.element.as_str().to_owned();
    let kind = item.kind.name();
    match &item.kind {
        ItemKind::Basic {
            text,
            regex,
            match_case,
        } => GrammarItemDto::Basic {
            element,
            kind,
            text: text.clone(),
            regex: *regex,
            match_case: *match_case,
        },
        ItemKind::Delimited {
            from,
            to,
            stop_at_eol,
            escape,
            regex,
            match_case,
        } => GrammarItemDto::Delimited {
            element,
            kind,
            from: from.clone(),
            to: to.clone(),
            escape: escape.map(String::from),
            stop_at_eol: *stop_at_eol,
            regex: *regex,
            match_case: *match_case,
        },
        ItemKind::List {
            tokens,
            regex,
            match_case,
        } => GrammarItemDto::List {
            element,
            kind,
            tokens: tokens.clone(),
            regex: *regex,
            match_case: *match_case,
        },
        ItemKind::Columns {
            from_column,
            to_column,
        } => GrammarItemDto::Columns {
            element,
            kind,
            from_column: *from_column,
            to_column: *to_column,
        },
        ItemKind::Lines {
            heading,
            regex,
            match_case,
            next_lines,
            or_line_one,
        } => GrammarItemDto::Lines {
            element,
            kind,
            heading: heading.clone(),
            next_lines: *next_lines,
            or_line_one: *or_line_one,
            regex: *regex,
            match_case: *match_case,
        },
    }
}

fn table_dto(table: &TableSettings) -> TableSettingsDto {
    TableSettingsDto {
        delimiter: table.delimiter.to_string(),
        qualifier: table.qualifier.map(String::from),
        consecutive_as_one: table.consecutive_as_one,
    }
}

/// `{"id", "name", "kind"}` of a format (`id` is `null` for Everything Else).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatRefDto {
    /// Id.
    pub id: Option<String>,
    /// Display name.
    pub name: String,
    /// Kind.
    pub kind: &'static str,
}

/// `hikasa.format_detect.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatDetectDto {
    /// The file as given (canonical spec; `-` for stdin).
    pub file: String,
    /// The detected format.
    pub format: FormatRefDto,
    /// Why (`{"reason", "id", "mask"}`, full shape).
    pub reason: DetectReasonJson,
}

/// `hikasa.format_tokens.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatTokensDto {
    /// The file (canonical spec).
    pub file: String,
    /// The effective format and why it was chosen.
    pub format: FormatJson,
    /// How the file was decoded.
    pub encoding: EncodingDto,
    /// Regex run-time limit hits while tokenising (each counts as "no match").
    pub regex_errors: u64,
    /// The selected lines.
    pub lines: Vec<TokenLineDto>,
}

/// Decoding facts of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EncodingDto {
    /// Encoding name (`UTF-8`, …).
    pub name: &'static str,
    /// How it was chosen (`utf8_valid`, `bom`, …).
    pub source: hikasa_formats::EncodingSource,
    /// A BOM was stripped.
    pub bom: bool,
    /// Malformed sequences replaced by U+FFFD.
    pub malformed: u64,
}

/// One line of `format tokens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenLineDto {
    /// 1-based line number.
    pub line: u32,
    /// Tokenizer state at the line start.
    pub state_in: LineStateDto,
    /// The tokens (they cover the line).
    pub tokens: Vec<TokenDto>,
}

/// `{"state": "normal"|"in_delimited"|"in_lines", "item", "remaining"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LineStateDto {
    /// State name.
    pub state: &'static str,
    /// Grammar item index (`null` for `normal`).
    pub item: Option<u16>,
    /// Lines still covered (`in_lines` only).
    pub remaining: Option<u32>,
}

impl LineStateDto {
    /// The DTO of a tokenizer state.
    pub fn new(state: LineState) -> LineStateDto {
        match state {
            LineState::Normal => LineStateDto {
                state: "normal",
                item: None,
                remaining: None,
            },
            LineState::InDelimited { item } => LineStateDto {
                state: "in_delimited",
                item: Some(item),
                remaining: None,
            },
            LineState::InLines { item, remaining } => LineStateDto {
                state: "in_lines",
                item: Some(item),
                remaining: Some(remaining),
            },
        }
    }
}

/// One token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenDto {
    /// Start byte (0-based).
    pub start: u32,
    /// End byte (exclusive).
    pub end: u32,
    /// Element name.
    pub element: String,
    /// Grammar item index (`null` for everything-else runs).
    pub item: Option<u16>,
    /// The token text.
    pub text: String,
}

/// `VfsEntry` (§4.3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsEntryDto {
    /// Name (UTF-8, lossy when needed).
    pub name: String,
    /// True when `name` is lossy.
    pub name_lossy: bool,
    /// `dir`, `file`, `symlink` or `other`.
    pub kind: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// Modification time (RFC 3339 UTC).
    pub mtime: Option<String>,
    /// Creation time.
    pub ctime: Option<String>,
    /// DOS attribute letters (`null` without DOS attributes).
    pub attrs: Option<String>,
    /// Unix permission bits.
    pub mode: Option<u32>,
    /// `drwxr-xr-x` style string.
    pub permissions: Option<String>,
    /// Owner uid.
    pub uid: Option<u32>,
    /// Group gid.
    pub gid: Option<u32>,
    /// Symlink target (lossy text).
    pub link_target: Option<String>,
    /// The entry describes a symlink's target.
    pub followed: bool,
    /// CRC32 provided by the back-end (8 lowercase hex digits).
    pub crc32: Option<String>,
}

impl VfsEntryDto {
    /// The DTO of an entry.
    pub fn new(entry: &Entry) -> VfsEntryDto {
        let unix = entry.attrs.unix();
        VfsEntryDto {
            name: entry.name.as_str().to_owned(),
            name_lossy: entry.name.is_lossy(),
            kind: entry.kind.as_str(),
            size: entry.size,
            mtime: entry.mtime.map(|t| t.to_rfc3339()),
            ctime: entry.ctime.map(|t| t.to_rfc3339()),
            attrs: match entry.attrs {
                Attrs::Dos { attrs } => Some(attrs.letters()),
                Attrs::None | Attrs::Unix { .. } => None,
            },
            mode: unix.map(|m| m.mode),
            permissions: unix.map(|m| m.permissions_string(entry.kind)),
            uid: unix.map(|m| m.uid),
            gid: unix.map(|m| m.gid),
            link_target: entry.link_target.clone(),
            followed: entry.followed,
            crc32: entry.crc32.map(|c| format!("{c:08x}")),
        }
    }
}

/// `hikasa.vfs_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsListDto {
    /// The folder (canonical spec).
    pub spec: String,
    /// Entries sorted by name bytes.
    pub entries: Vec<VfsEntryDto>,
    /// Entries whose metadata could not be read.
    pub errors: Vec<VfsListErrorDto>,
}

/// A per-entry failure of `vfs ls`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsListErrorDto {
    /// The entry name.
    pub name: String,
    /// The failure.
    pub error: ErrorReport,
}

/// `hikasa.vfs_stat.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsStatDto {
    /// The spec (canonical).
    pub spec: String,
    /// Its metadata.
    pub entry: VfsEntryDto,
    /// The back-end's capabilities there (snake_case names in bit order).
    pub capabilities: Vec<&'static str>,
}

impl VfsStatDto {
    /// Capability names in canonical bit order (§4.1 rule 9).
    pub fn capability_names(caps: Capabilities) -> Vec<&'static str> {
        caps.names()
    }
}

/// `hikasa.vfs_cat.v1` (D-32).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsCatDto {
    /// The spec (canonical).
    pub spec: String,
    /// Bytes.
    pub size: u64,
    /// The content, base64 (RFC 4648, with padding).
    pub data_base64: String,
}

impl VfsCatDto {
    /// The DTO of a file's content.
    pub fn new(spec: String, bytes: &[u8]) -> VfsCatDto {
        VfsCatDto {
            spec,
            size: bytes.len() as u64,
            data_base64: base64_encode(bytes),
        }
    }
}

/// Standard base64 (RFC 4648 §4, with `=` padding). In-house: no base64 crate is approved.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let symbol = |shift: u32| char::from(ALPHABET[((n >> shift) & 63) as usize]);
        out.push(symbol(18));
        out.push(symbol(12));
        out.push(if chunk.len() > 1 { symbol(6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(0) } else { '=' });
    }
    out
}

/// `hikasa.gui_open.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuiOpenDto {
    /// The opened view.
    pub view: u64,
    /// `--wait` was given and the view closed.
    pub waited: bool,
    /// The view's result (`null` without `--wait`).
    pub result: Option<ViewResult>,
    /// The view's exit code (`null` without `--wait`).
    pub view_exit_code: Option<u8>,
    /// The file stdin was spooled to (`gui open -`, D-49).
    pub spooled_stdin: Option<String>,
    /// `gui workspace` (and `gui open <workspace>`, D-88): every view the workspace opened (`view` is the
    /// first, 0 when none); absent for one view. Additive (M2 W2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<u64>,
}

/// `hikasa.gui_ping.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuiPingDto {
    /// The endpoint (socket path or pipe name).
    pub endpoint: String,
    /// Server identity.
    pub server: String,
    /// Negotiated protocol version.
    pub protocol_version: u32,
}

impl GuiPingDto {
    /// The DTO of a ping.
    pub fn new(endpoint: String, info: &ServerInfo) -> GuiPingDto {
        GuiPingDto {
            endpoint,
            server: info.server.clone(),
            protocol_version: info.version,
        }
    }
}

/// `hikasa.gui_list.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuiListDto {
    /// The open views.
    pub views: Vec<ViewInfo>,
}

/// `hikasa.gui_close.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GuiCloseDto {
    /// The closed view.
    pub view: u64,
}

/// `hikasa.gui_quit.v1` (`{}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct GuiQuitDto {}

/// `hikasa.version.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionDto {
    /// Version of this build.
    pub version: &'static str,
    /// IPC protocol version.
    pub ipc_protocol: u32,
    /// Every JSON schema id this build writes.
    pub schemas: Vec<&'static str>,
}

impl VersionDto {
    /// This build.
    pub fn current() -> VersionDto {
        VersionDto {
            version: HIKASA_VERSION,
            ipc_protocol: PROTOCOL_VERSION,
            schemas: SCHEMAS.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::write_json;

    /// A hostile name in `--json`: valid JSON, the same text after parsing, nothing a terminal would act on.
    #[test]
    fn json_escapes_what_a_terminal_acts_on() {
        let name = "esc\u{1b}[31m del\u{7f} csi\u{9b}31m rtl\u{202e}txt.exe bidi\u{2066}x ls\u{2028} é ü 日本 😀";
        let mut out = Vec::new();
        write_json(&mut out, &serde_json::json!({ "name": name })).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\\u001b[31m"), "{text}");
        assert!(
            text.contains("\\u007f") && text.contains("\\u009b31m"),
            "{text}"
        );
        assert!(
            text.contains("\\u202etxt.exe") && text.contains("\\u2066x"),
            "{text}"
        );
        assert!(text.contains("\\u2028"), "{text}");
        assert!(text.contains(" é ü 日本 😀"), "ordinary text stays: {text}");
        assert!(
            !text
                .chars()
                .any(|c| c != '\n' && (c.is_control() || super::json_escaped(c)))
        );
        let back: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back["name"], name);
    }

    use super::*;

    #[test]
    fn base64_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(input.as_bytes()), expected);
        }
        assert_eq!(base64_encode(&[0xff, 0xfe, 0x00]), "//4A");
    }
}

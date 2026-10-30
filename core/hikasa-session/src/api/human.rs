//! Human (non-JSON) lines of the commands whose output is a list (§1.9.2 amendment, wave-4 review): the CLI
//! prints these lines as they are, so the escaping of names and file text (§1.7.3 item 5: control and
//! bidirectional formatting characters as `\n`, `\t`, `\u{…}`, a backslash as `\\`) lives in one place.
//!
//! | Command | Line |
//! |---|---|
//! | `format tokens` | `L<line>:<start>-<end>\t<element>\t<escaped text>` ([`token_lines`]) |
//! | `vfs ls` | `<kind><attrs> <size> <mtime> <escaped name>` ([`vfs_ls_line`]) |
//! | `vfs stat` | the `vfs ls` line of the entry, then `capabilities: <names>` ([`vfs_stat_lines`]) |
//! | `gui list` | `<view>\t<kind>\t<escaped title>\t<escaped spec>…` ([`gui_list_line`]) |
//! | `format list` | `<position>\t<id>\t<status>\t<name>\t<mask>` ([`format_list_line`]) |
//! | `format detect` | `<id or ->\t<name>` ([`format_detect_line`]) |
//! | `settings get` | the TOML value; a subtree as a TOML document ([`settings_get_text`]) |
//! | `settings set/unset` | `<key> = <value>`, then `(file unchanged)` when nothing was written ([`settings_set_lines`]) |
//! | `settings list` | `<key> = <value>`, with `  # default: <value>` when changed ([`settings_list_lines`]) |
//! | `settings restore-defaults` | `delete <path>` or `absent <path>` per file ([`settings_restore_lines`]) |
//! | `format` writes | `wrote <path>`, `removed <path>` ([`format_write_lines`]); `format restore`: `delete`/`absent` |
//! | `session list` | `folder <ref>/`, `<ref>\t<type>\t<flags>\t<description>`, `defaults <type>`, `autosaved <id>\t<type>\t<group>\t<label>` ([`session_list_lines`]) |
//! | `session`/`workspace` writes | `wrote <path>`, `removed <path>` ([`session_write_lines`], [`workspace_write_lines`]) |
//! | `workspace list` | `<name>\t<windows> windows\t<tabs> tabs\t<shortcut>` ([`workspace_list_lines`]) |
//! | `settings export` | `<category>\t<path>` per item, then `wrote <package>` ([`settings_export_lines`]) |
//! | `settings import` | `add`/`replace <path>`, `delete <path>`, one summary line per category, failures ([`settings_import_lines`]) |

use hikasa_report::{escape_controls, escape_name};
use hikasa_vfs::EntryKind;
use hikasa_vfs::ops::ItemStatus;

use super::dto::{FormatDetectDto, FormatListItemDto, FormatTokensDto, VfsEntryDto, VfsStatDto};
use super::package::SettingsImportDto;
use super::sessions::{SessionListDto, SessionWriteDto, WorkspaceListDto, WorkspaceWriteDto};
use super::settings::{
    FormatRestoreDto, FormatWriteDto, SettingsListDto, SettingsRestoreDto, SettingsSetDto,
    SettingsValueDto,
};
use super::vfs_ops::{VfsOpDto, VfsOpItemDto};
use crate::ipc::ViewInfo;
use crate::settings::ShortcutConflict;
use crate::settings::tree::inline;

/// The `format tokens` lines: one per token, `L<line>:<start>-<end>\t<element>\t<escaped text>` (byte offsets,
/// end exclusive).
pub fn token_lines(dto: &FormatTokensDto) -> Vec<String> {
    let mut out = Vec::new();
    for line in &dto.lines {
        for token in &line.tokens {
            out.push(format!(
                "L{}:{}-{}\t{}\t{}",
                line.line,
                token.start,
                token.end,
                escape_controls(&token.element),
                escape_name(&token.text)
            ));
        }
    }
    out
}

/// The kind letter of an entry: `d` folder, `-` file, `l` symbolic link, `?` other (as in `ls -l`).
fn kind_letter(kind: &str) -> char {
    if kind == EntryKind::Dir.as_str() {
        'd'
    } else if kind == EntryKind::File.as_str() {
        '-'
    } else if kind == EntryKind::Symlink.as_str() {
        'l'
    } else {
        '?'
    }
}

/// One `vfs ls` line: `<kind><attrs> <size> <mtime> <escaped name>`. `<kind>` is `d` (folder), `-` (file), `l` (link) or `?`; `<attrs>` is
/// the Unix permission string without its kind letter (`rwxr-xr-x`), else the DOS attribute letters, else `-`;
/// `<mtime>` is RFC 3339 UTC or `-`; a symbolic link adds ` -> <escaped target>`.
pub fn vfs_ls_line(entry: &VfsEntryDto) -> String {
    let attrs = match (&entry.permissions, &entry.attrs) {
        (Some(permissions), _) => permissions.chars().skip(1).collect::<String>(),
        (None, Some(letters)) if !letters.is_empty() => letters.clone(),
        (None, _) => "-".to_owned(),
    };
    let mut line = format!(
        "{}{} {} {} {}",
        kind_letter(entry.kind),
        attrs,
        entry.size,
        entry.mtime.as_deref().unwrap_or("-"),
        escape_name(&entry.name)
    );
    if let Some(target) = &entry.link_target {
        line.push_str(" -> ");
        line.push_str(&escape_name(target));
    }
    line
}

/// The `vfs stat` lines: the [`vfs_ls_line`] of the entry, then `capabilities: <names, space-separated>`.
pub fn vfs_stat_lines(dto: &VfsStatDto) -> Vec<String> {
    vec![
        vfs_ls_line(&dto.entry),
        format!("capabilities: {}", dto.capabilities.join(" ")),
    ]
}

/// One `gui list` line: `<view>\t<kind>\t<escaped title>\t<escaped spec>…` (tab-separated).
pub fn gui_list_line(view: &ViewInfo) -> String {
    let mut line = format!(
        "{}\t{}\t{}",
        view.view.0,
        view.kind.as_str(),
        escape_name(&view.title)
    );
    for spec in &view.specs {
        line.push('\t');
        line.push_str(&escape_name(spec));
    }
    line
}

/// One `format list` line: `<position>\t<id>\t<status>\t<name>\t<mask>`.
pub fn format_list_line(item: &FormatListItemDto) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}",
        item.position,
        item.id,
        item.status.status,
        escape_controls(&item.name),
        escape_controls(&item.mask)
    )
}

/// The `format detect` line: `<id, or - for Everything Else>\t<name>`.
pub fn format_detect_line(dto: &FormatDetectDto) -> String {
    format!(
        "{}\t{}",
        dto.format.id.as_deref().unwrap_or("-"),
        escape_controls(&dto.format.name)
    )
}

/// The value of `settings get`: a leaf as its TOML value on one line, a subtree as a TOML document, `(not set)`
/// for an optional key without a value.
pub fn settings_get_text(dto: &SettingsValueDto) -> String {
    match &dto.toml {
        None => "(not set)\n".to_owned(),
        Some(toml::Value::Table(table)) => toml::to_string(table).unwrap_or_default(),
        Some(value) => format!("{}\n", inline(value)),
    }
}

/// The lines of `settings set` and `settings unset`.
pub fn settings_set_lines(dto: &SettingsSetDto) -> Vec<String> {
    let value = dto
        .toml
        .as_ref()
        .map_or_else(|| "(not set)".to_owned(), inline);
    let mut lines = vec![format!("{} = {value}", dto.key)];
    if !dto.written {
        lines.push("(file unchanged)".to_owned());
    }
    lines
}

/// The lines of `settings list`: `<key> = <value>`, then `  # default: <value>` for a changed key or
/// `  # unknown key (kept)` for a key this version does not know.
pub fn settings_list_lines(dto: &SettingsListDto) -> Vec<String> {
    dto.entries
        .iter()
        .map(|e| {
            let mut line = format!("{} = {}", e.key, inline(&e.toml));
            if e.unknown {
                line.push_str("  # unknown key (kept)");
            } else if !e.is_default
                && let Some(default) = &e.default_toml
            {
                line.push_str(&format!("  # default: {}", inline(default)));
            }
            line
        })
        .collect()
}

/// A shortcut conflict as a warning text: `shortcut conflict in <view>: <shortcut> is bound to A, B`.
pub fn shortcut_conflict_text(conflict: &ShortcutConflict) -> String {
    format!(
        "shortcut conflict in {}: {} is bound to {}",
        conflict.view,
        conflict.shortcut,
        conflict.commands.join(", ")
    )
}

/// The lines of `settings restore-defaults`: `delete <path>` for a file that exists, `absent <path>` for one
/// that does not.
pub fn settings_restore_lines(dto: &SettingsRestoreDto) -> Vec<String> {
    dto.files
        .iter()
        .map(|f| {
            let word = if f.existed { "delete" } else { "absent" };
            format!("{word} {}", escape_controls(&f.path))
        })
        .collect()
}

/// The lines of a `format` write: `wrote <path>` and `removed <path>`.
pub fn format_write_lines(dto: &FormatWriteDto) -> Vec<String> {
    dto.written
        .iter()
        .map(|p| format!("wrote {}", escape_controls(p)))
        .chain(
            dto.removed
                .iter()
                .map(|p| format!("removed {}", escape_controls(p))),
        )
        .collect()
}

/// The lines of `format restore`: `delete <path>` per removed file, `absent <path>` per named id without one.
pub fn format_restore_lines(dto: &FormatRestoreDto) -> Vec<String> {
    dto.removed
        .iter()
        .map(|p| format!("delete {}", escape_controls(p)))
        .chain(
            dto.absent
                .iter()
                .map(|p| format!("absent {}", escape_controls(p))),
        )
        .collect()
}

/// Written and removed paths.
fn write_lines(written: &[String], removed: &[String]) -> Vec<String> {
    written
        .iter()
        .map(|p| format!("wrote {}", escape_controls(p)))
        .chain(
            removed
                .iter()
                .map(|p| format!("removed {}", escape_controls(p))),
        )
        .collect()
}

/// The lines of `session list`: folders (`folder <ref>/`), named sessions (`<ref>\t<type>\t<flags>\t<description>`,
/// flags `L` locked and `F` favorite or `-`), then `defaults <type>` and `autosaved <id>\t<type>\t<group>\t<label>`.
pub fn session_list_lines(dto: &SessionListDto) -> Vec<String> {
    let mut lines: Vec<String> = dto
        .folders
        .iter()
        .map(|f| format!("folder {}/", escape_controls(f)))
        .collect();
    for s in &dto.sessions {
        let mut flags = String::new();
        if s.locked {
            flags.push('L');
        }
        if s.favorite {
            flags.push('F');
        }
        if flags.is_empty() {
            flags.push('-');
        }
        lines.push(format!(
            "{}\t{}\t{flags}\t{}",
            escape_controls(&s.reference),
            s.kind,
            escape_controls(&s.description)
        ));
    }
    lines.extend(dto.defaults.iter().map(|d| format!("defaults {d}")));
    lines.extend(dto.autosaved.iter().map(|a| {
        format!(
            "autosaved {}\t{}\t{}\t{}",
            a.id,
            a.kind,
            a.group,
            escape_controls(&a.label)
        )
    }));
    lines
}

/// The lines of a `session` write.
pub fn session_write_lines(dto: &SessionWriteDto) -> Vec<String> {
    write_lines(&dto.written, &dto.removed)
}

/// The lines of a `workspace` write.
pub fn workspace_write_lines(dto: &WorkspaceWriteDto) -> Vec<String> {
    write_lines(&dto.written, &dto.removed)
}

/// The lines of `workspace list`.
pub fn workspace_list_lines(dto: &WorkspaceListDto) -> Vec<String> {
    dto.workspaces
        .iter()
        .map(|w| {
            format!(
                "{}\t{} windows\t{} tabs\t{}",
                escape_controls(&w.name),
                w.windows,
                w.tabs,
                escape_controls(&w.shortcut)
            )
        })
        .collect()
}

/// The lines of `settings export`.
pub fn settings_export_lines(dto: &crate::package::ExportDto) -> Vec<String> {
    dto.items
        .iter()
        .map(|i| format!("{}\t{}", i.category, escape_controls(&i.path)))
        .chain(std::iter::once(format!(
            "wrote {}",
            escape_controls(&dto.path)
        )))
        .collect()
}

/// The lines of `settings import`: the deletes, the items (`add`/`replace <path>`), one summary line per
/// category (`<category>: <n> added, <n> replaced, <n> deleted`), then `failed <name>: <error>` per failure.
pub fn settings_import_lines(dto: &SettingsImportDto) -> Vec<String> {
    let plan = &dto.plan;
    let mut lines: Vec<String> = plan
        .deletes
        .iter()
        .map(|p| format!("delete {}", escape_controls(p)))
        .collect();
    for item in &plan.items {
        let word = match item.action {
            crate::package::PlanAction::Add => "add",
            crate::package::PlanAction::Replace => "replace",
            crate::package::PlanAction::SkipLocked => "skip (locked)",
        };
        lines.push(format!("{word} {}", escape_controls(&item.path)));
    }
    for (category, c) in &plan.summary {
        lines.push(format!(
            "{category}: {} added, {} replaced, {} deleted",
            c.added, c.replaced, c.deleted
        ));
    }
    for r in dto.results.iter().filter(|r| !r.ok) {
        lines.push(format!(
            "failed {}: {}",
            escape_controls(&r.name),
            escape_controls(r.error.as_deref().unwrap_or(""))
        ));
    }
    lines
}

/// The word of a `vfs` write item (§1.3.7): `mkdir`, `exists`, `copy`, `replace`, `link`, `rename`, `move`,
/// `remove`, `rmdir`, `trash`, `touch`, `create`, `skip`.
fn vfs_op_word(item: &VfsOpItemDto) -> &'static str {
    match item.action {
        "create_dir" => "mkdir",
        "keep_dir" => "exists",
        "copy" if item.replaces => "replace",
        "remove_dir" => "rmdir",
        other => other,
    }
}

/// The subject of an item line: `<source> -> <target>` for copies, links, renames and moves, else `<target>`.
fn vfs_op_subject(item: &VfsOpItemDto) -> String {
    let target = escape_controls(&item.target);
    match (&item.source, item.action) {
        (Some(source), "copy" | "link" | "rename" | "move") => {
            format!("{} -> {target}", escape_controls(source))
        }
        _ => target.into_owned(),
    }
}

/// `hikasa vfs cp|mv|rm|mkdir|touch` (§1.3.7, ARCH-101): one line per plan item in plan order, then the summary
/// (`<verb> <done> of <planned> items (<skipped> skipped, <failed> failed)`; a dry run ends with `would <base
/// verb> <planned> items` — `would copy 4 items` —, a cancelled run with `cancelled`).
pub fn vfs_op_lines(dto: &VfsOpDto) -> Vec<String> {
    let mut lines = Vec::with_capacity(dto.items.len() + 2);
    for item in &dto.items {
        let word = vfs_op_word(item);
        let target = escape_controls(&item.target);
        let line = match item.status {
            ItemStatus::Failed => {
                let message = item.error.as_ref().map_or("", |e| e.message.as_str());
                format!("failed {word} {target}: {}", escape_controls(message))
            }
            ItemStatus::NotRun => format!("not run {word} {target}"),
            _ => match item.reason {
                Some(reason) => format!("skip ({}) {target}", reason.as_str()),
                None => format!("{word} {}", vfs_op_subject(item)),
            },
        };
        lines.push(line);
    }
    let (verb, base) = match dto.op {
        "copy" => ("copied", "copy"),
        "move" => ("moved", "move"),
        "remove" => ("removed", "remove"),
        "mkdir" => ("created", "create"),
        _ => ("touched", "touch"),
    };
    let s = &dto.summary;
    if dto.dry_run {
        lines.push(format!("would {base} {} items", s.planned));
    } else {
        lines.push(format!(
            "{verb} {} of {} items ({} skipped, {} failed)",
            s.done, s.planned, s.skipped, s.failed
        ));
        if dto.cancelled {
            lines.push("cancelled".to_owned());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::dto::{LineStateDto, TokenDto, TokenLineDto};
    use crate::ipc::{ViewId, ViewKind};

    fn entry(name: &str, kind: &'static str) -> VfsEntryDto {
        VfsEntryDto {
            name: name.to_owned(),
            name_lossy: false,
            kind,
            size: 12,
            mtime: Some("2026-10-19T13:12:00Z".into()),
            ctime: None,
            attrs: None,
            mode: Some(0o644),
            permissions: Some("-rw-r--r--".into()),
            uid: Some(1),
            gid: Some(1),
            link_target: None,
            followed: false,
            crc32: None,
        }
    }

    #[test]
    fn ls_lines_escape_names() {
        assert_eq!(
            vfs_ls_line(&entry("a\u{1b}[31mb\\c", "file")),
            "-rw-r--r-- 12 2026-10-19T13:12:00Z a\\u{1b}[31mb\\\\c"
        );
        let mut dos = entry("x\u{202e}txt", "dir");
        dos.permissions = None;
        dos.attrs = Some("HR".into());
        dos.mtime = None;
        assert_eq!(vfs_ls_line(&dos), "dHR 12 - x\\u{202e}txt");
        let mut link = entry("l", "symlink");
        link.permissions = None;
        link.link_target = Some("t\nx".into());
        assert_eq!(vfs_ls_line(&link), "l- 12 2026-10-19T13:12:00Z l -> t\\nx");
    }

    #[test]
    fn token_and_list_lines() {
        let dto = FormatTokensDto {
            file: "/a".into(),
            format: hikasa_report::json::FormatJson {
                id: None,
                name: "Everything Else".into(),
                kind: "text",
                reason: None,
            },
            encoding: crate::api::dto::EncodingDto {
                name: "UTF-8",
                source: hikasa_formats::EncodingSource::Utf8Valid,
                bom: false,
                malformed: 0,
            },
            regex_errors: 0,
            lines: vec![TokenLineDto {
                line: 3,
                state_in: LineStateDto::new(hikasa_formats::LineState::Normal),
                tokens: vec![TokenDto {
                    start: 0,
                    end: 4,
                    element: "String".into(),
                    item: Some(1),
                    text: "\"a\tb\"".into(),
                }],
            }],
        };
        assert_eq!(
            token_lines(&dto),
            vec!["L3:0-4\tString\t\"a\\tb\"".to_owned()]
        );
        let view = ViewInfo {
            view: ViewId(7),
            kind: ViewKind::TextCompare,
            title: "a\nb".into(),
            specs: vec!["/x".into(), "/y\\z".into()],
        };
        assert_eq!(gui_list_line(&view), "7\ttext_compare\ta\\nb\t/x\t/y\\\\z");
    }
}

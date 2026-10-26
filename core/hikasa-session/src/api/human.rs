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

use hikasa_report::{escape_controls, escape_name};
use hikasa_vfs::EntryKind;

use super::dto::{FormatDetectDto, FormatListItemDto, FormatTokensDto, VfsEntryDto, VfsStatDto};
use crate::ipc::ViewInfo;

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

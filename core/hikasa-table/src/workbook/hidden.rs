//! Hidden rows and columns of xlsx and xlsm sheets (review F13). calamine does not report them, so each worksheet
//! part is read once more, beside calamine's pass: the `hidden` attributes of `<row>` and `<col>`, and
//! `<sheetFormatPr zeroHeight="1">` (rows hidden unless listed). xlsb, xls and ods report none.

use hikasa_core::CancelToken;

use super::package::{self, Member};

/// Rows a sheet may have (the complement of the visible rows of a `zeroHeight` sheet ends here).
const LAST_ROW: u32 = 1_048_575;

/// Hidden rows and columns of one sheet: sorted, merged, inclusive ranges in sheet coordinates (0-based).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Hidden {
    pub rows: Vec<(u32, u32)>,
    pub cols: Vec<(u32, u32)>,
}

/// Sorts and merges inclusive ranges.
pub(crate) fn merge(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The attributes of a start tag's content (`name a="1" b='2'`), local names (prefixes dropped), values with the
/// five predefined entities and character references decoded.
fn attributes(tag: &[u8]) -> Vec<(&[u8], String)> {
    let mut out = Vec::new();
    let mut i = tag
        .iter()
        .position(|b| b.is_ascii_whitespace())
        .unwrap_or(tag.len());
    while i < tag.len() {
        while i < tag.len() && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < tag.len() && tag[i] != b'=' && !tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = &tag[start..i];
        while i < tag.len() && (tag[i] == b'=' || tag[i].is_ascii_whitespace()) {
            i += 1;
        }
        let Some(&quote) = tag.get(i).filter(|q| **q == b'"' || **q == b'\'') else {
            break;
        };
        let vstart = i + 1;
        let Some(len) = tag[vstart..].iter().position(|&b| b == quote) else {
            break;
        };
        let value = &tag[vstart..vstart + len];
        i = vstart + len + 1;
        let local = name.rsplit(|&b| b == b':').next().unwrap_or(name);
        out.push((local, unescape(value)));
    }
    out
}

fn unescape(v: &[u8]) -> String {
    let s = String::from_utf8_lossy(v);
    if !s.contains('&') {
        return s.into_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = &*s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..end];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Calls `f(local name, whole tag content)` for every start tag of `xml` whose local name is in `names`;
/// returns the offset of an unfinished tag at the end (to carry into the next chunk).
fn tags(xml: &[u8], names: &[&[u8]], f: &mut dyn FnMut(&[u8], &[u8])) -> usize {
    let mut i = 0;
    while let Some(lt) = memchr::memchr(b'<', &xml[i..]) {
        let at = i + lt;
        let name_start = at + 1;
        let name_end = xml[name_start..]
            .iter()
            .position(|&b| b.is_ascii_whitespace() || b == b'>' || b == b'/')
            .map(|p| name_start + p);
        let Some(name_end) = name_end else {
            return at;
        };
        let name = &xml[name_start..name_end];
        let local = name.rsplit(|&b| b == b':').next().unwrap_or(name);
        if names.contains(&local) {
            match memchr::memchr(b'>', &xml[name_end..]) {
                Some(gt) => {
                    f(local, &xml[name_start..name_end + gt]);
                    i = name_end + gt + 1;
                }
                None => return at,
            }
        } else {
            i = name_end;
        }
    }
    xml.len()
}

fn truthy(v: &str) -> bool {
    v == "1" || v.eq_ignore_ascii_case("true")
}

/// The worksheet part of every sheet name, from `xl/workbook.xml` and its relationships.
fn sheet_parts(bytes: &[u8], members: &[Member]) -> Vec<(String, String)> {
    let read = |name: &str| {
        package::find(members, name).and_then(|m| package::read_member(bytes, m, 64 << 20))
    };
    let (Some(workbook), Some(rels)) =
        (read("xl/workbook.xml"), read("xl/_rels/workbook.xml.rels"))
    else {
        return Vec::new();
    };
    let mut targets: Vec<(String, String)> = Vec::new();
    tags(&rels, &[b"Relationship"], &mut |_, tag| {
        let a = attributes(tag);
        let get = |n: &[u8]| a.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone());
        if let (Some(id), Some(target)) = (get(b"Id"), get(b"Target")) {
            let path = match target.strip_prefix('/') {
                Some(abs) => abs.to_owned(),
                None => format!("xl/{target}"),
            };
            targets.push((id, path));
        }
    });
    let mut out = Vec::new();
    tags(&workbook, &[b"sheet"], &mut |_, tag| {
        let a = attributes(tag);
        let name = a
            .iter()
            .find(|(k, _)| *k == b"name")
            .map(|(_, v)| v.clone());
        let id = a.iter().find(|(k, _)| *k == b"id").map(|(_, v)| v.clone());
        if let (Some(name), Some(id)) = (name, id)
            && let Some((_, path)) = targets.iter().find(|(i, _)| *i == id)
        {
            out.push((name, path.clone()));
        }
    });
    out
}

/// The hidden rows and columns of one worksheet part.
fn scan_sheet(bytes: &[u8], m: &Member, limit: u64, cancel: &CancelToken) -> Option<Hidden> {
    let mut carry: Vec<u8> = Vec::new();
    let mut next_row: u32 = 0;
    let mut zero_height = false;
    let mut rows: Vec<(u32, u32)> = Vec::new(); // hidden rows, or the visible ones of a zeroHeight sheet
    let mut cols: Vec<(u32, u32)> = Vec::new();
    let mut cancelled = false;
    let mut chunks = 0u32;
    let mut on_tag = |name: &[u8], tag: &[u8]| {
        let a = attributes(tag);
        let get = |n: &[u8]| a.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str());
        match name {
            b"row" => {
                let r = get(b"r")
                    .and_then(|v| v.parse::<u32>().ok())
                    .map_or(next_row, |r| r.saturating_sub(1));
                next_row = r.saturating_add(1);
                let hidden = get(b"hidden").is_some_and(truthy);
                if hidden != zero_height {
                    rows.push((r, r));
                }
            }
            b"col" => {
                if get(b"hidden").is_some_and(truthy) {
                    let num = |n: &[u8]| get(n).and_then(|v| v.parse::<u32>().ok());
                    if let (Some(lo), Some(hi)) = (num(b"min"), num(b"max"))
                        && lo >= 1
                        && hi >= lo
                    {
                        cols.push((lo - 1, hi - 1));
                    }
                }
            }
            _ => zero_height = get(b"zeroHeight").is_some_and(truthy),
        }
    };
    let names: [&[u8]; 3] = [b"row", b"col", b"sheetFormatPr"];
    package::stream_member(bytes, m, limit, &mut |chunk| {
        chunks += 1;
        if chunks.is_multiple_of(16) && cancel.is_cancelled() {
            cancelled = true;
            return false;
        }
        carry.extend_from_slice(chunk);
        let done = tags(&carry, &names, &mut on_tag);
        carry.drain(..done);
        // A tag never runs this long: a damaged part.
        carry.len() < 1 << 20
    })?;
    if cancelled {
        return None;
    }
    let rows = if zero_height {
        // Every row not listed visible is hidden.
        let visible = merge(rows);
        let mut hidden = Vec::new();
        let mut from = 0u32;
        for (a, b) in visible {
            if a > from {
                hidden.push((from, a - 1));
            }
            from = b.saturating_add(1);
        }
        if from <= LAST_ROW {
            hidden.push((from, LAST_ROW));
        }
        hidden
    } else {
        merge(rows)
    };
    Some(Hidden {
        rows,
        cols: merge(cols),
    })
}

/// The hidden rows and columns of every sheet of an xlsx or xlsm package, by sheet name (sheets without any are
/// left out). Parts larger than `limit` are not read.
pub(crate) fn read(bytes: &[u8], limit: u64, cancel: &CancelToken) -> Vec<(String, Hidden)> {
    let Ok(members) = package::directory(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, path) in sheet_parts(bytes, &members) {
        if cancel.is_cancelled() {
            break;
        }
        let Some(m) = package::find(&members, &path) else {
            continue;
        };
        if let Some(h) = scan_sheet(bytes, m, limit, cancel)
            && (!h.rows.is_empty() || !h.cols.is_empty())
        {
            out.push((name, h));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_attributes() {
        let xml = br#"<x:row r="5" hidden="1" spans="1:3"><c r="A5"/></x:row><col min="2" max='4' hidden="true"/><row"#;
        let mut seen = Vec::new();
        let rest = tags(xml, &[b"row", b"col"], &mut |n, t| {
            seen.push((
                String::from_utf8_lossy(n).into_owned(),
                attributes(t)
                    .into_iter()
                    .map(|(k, v)| (String::from_utf8_lossy(k).into_owned(), v))
                    .collect::<Vec<_>>(),
            ));
        });
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].1[1], ("hidden".to_owned(), "1".to_owned()));
        assert_eq!(seen[1].1[1], ("max".to_owned(), "4".to_owned()));
        assert_eq!(&xml[rest..], b"<row");
        assert_eq!(
            unescape(b"a &amp; b &#x41;&#66; &bogus;"),
            "a & b AB &bogus;"
        );
        assert_eq!(
            merge(vec![(5, 6), (1, 2), (3, 3), (9, 9), (4, 4)]),
            vec![(1, 6), (9, 9)]
        );
        assert_eq!(
            merge(vec![(5, 6), (1, 2), (9, 9)]),
            vec![(1, 2), (5, 6), (9, 9)]
        );
    }
}

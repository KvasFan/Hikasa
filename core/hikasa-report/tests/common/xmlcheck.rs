//! The structure checks of the XML dumps (M2 W7, D-199) with `roxmltree`: the namespace, `version`, and for every
//! element the row of the table of m2-contracts §1.7.6 (parent, required attributes, no attribute it does not list,
//! enumerations); the text/folder specifics the schema cannot say; and the read-back helpers of the round trips of
//! A7.5 and A7.6.

use roxmltree::{Document, Node};

/// The namespace of version 1.
pub const NAMESPACE: &str = "urn:hikasa:report:1";

/// What the table of §1.7.6 says of one element.
struct Rule {
    parents: &'static [&'static str],
    required: &'static [&'static str],
    optional: &'static [&'static str],
}

fn rule(name: &str, parent: &str) -> Option<Rule> {
    let r = |parents, required, optional| {
        Some(Rule {
            parents,
            required,
            optional,
        })
    };
    match (name, parent) {
        ("hikasa-report", _) => r(&[""], &["version", "type"], &[]),
        ("header", _) => r(&["hikasa-report"], &[], &[]),
        ("title" | "rules", _) => r(&["header"], &[], &[]),
        ("generated", _) => r(&["header"], &["time", "by"], &[]),
        ("side", _) => r(
            &["header"],
            &["id", "spec"],
            &["encoding", "bom", "eol", "binary"],
        ),
        ("mode", _) => r(
            &["header"],
            &["display", "ignore-unimportant"],
            &["context", "folders", "suppress-filters", "scope"],
        ),
        ("error", _) => r(&["header"], &["side"], &[]),
        ("status", _) => r(&["header"], &["complete"], &[]),
        ("warning", _) => r(&["header"], &["code"], &[]),
        ("rows", _) => r(&["hikasa-report"], &[], &[]),
        ("tree", _) => r(&["hikasa-report"], &[], &["flat"]),
        ("row", _) => r(
            &["rows"],
            &["state"],
            &["importance", "left", "right", "left-piece", "right-piece"],
        ),
        ("l" | "r", _) => r(&["row"], &[], &[]),
        ("d" | "u", _) => r(&["l", "r"], &[], &[]),
        ("c", _) => r(
            &[
                "l", "r", "d", "u", "title", "rules", "error", "status", "warning",
            ],
            &["x"],
            &[],
        ),
        ("hidden", _) => r(&["rows"], &["rows"], &[]),
        ("folder", _) => r(
            &["tree", "folder"],
            &["name", "path", "state", "marker"],
            &["ignored"],
        ),
        ("entry", _) => r(
            &["tree", "folder"],
            &["name", "path", "state", "marker", "kind"],
            &["ignored"],
        ),
        ("left" | "right", _) => r(
            &["folder", "entry"],
            &["name"],
            &[
                "raw-name",
                "size",
                "modified",
                "attributes",
                "crc",
                "owner",
                "group",
                "link-target",
                "error",
            ],
        ),
        ("file-report", _) => r(&["entry"], &["status"], &["href", "reason"]),
        ("summary", _) => r(&["hikasa-report"], &["verdict"], &["complete"]),
        ("count", _) => r(&["summary"], &["group", "name", "value"], &[]),
        _ => None,
    }
}

/// The values an attribute may take (`None`: any).
fn enumeration(element: &str, attr: &str) -> Option<&'static [&'static str]> {
    const BOOL: &[&str] = &["true", "false"];
    Some(match (element, attr) {
        ("hikasa-report", "version") => &["1"],
        ("hikasa-report", "type") => &["text", "folder"],
        ("side" | "error", "id" | "side") => &["left", "right"],
        ("side", "bom" | "binary") | ("mode", "ignore-unimportant" | "suppress-filters") => BOOL,
        ("side", "eol") => &["lf", "cr_lf", "cr", "mixed", "none"],
        ("mode", "folders") => &["always", "structure", "files-only"],
        ("status", "complete") => &["false"],
        ("tree", "flat") | ("folder" | "entry", "ignored") => &["true"],
        ("row", "state") => &["same", "changed", "left-only", "right-only"],
        ("row", "importance") => &["important", "unimportant"],
        ("folder" | "entry", "state") => &[
            "same",
            "left-newer",
            "right-newer",
            "different",
            "unimportant",
            "left-only",
            "right-only",
            "unknown",
            "error",
            "filtered",
        ],
        ("folder" | "entry", "marker") => &["=", ">", "<", "!", "~", "-", "+", "?", "E", "."],
        ("entry", "kind") => &["file", "link", "other"],
        ("file-report", "status") => &["written", "skipped", "failed"],
        ("summary", "complete") => BOOL,
        ("summary", "verdict") => &[
            "binary_same",
            "rules_same",
            "unimportant_only",
            "important_differences",
            "binary_different",
            "error",
            "no_differences",
            "differences",
            "incomplete",
        ],
        _ => return None,
    })
}

/// Checks a whole document: parse, namespace, every element against its row, the type's specifics. Also that the
/// bytes start with the declaration, end with `\n`, hold no `\r` and no BOM.
pub fn check(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    if !text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n") {
        return Err("no declaration".into());
    }
    if !text.ends_with("</hikasa-report>\n") || text.contains('\r') {
        return Err("line ends".into());
    }
    raw_markup(text)?;
    let doc = Document::parse(text).map_err(|e| format!("not well-formed: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "hikasa-report" {
        return Err("root".into());
    }
    let kind = root.attribute("type").unwrap_or_default();
    for node in root.descendants().filter(Node::is_element) {
        let name = node.tag_name().name();
        if node.tag_name().namespace() != Some(NAMESPACE) {
            return Err(format!(
                "{name}: namespace {:?}",
                node.tag_name().namespace()
            ));
        }
        let parent = node.parent_element().map_or("", |p| p.tag_name().name());
        let rule = rule(name, parent).ok_or_else(|| format!("unknown element {name}"))?;
        if !rule.parents.contains(&parent) {
            return Err(format!("{name} below {parent}"));
        }
        for required in rule.required {
            if node.attribute(*required).is_none() {
                return Err(format!("{name} without {required}"));
            }
        }
        for attr in node.attributes() {
            if attr.namespace().is_some() {
                return Err(format!("{name}: namespaced attribute {}", attr.name()));
            }
            if !rule.required.contains(&attr.name()) && !rule.optional.contains(&attr.name()) {
                return Err(format!("{name}: unknown attribute {}", attr.name()));
            }
            if let Some(values) = enumeration(name, attr.name())
                && !values.contains(&attr.value())
            {
                return Err(format!("{name}/@{} = {:?}", attr.name(), attr.value()));
            }
        }
        // Text content only where the table puts it.
        let mixed = matches!(
            name,
            "title" | "rules" | "error" | "status" | "warning" | "l" | "r" | "d" | "u"
        );
        if !mixed {
            for child in node.children().filter(Node::is_text) {
                if !child.text().unwrap_or_default().trim().is_empty() {
                    return Err(format!("{name}: text {:?}", child.text()));
                }
            }
        }
        match name {
            "side" if kind == "text" => {
                for attr in ["encoding", "bom", "eol", "binary"] {
                    if node.attribute(attr).is_none() {
                        return Err(format!("text side without {attr}"));
                    }
                }
            }
            "mode" if kind == "folder" => {
                for attr in ["folders", "suppress-filters"] {
                    if node.attribute(attr).is_none() {
                        return Err(format!("folder mode without {attr}"));
                    }
                }
            }
            "rows" if kind != "text" => return Err("rows in a folder report".into()),
            "tree" if kind != "folder" => return Err("tree in a text report".into()),
            "row" => {
                let same = node.attribute("state") == Some("same");
                if same && node.attribute("importance").is_some() {
                    return Err("a same row with an importance".into());
                }
                let has = |n: &str| node.children().any(|c| c.has_tag_name((NAMESPACE, n)));
                if same && !(has("l") && has("r")) {
                    return Err("a same row without both lines".into());
                }
            }
            "c" => {
                let x = node.attribute("x").unwrap_or_default();
                if x.is_empty()
                    || !x
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                {
                    return Err(format!("c/@x {x:?}"));
                }
            }
            "folder" | "entry" => {
                if let Some(up) = node
                    .parent_element()
                    .filter(|p| p.has_tag_name((NAMESPACE, "folder")))
                {
                    let (path, above) = (
                        node.attribute("path").unwrap_or_default(),
                        up.attribute("path").unwrap_or_default(),
                    );
                    // A child is its folder's path plus one name.
                    let child = path
                        .strip_prefix(above)
                        .and_then(|rest| rest.strip_prefix('/'))
                        .is_some_and(|name| !name.is_empty() && !name.contains('/'));
                    if !child {
                        return Err(format!("{path} nested in {above}"));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// §1.7.6 escapes `&`, `<` and `>` always: in the raw bytes a `<` only opens markup and a `>` only closes it, and no
/// attribute value holds either (parsers accept a raw `>` in text, so this is checked on the bytes).
fn raw_markup(text: &str) -> Result<(), String> {
    let (mut tag, mut quoted) = (false, false);
    for (at, c) in text.char_indices() {
        let bad = match (tag, quoted, c) {
            (false, _, '<') => {
                tag = true;
                false
            }
            (false, _, '>') | (true, true, '<' | '>') | (true, false, '<') => true,
            (true, false, '>') => {
                tag = false;
                false
            }
            (true, _, '"') => {
                quoted = !quoted;
                false
            }
            _ => false,
        };
        if bad {
            let from = text[..at]
                .char_indices()
                .rev()
                .nth(30)
                .map_or(0, |(i, _)| i);
            return Err(format!(
                "a raw {c:?} at byte {at}: {:?}",
                &text[from..at + 1]
            ));
        }
    }
    Ok(())
}

/// The text of a line element: its text, `d` and `u` contents, `c` as their characters.
pub fn line_text(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    for child in node.children() {
        if child.is_text() {
            out.push_str(child.text().unwrap_or_default());
        } else if child.has_tag_name((NAMESPACE, "c")) {
            let x = u32::from_str_radix(child.attribute("x").unwrap_or("0"), 16).unwrap_or(0);
            out.push(char::from_u32(x).unwrap_or('\u{fffd}'));
        } else {
            out.push_str(&line_text(child));
        }
    }
    out
}

/// The lines of one side read back from a text dump: the `l` (or `r`) of each row in order, a row whose
/// `left-piece` (or `right-piece`) is 2, 3, … joined to the line before, which must carry the same number.
pub fn side_lines(doc: &Document<'_>, left: bool) -> Vec<String> {
    let (attr, piece, element) = if left {
        ("left", "left-piece", "l")
    } else {
        ("right", "right-piece", "r")
    };
    let mut out: Vec<(String, u32, String)> = Vec::new();
    for row in doc
        .descendants()
        .filter(|n| n.has_tag_name((NAMESPACE, "row")))
    {
        let Some(number) = row.attribute(attr) else {
            continue;
        };
        let Some(line) = row
            .children()
            .find(|c| c.has_tag_name((NAMESPACE, element)))
        else {
            continue;
        };
        let text = line_text(line);
        match row.attribute(piece).map(|p| p.parse::<u32>().unwrap_or(0)) {
            Some(p) => {
                let (n, last, t) = out
                    .last_mut()
                    .unwrap_or_else(|| panic!("{piece}={p} without a line before"));
                assert!(
                    n == number && p == *last + 1,
                    "{piece}={p} of line {number} after piece {last} of line {n}"
                );
                *last = p;
                t.push_str(&text);
            }
            None => out.push((number.to_owned(), 1, text)),
        }
    }
    out.into_iter().map(|(_, _, t)| t).collect()
}

/// The `(group, name, value)` of the summary's counts.
pub fn counts(doc: &Document<'_>) -> Vec<(String, String, u64)> {
    doc.descendants()
        .filter(|n| n.has_tag_name((NAMESPACE, "count")))
        .map(|n| {
            (
                n.attribute("group").unwrap_or_default().to_owned(),
                n.attribute("name").unwrap_or_default().to_owned(),
                n.attribute("value")
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(u64::MAX),
            )
        })
        .collect()
}

/// Decodes an attribute value as the dumps escape it: `\\` a backslash, `\n`, `\t`, `\r` (names, `escape_name`),
/// `\u{…}` a character; any other backslash is an error, so an undoubled backslash is caught.
pub fn unescape(value: &str) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('u') => {
                if chars.next() != Some('{') {
                    return Err(format!("\\u without {{ in {value:?}"));
                }
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let code = u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("\\u{{{hex}}} in {value:?}"))?;
                out.push(code);
            }
            other => return Err(format!("a lone backslash before {other:?} in {value:?}")),
        }
    }
    Ok(out)
}

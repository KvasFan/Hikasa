//! Architecture test (docs/design/m1-contracts.md §6.1). It reads the workspace sources through
//! `CARGO_MANIFEST_DIR/../..` and checks:
//!
//! 1. the `unsafe` keyword (`\bunsafe\s*(\{|fn\b|impl\b|trait\b|extern\b)`, §1.2.6) appears only in
//!    `core/hikasa-vfs/src/mmap.rs`, every occurrence preceded by a `// SAFETY:` comment. The one exemption is
//!    the allocation counting of §6.1: a file in a crate's own `tests/` or `benches/` directory
//!    (`<top>/<crate>/tests/…`, `<top>/<crate>/benches/…`; never library code) whose code installs a
//!    `#[global_allocator]` may use `unsafe`, because `GlobalAlloc` cannot be implemented without it; such a file
//!    must say `#![allow(unsafe_code)]`, and each occurrence still needs its `// SAFETY:` comment;
//! 2. no banned tokens in `core/*/src` (T1, T8) except at the three allow-points, `tracing::` only in
//!    hikasa-session, and `capture_env`/`SystemClock` referenced nowhere under `core/` except their definition
//!    (and hikasa-session's own tests). Imports are expanded first, so grouped and aliased forms are caught as
//!    well (`use std::{env, thread::{self as th}}`, `use std::time::Instant as Clock; Clock::now()`), and
//!    `.elapsed()` counts as a clock read;
//! 3. every dependency of every workspace member is `workspace = true` and listed in `[workspace.dependencies]`,
//!    each member's `[dependencies]` and `[dev-dependencies]` equal the §0.5 table, there are no other dependency
//!    sections, and the dependency direction holds: no `core/*` crate depends on cli/gui, and the engine crates
//!    do not depend on hikasa-session (ARCH-010/011/012);
//! 4. the stub crates export no `pub` items (§1.11);
//! 5. `[profile.release]` does not set `panic = "abort"` (§5.6);
//! 6. no `HashMap`/`HashSet` type appears in a DTO or persisted config struct: the files under `json/` or
//!    `model/` folders and the files named `dto.rs`, `protocol.rs`, `rules.rs` and `model.rs` in `core/*/src`;
//! 7. `cli/` and `gui/` sources contain no `impl FromStr` and no matching or comparing against string literals of
//!    the core vocabularies (`--display`, `--folders`, `--criteria`, `--unimportant`, the legacy `gui open`
//!    switches): those parsers live in core (T11).
//!
//! The checks look at code only: comments, string literals and char literals are blanked first, so prose that
//! mentions a banned token never fails the build, and the patterns below can be written as plain strings
//! (check 7 alone reads string literals, from comment-free text).

use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------------------
// Workspace walking
// ---------------------------------------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    root.canonicalize().unwrap_or(root)
}

/// Every `.rs` file below `dir`, skipping `target` and hidden directories, in sorted order.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if kind.is_file() && name.ends_with(".rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// `path` relative to the workspace root, with `/` separators.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The crate directories under `core/`.
fn core_crates(root: &Path) -> Vec<(String, PathBuf)> {
    let mut crates: Vec<(String, PathBuf)> = fs::read_dir(root.join("core"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("Cargo.toml").is_file())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .collect()
        })
        .unwrap_or_default();
    crates.sort();
    crates
}

// ---------------------------------------------------------------------------------------------------------
// A small Rust lexer: blank comments, strings and char literals (newlines are kept, so lines still match)
// ---------------------------------------------------------------------------------------------------------

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn blank(out: &mut String, c: char) {
    out.push(if c == '\n' { '\n' } else { ' ' });
}

/// `src` with every comment, string literal (plain, byte, C, raw) and char/byte literal replaced by spaces.
fn code_only(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let prev_ident = i > 0 && is_ident(chars[i - 1]);
        // Line comment.
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                blank(&mut out, chars[i]);
                i += 1;
            }
            continue;
        }
        // Block comment (nested).
        if c == '/' && next == Some('*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    blank(&mut out, '/');
                    blank(&mut out, '*');
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    blank(&mut out, '*');
                    blank(&mut out, '/');
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, chars[i]);
                    i += 1;
                }
            }
            continue;
        }
        // Raw strings: r"…", r#"…"#, br"…", cr#"…"#.
        if !prev_ident {
            let prefix_len = match (c, next) {
                ('r', _) => Some(1),
                ('b' | 'c', Some('r')) => Some(2),
                _ => None,
            };
            if let Some(len) = prefix_len {
                let mut j = i + len;
                let mut hashes = 0;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) == Some(&'"') {
                    let mut k = j + 1;
                    loop {
                        if k >= chars.len() {
                            break;
                        }
                        if chars[k] == '"'
                            && (0..hashes).all(|h| chars.get(k + 1 + h) == Some(&'#'))
                        {
                            k += 1 + hashes;
                            break;
                        }
                        k += 1;
                    }
                    for &ch in &chars[i..k.min(chars.len())] {
                        blank(&mut out, ch);
                    }
                    i = k;
                    continue;
                }
            }
        }
        // Plain, byte and C strings.
        let string_start = c == '"' || (!prev_ident && matches!(c, 'b' | 'c') && next == Some('"'));
        if string_start {
            let mut k = if c == '"' { i + 1 } else { i + 2 };
            while k < chars.len() && chars[k] != '"' {
                k += if chars[k] == '\\' { 2 } else { 1 };
            }
            let end = (k + 1).min(chars.len());
            for &ch in &chars[i..end] {
                blank(&mut out, ch);
            }
            i = end;
            continue;
        }
        // Char and byte literals ('a', '\n', '\u{1F600}', b'x'); lifetimes and labels stay.
        let quote = if c == '\'' {
            Some(i)
        } else if c == 'b' && !prev_ident && next == Some('\'') {
            Some(i + 1)
        } else {
            None
        };
        if let Some(q) = quote {
            let end = match chars.get(q + 1) {
                // Skip the escaped char itself (it may be a quote), then find the closing quote.
                Some('\\') => (q + 3..chars.len())
                    .find(|&k| chars[k] == '\'')
                    .map(|k| k + 1),
                Some(_) if chars.get(q + 2) == Some(&'\'') => Some(q + 3),
                _ => None,
            };
            if let Some(end) = end {
                for &ch in &chars[i..end] {
                    blank(&mut out, ch);
                }
                i = end;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Byte offsets of `token` in `code` with identifier boundaries on both ends (the token's own first and last
/// chars decide whether a boundary is required on that side).
fn find_token(code: &str, token: &str) -> Vec<usize> {
    let first_ident = token.chars().next().is_some_and(is_ident);
    let last_ident = token.chars().last().is_some_and(is_ident);
    let mut hits = Vec::new();
    let mut from = 0;
    while let Some(pos) = code[from..].find(token) {
        let at = from + pos;
        let before_ok = !first_ident || !code[..at].chars().next_back().is_some_and(is_ident);
        let after_ok = !last_ident
            || !code[at + token.len()..]
                .chars()
                .next()
                .is_some_and(is_ident);
        if before_ok && after_ok {
            hits.push(at);
        }
        from = at + token.len().max(1);
    }
    hits
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// Offsets of `unsafe` followed by `{`, `fn`, `impl`, `trait` or `extern` (the regex of §1.2.6).
fn unsafe_uses(code: &str) -> Vec<usize> {
    find_token(code, "unsafe")
        .into_iter()
        .filter(|&at| {
            let rest = code[at + "unsafe".len()..].trim_start();
            rest.starts_with('{')
                || ["fn", "impl", "trait", "extern"].iter().any(|kw| {
                    rest.strip_prefix(kw)
                        .is_some_and(|after| !after.chars().next().is_some_and(is_ident))
                })
        })
        .collect()
}

/// True when the `unsafe` at `line` (1-based) is preceded by a `// SAFETY:` comment: on the same line before it,
/// or in the run of comment and attribute lines directly above.
fn has_safety_comment(raw_lines: &[&str], line: usize, column_text: &str) -> bool {
    if column_text.contains("SAFETY:") {
        return true;
    }
    let mut k = line - 1;
    while k > 0 {
        let above = raw_lines[k - 1].trim_start();
        if above.starts_with("//") {
            if above.contains("SAFETY:") {
                return true;
            }
        } else if !above.starts_with("#[") {
            return false;
        }
        k -= 1;
    }
    false
}

// ---------------------------------------------------------------------------------------------------------
// Check 1: `unsafe`
// ---------------------------------------------------------------------------------------------------------

const MMAP_FILE: &str = "core/hikasa-vfs/src/mmap.rs";

/// `code` without whitespace (attribute spellings such as `# [ global_allocator ]` compare equal).
fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// True when `rel` lies in a crate's own `tests/` or `benches/` directory (`<top>/<crate>/tests/…` or
/// `<top>/<crate>/benches/…`): an integration test or bench binary, or a module of one. Library code never
/// qualifies, not even a `src/tests/` module.
fn in_test_or_bench_dir(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() >= 4
        && matches!(parts[0], "core" | "cli" | "gui")
        && matches!(parts[2], "tests" | "benches")
}

/// True when the (comment- and literal-free) `code` installs a `#[global_allocator]`.
fn installs_global_allocator(code: &str) -> bool {
    compact(code).contains("#[global_allocator]")
}

/// True when the (comment- and literal-free) `code` carries the crate-level `#![allow(unsafe_code)]` (alone or
/// in a list).
fn allows_unsafe_code(code: &str) -> bool {
    let compact = compact(code);
    let mut rest = compact.as_str();
    while let Some(at) = rest.find("#![allow(") {
        let after = &rest[at + "#![allow(".len()..];
        let list = after.split(")]").next().unwrap_or("");
        if list.split(',').any(|lint| lint == "unsafe_code") {
            return true;
        }
        rest = after;
    }
    false
}

/// Check 1 for one file (`rel` relative to the workspace root, `raw` its text). `unsafe` is allowed in
/// [`MMAP_FILE`], and in a file of a crate's `tests/` or `benches/` directory that installs a counting
/// `#[global_allocator]` (§6.1 "Allocation counting": `GlobalAlloc` cannot be implemented without `unsafe`);
/// such a file must opt out of the workspace lint with `#![allow(unsafe_code)]`. Every allowed occurrence needs
/// a preceding `// SAFETY:` comment.
fn unsafe_violations(rel: &str, raw: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let code = code_only(raw);
    let uses = unsafe_uses(&code);
    if uses.is_empty() {
        return violations;
    }
    let allocator_binary = in_test_or_bench_dir(rel) && installs_global_allocator(&code);
    if rel != MMAP_FILE && !allocator_binary {
        for at in uses {
            violations.push(format!(
                "{rel}:{}: `unsafe` outside {MMAP_FILE} and outside the test and bench binaries that install a \
                 counting `#[global_allocator]`",
                line_of(&code, at)
            ));
        }
        return violations;
    }
    if allocator_binary && !allows_unsafe_code(&code) {
        violations.push(format!(
            "{rel}: a test or bench binary with `unsafe` must say `#![allow(unsafe_code)]`"
        ));
    }
    let raw_lines: Vec<&str> = raw.lines().collect();
    for at in uses {
        let line = line_of(&code, at);
        let line_start = code[..at].rfind('\n').map_or(0, |p| p + 1);
        let before_on_line = raw.get(line_start..at).unwrap_or("");
        if !has_safety_comment(&raw_lines, line, before_on_line) {
            violations.push(format!(
                "{rel}:{line}: `unsafe` without a preceding `// SAFETY:` comment"
            ));
        }
    }
    violations
}

fn check_unsafe(root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    for top in ["core", "cli", "gui"] {
        for file in rust_files(&root.join(top)) {
            let rel = relative(root, &file);
            let Ok(raw) = fs::read_to_string(&file) else {
                continue;
            };
            violations.extend(unsafe_violations(&rel, &raw));
        }
    }
    violations
}

// ---------------------------------------------------------------------------------------------------------
// Check 2: banned tokens (T1 purity, T8 no printing)
// ---------------------------------------------------------------------------------------------------------

/// Tokens banned in `core/*/src`. The contract's list, plus `eprint!` (printing, T8), the clock readers
/// `Instant::elapsed`/`SystemTime::elapsed` (`.elapsed()` is checked separately), and the clock, zone and
/// environment readers of the approved crates (`jiff`, `directories`), which T1 bans just as much.
const BANNED: &[&str] = &[
    "std::env",
    "SystemTime::now",
    "Instant::now",
    "SystemTime::elapsed",
    "Instant::elapsed",
    "thread::sleep",
    "thread_local!",
    "println!",
    "eprintln!",
    "print!",
    "eprint!",
    "dbg!",
    "process::exit",
    "Timestamp::now",
    "Zoned::now",
    "TimeZone::system",
    "TimeZone::try_system",
    "TimeZone::get",
    "tz::db",
    "directories::",
];

/// The pseudo-token of a `.elapsed()` method call (it reads the clock: `Instant::elapsed`, `SystemTime::elapsed`).
const ELAPSED_CALL: &str = ".elapsed()";

/// The three allow-points of §6.1 (each file carries a comment naming the rule).
const ALLOW_POINTS: &[(&str, &[&str])] = &[
    (
        "core/hikasa-core/src/progress.rs",
        &["Instant::now", "Instant::elapsed", ELAPSED_CALL],
    ),
    (
        "core/hikasa-formats/src/convert.rs",
        &[
            "Instant::now",
            "Instant::elapsed",
            ELAPSED_CALL,
            "thread::sleep",
        ],
    ),
    (
        "core/hikasa-session/src/host.rs",
        &[
            "std::env",
            "SystemTime::now",
            "Instant::now",
            "SystemTime::elapsed",
            "Instant::elapsed",
            ELAPSED_CALL,
            "Timestamp::now",
            "Zoned::now",
            "TimeZone::system",
            "TimeZone::try_system",
            "TimeZone::get",
            "tz::db",
            "directories::",
        ],
    ),
];

// ---- `use` declarations ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum UseToken {
    Ident(String),
    PathSep,
    Open,
    Close,
    Comma,
    Star,
    Other,
}

fn use_tokens(text: &str) -> Vec<UseToken> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ':' && chars.get(i + 1) == Some(&':') {
            tokens.push(UseToken::PathSep);
            i += 2;
        } else if is_ident(c) {
            let start = i;
            while i < chars.len() && is_ident(chars[i]) {
                i += 1;
            }
            let mut word: String = chars[start..i].iter().collect();
            if word == "r" && chars.get(i) == Some(&'#') {
                let raw_start = i + 1;
                i = raw_start;
                while i < chars.len() && is_ident(chars[i]) {
                    i += 1;
                }
                word = chars[raw_start..i].iter().collect();
            }
            tokens.push(UseToken::Ident(word));
        } else {
            tokens.push(match c {
                '{' => UseToken::Open,
                '}' => UseToken::Close,
                ',' => UseToken::Comma,
                '*' => UseToken::Star,
                _ => UseToken::Other,
            });
            i += 1;
        }
    }
    tokens
}

/// One imported path and the name it is visible under (`None` for a glob).
type Import = (Vec<String>, Option<String>);

/// Expands the use tree starting at `tokens[*pos]` below `prefix` (`a::{b, c::{self as d}, e::*}`).
fn expand_use_tree(tokens: &[UseToken], pos: &mut usize, prefix: &[String], out: &mut Vec<Import>) {
    let mut path = prefix.to_vec();
    loop {
        match tokens.get(*pos) {
            Some(UseToken::PathSep) => *pos += 1,
            Some(UseToken::Ident(word)) if word != "as" => {
                path.push(word.clone());
                *pos += 1;
                if tokens.get(*pos) != Some(&UseToken::PathSep) {
                    break;
                }
            }
            Some(UseToken::Open) => {
                *pos += 1;
                loop {
                    match tokens.get(*pos) {
                        None => return,
                        Some(UseToken::Close) => {
                            *pos += 1;
                            return;
                        }
                        Some(UseToken::Comma) => *pos += 1,
                        Some(_) => {
                            let before = *pos;
                            expand_use_tree(tokens, pos, &path, out);
                            if *pos == before {
                                *pos += 1;
                            }
                        }
                    }
                }
            }
            Some(UseToken::Star) => {
                *pos += 1;
                path.push("*".to_owned());
                out.push((path, None));
                return;
            }
            _ => break,
        }
    }
    if path.last().is_some_and(|last| last == "self") {
        path.pop();
    }
    let mut local = path.last().cloned();
    if matches!(tokens.get(*pos), Some(UseToken::Ident(word)) if word == "as") {
        *pos += 1;
        if let Some(UseToken::Ident(alias)) = tokens.get(*pos) {
            local = Some(alias.clone());
            *pos += 1;
        }
    }
    if !path.is_empty() {
        out.push((path, local));
    }
}

/// Every import of `code` (comment- and literal-free) with the offset of its `use` keyword.
fn imports(code: &str) -> Vec<(usize, Import)> {
    let mut out = Vec::new();
    for at in find_token(code, "use") {
        let rest = &code[at + "use".len()..];
        if rest.trim_start().starts_with('<') {
            continue; // `impl Trait + use<'a>` (precise capturing), not an import
        }
        let Some(end) = rest.find(';') else {
            continue;
        };
        let tokens = use_tokens(&rest[..end]);
        let mut expanded = Vec::new();
        let mut pos = 0;
        expand_use_tree(&tokens, &mut pos, &[], &mut expanded);
        out.extend(expanded.into_iter().map(|import| (at, import)));
    }
    out
}

/// The banned token an import brings in, if any (any alias: the import itself is the violation).
fn banned_import(path: &[String]) -> Option<&'static str> {
    let path: Vec<&str> = path.iter().map(String::as_str).collect();
    match path.as_slice() {
        ["std", "env", ..] | ["std", "*"] => Some("std::env"),
        ["std", "thread", "sleep" | "*"] => Some("thread::sleep"),
        ["std", "process", "exit" | "*"] => Some("process::exit"),
        ["std", "thread_local"] => Some("thread_local!"),
        ["std", "println"] => Some("println!"),
        ["std", "eprintln"] => Some("eprintln!"),
        ["std", "print"] => Some("print!"),
        ["std", "eprint"] => Some("eprint!"),
        ["std", "dbg"] => Some("dbg!"),
        _ => None,
    }
}

/// `code` with every `name::` of an imported module or type spelled out as its full path (`Clock::now` becomes
/// `std::time::Instant::now`), so aliased uses meet the banned tokens. Newlines are kept.
fn resolve_aliases(code: &str, imports: &[(usize, Import)]) -> String {
    let mut resolved = code.to_owned();
    for (_, (path, local)) in imports {
        let Some(local) = local else { continue };
        if local == "_" || path.len() < 2 {
            continue;
        }
        let full = path.join("::");
        let mut out = String::with_capacity(resolved.len());
        let mut last = 0;
        for at in find_token(&resolved, local) {
            let after = &resolved[at + local.len()..];
            let qualified = resolved[..at].ends_with(':');
            if after.starts_with("::") && !qualified {
                out.push_str(&resolved[last..at]);
                out.push_str(&full);
                last = at + local.len();
            }
        }
        out.push_str(&resolved[last..]);
        resolved = out;
    }
    resolved
}

/// Offsets of `.elapsed()` method calls (whitespace allowed around the dot and before the parenthesis).
fn elapsed_calls(code: &str) -> Vec<usize> {
    find_token(code, "elapsed")
        .into_iter()
        .filter(|&at| {
            code[..at].trim_end().ends_with('.')
                && code[at + "elapsed".len()..].trim_start().starts_with('(')
        })
        .collect()
}

/// `(line, token)` of every banned token in a file's code: direct, through imports and through aliases.
fn banned_hits(code: &str) -> std::collections::BTreeSet<(usize, &'static str)> {
    let mut hits = std::collections::BTreeSet::new();
    let file_imports = imports(code);
    let resolved = resolve_aliases(code, &file_imports);
    for token in BANNED {
        for at in find_token(code, token) {
            hits.insert((line_of(code, at), *token));
        }
        for at in find_token(&resolved, token) {
            hits.insert((line_of(&resolved, at), *token));
        }
    }
    for (at, (path, _)) in &file_imports {
        if let Some(token) = banned_import(path) {
            hits.insert((line_of(code, *at), token));
        }
    }
    for at in elapsed_calls(code) {
        hits.insert((line_of(code, at), ELAPSED_CALL));
    }
    hits
}

fn allowed(rel: &str, token: &str) -> bool {
    ALLOW_POINTS
        .iter()
        .any(|(file, tokens)| *file == rel && tokens.contains(&token))
}

/// `static mut` (two words, any whitespace between them).
fn static_mut_uses(code: &str) -> Vec<usize> {
    find_token(code, "static")
        .into_iter()
        .filter(|&at| {
            let rest = &code[at + "static".len()..];
            rest.starts_with(char::is_whitespace)
                && rest
                    .trim_start()
                    .strip_prefix("mut")
                    .is_some_and(|after| !after.chars().next().is_some_and(is_ident))
        })
        .collect()
}

/// True for lines that only declare imports or re-exports (`use …;`, `pub use …;`).
fn is_use_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("use ") || t.starts_with("pub use ") || t.starts_with("pub(crate) use ")
}

fn check_banned_tokens(root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, dir) in core_crates(root) {
        for file in rust_files(&dir.join("src")) {
            let rel = relative(root, &file);
            let Ok(raw) = fs::read_to_string(&file) else {
                continue;
            };
            let code = code_only(&raw);
            for (line, token) in banned_hits(&code) {
                if !allowed(&rel, token) {
                    violations.push(format!("{rel}:{line}: banned token `{token}`"));
                }
            }
            for at in static_mut_uses(&code) {
                violations.push(format!(
                    "{rel}:{}: banned token `static mut`",
                    line_of(&code, at)
                ));
            }
            if name != "hikasa-session" {
                for at in find_token(&code, "tracing::")
                    .into_iter()
                    .chain(find_token(&code, "tracing"))
                {
                    let line = line_of(&code, at);
                    let text = code.lines().nth(line - 1).unwrap_or("");
                    if text.contains("tracing::") || is_use_line(text) {
                        violations.push(format!("{rel}:{line}: `tracing` outside hikasa-session"));
                    }
                }
            }
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

/// `capture_env` and `SystemClock` are adapter-only (§1.8.2): nothing under `core/` references them except their
/// definition in hikasa-session's `host.rs`, re-export lines of hikasa-session, and hikasa-session's own tests.
fn check_adapter_only(root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    for file in rust_files(&root.join("core")) {
        let rel = relative(root, &file);
        if rel == "core/hikasa-session/src/host.rs" || rel.starts_with("core/hikasa-session/tests/")
        {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&file) else {
            continue;
        };
        let code = code_only(&raw);
        let lines: Vec<&str> = code.lines().collect();
        for token in ["capture_env", "SystemClock"] {
            for at in find_token(&code, token) {
                let line = line_of(&code, at);
                let reexport =
                    rel.starts_with("core/hikasa-session/src/") && is_use_line(lines[line - 1]);
                if !reexport {
                    violations.push(format!(
                        "{rel}:{line}: `{token}` is adapter-only (cli/ and gui/)"
                    ));
                }
            }
        }
    }
    violations
}

// ---------------------------------------------------------------------------------------------------------
// Check 4: stub crates stay empty; check 5: release profile keeps unwinding
// ---------------------------------------------------------------------------------------------------------

const STUB_CRATES: [&str; 5] = [
    "hikasa-table",
    "hikasa-hex",
    "hikasa-picture",
    "hikasa-metadata",
    "hikasa-script",
];

fn check_stub_crates(root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    for name in STUB_CRATES {
        for file in rust_files(&root.join("core").join(name).join("src")) {
            let rel = relative(root, &file);
            let Ok(raw) = fs::read_to_string(&file) else {
                continue;
            };
            let code = code_only(&raw);
            for at in find_token(&code, "pub") {
                violations.push(format!(
                    "{rel}:{}: stub crates export no `pub` items (§1.11)",
                    line_of(&code, at)
                ));
            }
        }
    }
    violations
}

/// Violations of "the release profile must keep `panic = "unwind"`" in a Cargo manifest's text.
fn panic_abort_violations(manifest: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let mut section = String::new();
    for (k, raw_line) in manifest.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            section = line
                .trim_matches(|c| c == '[' || c == ']')
                .trim()
                .to_owned();
            continue;
        }
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        let in_profile = section == "profile" || section.starts_with("profile.");
        let sets_abort = compact.contains("panic=\"abort\"") || compact.contains("panic='abort'");
        if sets_abort && (in_profile || compact.starts_with("profile.")) {
            violations.push(format!(
                "Cargo.toml:{}: [{section}] sets panic = \"abort\" (§5.6)",
                k + 1
            ));
        }
    }
    violations
}

// ---------------------------------------------------------------------------------------------------------
// Check 3: manifests match §0.5; dependency direction
// ---------------------------------------------------------------------------------------------------------

/// The §0.5 table: `(crate, [dependencies], [dev-dependencies])` of every workspace member.
const DEPENDENCY_TABLE: &[(&str, &[&str], &[&str])] = &[
    (
        "hikasa-core",
        &[
            "serde",
            "thiserror",
            "bitflags",
            "jiff",
            "unicode-normalization",
        ],
        &[
            "proptest",
            "pretty_assertions",
            "regex",
            "serde_json",
            "criterion",
        ],
    ),
    (
        "hikasa-vfs",
        &[
            "hikasa-core",
            "serde",
            "thiserror",
            "crc32fast",
            "memmap2",
            "filetime",
            "bitflags",
            "same-file",
        ],
        &["tempfile", "proptest", "pretty_assertions", "criterion"],
    ),
    (
        "hikasa-diff",
        &[
            "hikasa-core",
            "imara-diff",
            "serde",
            "thiserror",
            "smallvec",
        ],
        &[
            "proptest",
            "pretty_assertions",
            "criterion",
            "similar",
            "serde_json",
        ],
    ),
    (
        "hikasa-formats",
        &[
            "hikasa-core",
            "serde",
            "toml",
            "regex",
            "fancy-regex",
            "encoding_rs",
            "chardetng",
            "thiserror",
            "memchr",
            "rayon",
        ],
        &[
            "proptest",
            "pretty_assertions",
            "tempfile",
            "criterion",
            "serde_json",
        ],
    ),
    (
        "hikasa-text",
        &[
            "hikasa-core",
            "hikasa-diff",
            "hikasa-formats",
            "hikasa-vfs",
            "ropey",
            "serde",
            "thiserror",
            "unicode-segmentation",
            "memchr",
            "rayon",
            "toml",
            "bitflags",
            "encoding_rs",
            "regex",
            "unicode-width",
        ],
        &[
            "proptest",
            "pretty_assertions",
            "tempfile",
            "criterion",
            "serde_json",
        ],
    ),
    (
        "hikasa-folder",
        &[
            "hikasa-core",
            "hikasa-vfs",
            "rayon",
            "serde",
            "thiserror",
            "unicode-normalization",
            "crossbeam-channel",
            "parking_lot",
            "jiff",
            "bitflags",
            "toml",
        ],
        &[
            "tempfile",
            "filetime",
            "proptest",
            "pretty_assertions",
            "criterion",
            "serde_json",
        ],
    ),
    (
        "hikasa-report",
        &[
            "hikasa-core",
            "hikasa-diff",
            "hikasa-text",
            "hikasa-folder",
            "serde",
            "serde_json",
            "jiff",
            "thiserror",
            "unicode-width",
            "bitflags",
        ],
        &[
            "pretty_assertions",
            "tempfile",
            "diffy",
            "proptest",
            "hikasa-vfs",
        ],
    ),
    (
        "hikasa-session",
        &[
            "hikasa-core",
            "hikasa-vfs",
            "hikasa-diff",
            "hikasa-formats",
            "hikasa-text",
            "hikasa-folder",
            "hikasa-report",
            "serde",
            "serde_json",
            "toml",
            "directories",
            "interprocess",
            "thiserror",
            "tracing",
            "rayon",
            "jiff",
            "parking_lot",
            "crossbeam-channel",
        ],
        &["tempfile", "pretty_assertions", "proptest"],
    ),
    (
        "hikasa-cli",
        &[
            "hikasa-core",
            "hikasa-vfs",
            "hikasa-diff",
            "hikasa-formats",
            "hikasa-text",
            "hikasa-folder",
            "hikasa-report",
            "hikasa-session",
            "clap",
            "clap_complete",
            "anyhow",
            "serde",
            "serde_json",
            "tracing",
            "tracing-subscriber",
            "jiff",
        ],
        &[
            "assert_cmd",
            "predicates",
            "tempfile",
            "pretty_assertions",
            "filetime",
        ],
    ),
    // Wave 6: the approved GUI toolkit (D-56) plus the core crates it renders (§1.10).
    (
        "hikasa-gui",
        &[
            "hikasa-core",
            "hikasa-vfs",
            "hikasa-formats",
            "hikasa-text",
            "hikasa-folder",
            "hikasa-report",
            "hikasa-session",
            "dioxus",
            "anyhow",
            "serde",
            "serde_json",
            "tracing",
            "tracing-subscriber",
            "parking_lot",
            "crossbeam-channel",
        ],
        &["tempfile", "pretty_assertions"],
    ),
    ("hikasa-table", &[], &[]),
    ("hikasa-hex", &[], &[]),
    ("hikasa-picture", &[], &[]),
    ("hikasa-metadata", &[], &[]),
    ("hikasa-script", &[], &[]),
];

/// The engine crates, which must never depend on the session layer or an adapter (ARCH-010/011/012).
const ENGINE_CRATES: &[&str] = &[
    "hikasa-core",
    "hikasa-vfs",
    "hikasa-diff",
    "hikasa-formats",
    "hikasa-text",
    "hikasa-folder",
    "hikasa-report",
    "hikasa-table",
    "hikasa-hex",
    "hikasa-picture",
    "hikasa-metadata",
    "hikasa-script",
];

/// Dependency sections of a manifest (top-level and `[target.<cfg>.*]`), as `(section name, table)`.
fn dependency_sections(manifest: &toml::Table) -> Vec<(String, &toml::Table)> {
    let mut out = Vec::new();
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(toml::Value::Table(t)) = manifest.get(kind) {
            out.push((kind.to_owned(), t));
        }
    }
    if let Some(toml::Value::Table(targets)) = manifest.get("target") {
        for (cfg, value) in targets {
            if let toml::Value::Table(target) = value {
                for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(toml::Value::Table(t)) = target.get(kind) {
                        out.push((format!("target.{cfg}.{kind}"), t));
                    }
                }
            }
        }
    }
    out
}

/// Check 3 for one member manifest (`rel` names it in messages).
fn manifest_violations(
    rel: &str,
    manifest: &toml::Table,
    workspace_deps: &toml::Table,
) -> Vec<String> {
    let mut violations = Vec::new();
    let Some(name) = manifest
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
    else {
        return vec![format!("{rel}: no [package] name")];
    };
    let mut normal: Vec<String> = Vec::new();
    let mut dev: Vec<String> = Vec::new();
    for (section, table) in dependency_sections(manifest) {
        for (dep, spec) in table {
            let inherited = spec
                .as_table()
                .and_then(|t| t.get("workspace"))
                .and_then(toml::Value::as_bool)
                == Some(true);
            if !inherited {
                violations.push(format!(
                    "{rel}: [{section}] {dep} is not `workspace = true`"
                ));
            }
            if !workspace_deps.contains_key(dep) {
                violations.push(format!(
                    "{rel}: [{section}] {dep} is not in [workspace.dependencies]"
                ));
            }
            match section.as_str() {
                "dependencies" => normal.push(dep.clone()),
                "dev-dependencies" => dev.push(dep.clone()),
                other => violations.push(format!("{rel}: [{other}] is not in the §0.5 table")),
            }
            let is_adapter = dep == "hikasa-cli" || dep == "hikasa-gui";
            if rel.starts_with("core/") && is_adapter {
                violations.push(format!("{rel}: a core crate depends on {dep}"));
            }
            if ENGINE_CRATES.contains(&name) && dep == "hikasa-session" {
                violations.push(format!(
                    "{rel}: the engine crate {name} depends on hikasa-session"
                ));
            }
        }
    }
    match DEPENDENCY_TABLE
        .iter()
        .find(|(crate_name, _, _)| *crate_name == name)
    {
        Some((_, expected_normal, expected_dev)) => {
            for (kind, actual, expected) in [
                ("[dependencies]", &normal, *expected_normal),
                ("[dev-dependencies]", &dev, *expected_dev),
            ] {
                let mut actual: Vec<&str> = actual.iter().map(String::as_str).collect();
                let mut expected: Vec<&str> = expected.to_vec();
                actual.sort_unstable();
                expected.sort_unstable();
                if actual != expected {
                    violations.push(format!(
                        "{rel}: {kind} {actual:?} differ from the §0.5 table {expected:?}"
                    ));
                }
            }
        }
        None => violations.push(format!("{rel}: crate {name} is not in the §0.5 table")),
    }
    violations
}

fn check_manifests(root: &Path) -> Vec<String> {
    let parse = |path: &Path| -> Option<toml::Table> {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
    };
    let Some(workspace) = parse(&root.join("Cargo.toml")) else {
        return vec!["Cargo.toml: cannot be parsed".into()];
    };
    let ws = workspace.get("workspace").and_then(toml::Value::as_table);
    let deps = ws
        .and_then(|w| w.get("dependencies"))
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    let members: Vec<String> = ws
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .map(|m| {
            m.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if members.is_empty() {
        return vec!["Cargo.toml: no workspace members".into()];
    }
    let mut violations = Vec::new();
    let mut seen = Vec::new();
    for member in &members {
        let rel = format!("{member}/Cargo.toml");
        match parse(&root.join(member).join("Cargo.toml")) {
            Some(manifest) => {
                if let Some(name) = manifest
                    .get("package")
                    .and_then(|p| p.get("name"))
                    .and_then(toml::Value::as_str)
                {
                    seen.push(name.to_owned());
                }
                violations.extend(manifest_violations(&rel, &manifest, &deps));
            }
            None => violations.push(format!("{rel}: missing or not valid TOML")),
        }
    }
    for (name, _, _) in DEPENDENCY_TABLE {
        if !seen.iter().any(|s| s == name) {
            violations.push(format!("the §0.5 crate {name} is not a workspace member"));
        }
    }
    violations
}

// ---------------------------------------------------------------------------------------------------------
// Check 6: no HashMap/HashSet in DTOs and persisted configs
// ---------------------------------------------------------------------------------------------------------

/// True for the files whose types are DTOs or persisted configuration (check 6).
fn is_dto_file(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let file = parts.last().copied().unwrap_or("");
    let in_src = parts.len() >= 4 && parts[0] == "core" && parts[2] == "src";
    in_src
        && (matches!(file, "dto.rs" | "protocol.rs" | "rules.rs" | "model.rs")
            || parts[3..parts.len() - 1]
                .iter()
                .any(|d| *d == "json" || *d == "model"))
}

/// `HashMap`/`HashSet` uses in one file's code.
fn hash_violations(rel: &str, raw: &str) -> Vec<String> {
    let code = code_only(raw);
    let mut violations = Vec::new();
    for token in ["HashMap", "HashSet"] {
        for at in find_token(&code, token) {
            violations.push(format!(
                "{rel}:{}: `{token}` in a DTO or persisted config file (T5: use BTreeMap/BTreeSet)",
                line_of(&code, at)
            ));
        }
    }
    violations
}

fn check_no_hash_collections(root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    for file in rust_files(&root.join("core")) {
        let rel = relative(root, &file);
        if !is_dto_file(&rel) {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(&file) {
            violations.extend(hash_violations(&rel, &raw));
        }
    }
    violations
}

// ---------------------------------------------------------------------------------------------------------
// Check 7: no vocabulary parsing in the adapters
// ---------------------------------------------------------------------------------------------------------

/// The words of the core vocabularies (lower case, without `--`, `-`, `/` or `display-` prefixes): `--display`
/// (folders and text), `--folders`, `--criteria`, `--unimportant`, and the legacy `gui open` switches. Generic
/// global flags (`json`, `quiet`, `log`, `verbose`, `help`, `format`, `config-dir`, `no-color`) are left out: the
/// adapters own them through clap.
const VOCABULARY: &[&str] = &[
    // --display
    "all",
    "differences",
    "mismatches",
    "same",
    "matches",
    "context",
    "none",
    "no-orphans",
    "differences-no-orphans",
    "mismatches-no-orphans",
    "orphans",
    "left-newer",
    "right-newer",
    "left-newer-orphans",
    "left-newer-and-left-orphans",
    "right-newer-orphans",
    "right-newer-and-right-orphans",
    "left-orphans",
    "right-orphans",
    // --folders
    "always",
    "structure",
    "files-only",
    "always-show",
    "always-show-folders",
    "compare-structure",
    "only-files",
    "only-compare-files",
    // --criteria
    "size",
    "owner",
    "group",
    "permissions",
    "filename-case",
    "follow-symlinks",
    "ignore-unimportant",
    "crc",
    "binary",
    "rules-based",
    "rules",
    "timestamp",
    // --unimportant
    "leading-ws",
    "embedded-ws",
    "trailing-ws",
    "whitespace",
    "case",
    "everything-else",
    // legacy gui open switches (CLI-001…003)
    "wait",
    "no-launch",
    "nolaunch",
    "fv",
    "fileviewer",
    "title1",
    "title2",
    "title3",
    "title4",
    "lefttitle",
    "righttitle",
    "centertitle",
    "outputtitle",
    "ro",
    "readonly",
    "ro1",
    "lro",
    "leftreadonly",
    "ro2",
    "rro",
    "rightreadonly",
    "filters",
    "expandall",
    "expand-all",
    "iu",
    "ignoreunimportant",
    "stdin-name",
    "solo",
    "automerge",
    "center",
    "mergeoutput",
    "force",
    "reviewconflicts",
    "favorleft",
    "favorright",
    "closescript",
    "silent",
    "edit",
    "sync",
    "nobackups",
    "qc",
    "quickcompare",
    "savetarget",
    "vcs1",
    "vcs2",
    "vcs3",
    "vcs4",
    "vcsleft",
    "vcsright",
    "vcscenter",
    "vcsoutput",
];

/// Prefixes of the parameterised vocabulary words (`timestamp:2sec`, `timezone:left+1`, `attrib:HRS`,
/// `element:Comment`).
const VOCABULARY_PREFIXES: &[&str] = &["timestamp:", "timezone:", "attrib:", "element:"];

/// True when a string literal's content is a vocabulary word.
fn is_vocabulary_word(literal: &str) -> bool {
    let lower = literal.trim().to_ascii_lowercase();
    let word = lower
        .trim_start_matches("--")
        .trim_start_matches(['-', '/'])
        .trim_start_matches("display-");
    let word = word.split('=').next().unwrap_or(word);
    !word.is_empty()
        && (VOCABULARY.contains(&word) || VOCABULARY_PREFIXES.iter().any(|p| word.starts_with(p)))
}

/// `src` with comments blanked but string literals kept (check 7 reads the literals).
fn without_comments(src: &str) -> String {
    let code = code_only(src);
    // `code_only` keeps one char per char, so both sequences align: a char that `code_only` blanked outside a
    // comment belongs to a string or char literal and is restored from the original.
    let mut out = String::with_capacity(src.len());
    let original: Vec<char> = src.chars().collect();
    let blanked: Vec<char> = code.chars().collect();
    let comments = comment_mask(&original);
    for (k, &c) in original.iter().enumerate() {
        let kept = blanked.get(k).copied().unwrap_or(' ');
        if comments.get(k).copied().unwrap_or(false) {
            out.push(if c == '\n' { '\n' } else { ' ' });
        } else if kept == ' ' && c != ' ' {
            out.push(c);
        } else {
            out.push(kept);
        }
    }
    out
}

/// For each char of `chars`, whether it is part of a comment (line or nested block), skipping string and char
/// literals so that `"//"` inside a string is not a comment.
fn comment_mask(chars: &[char]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        // Char literals that hold a quote or an escape (`'"'`, `'\"'`, `'\n'`): skip them whole.
        if c == '\'' && next == Some('"') && chars.get(i + 2) == Some(&'\'') {
            i += 3;
            continue;
        }
        if c == '\'' && next == Some('\\') {
            let close = (i + 3..chars.len().min(i + 12)).find(|&k| chars[k] == '\'');
            i = close.map_or(i + 1, |k| k + 1);
            continue;
        }
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                mask[i] = true;
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    mask[i] = true;
                    mask[i + 1] = true;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    mask[i] = true;
                    mask[i + 1] = true;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    mask[i] = true;
                    i += 1;
                }
            }
            continue;
        }
        i += 1;
    }
    mask
}

/// String literals of `text` (comment-free) that sit where a match pattern or an equality comparison is:
/// followed by `=>`, `|`, `if` (a guard) or `==`/`!=`, or preceded by `|`, `==` or `!=`. Returns
/// `(byte offset, content)`.
fn compared_literals(text: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] != b'"' {
            j += if bytes[j] == b'\\' { 2 } else { 1 };
        }
        let end = (j + 1).min(bytes.len());
        let content = text
            .get(start + 1..end.saturating_sub(1))
            .unwrap_or("")
            .to_owned();
        let after = text.get(end..).unwrap_or("").trim_start();
        let before = text.get(..start).unwrap_or("").trim_end();
        let followed = after.starts_with("=>")
            || (after.starts_with('|') && !after.starts_with("||"))
            || after.starts_with("==")
            || after.starts_with("!=")
            || after
                .strip_prefix("if")
                .is_some_and(|r| r.starts_with(char::is_whitespace));
        let preceded = (before.ends_with('|') && !before.ends_with("||"))
            || before.ends_with("==")
            || before.ends_with("!=");
        if followed || preceded {
            out.push((start, content));
        }
        i = end;
    }
    out
}

/// Check 7 for one adapter source file.
fn adapter_violations(rel: &str, raw: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let code = code_only(raw);
    for at in find_token(&code, "impl") {
        let header_end = code[at..].find('{').map_or(code.len(), |p| at + p);
        let header = &code[at..header_end];
        if !find_token(header, "FromStr").is_empty() {
            violations.push(format!(
                "{rel}:{}: `impl FromStr` in an adapter (T11: vocabulary parsers live in core)",
                line_of(&code, at)
            ));
        }
    }
    let text = without_comments(raw);
    for (at, literal) in compared_literals(&text) {
        if is_vocabulary_word(&literal) {
            violations.push(format!(
                "{rel}:{}: matches or compares the vocabulary word {literal:?} (T11: use the core parser)",
                line_of(&text, at)
            ));
        }
    }
    violations
}

fn check_adapters(root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    for top in ["cli", "gui"] {
        for file in rust_files(&root.join(top)) {
            let rel = relative(root, &file);
            let parts: Vec<&str> = rel.split('/').collect();
            let is_source =
                parts.get(2).is_some_and(|p| *p == "src") || parts.last() == Some(&"build.rs");
            if !is_source {
                continue;
            }
            if let Ok(raw) = fs::read_to_string(&file) {
                violations.extend(adapter_violations(&rel, &raw));
            }
        }
    }
    violations
}

// ---------------------------------------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------------------------------------

fn assert_none(check: &str, violations: Vec<String>) {
    assert!(
        violations.is_empty(),
        "architecture check {check} failed:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn check_1_unsafe_only_in_mmap() {
    assert_none("1 (unsafe)", check_unsafe(&workspace_root()));
}

#[test]
fn check_2_no_banned_tokens_in_core() {
    let root = workspace_root();
    let mut violations = check_banned_tokens(&root);
    violations.extend(check_adapter_only(&root));
    assert_none("2 (banned tokens, adapter-only functions)", violations);
}

#[test]
fn check_3_manifests_match_the_dependency_table() {
    assert_none("3 (manifests, §0.5)", check_manifests(&workspace_root()));
}

#[test]
fn check_4_stub_crates_export_nothing() {
    assert_none("4 (stub crates)", check_stub_crates(&workspace_root()));
}

#[test]
fn check_5_release_profile_unwinds() {
    let manifest =
        fs::read_to_string(workspace_root().join("Cargo.toml")).expect("workspace Cargo.toml");
    assert_none("5 (panic = \"abort\")", panic_abort_violations(&manifest));
}

#[test]
fn check_6_no_hash_collections_in_dtos() {
    assert_none(
        "6 (HashMap/HashSet in DTOs)",
        check_no_hash_collections(&workspace_root()),
    );
}

#[test]
fn check_7_adapters_parse_no_vocabulary() {
    assert_none(
        "7 (vocabulary parsing in cli/gui)",
        check_adapters(&workspace_root()),
    );
}

#[test]
fn the_scan_sees_the_workspace() {
    let root = workspace_root();
    assert!(
        root.join("core/hikasa-core/src/lib.rs").is_file(),
        "workspace root {}",
        root.display()
    );
    assert!(rust_files(&root.join("core")).len() >= 13);
    assert!(core_crates(&root).len() >= 13);
}

// ---------------------------------------------------------------------------------------------------------
// Self-tests of the scanner
// ---------------------------------------------------------------------------------------------------------

#[test]
fn lexer_blanks_comments_and_literals() {
    let src = concat!(
        "let a = \"println!(x) unsafe { }\"; // std::env::var\n",
        "/* outer /* inner dbg!() */ still comment */ let b = 'x';\n",
        "let c = r#\"thread::sleep \"quoted\" \"#; let d = b'\\''; let e = '\\u{1F600}';\n",
        "fn f<'a>(x: &'a str) -> &'a str { x } let g = br\"Instant::now\"; let h = c\"process::exit\";\n",
        "let r#type = 1; real_code();\n",
    );
    let code = code_only(src);
    assert_eq!(
        code.lines().count(),
        src.lines().count(),
        "newlines are kept"
    );
    for token in [
        "println!",
        "unsafe",
        "std::env",
        "dbg!",
        "thread::sleep",
        "Instant::now",
        "process::exit",
        "quoted",
    ] {
        assert!(
            find_token(&code, token).is_empty(),
            "{token} leaked into {code:?}"
        );
    }
    assert!(
        code.contains("fn f<'a>(x: &'a str)"),
        "lifetimes stay: {code:?}"
    );
    assert!(code.contains("let r#type = 1; real_code();"));
}

#[test]
fn unsafe_pattern_matches_the_contract_regex() {
    let hits = |s: &str| unsafe_uses(&code_only(s)).len();
    assert_eq!(hits("unsafe { x() }"), 1);
    assert_eq!(hits("unsafe\n  {"), 1);
    assert_eq!(hits("pub unsafe fn f() {}"), 1);
    assert_eq!(hits("unsafe impl Send for X {}"), 1);
    assert_eq!(hits("unsafe trait T {}"), 1);
    assert_eq!(hits("unsafe extern \"C\" {}"), 1);
    assert_eq!(hits("#![forbid(unsafe_code)]"), 0);
    assert_eq!(hits("#[allow(unsafe_code)] mod mmap;"), 0);
    assert_eq!(hits("let not_unsafe { };"), 0);
    assert_eq!(hits("unsafe fnord()"), 0);
    assert_eq!(hits("// unsafe { }"), 0);
}

#[test]
fn safety_comments_are_found() {
    let src = "fn f() {\n    // SAFETY: fine\n    #[allow(x)]\n    let m = unsafe { g() };\n}\nunsafe fn h() {}\n";
    let code = code_only(src);
    let lines: Vec<&str> = src.lines().collect();
    let uses = unsafe_uses(&code);
    assert_eq!(uses.len(), 2);
    let check = |at: usize| {
        let line = line_of(&code, at);
        let start = code[..at].rfind('\n').map_or(0, |p| p + 1);
        has_safety_comment(&lines, line, &src[start..at])
    };
    assert!(check(uses[0]));
    assert!(!check(uses[1]));
}

#[test]
fn banned_tokens_respect_boundaries() {
    let code = code_only(
        "eprintln!(); println!(); print!(); eprint!(); dbg!(x); my_print!(); static mut X: u8 = 0;",
    );
    assert_eq!(find_token(&code, "print!").len(), 1);
    assert_eq!(find_token(&code, "println!").len(), 1);
    assert_eq!(find_token(&code, "eprint!").len(), 1);
    assert_eq!(static_mut_uses(&code).len(), 1);
    assert_eq!(find_token("use std::env;", "std::env").len(), 1);
    assert_eq!(find_token("std::environment", "std::env").len(), 0);
}

#[test]
fn grouped_and_aliased_imports_are_resolved() {
    let src = concat!(
        "use std::{env, thread::{self as th, sleep}};\n",
        "use std::time::{Instant as Clock, SystemTime as Wall, UNIX_EPOCH};\n",
        "use jiff::{Timestamp as Ts, tz::TimeZone as Tz};\n",
        "use std::process::{self as p};\n",
        "fn ambient() -> u64 {\n",
        "    let home = env::var(\"HOME\").unwrap_or_default();\n",
        "    sleep(std::time::Duration::from_millis(1));\n",
        "    th::sleep(std::time::Duration::from_millis(1));\n",
        "    let t0 = Clock::now();\n",
        "    let _ = Wall::now();\n",
        "    let s = UNIX_EPOCH.elapsed().map(|d| d.as_secs()).unwrap_or(0);\n",
        "    let _ = (Ts::now(), Tz::system());\n",
        "    p::exit(0);\n",
        "    home.len() as u64 + s + t0 . elapsed ().as_nanos() as u64\n",
        "}\n",
    );
    let hits = banned_hits(&code_only(src));
    for expected in [
        (1, "std::env"),
        (1, "thread::sleep"),
        (6, "std::env"),
        (8, "thread::sleep"),
        (9, "Instant::now"),
        (10, "SystemTime::now"),
        (11, ELAPSED_CALL),
        (12, "Timestamp::now"),
        (12, "TimeZone::system"),
        (13, "process::exit"),
        (14, ELAPSED_CALL),
    ] {
        assert!(
            hits.contains(&expected),
            "{expected:?} missing from {hits:?}"
        );
    }
    let clean = "use std::time::{Duration, Instant};\nfn f(d: Duration, at: Instant) -> Instant { at + d }\n";
    assert!(banned_hits(&code_only(clean)).is_empty());
    let globs = banned_hits(&code_only(
        "use std::thread::*;\nuse std::env::*;\nuse std::*;\n",
    ));
    assert!(globs.contains(&(1, "thread::sleep")) && globs.contains(&(2, "std::env")));
    assert!(globs.contains(&(3, "std::env")));
    let imported: Vec<Import> = imports(&code_only("pub(crate) use a::{b::{self, c as d}, e::*};"))
        .into_iter()
        .map(|(_, import)| import)
        .collect();
    let path = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        imported,
        vec![
            (path(&["a", "b"]), Some("b".to_owned())),
            (path(&["a", "b", "c"]), Some("d".to_owned())),
            (path(&["a", "e", "*"]), None),
        ]
    );
    assert!(imports(&code_only("fn f() -> impl Sized + use<> {}")).is_empty());
}

#[test]
fn allocator_exemption_is_limited_to_test_and_bench_binaries() {
    let allocator = concat!(
        "#![allow(unsafe_code)]\n",
        "// SAFETY: forwards to System.\n",
        "unsafe impl GlobalAlloc for Counting {\n",
        "    // SAFETY: forwarded.\n",
        "    unsafe fn alloc(&self, l: Layout) -> *mut u8 {\n",
        "        // SAFETY: forwarded.\n",
        "        unsafe { System.alloc(l) }\n",
        "    }\n",
        "}\n",
        "#[global_allocator]\n",
        "static GLOBAL: Counting = Counting;\n",
    );
    for rel in [
        "core/hikasa-core/tests/alloc.rs",
        "core/hikasa-core/benches/mask_match.rs",
        "core/hikasa-x/tests/common/alloc.rs",
        "cli/hikasa-cli/tests/alloc.rs",
    ] {
        assert_eq!(
            unsafe_violations(rel, allocator),
            Vec::<String>::new(),
            "{rel}"
        );
    }
    // Library code never qualifies, not even a `src/tests/` module, and neither do other directories.
    for rel in [
        "core/hikasa-core/src/alloc.rs",
        "core/hikasa-core/src/tests/alloc.rs",
        "core/hikasa-core/examples/alloc.rs",
        "core/tests/alloc.rs",
    ] {
        assert_eq!(unsafe_violations(rel, allocator).len(), 3, "{rel}");
    }
    // A test binary without a `#[global_allocator]` gets no exemption.
    let no_allocator = "// SAFETY: no.\nunsafe impl Send for X {}\n";
    assert_eq!(
        unsafe_violations("core/hikasa-core/tests/send.rs", no_allocator).len(),
        1
    );
    // A `#[global_allocator]` mentioned only in a comment or string does not count.
    let commented = "// #[global_allocator]\nconst S: &str = \"#[global_allocator]\";\n// SAFETY: no.\nunsafe fn f() {}\n";
    assert_eq!(
        unsafe_violations("core/hikasa-core/tests/c.rs", commented).len(),
        1
    );
    // Inside the exemption every `unsafe` still needs its `// SAFETY:` comment and the file its lint opt-out.
    let unexplained = allocator.replace("        // SAFETY: forwarded.\n", "");
    let violations = unsafe_violations("core/hikasa-core/tests/alloc.rs", &unexplained);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].ends_with("without a preceding `// SAFETY:` comment"));
    let no_opt_out = allocator.replace("#![allow(unsafe_code)]\n", "");
    let violations = unsafe_violations("core/hikasa-core/tests/alloc.rs", &no_opt_out);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("#![allow(unsafe_code)]"));
    assert!(allows_unsafe_code(&code_only(
        "#![allow(dead_code, unsafe_code)]"
    )));
    assert!(!allows_unsafe_code(&code_only(
        "#[allow(unsafe_code)] mod m;"
    )));
    // mmap.rs is the one library file with `unsafe`.
    let mmap = "// SAFETY: see the module docs.\nunsafe { memmap2::Mmap::map(file) }\n";
    assert!(unsafe_violations(MMAP_FILE, mmap).is_empty());
    assert_eq!(
        unsafe_violations(MMAP_FILE, "unsafe { memmap2::Mmap::map(file) }\n").len(),
        1
    );
}

#[test]
fn panic_abort_is_detected() {
    let bad = "[profile.release]\nlto = true\npanic = \"abort\"\n";
    assert_eq!(panic_abort_violations(bad).len(), 1);
    let fine = "[profile.release]\nlto = \"fat\"\n# panic = \"abort\"\n[dependencies]\npanic = \"abort\"\n";
    assert!(panic_abort_violations(fine).is_empty());
    assert_eq!(
        panic_abort_violations("[profile.bench]\npanic='abort'\n").len(),
        1
    );
}

#[test]
fn manifest_rules_are_enforced() {
    let workspace: toml::Table = toml::from_str(
        "[dependencies]\nserde = \"1\"\nthiserror = \"2\"\nbitflags = \"2\"\njiff = \"0.2\"\n\
         unicode-normalization = \"0.1\"\nproptest = \"1\"\npretty_assertions = \"1\"\nregex = \"1\"\n\
         serde_json = \"1\"\ncriterion = \"0.8\"\nhikasa-session = { path = \"x\" }\n",
    )
    .unwrap();
    let deps = workspace["dependencies"].as_table().unwrap().clone();
    let good: toml::Table = toml::from_str(
        "[package]\nname = \"hikasa-core\"\n[dependencies]\nserde.workspace = true\nthiserror.workspace = true\n\
         bitflags.workspace = true\njiff.workspace = true\nunicode-normalization.workspace = true\n\
         [dev-dependencies]\nproptest.workspace = true\npretty_assertions.workspace = true\n\
         regex.workspace = true\nserde_json.workspace = true\ncriterion = { workspace = true }\n",
    )
    .unwrap();
    assert_eq!(
        manifest_violations("core/hikasa-core/Cargo.toml", &good, &deps),
        Vec::<String>::new()
    );
    let mut bad = good.clone();
    let normal = bad["dependencies"].as_table_mut().unwrap();
    normal.insert("serde".into(), toml::Value::String("1".into()));
    normal.insert(
        "hikasa-session".into(),
        toml::from_str("workspace = true").unwrap(),
    );
    normal.insert("rand".into(), toml::from_str("workspace = true").unwrap());
    bad.insert(
        "build-dependencies".into(),
        toml::from_str("cc = { workspace = true }").unwrap(),
    );
    let violations = manifest_violations("core/hikasa-core/Cargo.toml", &bad, &deps);
    let joined = violations.join("\n");
    for expected in [
        "serde is not `workspace = true`",
        "rand is not in [workspace.dependencies]",
        "depends on hikasa-session",
        "[build-dependencies] is not in the §0.5 table",
        "differ from the §0.5 table",
    ] {
        assert!(
            joined.contains(expected),
            "{expected} missing from:\n{joined}"
        );
    }
    let adapter: toml::Table = toml::from_str(
        "[package]\nname = \"hikasa-vfs\"\n[dev-dependencies]\nhikasa-cli = { workspace = true }\n",
    )
    .unwrap();
    let joined = manifest_violations("core/hikasa-vfs/Cargo.toml", &adapter, &deps).join("\n");
    assert!(
        joined.contains("a core crate depends on hikasa-cli"),
        "{joined}"
    );
    let unknown: toml::Table = toml::from_str("[package]\nname = \"hikasa-new\"\n").unwrap();
    assert!(
        manifest_violations("core/hikasa-new/Cargo.toml", &unknown, &deps)[0]
            .contains("not in the §0.5 table")
    );
}

#[test]
fn hash_collections_in_dtos_are_found() {
    assert!(is_dto_file("core/hikasa-report/src/json/text.rs"));
    assert!(is_dto_file("core/hikasa-folder/src/model/store.rs"));
    assert!(is_dto_file("core/hikasa-session/src/api/dto.rs"));
    assert!(is_dto_file("core/hikasa-session/src/ipc/protocol.rs"));
    assert!(is_dto_file("core/hikasa-text/src/rules.rs"));
    assert!(is_dto_file("core/hikasa-formats/src/model.rs"));
    assert!(!is_dto_file("core/hikasa-diff/src/intern.rs"));
    assert!(!is_dto_file("core/hikasa-session/tests/dto.rs"));
    assert!(!is_dto_file("cli/hikasa-cli/src/dto.rs"));
    let bad = "use std::collections::HashMap;\npub struct X { m: std::collections::HashSet<u8> }\n";
    assert_eq!(hash_violations("core/x/src/dto.rs", bad).len(), 2);
    let fine = "// a HashMap would break T5\npub struct X { m: BTreeMap<String, u8>, s: &'static str }\nconst N: &str = \"HashMap\";\n";
    assert!(hash_violations("core/x/src/dto.rs", fine).is_empty());
}

#[test]
fn adapter_vocabulary_checks() {
    let parse_display = "fn f(s: &str) -> u8 {\n    match s {\n        \"all\" => 1,\n        \"mismatches\" | \"differences\" => 2,\n        _ => 3,\n    }\n}\n";
    let v = adapter_violations("cli/hikasa-cli/src/x.rs", parse_display);
    assert_eq!(v.len(), 3, "{v:?}");
    assert!(v[0].contains(":3:"), "{v:?}");
    let compare = "fn g(a: &str) -> bool { a == \"--ro\" || \"leading-ws\" != a || a == \"timestamp:2sec\" }\n";
    assert_eq!(
        adapter_violations("cli/hikasa-cli/src/x.rs", compare).len(),
        3
    );
    let from_str = "impl std::str::FromStr for Mode { type Err = (); fn from_str(s: &str) -> Result<Self, ()> { Err(()) } }\n\
                    impl<T> core::str::FromStr for W<T> {}\n";
    assert_eq!(
        adapter_violations("cli/hikasa-cli/src/x.rs", from_str).len(),
        2
    );
    // Presentation logic, clap attributes, comments and unrelated words are fine.
    let fine = "#[arg(long, value_name = \"DIR\", default_value = \"all\")]\nstruct A;\n\
                // match s { \"all\" => 1 }\n\
                fn h(shell: &str, s: &str) -> bool { let msg = \"all good\"; matches!(shell, \"bash\" | \"zsh\") && s == \"json-ish\" && !msg.is_empty() }\n\
                const DOC: &str = \"use --display all\";\nfn i(x: bool) -> bool { x || \"a\".is_empty() }\n";
    let v = adapter_violations("cli/hikasa-cli/src/x.rs", fine);
    assert!(v.is_empty(), "{v:?}");
    assert!(is_vocabulary_word("--Display-Mismatches"));
    assert!(is_vocabulary_word("/ro"));
    assert!(is_vocabulary_word("-lefttitle=x"));
    assert!(is_vocabulary_word("element:Comment"));
    assert!(!is_vocabulary_word("bash"));
    assert!(!is_vocabulary_word(""));
    // Comment-free text keeps literals, even ones containing `//`.
    let text = without_comments(
        "let u = \"http://x\"; let q = '\"'; // gone \"all\" =>\n/* \"none\" => */ let k = '\\n';",
    );
    assert!(text.contains("\"http://x\""));
    assert!(!text.contains("gone") && !text.contains("none"));
}

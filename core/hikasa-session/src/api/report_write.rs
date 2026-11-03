//! Writing reports (M2 W7, §1.7.8, §1.7.9): the names of a linked report's files. Pure: the plans and the writes that
//! use them are W7b's. No `std::fs` here (architecture check 2): every write goes through the VFS.

use std::ffi::OsStr;

use hikasa_vfs::archive::name::windows_name_problem;

/// The name of a linked report's files folder's marker.
pub const MARKER: &str = ".hikasa-report.toml";

/// A name that is safe on every file system and in a link (normative, §1.7.8): every char that is not a letter or a
/// digit (Unicode) or one of `.`, `-`, `_` becomes `_` (a lossy U+FFFD too); a leading `.` or `-` becomes `_`; the name
/// is cut at a char boundary to `max` bytes; a name Windows refuses (a device name, also before an extension) gets a
/// leading `_`, and a trailing `.` becomes `_` (W7a note: a leading `_` alone leaves a trailing dot); empty → `file`.
/// The result passes [`windows_name_problem`], and its ASCII bytes are all RFC 3986 unreserved, so an href keeps
/// everything but its non-ASCII letters.
pub fn safe_component(name: &str, max: usize) -> String {
    let max = max.max(1);
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    truncate(&mut out, max);
    if out.starts_with(['.', '-']) {
        out.replace_range(..1, "_");
    }
    if out.ends_with('.') {
        out.pop();
        out.push('_');
    }
    if windows_name_problem(&out).is_some() {
        out.insert(0, '_');
        truncate(&mut out, max);
        if out.ends_with('.') {
            out.pop();
            out.push('_');
        }
    }
    if out.is_empty() {
        return "file".to_owned();
    }
    out
}

/// Cuts `s` to at most `max` bytes at a char boundary.
fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut at = max;
        while !s.is_char_boundary(at) {
            at -= 1;
        }
        s.truncate(at);
    }
}

/// The stem of the main file's final name: without its last extension; `report` when that leaves nothing
/// (`report.html` → `report`, `.html` → `report`).
pub fn main_stem(final_name: &OsStr) -> String {
    let name = final_name.to_string_lossy();
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_ref(), |(stem, _)| stem);
    if stem.is_empty() {
        "report".to_owned()
    } else {
        stem.to_owned()
    }
}

/// The files folder wanted next to the main file: `<safe_component(stem, 64)>_files`; `-2`, `-3`, … when that one is
/// not this report's (`attempt` 1 is the plain name).
pub fn files_folder_name(final_name: &OsStr, attempt: u32) -> String {
    let base = format!("{}_files", safe_component(&main_stem(final_name), 64));
    if attempt <= 1 {
        base
    } else {
        format!("{base}-{attempt}")
    }
}

/// The file of pair report `n` (1-based) of `total`: `<n>-<safe_component(left final name, 64)>.html`, `n` zero-padded
/// to the digits of `total`. The numbers make the names distinct on every file system, case-insensitive ones too.
pub fn pair_file_name(n: usize, total: usize, left_name: &OsStr) -> String {
    let width = total.max(1).checked_ilog10().map_or(1, |d| d as usize + 1);
    format!(
        "{n:0width$}-{}.html",
        safe_component(&left_name.to_string_lossy(), 64)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A7.9: the `safe_component` table.
    #[test]
    fn a7_9_safe_component_table() {
        let table: [(&str, &str); 26] = [
            ("main.c", "main.c"),
            ("a\u{1}b\u{1b}c", "a_b_c"),
            ("a/b\\c", "a_b_c"),
            ("x:y*z?", "x_y_z_"),
            ("\"q\"<l>|p", "_q__l__p"),
            ("r#1%2", "r_1_2"),
            (".hidden", "_hidden"),
            ("-rf", "_rf"),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("NUL.x", "_NUL.x"),
            ("COM1", "_COM1"),
            ("lpt9.tar.gz", "_lpt9.tar.gz"),
            ("name.", "name_"),
            ("bad\u{fffd}.txt", "bad_.txt"),
            ("", "file"),
            ("é日本", "é日本"),
            ("a b", "a_b"),
            ("tab\there", "tab_here"),
            ("\u{202e}txt.exe", "_txt.exe"),
            ("..", "__"),
            ("...", "_._"),
            ("~home", "_home"),
            ("a+b=c", "a_b_c"),
            ("CONSOLE", "CONSOLE"),
            ("CON .txt", "CON_.txt"),
        ];
        for (name, want) in table {
            let got = safe_component(name, 64);
            assert_eq!(got, want, "{name:?}");
            assert_eq!(windows_name_problem(&got), None, "{name:?} → {got:?}");
        }
        // 300 bytes of `é` (2 bytes each): cut at a char boundary to 64 bytes.
        let long = "é".repeat(150);
        let got = safe_component(&long, 64);
        assert_eq!(got.len(), 64);
        assert_eq!(got, "é".repeat(32));
        let got = safe_component(&long, 63);
        assert_eq!(got.len(), 62, "never half a char");
        // A cut that makes a device name or a trailing dot is fixed after the cut.
        assert_eq!(safe_component("CONxyz", 3), "_CO");
        assert_eq!(safe_component("ab.cd", 3), "ab_");
        // The device prefix moves a dot to the end of the cut: `_CON.` is refused, so it becomes `_`.
        for (name, max, want) in [
            ("CON.x", 5, "_CON_"),
            ("nul.tar", 5, "_nul_"),
            ("COM1.c", 6, "_COM1_"),
        ] {
            let got = safe_component(name, max);
            assert_eq!(got, want, "{name:?} at {max}");
            assert_eq!(windows_name_problem(&got), None, "{name:?} → {got:?}");
        }
    }

    #[test]
    fn a7_9_names_of_the_files() {
        assert_eq!(main_stem(OsStr::new("report.html")), "report");
        assert_eq!(main_stem(OsStr::new(".html")), "report");
        assert_eq!(main_stem(OsStr::new("a.b.html")), "a.b");
        assert_eq!(main_stem(OsStr::new("noext")), "noext");
        assert_eq!(
            files_folder_name(OsStr::new("report.html"), 1),
            "report_files"
        );
        assert_eq!(
            files_folder_name(OsStr::new("my report.html"), 2),
            "my_report_files-2"
        );
        assert_eq!(files_folder_name(OsStr::new("CON.html"), 1), "_CON_files");
        // Zero padding to the digits of the number of linked rows.
        assert_eq!(pair_file_name(1, 9, OsStr::new("a.c")), "1-a.c.html");
        assert_eq!(pair_file_name(9, 9, OsStr::new("a.c")), "9-a.c.html");
        assert_eq!(pair_file_name(1, 10, OsStr::new("a.c")), "01-a.c.html");
        assert_eq!(pair_file_name(10, 10, OsStr::new("a.c")), "10-a.c.html");
        assert_eq!(pair_file_name(7, 1_000, OsStr::new("x")), "0007-x.html");
        assert_eq!(pair_file_name(1_000, 1_000, OsStr::new("x")), "1000-x.html");
        // Two pairs whose names differ in case only get names whose case folds differ.
        let (a, b) = (
            pair_file_name(1, 2, OsStr::new("A.txt")),
            pair_file_name(2, 2, OsStr::new("a.txt")),
        );
        assert_ne!(a.to_lowercase(), b.to_lowercase());
        assert_eq!((a.as_str(), b.as_str()), ("1-A.txt.html", "2-a.txt.html"));
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            failure_persistence: None,
            ..proptest::test_runner::Config::default()
        })]

        /// P-LINKS, the names: over random names, the safe name passes `windows_name_problem`, fits `max` bytes, is never
        /// empty, and holds no byte an href escapes but those of non-ASCII letters and digits. Half the cases cut the name
        /// at or just after one of its dots (counting the device prefix), where a cut leaves a trailing dot.
        #[test]
        fn p_links_safe_names(
            name in "\\PC{0,40}|[.\\-]{0,3}(CON|con|NUL|com1|LPT2)(\\.[a-z]{0,3})?\\.?|[a-z.]{0,6}\\.[a-z.]{0,6}",
            max in 1usize..80,
            near_dot in proptest::option::of((proptest::prelude::any::<proptest::sample::Index>(), 0usize..3)),
        ) {
            let dots: Vec<usize> = name.match_indices('.').map(|(at, _)| at).collect();
            let max = match near_dot {
                Some((which, near)) if !dots.is_empty() => dots[which.index(dots.len())] + near + 1,
                _ => max,
            };
            let safe = safe_component(&name, max);
            proptest::prop_assert!(windows_name_problem(&safe).is_none(), "{name:?} → {safe:?}");
            proptest::prop_assert!(!safe.is_empty() && safe.len() <= max.max(4), "{name:?} → {safe:?}");
            proptest::prop_assert!(!safe.starts_with(['.', '-']));
            for c in safe.chars() {
                proptest::prop_assert!(
                    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') || (!c.is_ascii() && c.is_alphanumeric()),
                    "{name:?} → {safe:?}"
                );
            }
        }
    }
}

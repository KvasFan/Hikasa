//! `hikasa table compare` end to end (m2-table-contract §6.5, §7 AT.20, AT.21, AT.24): the human output of a CSV pair
//! and of generated workbooks (goldens), every exit code (1, 2, 12, 2 with `--ignore-unimportant`, 13, 100), the
//! `hikasa.table_compare.v1` goldens (validated against `docs/schemas/table_compare.v1.json` by
//! `core/hikasa-report/tests/table_json_schema.rs`), `--report summary` as text and HTML and the other layouts, a bad
//! word refused before anything is read, `--gui` with report flags refused, the 20-row limit of the human
//! output, standard input, `quick` on tables, `session run` of a table-compare session and a script's `table-report`.
//! Workbooks are generated with `rust_xlsxwriter` (fixed creation time: the bytes do not depend on the clock).

mod common;

use common::{Sandbox, assert_golden, code, json, stderr, stdout};
use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, Workbook};

/// A cell of a generated workbook.
enum C<'a> {
    N(f64),
    S(&'a str),
    D(u16, u8, u8),
}

fn xlsx(sheets: &[(&str, Vec<Vec<C<'_>>>)]) -> Vec<u8> {
    let mut book = Workbook::new();
    let created = ExcelDateTime::from_ymd(2026, 1, 1).unwrap();
    book.set_properties(&DocProperties::new().set_creation_datetime(&created));
    let date = Format::new().set_num_format("yyyy-mm-dd");
    for (name, rows) in sheets {
        let sheet = book.add_worksheet();
        sheet.set_name(*name).unwrap();
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let (r, c) = (r as u32, c as u16);
                match cell {
                    C::N(n) => sheet.write_number(r, c, *n).map(drop),
                    C::S(s) => sheet.write_string(r, c, *s).map(drop),
                    C::D(y, m, d) => sheet
                        .write_datetime_with_format(
                            r,
                            c,
                            ExcelDateTime::from_ymd(*y, *m, *d).unwrap(),
                            &date,
                        )
                        .map(drop),
                }
                .unwrap();
            }
        }
    }
    book.save_to_buffer().unwrap()
}

/// A row of the price sheet: id, name, price, date.
type PriceRow = (f64, &'static str, f64, (u16, u8, u8));

/// The workbooks of the goldens: `Prices` (keyed by `id`: a changed price, a price within 0.02, a case change, a row
/// on each side only), `Notes` (same, `notes` on the right) and `Old` (left only).
fn books(s: &Sandbox) {
    let prices = |rows: &[PriceRow]| {
        let mut out = vec![vec![C::S("id"), C::S("name"), C::S("price"), C::S("date")]];
        for &(id, name, price, (y, m, d)) in rows {
            out.push(vec![C::N(id), C::S(name), C::N(price), C::D(y, m, d)]);
        }
        out
    };
    let notes = || vec![vec![C::S("note")], vec![C::S("first <draft> & \"q\"")]];
    let left = xlsx(&[
        (
            "Prices",
            prices(&[
                (1.0, "apple", 1.5, (2024, 1, 31)),
                (2.0, "pear", 2.0, (2024, 2, 1)),
                (3.0, "plum", 0.75, (2024, 2, 2)),
                (4.0, "fig", 3.0, (2024, 2, 3)),
            ]),
        ),
        ("Notes", notes()),
        ("Old", vec![vec![C::S("k")], vec![C::S("x")]]),
    ]);
    let right = xlsx(&[
        (
            "Prices",
            prices(&[
                (1.0, "apple", 1.5, (2024, 1, 31)),
                (2.0, "Pear", 2.5, (2024, 2, 1)),
                (3.0, "plum", 0.76, (2024, 2, 2)),
                (5.0, "kiwi", 1.0, (2024, 2, 5)),
            ]),
        ),
        ("notes", notes()),
    ]);
    s.file("left.xlsx", left);
    s.file("right.xlsx", right);
}

/// The CSV pair of the goldens.
fn csvs(s: &Sandbox) {
    s.file(
        "a.csv",
        "id,name,price,date\n1,apple,1.50,2024-01-31\n2,pear,2.00,2024-02-01\n3,plum,0.75,2024-02-02\n\
         4,fig,3.00,2024-02-03\n",
    );
    s.file(
        "b.csv",
        "id,name,price,date\n1,apple,1.5,2024-01-31\n2,Pear,2.50,2024-02-01\n3,plum,0.76,2024-02-02\n\
         5,kiwi,1.00,2024-02-05\n",
    );
    s.file(
        "a2.csv",
        "id,name,price,date\n1,apple,1.5,2024-01-31\n2,pear,2,2024-02-01\n3,plum,0.75,2024-02-02\n\
         4,fig,3,2024-02-03\n",
    );
}

fn run(s: &Sandbox, args: &[&str]) -> std::process::Output {
    s.cmd()
        .args(["table", "compare"])
        .args(args)
        .output()
        .unwrap()
}

/// AT.20: the human output of a CSV pair and of two workbooks.
#[test]
fn at20_human_output_goldens() {
    let s = Sandbox::new();
    csvs(&s);
    books(&s);
    let out = run(
        &s,
        &["a.csv", "b.csv", "--key", "id", "--tolerance", "price=0.02"],
    );
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert_golden("table/human_csv.txt", &s.normalise(&stdout(&out)));
    let out = run(
        &s,
        &[
            "left.xlsx",
            "right.xlsx",
            "--key",
            "id",
            "--tolerance",
            "price=0.02",
            "--ignore-case",
            "name",
        ],
    );
    assert_eq!(code(&out), 13, "{}", stderr(&out));
    assert_golden("table/human_xlsx.txt", &s.normalise(&stdout(&out)));
    // A CSV against a workbook sheet (T-3): the left sheet of the same data.
    let out = run(
        &s,
        &["a2.csv", "left.xlsx", "--sheet", "1=Prices", "--key", "id"],
    );
    assert_eq!(code(&out), 2, "{}{}", stdout(&out), stderr(&out));
}

/// AT.20, T-16: every exit code, `--ignore-unimportant` (`--iu`) included.
#[test]
fn at20_exit_codes() {
    let s = Sandbox::new();
    csvs(&s);
    s.file("near.csv", "id,v\n1,1.50\n2,7\n");
    s.file("near2.csv", "id,v\n1,1.51\n2,7\n");
    let cases: &[(&[&str], i32)] = &[
        (&["a.csv", "a.csv"], 1),
        (&["a.csv", "a2.csv"], 2),
        (&["near.csv", "near2.csv", "--tolerance", "0.02"], 12),
        (&["near.csv", "near2.csv", "--tolerance", "0.02", "--iu"], 2),
        (
            &[
                "near.csv",
                "near2.csv",
                "--tolerance",
                "0.02",
                "--ignore-unimportant",
            ],
            2,
        ),
        (&["a.csv", "b.csv"], 13),
        (&["a.csv", "missing.csv"], 100),
        (&["a.csv", "b.csv", "--key", "nope"], 100),
    ];
    for (args, want) in cases {
        let out = run(&s, args);
        assert_eq!(
            code(&out),
            *want,
            "{args:?}: {}{}",
            stdout(&out),
            stderr(&out)
        );
    }
}

/// AT.20: a bad word is exit 100 `invalid_input` before anything is read; so are report flags with `--gui`.
#[test]
fn at20_refusals() {
    let s = Sandbox::new();
    for (args, word) in [
        (&["--sheets", "sideways"][..], "sideways"),
        (&["--columns", "diagonal"][..], "diagonal"),
        (&["--type", "xml"][..], "xml"),
        (&["--tolerance", "lots"][..], "lots"),
        (&["--date-tolerance", "1y"][..], "1y"),
        (&["--widths", "3;0"][..], "0"),
        (&["--display", "some"][..], "some"),
        (&["--cells", "few"][..], "few"),
        (&["--algorithm", "quantum"][..], "quantum"),
        (&["--first-line", "maybe"][..], "maybe"),
        (&["--date-order", "ydm"][..], "ydm"),
    ] {
        let mut all = vec!["no-left.csv", "no-right.csv", "--json"];
        all.extend_from_slice(args);
        let out = run(&s, &all);
        assert_eq!(code(&out), 100, "{args:?}");
        let doc = json(&out);
        assert_eq!(doc["schema"], "hikasa.table_compare.v1");
        assert_eq!(doc["error"]["kind"], "invalid_input", "{args:?}: {doc}");
        assert!(
            doc["error"]["message"].as_str().unwrap().contains(word),
            "{args:?}: {doc}"
        );
    }
    s.file("a.csv", "x\n1\n");
    // `--gui` (phase 3) takes no report flags; the window itself is `cli_table_gui.rs`'s.
    let out = run(
        &s,
        &[
            "a.csv",
            "a.csv",
            "--gui",
            "--no-launch",
            "--report",
            "summary",
            "--json",
        ],
    );
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "invalid_input");
    // A damaged workbook names itself.
    s.file("broken.xlsx", b"PK\x03\x04 truncated");
    let out = run(&s, &["a.csv", "broken.xlsx", "--json"]);
    assert_eq!(code(&out), 100);
    assert_eq!(json(&out)["error"]["kind"], "invalid_data");
}

/// AT.20: the JSON goldens (`hikasa.table_compare.v1`, checked against the JSON Schema by hikasa-report's test).
#[test]
fn at20_json_goldens() {
    let s = Sandbox::new();
    csvs(&s);
    books(&s);
    let out = run(&s, &["a.csv", "b.csv", "--key", "id", "--json"]);
    assert_eq!(code(&out), 13);
    let doc = json(&out);
    assert_eq!(doc["exit_code"], 13);
    assert_golden("table/csv.json", &s.normalise(&stdout(&out)));
    let out = run(
        &s,
        &[
            "left.xlsx",
            "right.xlsx",
            "--key",
            "id",
            "--tolerance",
            "price=0.02",
            "--display",
            "all",
            "--cells",
            "all",
            "--json",
        ],
    );
    assert_eq!(code(&out), 13);
    assert_golden("table/xlsx.json", &s.normalise(&stdout(&out)));
    // The same bytes twice: nothing in the document depends on the clock or the order of threads.
    let again = run(
        &s,
        &[
            "left.xlsx",
            "right.xlsx",
            "--key",
            "id",
            "--tolerance",
            "price=0.02",
            "--display",
            "all",
            "--cells",
            "all",
            "--json",
        ],
    );
    assert_eq!(stdout(&again), stdout(&out));
}

/// AT.20: `--report summary` as text and HTML, side-by-side and interleaved, to stdout and to a file.
#[test]
fn at20_report_goldens() {
    let s = Sandbox::new();
    books(&s);
    let base = [
        "left.xlsx",
        "right.xlsx",
        "--key",
        "id",
        "--tolerance",
        "price=0.02",
    ];
    for (name, extra) in [
        ("summary.txt", &["--report", "summary"][..]),
        ("summary.html", &["--report", "summary", "--html"][..]),
        (
            "sbs.txt",
            &["--report", "side-by-side", "--width", "100"][..],
        ),
        (
            "interleaved.html",
            &["--report", "interleaved", "--html-mono"][..],
        ),
        (
            "sbs_differences.txt",
            &["--layout", "side-by-side", "--display", "differences"][..],
        ),
    ] {
        let mut args = base.to_vec();
        args.extend_from_slice(extra);
        let out = run(&s, &args);
        assert_eq!(code(&out), 13, "{name}: {}", stderr(&out));
        assert_golden(&format!("table/report_{name}"), &s.normalise(&stdout(&out)));
    }
    // To a file: nothing on stdout, the file holds the same report; with --json the envelope goes to stdout.
    s.dir("out");
    let mut args = base.to_vec();
    args.extend_from_slice(&["--report", "summary", "--output", "out/r.txt", "--json"]);
    let out = run(&s, &args);
    assert_eq!(code(&out), 13, "{}", stdout(&out));
    assert_eq!(json(&out)["schema"], "hikasa.table_compare.v1");
    let written = std::fs::read_to_string(s.join("out/r.txt")).unwrap();
    assert!(written.starts_with("Table Compare\n"), "{written}");
    // A word of another kind is refused before the comparison.
    let mut args = base.to_vec();
    args.extend_from_slice(&["--report", "summary", "--options", "patch-unified"]);
    let out = run(&s, &args);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("text reports"), "{}", stderr(&out));
}

/// §6.5: at most 20 rows a sheet unless `--display all`; standard input with `--stdin-name`; `quick` on tables.
#[test]
fn human_limits_stdin_and_quick() {
    let s = Sandbox::new();
    let mut a = String::from("id,v\n");
    let mut b = String::from("id,v\n");
    for i in 0..30 {
        a.push_str(&format!("{i},{i}\n"));
        b.push_str(&format!("{i},{}\n", i + 100));
    }
    s.file("a.csv", &a);
    s.file("b.csv", &b);
    let out = run(&s, &["a.csv", "b.csv"]);
    let text = stdout(&out);
    assert_eq!(text.matches("  row ").count(), 20, "{text}");
    assert!(text.contains("10 more rows"), "{text}");
    let all = stdout(&run(&s, &["a.csv", "b.csv", "--display", "all"]));
    assert_eq!(all.matches("  row ").count(), 30, "{all}");
    let out = s
        .cmd()
        .args([
            "table",
            "compare",
            "-",
            "b.csv",
            "--stdin-name",
            "piped.csv",
        ])
        .write_stdin(b.clone())
        .output()
        .unwrap();
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let respelled: String = b
        .lines()
        .enumerate()
        .map(|(k, line)| {
            if k == 0 {
                format!("{line}\n")
            } else {
                format!("{line}.0\n")
            }
        })
        .collect();
    s.file("respelled.csv", respelled);
    let quick = s
        .cmd()
        .args(["quick", "b.csv", "respelled.csv"])
        .output()
        .unwrap();
    assert_eq!(code(&quick), 2, "{}", stderr(&quick));
}

/// AT.21: `session run` of a table-compare session, and a script's `table-report`.
#[test]
fn at21_session_run_and_script() {
    let s = Sandbox::new();
    s.file("a.csv", "id;v\n1;1,5\n2;7\n");
    s.file("b.csv", "id;v\n2;7\n1;1,50\n");
    let left = s.join("a.csv").display().to_string().replace('\\', "\\\\");
    let right = s.join("b.csv").display().to_string().replace('\\', "\\\\");
    let session = format!(
        "schema_version = 1\ntype = \"table-compare\"\n\n[specs]\nleft = \"{left}\"\nright = \"{right}\"\n\n\
         [rules]\nkeys = [\"id\"]\n\n[rules.parse]\ndelimiters = [\";\"]\ndecimal = \",\"\nthousands = \"none\"\n"
    );
    std::fs::create_dir_all(s.join("cfg/sessions")).unwrap();
    std::fs::write(s.join("cfg/sessions/prices.toml"), session).unwrap();
    let out = s.cmd().args(["session", "run", "prices"]).output().unwrap();
    assert_eq!(code(&out), 2, "{}{}", stdout(&out), stderr(&out));
    let out = s
        .cmd()
        .args(["session", "run", "prices", "--json"])
        .output()
        .unwrap();
    assert_eq!(json(&out)["schema"], "hikasa.table_compare.v1");
    s.file(
        "report.txt",
        "table-report layout:summary a.csv b.csv output-to:out.txt\ndata-report prices output-to:out2.txt \
         options:display-all\n",
    );
    let out = s
        .cmd()
        .args(["script", "run", "report.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    let summary = std::fs::read_to_string(s.join("out.txt")).unwrap();
    assert!(summary.starts_with("Table Compare\n"), "{summary}");
    let sbs = std::fs::read_to_string(s.join("out2.txt")).unwrap();
    assert!(sbs.contains("Sheet 1:"), "{sbs}");
}

// ---- the phase-2 review fixes (contract §10.6) ---------------------------------------------------------------------

/// A table-compare session `t/one` of `a.csv` and `b.csv` (`id`, `v`; one value within 0.02).
fn table_session(s: &Sandbox, extra: &str) {
    s.file("a.csv", "id,v\n1,1.50\n2,7\n3,8\n");
    s.file("b.csv", "id,v\n1,1.51\n2,7\n3,8\n");
    let path = |name: &str| s.join(name).display().to_string().replace('\\', "\\\\");
    std::fs::create_dir_all(s.join("cfg/sessions/t")).unwrap();
    std::fs::write(
        s.join("cfg/sessions/t/one.toml"),
        format!(
            "schema_version = 1\ntype = \"table-compare\"\n\n[specs]\nleft = \"{}\"\nright = \"{}\"\n{extra}",
            path("a.csv"),
            path("b.csv")
        ),
    )
    .unwrap();
}

/// F2: `session edit --set rules.*` and `--defaults table-compare` take every table rule key with its type, check the
/// words, and suggest table keys for a typo.
#[test]
fn f2_session_edit_sets_table_rule_keys() {
    let s = Sandbox::new();
    table_session(&s, "");
    let edit = |set: &str| {
        s.cmd()
            .args(["session", "edit", "t/one", "--set", set])
            .output()
            .unwrap()
    };
    for set in [
        "rules.keys=[\"id\"]",
        "rules.formulas=true",
        "rules.sheets=position",
        "rules.sheet=[{sheet = \"1\", tolerance = [\"v=0.02\"]}]",
        "rules.parse.decimal=.",
        "rules.left_parse.encoding=utf-8",
        "rules.skew=10",
    ] {
        let out = edit(set);
        assert_eq!(code(&out), 0, "{set}: {}", stderr(&out));
    }
    let text = std::fs::read_to_string(s.join("cfg/sessions/t/one.toml")).unwrap();
    for line in [
        "keys = [\"id\"]",
        "formulas = true",
        "sheets = \"position\"",
        "[[rules.sheet]]",
        "[rules.parse]",
        "[rules.left_parse]",
        "skew = 10",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    // A bare word is a one-word list.
    assert_eq!(code(&edit("rules.unimportant=v")), 0);
    let text = std::fs::read_to_string(s.join("cfg/sessions/t/one.toml")).unwrap();
    assert!(text.contains("unimportant = [\"v\"]"), "{text}");
    // The session runs with them: keyed, the tolerance of sheet 1 makes the change unimportant.
    let out = s.cmd().args(["session", "run", "t/one"]).output().unwrap();
    assert_eq!(code(&out), 12, "{}{}", stdout(&out), stderr(&out));
    // Words that do not parse, a type that does not fit, a typo.
    for (set, says) in [
        ("rules.sheets=sideways", "sideways"),
        ("rules.parse.decimal=ab", "--decimal"),
        ("rules.skew=many", "int"),
        ("rules.tolerance=[\"lots\"]", "--tolerance"),
        ("display.filter=bogus", "bogus"),
    ] {
        let out = edit(set);
        assert_eq!(code(&out), 100, "{set}");
        assert!(stderr(&out).contains(says), "{set}: {}", stderr(&out));
        assert!(!stderr(&out).contains("left side"), "{}", stderr(&out));
    }
    let out = edit("rules.key=id");
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("did you mean rules.keys"),
        "{}",
        stderr(&out)
    );
    // The defaults of the type.
    let out = s
        .cmd()
        .args([
            "session",
            "edit",
            "--defaults",
            "table-compare",
            "--set",
            "rules.ignore_case=[\"\"]",
        ])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let out = s
        .cmd()
        .args(["session", "show", "--defaults", "table-compare"])
        .output()
        .unwrap();
    assert!(
        stdout(&out).contains("ignore_case = [\"\"]"),
        "{}",
        stdout(&out)
    );
}

/// F3 (T-44): a `;`-delimited `.csv` against its workbook through the binary is the same.
#[test]
fn f3_a_semicolon_csv_against_its_workbook() {
    let s = Sandbox::new();
    s.file(
        "data.csv",
        "id;region;price;date\n1;south;1.35;2025-02-02\n2;east;2.6;2025-03-03\n",
    );
    let book = xlsx(&[(
        "Data",
        vec![
            vec![C::S("id"), C::S("region"), C::S("price"), C::S("date")],
            vec![C::N(1.0), C::S("south"), C::N(1.35), C::D(2025, 2, 2)],
            vec![C::N(2.0), C::S("east"), C::N(2.6), C::D(2025, 3, 3)],
        ],
    )]);
    s.file("data.xlsx", book);
    let out = run(&s, &["data.csv", "data.xlsx"]);
    assert_eq!(code(&out), 2, "{}{}", stdout(&out), stderr(&out));
    assert!(
        stdout(&out).starts_with("= data.csv \u{2192} Data\n"),
        "{}",
        stdout(&out)
    );
    let out = run(&s, &["data.csv", "data.xlsx", "--delimiter", ","]);
    assert_eq!(code(&out), 13);
    assert!(
        stderr(&out).contains("every row has one field")
            && stderr(&out).contains("try --delimiter ';'"),
        "{}",
        stderr(&out)
    );
}

/// F10 (T-46): `--cells` maps into `--report json`; output flags the output does not use are refused; `--max-rows`
/// limits the human output.
#[test]
fn f10_output_flags_against_the_output() {
    let s = Sandbox::new();
    let mut a = String::from("id,v,w\n");
    let mut b = String::from("id,v,w\n");
    for i in 0..5 {
        a.push_str(&format!("{i},{i},x\n"));
        b.push_str(&format!("{i},{},x\n", i + 10));
    }
    s.file("a.csv", &a);
    s.file("b.csv", &b);
    let cells = |extra: &[&str]| -> usize {
        let mut args = vec!["a.csv", "b.csv"];
        args.extend_from_slice(extra);
        let out = run(&s, &args);
        assert_eq!(code(&out), 13, "{extra:?}: {}", stderr(&out));
        let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        // `--json` wraps the document in its envelope (`result`); `--report json` writes it alone.
        let doc = if doc.get("result").is_some() {
            doc["result"].clone()
        } else {
            doc
        };
        doc["sheets"][0]["rows"][0]["cells"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(cells(&["--report", "json"]), 1);
    assert_eq!(cells(&["--report", "json", "--cells", "all"]), 3);
    assert_eq!(cells(&["--json", "--cells", "all"]), 3);
    for extra in [
        &["--cells", "all"][..],
        &["--report", "summary", "--cells", "all"],
        &["--report", "xml", "--cells", "all"],
        &["--max-rows", "2", "--json"],
        &["--max-rows", "2", "--report", "summary"],
        &["--report", "json", "--width", "80"],
        &["--report", "summary", "--width", "80"],
        &["--report", "side-by-side", "--html", "--width", "80"],
        &["--report", "summary", "--no-line-numbers"],
        &["--report", "json", "--line-numbers"],
        &["--report", "json", "--title", "T"],
        &["--report", "summary", "--display", "same"],
    ] {
        let mut args = vec!["a.csv", "b.csv"];
        args.extend_from_slice(extra);
        let out = run(&s, &args);
        assert_eq!(code(&out), 100, "{extra:?}");
        assert!(
            stderr(&out).contains("means nothing with this output"),
            "{extra:?}: {}",
            stderr(&out)
        );
    }
    for extra in [
        &["--report", "side-by-side", "--width", "100"][..],
        &["--report", "xml", "--no-line-numbers"],
        &[
            "--report",
            "interleaved",
            "--title",
            "T",
            "--display",
            "all",
        ],
    ] {
        let mut args = vec!["a.csv", "b.csv"];
        args.extend_from_slice(extra);
        assert_eq!(code(&run(&s, &args)), 13, "{extra:?}");
    }
    let text = stdout(&run(&s, &["a.csv", "b.csv", "--max-rows", "2"]));
    assert_eq!(text.matches("  row ").count(), 2, "{text}");
    assert!(text.contains("\u{2026} 3 more rows"), "{text}");
    let text = stdout(&run(&s, &["a.csv", "b.csv", "--max-rows", "0"]));
    assert_eq!(text.matches("  row ").count(), 5, "{text}");
}

/// F11, F13 (T-43): selectors as written; per-side parser flags; `--type fixed` without widths refused.
#[test]
fn f11_f13_selectors_and_per_side_flags() {
    let s = Sandbox::new();
    s.file("de.csv", "n;d\n1.234,5;31.01.2024\n");
    s.file("en.csv", "n,d\n\"1,234.5\",01/31/2024\n");
    let out = run(&s, &["de.csv", "en.csv", "--sheet", "9=1"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("no left sheet `9`"),
        "{}",
        stderr(&out)
    );
    let out = run(
        &s,
        &[
            "de.csv",
            "en.csv",
            "--left-delimiter",
            ";",
            "--left-decimal",
            ",",
            "--left-thousands",
            ".",
            "--left-date-order",
            "dmy",
            "--right-date-order",
            "mdy",
        ],
    );
    assert_eq!(code(&out), 2, "{}{}", stdout(&out), stderr(&out));
    let out = run(&s, &["de.csv", "en.csv", "--decimal", ","]);
    assert_eq!(code(&out), 13, "both sides take the both-sides flag");
    let out = run(&s, &["de.csv", "en.csv", "--type", "fixed"]);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("--widths"), "{}", stderr(&out));
    let out = run(&s, &["de.csv", "en.csv", "--right-decimal", "ab"]);
    assert_eq!(code(&out), 100);
    assert!(
        stderr(&out).contains("right side: --decimal"),
        "{}",
        stderr(&out)
    );
}

/// R14, R15, R16, R19: `session run --display` of a table session; the report word `ignore-unimportant` reaches the
/// verdict and the exit code (`table compare --report`, a script's `table-report`); `--wrap` is refused for a table.
#[test]
fn r14_r19_session_run_report_words_and_wrap() {
    let s = Sandbox::new();
    table_session(&s, "\n[rules]\nkeys = [\"id\"]\ntolerance = [\"v=0.02\"]\n");
    let out = s.cmd().args(["session", "run", "t/one"]).output().unwrap();
    assert_eq!(code(&out), 12);
    assert!(!stdout(&out).contains(": same"), "{}", stdout(&out));
    let out = s
        .cmd()
        .args(["session", "run", "t/one", "--display", "all"])
        .output()
        .unwrap();
    assert!(stdout(&out).contains("  row 3/3: same"), "{}", stdout(&out));
    let out = s
        .cmd()
        .args(["session", "run", "t/one", "--wrap", "word"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("--wrap"), "{}", stderr(&out));
    let out = run(
        &s,
        &[
            "a.csv",
            "b.csv",
            "--key",
            "id",
            "--tolerance",
            "v=0.02",
            "--report",
            "summary",
        ],
    );
    assert_eq!(code(&out), 12);
    let out = run(
        &s,
        &[
            "a.csv",
            "b.csv",
            "--key",
            "id",
            "--tolerance",
            "v=0.02",
            "--report",
            "summary",
            "--options",
            "ignore-unimportant",
        ],
    );
    assert_eq!(code(&out), 2, "{}", stderr(&out));
    assert!(
        stdout(&out).contains("Ignore unimportant:  yes"),
        "{}",
        stdout(&out)
    );
    s.file(
        "report.txt",
        "table-report layout:summary t/one output-to:iu.txt options:ignore-unimportant\n\
         table-report layout:summary t/one output-to:plain.txt\n",
    );
    let out = s
        .cmd()
        .args(["script", "run", "report.txt"])
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    let verdict = |name: &str| {
        std::fs::read_to_string(s.join(name))
            .unwrap()
            .lines()
            .find(|l| l.starts_with("Verdict:"))
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        verdict("iu.txt"),
        "Verdict:             same under the rules"
    );
    assert_eq!(
        verdict("plain.txt"),
        "Verdict:             unimportant differences only"
    );
}

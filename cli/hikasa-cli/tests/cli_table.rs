//! `hikasa table compare` end to end (m2-table-contract §6.5, §7 AT.20, AT.21, AT.24): the human output of a CSV pair
//! and of generated workbooks (goldens), every exit code (1, 2, 12, 2 with `--ignore-unimportant`, 13, 100), the
//! `hikasa.table_compare.v1` goldens (validated against `docs/schemas/table_compare.v1.json` by
//! `core/hikasa-report/tests/table_json_schema.rs`), `--report summary` as text and HTML and the other layouts, a bad
//! word refused before anything is read, `--gui` refused until the table view exists, the 20-row limit of the human
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

/// AT.20: a bad word is exit 100 `invalid_input` before anything is read; `--gui` is refused (`unsupported`).
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
    let out = run(&s, &["a.csv", "a.csv", "--gui"]);
    assert_eq!(code(&out), 100);
    assert!(stderr(&out).contains("no GUI view"), "{}", stderr(&out));
    let out = run(&s, &["a.csv", "a.csv", "--gui", "--json"]);
    assert_eq!(json(&out)["error"]["kind"], "unsupported");
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

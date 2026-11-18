//! Table Compare phase 2 in the Service (m2-table-contract §6.4, §7 AT.20–AT.22): `Service::table_compare` on CSV and
//! generated xlsx pairs with every exit code and the errors of exit 100, a workbook inside a zip read through the VFS,
//! the folder content test of a tree of CSV and xlsx files under the Rules method (T-21), `table-compare` sessions
//! and `session run`, the Spreadsheet text conversion (T-22) and the regional settings of the locale (A16).

mod common;

use std::io::Write as _;
use std::path::Path;

use common::{facts, fixed_clock, job, os, service, write};
use hikasa_core::{ContentKind, ContentResult, ErrorKind, ExitCode, JobKind, Pair};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, NullObserver};
use hikasa_session::api::{
    FolderCompareRequest, SessionRunRequest, SheetRuleWords, SheetRulesWords, TableCompareRequest,
    TableFormatWords, TableRuleWords, TextCompareRequest,
};
use hikasa_session::sessions::{Session, SessionRef, SessionRules, SessionType};
use hikasa_session::{ApiError, Service, ServiceInit};
use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, Workbook};

/// A cell of a generated workbook.
enum C<'a> {
    N(f64),
    S(&'a str),
    D(u16, u8, u8),
    B(bool),
}

/// An xlsx of `sheets` (name, rows), with a fixed creation time so the bytes do not depend on the clock.
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
                    C::B(b) => sheet.write_boolean(r, c, *b).map(drop),
                }
                .unwrap();
            }
        }
    }
    book.save_to_buffer().unwrap()
}

/// The price list as a workbook: `id, name, price, date, ok` and three rows.
fn prices(pear: f64) -> Vec<u8> {
    xlsx(&[(
        "Prices",
        vec![
            vec![
                C::S("id"),
                C::S("name"),
                C::S("price"),
                C::S("date"),
                C::S("ok"),
            ],
            vec![
                C::N(1.0),
                C::S("apple"),
                C::N(1.5),
                C::D(2024, 1, 31),
                C::B(true),
            ],
            vec![
                C::N(2.0),
                C::S("pear"),
                C::N(pear),
                C::D(2024, 2, 1),
                C::B(false),
            ],
            vec![
                C::N(3.0),
                C::S("plum"),
                C::N(0.1 + 0.2),
                C::D(2024, 2, 2),
                C::B(true),
            ],
        ],
    )])
}

/// The price list as its CSV export (other spellings of the same values).
const PRICES_CSV: &str = "id,name,price,date,ok\n1,apple,1.50,2024-01-31,TRUE\n2,pear,2,2024-02-01,FALSE\n\
                          3,plum,0.3,2024-02-02,TRUE\n";

fn request(left: &Path, right: &Path) -> TableCompareRequest {
    TableCompareRequest {
        left: os(left),
        right: os(right),
        ..TableCompareRequest::default()
    }
}

fn compare(
    service: &Service,
    req: &TableCompareRequest,
) -> Result<(ContentKind, ExitCode), ApiError> {
    let resp = service.table_compare(req, &job(service, JobKind::TableCompare))?;
    Ok((resp.kind, resp.exit_code))
}

/// AT.20: the verdicts and exit codes of T-16 (1, 2, 12 and 2 with `--ignore-unimportant`, 13).
#[test]
fn at20_exit_codes_of_every_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v\n1,1.50\n2,7\n");
    let same = write(&w, "same.csv", b"id,v\n1,1.50\n2,7\n");
    let spelled = write(&w, "spelled.csv", b"id,v\n1,1.5\n2,7\n");
    let near = write(&w, "near.csv", b"id,v\n1,1.51\n2,7\n");
    let changed = write(&w, "changed.csv", b"id,v\n1,9\n2,7\n");
    let tolerant = |right: &Path, iu: bool| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                tolerance: vec!["v=0.02".into()],
                ignore_case: vec![String::new()],
                keys: vec!["id".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ignore_unimportant: iu,
        ..request(&a, right)
    };
    assert_eq!(
        compare(&service, &request(&a, &same)).unwrap(),
        (ContentKind::BinarySame, ExitCode::BinarySame)
    );
    assert_eq!(
        compare(&service, &request(&a, &spelled)).unwrap(),
        (ContentKind::RulesSame, ExitCode::RulesSame)
    );
    assert_eq!(
        compare(&service, &tolerant(&near, false)).unwrap(),
        (ContentKind::UnimportantOnly, ExitCode::Similar)
    );
    assert_eq!(
        compare(&service, &tolerant(&near, true)).unwrap(),
        (ContentKind::RulesSame, ExitCode::RulesSame)
    );
    assert_eq!(
        compare(&service, &request(&a, &changed)).unwrap(),
        (ContentKind::ImportantDifferences, ExitCode::RulesDifferent)
    );
    assert_eq!(ExitCode::BinarySame.code(), 1);
    assert_eq!(ExitCode::Similar.code(), 12);
    assert_eq!(ExitCode::RulesDifferent.code(), 13);
}

/// AT.20: the failures of exit 100, each with its kind; a bad word is refused before anything is read.
#[test]
fn at20_errors_are_exit_100_with_their_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v\n1,2\n");
    let missing = w.join("missing.csv");
    let op = hikasa_session::api::ApiOp::TableCompare;
    let fails = |req: TableCompareRequest| -> ApiError {
        let e = compare(&service, &req).unwrap_err();
        assert_eq!(e.exit_code(op), ExitCode::UnknownError, "{e}");
        e
    };
    // A bad word is refused before the (missing) files are read.
    for words in [
        TableRuleWords {
            sheets: Some("sideways".into()),
            ..TableRuleWords::default()
        },
        TableRuleWords {
            rules: SheetRuleWords {
                tolerance: vec!["lots".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
    ] {
        let e = fails(TableCompareRequest {
            rules: words,
            ..request(&missing, &missing)
        });
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
    }
    let e = fails(TableCompareRequest {
        parse: TableFormatWords {
            widths: Some("3;0".into()),
            ..TableFormatWords::default()
        },
        ..request(&missing, &missing)
    });
    assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
    // A column reference that names nothing lists the columns.
    let e = fails(TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                keys: vec!["nope".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..request(&a, &a)
    });
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    assert!(e.to_string().contains("id, v"), "{e}");
    // A missing file, a folder, a damaged workbook, a text format forced on a table.
    assert!(matches!(
        fails(request(&a, &missing)),
        ApiError::LoadFailed { .. }
    ));
    assert_eq!(fails(request(&a, &w)).kind(), ErrorKind::InvalidInput);
    let broken = write(&w, "broken.xlsx", b"PK\x03\x04 not a zip at all");
    assert_eq!(fails(request(&a, &broken)).kind(), ErrorKind::InvalidData);
    let e = fails(TableCompareRequest {
        format: Some("c-cpp".into()),
        ..request(&a, &a)
    });
    assert!(e.to_string().contains("table kind"), "{e}");
    let e = fails(TableCompareRequest {
        format: Some("csv".into()),
        formats: Pair::new(Some("csv".into()), None),
        ..request(&a, &a)
    });
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
}

/// AT.20, T-3: an xlsx against its CSV export is the same (other spellings, 15 digits, dates, bools), a changed price
/// is an important difference with the cells and rows the JSON names.
#[test]
fn at20_a_workbook_against_its_csv_export() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let book = write(&w, "prices.xlsx", &prices(2.0));
    let csv = write(&w, "prices.csv", PRICES_CSV.as_bytes());
    let keyed = |right: &Path| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                keys: vec!["id".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..request(&book, right)
    };
    let resp = service
        .table_compare(&keyed(&csv), &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(
        resp.kind,
        ContentKind::RulesSame,
        "{:?}",
        resp.human_lines(None)
    );
    let ids = resp
        .formats
        .as_ref()
        .map(|f| f.format.as_ref().and_then(|f| f.id.clone()));
    assert_eq!(ids.left.as_deref(), Some("spreadsheets"));
    assert_eq!(ids.right.as_deref(), Some("csv"));
    let other = write(&w, "other.xlsx", &prices(2.25));
    let resp = service
        .table_compare(&keyed(&other), &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.exit_code, ExitCode::RulesDifferent);
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(json["verdict"]["result"], "important_differences");
    assert_eq!(json["left"]["source"], "xlsx");
    let rows = json["sheets"][0]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["cells"][0]["left"]["value"], "2");
    assert_eq!(rows[0]["cells"][0]["right"]["value"], "2.25");
    let lines = resp.human_lines(Some(20));
    assert!(
        lines.iter().any(|l| l.contains("price: 2 \u{2192} 2.25")),
        "{lines:?}"
    );
}

/// AT.20, T-10: several sheets pair by name (case-insensitively), a sheet on one side only is important; per-sheet rules
/// of a session key one sheet and leave the other unkeyed.
#[test]
fn at20_sheets_and_per_sheet_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let rows = |b: f64| {
        vec![
            vec![C::S("code"), C::S("n")],
            vec![C::S("a"), C::N(1.0)],
            vec![C::S("b"), C::N(b)],
        ]
    };
    let left = write(
        &w,
        "l.xlsx",
        &xlsx(&[
            ("Data", rows(2.0)),
            ("Notes", rows(5.0)),
            ("Old", rows(1.0)),
        ]),
    );
    let right = write(
        &w,
        "r.xlsx",
        &xlsx(&[("notes", rows(5.0)), ("Data", rows(3.0))]),
    );
    let resp = service
        .table_compare(
            &request(&left, &right),
            &job(&service, JobKind::TableCompare),
        )
        .unwrap();
    let names: Vec<_> = resp
        .comparison
        .sheets()
        .iter()
        .map(|s| (s.name().to_owned(), s.state().as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("Data".to_owned(), "important"),
            ("Notes".to_owned(), "same"),
            ("Old".to_owned(), "left_only"),
        ]
    );
    let lines = resp.human_lines(Some(20));
    assert!(
        lines.contains(&"= Notes \u{2192} notes".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"< Old (left only)".to_owned()), "{lines:?}");
    // Per-sheet rules (sessions only): Data keyed by `code`, the defaults elsewhere.
    let per = TableRuleWords {
        sheet: vec![SheetRulesWords {
            sheet: "data".into(),
            rules: SheetRuleWords {
                keys: vec!["code".into()],
                ..SheetRuleWords::default()
            },
        }],
        ..TableRuleWords::default()
    };
    let resp = service
        .table_compare(
            &TableCompareRequest {
                rules: per,
                ..request(&left, &right)
            },
            &job(&service, JobKind::TableCompare),
        )
        .unwrap();
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(json["sheets"][0]["alignment"]["mode"], "keyed");
    assert_eq!(json["sheets"][1]["alignment"]["mode"], "engine");
}

/// A workbook inside a zip archive loads through the VFS (T-4): the member against the file it was made from.
#[test]
fn a_workbook_member_of_a_zip_through_the_vfs() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let book = prices(2.0);
    let file = write(&w, "prices.xlsx", &book);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    zip.start_file("books/prices.xlsx", opts).unwrap();
    zip.write_all(&book).unwrap();
    zip.start_file("books/prices.csv", opts).unwrap();
    zip.write_all(PRICES_CSV.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let archive = write(&w, "books.zip", &bytes);
    let member = |name: &str| {
        let mut spec = archive.as_os_str().to_owned();
        spec.push(format!("/books/{name}"));
        spec
    };
    let req = TableCompareRequest {
        left: member("prices.xlsx"),
        right: os(&file),
        ..TableCompareRequest::default()
    };
    let resp = service
        .table_compare(&req, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.kind, ContentKind::BinarySame);
    assert!(
        resp.sides.left.spec.ends_with("prices.xlsx"),
        "{}",
        resp.sides.left.spec
    );
    let req = TableCompareRequest {
        left: member("prices.xlsx"),
        right: member("prices.csv"),
        ..TableCompareRequest::default()
    };
    assert_eq!(compare(&service, &req).unwrap().0, ContentKind::RulesSame);
}

/// AT.22, T-21: a folder comparison under the Rules method compares table files through the table engine: a
/// respelled CSV is same by the rules, a changed workbook cell is a difference, a damaged workbook is that pair's
/// error (never a byte comparison), a text file still goes through the text rules.
#[test]
fn at22_folder_compare_of_csv_and_xlsx_under_the_rules_method() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/data.csv", b"id,v\n1,1.50\n2,7\n");
    write(&w, "r/data.csv", b"id,v\n1,1.5\n2,7\n");
    write(&w, "l/book.xlsx", &prices(2.0));
    write(&w, "r/book.xlsx", &prices(2.5));
    write(&w, "l/same.xlsx", &prices(2.0));
    write(
        &w,
        "r/same.xlsx",
        &xlsx(&[(
            "Prices",
            vec![
                vec![
                    C::S("id"),
                    C::S("name"),
                    C::S("price"),
                    C::S("date"),
                    C::S("ok"),
                ],
                vec![
                    C::N(1.0),
                    C::S("apple"),
                    C::N(1.5),
                    C::D(2024, 1, 31),
                    C::B(true),
                ],
                vec![
                    C::N(2.0),
                    C::S("pear"),
                    C::N(2.0),
                    C::D(2024, 2, 1),
                    C::B(false),
                ],
                vec![
                    C::N(3.0),
                    C::S("plum"),
                    C::N(0.3),
                    C::D(2024, 2, 2),
                    C::B(true),
                ],
            ],
        )]),
    );
    write(&w, "l/bad.xlsx", b"not a workbook");
    write(&w, "r/bad.xlsx", b"not a workbook either");
    write(&w, "l/note.txt", b"a b\r\n");
    write(&w, "r/note.txt", b"a b\n");
    let req = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("rules-based".into()),
        ..FolderCompareRequest::default()
    };
    let resp = service
        .folder_compare(&req, &job(&service, JobKind::FolderCompare), &NullObserver)
        .unwrap();
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let json = serde_json::to_value(resp.json(&display)).unwrap();
    let result = |path: &str| -> String {
        let item = json["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["path"] == path)
            .unwrap_or_else(|| panic!("{path}: {json}"));
        item["content"]["result"]["result"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(result("data.csv"), "rules_same");
    assert_eq!(result("book.xlsx"), "important_differences");
    assert_eq!(result("same.xlsx"), "rules_same");
    assert_eq!(result("bad.xlsx"), "error");
    // Line endings are not compared by the text rules' defaults.
    assert_eq!(result("note.txt"), "rules_same");
}

/// AT.21: a table-compare session keeps the words (sparse against the defaults) and `session run` runs it.
#[test]
fn at21_sessions_round_trip_and_run() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id;v;w\n1;1,5;a\n2;7;b\n");
    let b = write(&w, "b.csv", b"id;v;w\n2;7;b\n1;1,50;a\n");
    let mut session = Session::builtin(SessionType::TableCompare);
    session.specs.left = a.display().to_string();
    session.specs.right = b.display().to_string();
    let SessionRules::Table(rules) = &mut session.rules else {
        panic!("{:?}", session.rules);
    };
    rules.parse.delimiters = vec![";".into()];
    rules.parse.decimal = Some(",".into());
    rules.parse.thousands = Some("none".into());
    rules.rules.rules.keys = vec!["id".into()];
    let reference = SessionRef::parse("tables/prices").unwrap();
    service.session_save(&reference, &session, false).unwrap();
    let text =
        std::fs::read_to_string(tmp.path().join("config/sessions/tables/prices.toml")).unwrap();
    assert!(text.contains("keys = [\"id\"]"), "{text}");
    assert!(text.contains("[rules.parse]"), "{text}");
    assert!(
        !text.contains("sort_rows"),
        "defaults are not written: {text}"
    );
    let SessionRunRequest::Table(req) = service.session_run_request(&reference).unwrap() else {
        panic!("not a table request");
    };
    assert_eq!(compare(&service, &req).unwrap().0, ContentKind::RulesSame);
    // Without the session's keys the rows differ by order.
    let unkeyed = TableCompareRequest {
        rules: TableRuleWords::default(),
        ..req
    };
    assert_ne!(
        compare(&service, &unkeyed).unwrap().0,
        ContentKind::RulesSame
    );
}

/// T-22: a Text Compare of two workbooks reads them as text through the Spreadsheet conversion (read-only).
#[test]
fn t22_text_compare_reads_workbooks_as_text() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let two = |pear: f64| {
        xlsx(&[
            ("One", vec![vec![C::S("a"), C::N(1.0)]]),
            ("Two", vec![vec![C::S("pear"), C::N(pear)]]),
        ])
    };
    let left = write(&w, "l.xlsx", &two(2.0));
    let right = write(&w, "r.xlsx", &two(3.0));
    let resp = service
        .text_compare(
            &TextCompareRequest {
                left: os(&left),
                right: os(&right),
                ..TextCompareRequest::default()
            },
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    let doc = resp.comparison.document(hikasa_core::Side::Left);
    let lines: Vec<String> = (0..doc.line_count())
        .map(|k| doc.line(hikasa_text::LineIdx(k)).into_owned())
        .collect();
    assert_eq!(lines, ["--- One ---", "a\t1", "--- Two ---", "pear\t2"]);
    assert!(doc.converted(), "read-only: converted");
    assert_eq!(
        resp.comparison.verdict(false),
        ContentResult::ImportantDifferences
    );
}

/// A16: `--regional system` reads numbers the locale's way (`de_DE`: `,` decimal, `.` thousands).
#[test]
fn a16_regional_settings_of_the_locale() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = facts(tmp.path());
    env.locale_numeric = Some("de_DE.UTF-8".into());
    env.locale_time = Some("de_DE.UTF-8".into());
    let init = ServiceInit {
        config_dir: Some(tmp.path().join("config")),
        threads: std::num::NonZeroUsize::new(2),
        ..ServiceInit::new(
            env,
            fixed_clock(),
            std::sync::Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"n;d\n1.234,5;31.01.2024\n");
    let b = write(&w, "b.csv", b"n;d\n1234,50;2024-01-31\n");
    let parse = |regional: Option<&str>| TableCompareRequest {
        parse: TableFormatWords {
            delimiters: vec![";".into()],
            regional: regional.map(str::to_owned),
            ..TableFormatWords::default()
        },
        ..request(&a, &b)
    };
    assert_eq!(
        compare(&service, &parse(Some("system"))).unwrap().0,
        ContentKind::RulesSame
    );
    assert_eq!(
        compare(&service, &parse(None)).unwrap().0,
        ContentKind::ImportantDifferences
    );
    let r = service.system_regional();
    assert_eq!((r.decimal, r.thousands), (',', Some('.')));
}

/// T-24 and T-30 through the words: `--number` equates `007` and `7` (a leading zero makes the column text by
/// default); a per-sheet rule must name a column of its sheet (strict), while the rules of every sheet skip one.
#[test]
fn number_columns_and_strict_per_sheet_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"code,v\n007,1\n010,2\n");
    let b = write(&w, "b.csv", b"code,v\n7,1\n10,2\n");
    assert_eq!(
        compare(&service, &request(&a, &b)).unwrap().0,
        ContentKind::ImportantDifferences
    );
    let number = TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                number: vec!["code".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..request(&a, &b)
    };
    assert_eq!(
        compare(&service, &number).unwrap().0,
        ContentKind::RulesSame
    );
    let text = TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                text: vec!["v".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..number.clone()
    };
    assert_eq!(
        compare(&service, &text).unwrap().0,
        ContentKind::ImportantDifferences
    );
    // Rules of one sheet are strict: a column the sheet lacks is an error.
    let per_sheet = |column: &str| TableCompareRequest {
        rules: TableRuleWords {
            sheet: vec![SheetRulesWords {
                sheet: "1".into(),
                rules: SheetRuleWords {
                    keys: vec![column.into()],
                    ..SheetRuleWords::default()
                },
            }],
            ..TableRuleWords::default()
        },
        ..request(&a, &a)
    };
    assert_eq!(
        compare(&service, &per_sheet("code")).unwrap().0,
        ContentKind::BinarySame
    );
    let e = compare(&service, &per_sheet("missing")).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
}

/// §6.5: with `--ignore-unimportant` the human output lists no unimportant cell; without, each is marked.
#[test]
fn the_human_output_under_ignore_unimportant() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v,w\n1,1.50,3\n2,7,4\n");
    let b = write(&w, "b.csv", b"id,v,w\n1,1.51,3\n2,7.01,5\n");
    let req = |iu: bool| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                tolerance: vec!["v=0.02".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ignore_unimportant: iu,
        ..request(&a, &b)
    };
    let lines = |iu: bool| {
        service
            .table_compare(&req(iu), &job(&service, JobKind::TableCompare))
            .unwrap()
            .human_lines(None)
    };
    let all = lines(false);
    assert!(
        all.iter()
            .any(|l| l.ends_with("v: 1.50 \u{2192} 1.51 (unimportant)")),
        "{all:?}"
    );
    assert!(
        all.iter().any(|l| l.ends_with("w: 4 \u{2192} 5")),
        "{all:?}"
    );
    assert!(
        all.iter()
            .any(|l| l.ends_with("v: 7 \u{2192} 7.01 (unimportant)")),
        "{all:?}"
    );
    // Row 2 still differs (w), so it is shown; its unimportant cell is not.
    let iu = lines(true);
    assert!(!iu.iter().any(|l| l.contains("(unimportant)")), "{iu:?}");
    assert!(iu.iter().any(|l| l.ends_with("w: 4 \u{2192} 5")), "{iu:?}");
}

// ---- the phase-2 review fixes (contract §10.6) ---------------------------------------------------------------------

/// The reviewer's workbook: `Data` keyed by `id` (one changed price) and `Notes` without an `id` column (one case
/// change).
fn data_and_notes(price: f64, note: &str) -> Vec<u8> {
    xlsx(&[
        (
            "Data",
            vec![
                vec![C::S("id"), C::S("region"), C::S("price")],
                vec![C::N(1.0), C::S("south"), C::N(1.35)],
                vec![C::N(2.0), C::S("east"), C::N(price)],
            ],
        ),
        (
            "Notes",
            vec![
                vec![C::S("topic"), C::S("text")],
                vec![C::S("a"), C::S(note)],
            ],
        ),
    ])
}

/// F1 (T-30): per-sheet rules inherit the keys of every sheet leniently. The reviewer's session (`[rules] keys =
/// ["id"]`, `[[rules.sheet]] sheet = "Notes"` with `ignore_case`) runs: exit 13, `Notes` compared without keys
/// (`rule_not_applied`, `keys_not_applied`, F9), its case change unimportant. A key written for the sheet stays strict.
#[test]
fn f1_inherited_keys_of_per_sheet_rules_are_lenient() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "left.xlsx", &data_and_notes(2.6, "hello"));
    let right = write(&w, "right.xlsx", &data_and_notes(2.7, "Hello"));
    let l = left.display().to_string().replace('\\', "\\\\");
    let r = right.display().to_string().replace('\\', "\\\\");
    std::fs::create_dir_all(tmp.path().join("config/sessions")).unwrap();
    std::fs::write(
        tmp.path().join("config/sessions/s1.toml"),
        format!(
            "schema_version = 1\ntype = \"table-compare\"\n\n[specs]\nleft = \"{l}\"\nright = \"{r}\"\n\n\
             [rules]\nkeys = [\"id\"]\n\n[[rules.sheet]]\nsheet = \"Notes\"\nignore_case = [\"\"]\n"
        ),
    )
    .unwrap();
    let SessionRunRequest::Table(req) = service
        .session_run_request(&SessionRef::parse("s1").unwrap())
        .unwrap()
    else {
        panic!("not a table request");
    };
    let resp = service
        .table_compare(&req, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.exit_code.code(), 13);
    let states: Vec<_> = resp
        .comparison
        .sheets()
        .iter()
        .map(|s| s.state().as_str())
        .collect();
    assert_eq!(states, ["important", "unimportant"]);
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(json["sheets"][0]["alignment"]["mode"], "keyed");
    assert_eq!(json["sheets"][1]["alignment"]["mode"], "engine");
    let codes: Vec<_> = resp.warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, ["rule_not_applied", "keys_not_applied"], "{codes:?}");
    assert!(
        resp.warnings[0].message.contains("sheet \"Notes\""),
        "{}",
        resp.warnings[0].message
    );
    // The keys written for Notes are strict.
    let own = TableCompareRequest {
        rules: TableRuleWords {
            sheet: vec![SheetRulesWords {
                sheet: "Notes".into(),
                rules: SheetRuleWords {
                    keys: vec!["id".into()],
                    ..SheetRuleWords::default()
                },
            }],
            ..req.rules.clone()
        },
        ..req.clone()
    };
    let e = compare(&service, &own).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
    assert!(e.to_string().contains("Notes"), "{e}");
}

/// F3 (T-44): the CSV preset reads a `;`-delimited `.csv` with `;` (the reviewer's file against its workbook: same);
/// `--delimiter ,` reads it as asked and warns `delimiter_mismatch`.
#[test]
fn f3_the_csv_preset_sniffs_another_delimiter() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let book = write(&w, "data.xlsx", &prices(2.0));
    let semi = write(
        &w,
        "data.csv",
        b"id;name;price;date;ok\n1;apple;1.50;2024-01-31;TRUE\n2;pear;2;2024-02-01;FALSE\n\
          3;plum;0.3;2024-02-02;TRUE\n",
    );
    let resp = service
        .table_compare(
            &request(&semi, &book),
            &job(&service, JobKind::TableCompare),
        )
        .unwrap();
    assert_eq!(
        resp.kind,
        ContentKind::RulesSame,
        "{:?}",
        resp.human_lines(None)
    );
    assert!(resp.warnings.is_empty(), "{:?}", resp.warnings);
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(
        json["format"]["left"]["delimiters"],
        serde_json::json!([";"])
    );
    let forced = TableCompareRequest {
        parse: TableFormatWords {
            delimiters: vec![",".into()],
            ..TableFormatWords::default()
        },
        ..request(&semi, &book)
    };
    let resp = service
        .table_compare(&forced, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.exit_code.code(), 13);
    let mismatch: Vec<_> = resp
        .warnings
        .iter()
        .filter(|w| w.code.as_str() == "delimiter_mismatch")
        .collect();
    assert_eq!(mismatch.len(), 1, "{:?}", resp.warnings);
    assert!(
        mismatch[0].message.contains("try --delimiter ';'"),
        "{}",
        mismatch[0].message
    );
}

/// F4, F5, F12: the human output leaves hidden columns out, says `same (unimportant differences)` for a changed row
/// with only unimportant cells under `--ignore-unimportant --display all`, counts the summary under `--iu`, and says
/// `1 row differs`.
#[test]
fn f4_f5_the_human_output_honours_hide_and_ignore_unimportant() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(
        &w,
        "a.csv",
        b"id,v,secret,w\n1,1.50,x,3\n2,7,y,4\n3,8,z,5\n",
    );
    let b = write(
        &w,
        "b.csv",
        b"id,v,secret,w\n1,1.51,x,3\n2,7,Y,9\n3,8,q,5\n",
    );
    let req = |iu: bool, hide: bool, display: &str| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                keys: vec!["id".into()],
                tolerance: vec!["v=0.02".into()],
                hide: if hide {
                    vec!["secret".into()]
                } else {
                    Vec::new()
                },
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ignore_unimportant: iu,
        display: Some(display.into()),
        ..request(&a, &b)
    };
    let lines = |iu: bool, hide: bool, display: &str| {
        service
            .table_compare(
                &req(iu, hide, display),
                &job(&service, JobKind::TableCompare),
            )
            .unwrap()
            .human_lines(None)
    };
    let plain = lines(false, false, "differences");
    assert!(
        plain.iter().any(|l| l.contains("secret: y \u{2192} Y")),
        "{plain:?}"
    );
    assert!(plain[0].contains("3 rows differ"), "{plain:?}");
    let hidden = lines(false, true, "differences");
    assert!(!hidden.iter().any(|l| l.contains("secret")), "{hidden:?}");
    assert!(
        hidden.contains(&"  row 4/4: differs in hidden columns only".to_owned()),
        "{hidden:?}"
    );
    assert!(
        hidden.iter().any(|l| l.ends_with("w: 4 \u{2192} 9")),
        "{hidden:?}"
    );
    let iu = lines(true, true, "all");
    assert!(
        iu.contains(&"  row 2/2: same (unimportant differences)".to_owned()),
        "{iu:?}"
    );
    assert!(iu[0].contains("2 rows differ"), "{iu:?}");
    let summary = iu.last().unwrap();
    assert_eq!(
        summary,
        "important differences: 1 of 1 sheet differs; rows: 0 same, 2 changed, 0 left only, 0 right only, 1 with \
         unimportant differences only"
    );
    // A sheet with unimportant differences only does not differ under --iu.
    let c = write(
        &w,
        "c.csv",
        b"id,v,secret,w\n1,1.51,x,3\n2,7,y,4\n3,8,z,5\n",
    );
    let only = TableCompareRequest {
        right: os(&c),
        ..req(true, false, "all")
    };
    let resp = service
        .table_compare(&only, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(
        resp.summary_line(),
        "same: 0 of 1 sheet differ; rows: 2 same, 0 changed, 0 left only, 0 right only, 1 with unimportant \
         differences only"
    );
    let one = TableCompareRequest {
        ignore_unimportant: false,
        ..only
    };
    let lines = service
        .table_compare(&one, &job(&service, JobKind::TableCompare))
        .unwrap()
        .human_lines(None);
    assert!(
        lines[0].starts_with("~ a.csv \u{2192} c.csv: 1 row differs;"),
        "{lines:?}"
    );
}

/// F6: a workbook the Spreadsheet conversion cannot read keeps the engine's kind and message (never "temporary
/// file"): `unsupported` for a password-protected one, `invalid_data` for a damaged one.
#[test]
fn f6_conversion_errors_keep_the_engines_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let mut cfb = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    cfb.resize(512, 0);
    cfb.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    let secret = write(&w, "secret.xlsx", &cfb);
    let mut truncated = prices(2.0);
    truncated.truncate(truncated.len() / 2);
    let broken = write(&w, "broken.xlsx", &truncated);
    let good = write(&w, "good.xlsx", &prices(2.0));
    for (bad, kind, words) in [
        (&secret, ErrorKind::Unsupported, "password-protected"),
        (&broken, ErrorKind::InvalidData, "broken.xlsx"),
    ] {
        let e = service
            .text_compare(
                &TextCompareRequest {
                    left: os(bad),
                    right: os(&good),
                    ..TextCompareRequest::default()
                },
                &job(&service, JobKind::TextCompare),
                None,
            )
            .unwrap_err();
        assert_eq!(e.kind(), kind, "{e}");
        let text = e.to_string();
        assert!(text.contains(words), "{text}");
        assert!(!text.contains("temporary file"), "{text}");
    }
}

/// F7: two identical workbooks are the same file in a folder comparison, even when they do not load (encrypted,
/// damaged); different ones that do not load are errors.
#[test]
fn f7_identical_bytes_are_the_same_before_parsing() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let mut cfb = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    cfb.resize(512, 0);
    cfb.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    for side in ["l", "r"] {
        write(&w, &format!("{side}/enc.xlsx"), &cfb);
        write(&w, &format!("{side}/bad.xlsx"), b"PK\x03\x04 damaged");
    }
    write(&w, "l/other.xlsx", b"PK\x03\x04 damaged");
    write(&w, "r/other.xlsx", b"PK\x03\x04 damaged too");
    let req = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("rules-based".into()),
        ..FolderCompareRequest::default()
    };
    let resp = service
        .folder_compare(&req, &job(&service, JobKind::FolderCompare), &NullObserver)
        .unwrap();
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let json = serde_json::to_value(resp.json(&display)).unwrap();
    let result = |path: &str| -> String {
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["path"] == path)
            .unwrap_or_else(|| panic!("{path}: {json}"))["content"]["result"]["result"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(result("enc.xlsx"), "binary_same");
    assert_eq!(result("bad.xlsx"), "binary_same");
    assert_eq!(result("other.xlsx"), "error");
}

/// F8, R2: `side.format` names a format of the Table kind only (a `.txt` matched by a text format is read with the
/// engine's preset: null); `format.<side>.detected` is true for a sniffed side, false for a CSV and a workbook.
#[test]
fn f8_side_format_is_a_table_format_or_null() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let txt = write(&w, "data.txt", b"id;v\n1;2\n");
    let csv = write(&w, "data.csv", b"id,v\n1,2\n");
    let resp = service
        .table_compare(&request(&txt, &csv), &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.kind, ContentKind::RulesSame);
    assert_eq!(resp.formats.left.format, None, "{:?}", resp.formats.left);
    assert_eq!(
        resp.formats
            .right
            .format
            .as_ref()
            .and_then(|f| f.id.as_deref()),
        Some("csv")
    );
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert!(json["left"]["format"].is_null(), "{}", json["left"]);
    assert_eq!(json["right"]["format"]["id"], "csv");
    assert_eq!(json["format"]["left"]["detected"], true);
    assert_eq!(json["format"]["right"]["detected"], false);
    let book = write(&w, "data.xlsx", &prices(2.0));
    let resp = service
        .table_compare(
            &request(&book, &book),
            &job(&service, JobKind::TableCompare),
        )
        .unwrap();
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(json["format"]["left"]["detected"], false);
}

/// R1, R3, R5: `--decimal` equal to the format's thousands separator clears it; `merged_cells` only with
/// `--verbose`; a read failure names its own side and spec.
#[test]
fn r1_r3_r5_words_warnings_and_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"k;v\nx;1,5\n");
    let b = write(&w, "b.csv", b"k;v\nx;1,50\n");
    let comma = TableCompareRequest {
        parse: TableFormatWords {
            delimiters: vec![";".into()],
            decimal: Some(",".into()),
            ..TableFormatWords::default()
        },
        ..request(&a, &b)
    };
    let resp = service
        .table_compare(&comma, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.formats.left.table.regional.thousands, None);
    assert_eq!(
        resp.kind,
        ContentKind::RulesSame,
        "{:?}",
        resp.human_lines(None)
    );
    // Merged cells.
    let merged = {
        let mut book = Workbook::new();
        let created = ExcelDateTime::from_ymd(2026, 1, 1).unwrap();
        book.set_properties(&DocProperties::new().set_creation_datetime(&created));
        let sheet = book.add_worksheet();
        sheet.write_string(0, 0, "a").unwrap();
        sheet.write_string(0, 1, "b").unwrap();
        sheet
            .merge_range(1, 0, 1, 1, "both", &Format::new())
            .unwrap();
        book.save_to_buffer().unwrap()
    };
    let m = write(&w, "m.xlsx", &merged);
    let codes = |verbose: bool| -> Vec<String> {
        service
            .table_compare(
                &TableCompareRequest {
                    verbose,
                    ..request(&m, &m)
                },
                &job(&service, JobKind::TableCompare),
            )
            .unwrap()
            .warnings
            .iter()
            .map(|w| w.code.as_str().to_owned())
            .collect()
    };
    assert!(codes(false).is_empty());
    assert_eq!(codes(true), ["merged_cells", "merged_cells"]);
    // A missing file (its stat fails) names its side; a read failure: `f15_an_unreadable_file_is_a_load_failure`.
    let missing = w.join("missing.csv");
    for (req, side) in [
        (request(&a, &missing), hikasa_core::Side::Right),
        (request(&missing, &a), hikasa_core::Side::Left),
    ] {
        match compare(&service, &req) {
            Err(ApiError::LoadFailed { side: s, spec, .. }) => {
                assert_eq!(s, side);
                assert!(spec.ends_with("missing.csv"), "{spec}");
            }
            other => panic!("{other:?}"),
        }
    }
}

/// F15: a file that cannot be read is `LoadFailed` with its side (Unix; skipped as root, who reads anything).
#[cfg(unix)]
#[test]
fn f15_an_unreadable_file_is_a_load_failure() {
    use std::os::unix::fs::PermissionsExt as _;
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id\n1\n");
    let locked = write(&w, "locked.csv", b"id\n1\n");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        eprintln!("skipped: the file is readable anyway (root)");
        return;
    }
    // The file is there (the stat works), its read fails: the engine's load failure names its own side and spec
    // (mutation R5).
    for (req, want) in [
        (request(&a, &locked), hikasa_core::Side::Right),
        (request(&locked, &a), hikasa_core::Side::Left),
    ] {
        match compare(&service, &req) {
            Err(e @ ApiError::LoadFailed { .. }) => {
                assert_eq!(e.kind(), ErrorKind::PermissionDenied, "{e}");
                let ApiError::LoadFailed { side, spec, .. } = e else {
                    unreachable!()
                };
                assert_eq!(side, want);
                assert!(spec.ends_with("locked.csv"), "{spec}");
            }
            other => panic!("{other:?}"),
        }
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// F13 (T-43): per-side parser words: a German CSV against an English one; `--type fixed` without widths is refused.
#[test]
fn f13_parser_words_per_side() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let de = write(&w, "de.csv", b"n;d\n1.234,5;31.01.2024\n");
    let en = write(&w, "en.csv", b"n,d\n\"1,234.5\",01/31/2024\n");
    let req = TableCompareRequest {
        side_parse: Pair::new(
            TableFormatWords {
                delimiters: vec![";".into()],
                decimal: Some(",".into()),
                thousands: Some(".".into()),
                date_order: Some("dmy".into()),
                ..TableFormatWords::default()
            },
            TableFormatWords {
                date_order: Some("mdy".into()),
                ..TableFormatWords::default()
            },
        ),
        ..request(&de, &en)
    };
    let resp = service
        .table_compare(&req, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(
        resp.kind,
        ContentKind::RulesSame,
        "{:?}",
        resp.human_lines(None)
    );
    // Both-sides words apply under the side's: the right side keeps `,` from them.
    assert!(matches!(
        &resp.formats.right.table.kind,
        hikasa_table::TableKind::Delimited(o) if o.delimiters == [',']
    ));
    let e = compare(
        &service,
        &TableCompareRequest {
            side_parse: Pair::new(
                TableFormatWords {
                    decimal: Some(",".into()),
                    thousands: Some(",".into()),
                    ..TableFormatWords::default()
                },
                TableFormatWords::default(),
            ),
            ..request(&de, &en)
        },
    )
    .unwrap_err();
    assert!(e.to_string().starts_with("left side: "), "{e}");
    let txt = write(&w, "t.txt", b"ab cd\n");
    let e = compare(
        &service,
        &TableCompareRequest {
            parse: TableFormatWords {
                kind: Some("fixed".into()),
                ..TableFormatWords::default()
            },
            ..request(&txt, &txt)
        },
    )
    .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    assert!(e.to_string().contains("--widths"), "{e}");
}

/// R12, R13 (T-43): a session's `[specs] left_encoding` and `right_encoding` are each side's; its `[display]
/// filter` is the display of `session run`.
#[test]
fn r12_r13_session_encodings_per_side_and_display() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let l = write(&w, "l1252.csv", b"id,name\n1,caf\xe9\n2,x\n");
    let r = write(&w, "rutf8.csv", "id,name\n1,café\n2,x\n".as_bytes());
    let path = |p: &Path| p.display().to_string().replace('\\', "\\\\");
    std::fs::create_dir_all(tmp.path().join("config/sessions")).unwrap();
    std::fs::write(
        tmp.path().join("config/sessions/enc.toml"),
        format!(
            "schema_version = 1\ntype = \"table-compare\"\n\n[specs]\nleft = \"{}\"\nright = \"{}\"\n\
             left_encoding = \"windows-1252\"\n\n[rules.right_parse]\nfirst_line = \"names\"\n\n\
             [display]\nfilter = \"all\"\n",
            path(&l),
            path(&r)
        ),
    )
    .unwrap();
    let SessionRunRequest::Table(req) = service
        .session_run_request(&SessionRef::parse("enc").unwrap())
        .unwrap()
    else {
        panic!("not a table request");
    };
    assert_eq!(
        req.side_parse.left.encoding.as_deref(),
        Some("windows-1252")
    );
    assert_eq!(req.side_parse.right.encoding, None);
    // `[rules.right_parse]` is the right side's only (mutation Q19).
    assert_eq!(req.side_parse.right.first_line.as_deref(), Some("names"));
    assert_eq!(req.side_parse.left.first_line, None);
    assert_eq!(req.display.as_deref(), Some("all"));
    let resp = service
        .table_compare(&req, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(
        resp.kind,
        ContentKind::RulesSame,
        "{:?}",
        resp.human_lines(None)
    );
    let lines = resp.human_lines(None);
    assert!(lines.contains(&"  row 3/3: same".to_owned()), "{lines:?}");
}

/// T-45: the regional settings of Windows (`reg query`) and macOS (`AppleLocale`), read through the host's runner when
/// no locale variable says them; a locale variable wins.
#[test]
fn t45_regional_settings_of_the_operating_system() {
    use hikasa_session::api::{regional_of_apple_locale, regional_of_windows_international};
    use hikasa_table::DateOrder;
    let reg = "\r\nHKEY_CURRENT_USER\\Control Panel\\International\r\n    Locale    REG_SZ    00000407\r\n    \
               LocaleName    REG_SZ    de-DE\r\n    sDecimal    REG_SZ    ,\r\n    sThousand    REG_SZ    \r\n    \
               sShortDate    REG_SZ    yyyy-MM-dd\r\n";
    let r = regional_of_windows_international(reg).unwrap();
    assert_eq!(
        (r.decimal, r.thousands, r.date_order, r.date_sep),
        (',', None, DateOrder::Ymd, Some('-'))
    );
    let us = "    LocaleName    REG_SZ    en-US\r\n    sShortDate    REG_SZ    M/d/yyyy\r\n    sThousand    REG_SZ     \r\n";
    let r = regional_of_windows_international(us).unwrap();
    assert_eq!(
        (r.decimal, r.thousands, r.date_order, r.date_sep),
        ('.', Some(' '), DateOrder::Mdy, Some('/'))
    );
    // The user's own separators and date pattern win over the locale's row (mutation Q17).
    let own = "    LocaleName    REG_SZ    en-US\r\n    sDecimal    REG_SZ    ,\r\n    sThousand    REG_SZ    .\r\n    \
               sShortDate    REG_SZ    dd.MM.yyyy\r\n";
    let r = regional_of_windows_international(own).unwrap();
    assert_eq!(
        (r.decimal, r.thousands, r.date_order, r.date_sep),
        (',', Some('.'), DateOrder::Dmy, Some('.'))
    );
    assert_eq!(regional_of_windows_international("nothing"), None);
    let ch = regional_of_apple_locale("de_CH\n").unwrap();
    assert_eq!((ch.decimal, ch.thousands), ('.', Some('\'')));
    let rg = regional_of_apple_locale("en_US@rg=dezzzz").unwrap();
    assert_eq!((rg.decimal, rg.date_order), (',', DateOrder::Dmy));
    assert_eq!(regional_of_apple_locale(" \n"), None);

    /// A runner whose host query answers with the canned text.
    struct Answer(&'static str, std::sync::atomic::AtomicUsize);
    impl hikasa_formats::ProcessRunner for Answer {
        fn run(
            &self,
            _: &[std::ffi::OsString],
            _: Option<std::time::Duration>,
            _: &hikasa_core::CancelToken,
        ) -> Result<i32, hikasa_formats::ConvertError> {
            Ok(1)
        }
        fn output(
            &self,
            argv: &[std::ffi::OsString],
            _: Option<std::time::Duration>,
            _: &hikasa_core::CancelToken,
        ) -> Result<Vec<u8>, hikasa_formats::ConvertError> {
            assert_eq!(argv[0], "reg");
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.0.as_bytes().to_vec())
        }
    }
    let windows = |numeric: Option<&str>| {
        let mut env = facts(tmp_root().path());
        env.platform = hikasa_session::host::Platform::Windows;
        env.locale_numeric = numeric.map(str::to_owned);
        let runner = std::sync::Arc::new(Answer(reg, std::sync::atomic::AtomicUsize::new(0)));
        let init = ServiceInit {
            threads: std::num::NonZeroUsize::new(1),
            ..ServiceInit::new(env, fixed_clock(), runner.clone())
        };
        (Service::new(init).unwrap(), runner)
    };
    let (service, runner) = windows(None);
    let r = service.system_regional();
    assert_eq!((r.decimal, r.date_order), (',', DateOrder::Ymd));
    let _ = service.system_regional();
    assert_eq!(
        runner.1.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "queried once"
    );
    let (service, _) = windows(Some("en_US.UTF-8"));
    let r = service.system_regional();
    assert_eq!(
        (r.decimal, r.date_order),
        ('.', DateOrder::Ymd),
        "numbers from the variable, dates from the system"
    );
}

/// A temporary folder that lives as long as the test process (the services of a test hold paths in it).
fn tmp_root() -> &'static tempfile::TempDir {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| tempfile::tempdir().unwrap())
}

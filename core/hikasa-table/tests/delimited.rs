//! AT.1, AT.2: the delimited and fixed parsers, sniffing, header detection, P-DELIM.

mod common;

use hikasa_core::CancelToken;
use hikasa_table::{
    DelimitedOptions, FirstLine, FixedOptions, LoadOptions, Regional, Sheet, TableFormat,
    TableKind, TableWarningCode, Workbook, load_bytes, write_delimited,
};
use proptest::prelude::*;

fn load(text: &[u8], kind: TableKind, first: FirstLine) -> Workbook {
    let format = TableFormat {
        kind,
        first_line: first,
        ..TableFormat::default()
    };
    load_bytes(
        text.to_vec(),
        "t.txt",
        &format,
        &LoadOptions::default(),
        &CancelToken::new(),
    )
    .unwrap()
}

fn rows_of(sheet: &Sheet) -> Vec<Vec<String>> {
    (0..sheet.row_count())
        .map(|r| {
            sheet
                .row(r)
                .values()
                .map(|v| v.display().into_owned())
                .collect()
        })
        .collect()
}

fn parse(text: &str, opts: DelimitedOptions) -> Vec<Vec<String>> {
    rows_of(&load(text.as_bytes(), TableKind::Delimited(opts), FirstLine::Data).sheets()[0])
}

fn v(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

#[test]
fn qualified_fields_hold_delimiters_line_ends_and_doubled_qualifiers() {
    let rows = parse(
        "a,\"b,c\",\"x \"\"q\"\" y\"\r\n\"multi\nline\",2\r3,\"\"\n",
        DelimitedOptions::with(','),
    );
    assert_eq!(
        rows,
        v(&[&["a", "b,c", "x \"q\" y"], &["multi\nline", "2"], &["3"]])
    );
    let wb = load(
        b"1,2\n\"a\nb\",3\n4,5\n",
        TableKind::Delimited(DelimitedOptions::with(',')),
        FirstLine::Data,
    );
    let s = &wb.sheets()[0];
    assert_eq!((s.row_label(0), s.row_label(1), s.row_label(2)), (1, 2, 4));
}

#[test]
fn stray_text_and_unterminated_qualifiers_are_kept_with_warnings() {
    let wb = load(
        b"\"ab\"cd,e\n\"open,f\n",
        TableKind::Delimited(DelimitedOptions::with(',')),
        FirstLine::Data,
    );
    assert_eq!(
        rows_of(&wb.sheets()[0]),
        v(&[&["abcd", "e"], &["open,f\n"]])
    );
    let codes: Vec<_> = wb.warnings().iter().map(|w| w.code).collect();
    assert_eq!(
        codes,
        vec![
            TableWarningCode::StrayAfterQualifier,
            TableWarningCode::UnterminatedQualifier
        ]
    );
}

#[test]
fn consecutive_delimiters_trimming_and_empty_fields() {
    let plain = DelimitedOptions::with(',');
    assert_eq!(
        parse("a,,b\na,\n,\n", plain.clone()),
        v(&[&["a", "", "b"], &["a"], &[]])
    );
    let consecutive = DelimitedOptions {
        delimiters: vec![' ', ';'],
        consecutive_as_one: true,
        ..plain.clone()
    };
    assert_eq!(
        parse("  a  ;; b  \nc\n", consecutive),
        v(&[&["a", "b"], &["c"]])
    );
    let trim = DelimitedOptions {
        trim_whitespace: true,
        ..plain.clone()
    };
    assert_eq!(parse(" a , \"b \" , c \n", trim), v(&[&["a", "b ", "c"]]));
    assert_eq!(parse(" a , b \n", plain.clone()), v(&[&[" a ", " b "]]));
    // Empty lines stay rows; a final line end opens none.
    assert_eq!(parse("a\n\nb\n", plain.clone()), v(&[&["a"], &[], &["b"]]));
    // A qualifier inside an unqualified field is data; no qualifier at all.
    assert_eq!(parse("a\"b,c\n", plain), v(&[&["a\"b", "c"]]));
    let none = DelimitedOptions {
        qualifier: None,
        ..DelimitedOptions::with(';')
    };
    assert_eq!(parse("\"a;b\"\n", none), v(&[&["\"a", "b\""]]));
}

#[test]
fn line_ends_ctrl_z_and_encodings() {
    let wb = load(
        b"a\rb\r\nc\nd",
        TableKind::Delimited(DelimitedOptions::with(',')),
        FirstLine::Data,
    );
    assert_eq!(
        rows_of(&wb.sheets()[0]),
        v(&[&["a"], &["b"], &["c"], &["d"]])
    );
    let format = TableFormat {
        kind: TableKind::Delimited(DelimitedOptions::with(',')),
        first_line: FirstLine::Data,
        ctrl_z_eof: true,
        ..TableFormat::default()
    };
    let wb = load_bytes(
        b"a,b\n\x1Ajunk\n".to_vec(),
        "t",
        &format,
        &LoadOptions::default(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(rows_of(&wb.sheets()[0]), v(&[&["a", "b"]]));
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("é,ü\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let wb = load(
        &utf16,
        TableKind::Delimited(DelimitedOptions::with(',')),
        FirstLine::Data,
    );
    assert_eq!(wb.encoding(), Some("UTF-16LE"));
    assert_eq!(rows_of(&wb.sheets()[0]), v(&[&["é", "ü"]]));
    let latin = TableFormat {
        encoding: Some("windows-1252".into()),
        first_line: FirstLine::Data,
        kind: TableKind::Delimited(DelimitedOptions::with(';')),
        ..TableFormat::default()
    };
    let wb = load_bytes(
        b"caf\xE9;\x80\n".to_vec(),
        "t",
        &latin,
        &LoadOptions::default(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(rows_of(&wb.sheets()[0]), v(&[&["café", "€"]]));
    // Non-ASCII delimiters and qualifiers.
    let odd = DelimitedOptions {
        delimiters: vec!['¦'],
        qualifier: Some('«'),
        ..DelimitedOptions::default()
    };
    assert_eq!(parse("«a¦b«¦é¦««\n", odd), v(&[&["a¦b", "é"]]));
}

#[test]
fn fixed_widths_cut_by_characters() {
    let wb = load(
        "ab  cdé  f extra\nx\n".as_bytes(),
        TableKind::Fixed(FixedOptions { widths: vec![4, 5] }),
        FirstLine::Data,
    );
    assert_eq!(
        rows_of(&wb.sheets()[0]),
        v(&[&["ab", "cdé", "f extra"], &["x"]])
    );
}

#[test]
fn detect_sniffs_and_reports_the_format() {
    let wb = load(
        b"id;price\n1;2,50\n2;3,75\n",
        TableKind::Detect,
        FirstLine::Detect,
    );
    match &wb.detected().kind {
        TableKind::Delimited(o) => assert_eq!(o.delimiters, vec![';']),
        other => panic!("{other:?}"),
    }
    let s = &wb.sheets()[0];
    assert_eq!(s.header(), Some(&["id".to_owned(), "price".to_owned()][..]));
    let wb = load(
        b"code  qty\nA1    10\nB22   3\n",
        TableKind::Detect,
        FirstLine::Detect,
    );
    assert!(matches!(wb.detected().kind, TableKind::Fixed(_)));
    assert_eq!(rows_of(&wb.sheets()[0]), v(&[&["A1", "10"], &["B22", "3"]]));
}

#[test]
fn header_detection_votes() {
    let h = |text: &str| {
        load(
            text.as_bytes(),
            TableKind::Delimited(DelimitedOptions::with(',')),
            FirstLine::Detect,
        )
        .sheets()[0]
            .header()
            .is_some()
    };
    assert!(h("name,amount\nx,1\ny,2\n"));
    assert!(h("code,city\nAB1,Rome\nCD2,Oslo\n"));
    assert!(!h("1,2\n3,4\n"));
    assert!(!h("a,b\na,c\n"));
    assert!(!h("only,row\n"));
    let wb = load(
        b"a,b\nx,y\n",
        TableKind::Delimited(DelimitedOptions::with(',')),
        FirstLine::Names,
    );
    let s = &wb.sheets()[0];
    assert_eq!((s.header().is_some(), s.row_count()), (true, 1));
    assert_eq!(s.column_name(1), "b");
}

#[test]
fn regional_settings_type_the_columns() {
    let format = TableFormat {
        kind: TableKind::Delimited(DelimitedOptions::with(';')),
        first_line: FirstLine::Names,
        regional: Regional {
            decimal: ',',
            thousands: Some('.'),
            date_order: hikasa_table::DateOrder::Dmy,
            date_sep: Some('.'),
        },
        ..TableFormat::default()
    };
    let wb = load_bytes(
        b"n;d;t\n1.234,5;31.01.2024;x\n2;01.02.2024;y\n".to_vec(),
        "t",
        &format,
        &LoadOptions::default(),
        &CancelToken::new(),
    )
    .unwrap();
    let s = &wb.sheets()[0];
    use hikasa_table::ColumnKind;
    assert_eq!(
        (s.column_kind(0), s.column_kind(1), s.column_kind(2)),
        (ColumnKind::Number, ColumnKind::DateTime, ColumnKind::Text)
    );
}

fn field() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just('a'),
            Just('Z'),
            Just(','),
            Just(';'),
            Just('"'),
            Just('\''),
            Just(' '),
            Just('\t'),
            Just('\n'),
            Just('\r'),
            Just('é'),
            Just('1'),
        ],
        0..8,
    )
    .prop_map(|c| c.into_iter().collect())
}

fn trimmed(rows: &[Vec<String>], trim: bool) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| {
            let mut r: Vec<String> = r
                .iter()
                .map(|f| {
                    if trim {
                        f.trim_matches([' ', '\t']).to_owned()
                    } else {
                        f.clone()
                    }
                })
                .collect();
            while r.last().is_some_and(String::is_empty) {
                r.pop();
            }
            r
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(512), ..ProptestConfig::default() })]

    /// P-DELIM: `parse(write(rows)) == rows` for every option set with a qualifier.
    #[test]
    fn p_delim_round_trip(
        rows in prop::collection::vec(prop::collection::vec(field(), 0..5), 1..6),
        delim in prop_oneof![Just(','), Just(';'), Just('\t')],
        qualifier in prop_oneof![Just('"'), Just('\'')],
        trim in any::<bool>(),
        consecutive in any::<bool>(),
    ) {
        let opts = DelimitedOptions {
            delimiters: vec![delim],
            qualifier: Some(qualifier),
            consecutive_as_one: consecutive,
            trim_whitespace: trim,
        };
        // Consecutive delimiters cannot express empty fields.
        prop_assume!(!consecutive || rows.iter().all(|r| r.iter().all(|f| !f.is_empty())));
        // With trimming, blank-only fields are empty fields.
        prop_assume!(!(consecutive && trim) || rows.iter().all(|r| r.iter().all(|f| !f.trim_matches([' ', '\t']).is_empty())));
        let text = write_delimited(&rows, &opts).unwrap();
        let parsed = parse(&text, opts);
        prop_assert_eq!(trimmed(&parsed, false), trimmed(&rows, false), "text {:?}", text);
    }
}

//! Tokenizer unit tests: the escape table of §2.10.3, D-13 trimming, multi-line constructs and the edge cases
//! listed in §6.2.4. Every case is also run in reference mode (no fast paths) and must agree.

use std::sync::Arc;

use super::*;
use crate::compile::CompiledFormat;
use crate::regex_engine::RegexLimits;
use crate::toml_io::parse_format;

fn format(body: &str) -> CompiledFormat {
    let text = format!("schema_version = 1\nid = \"t\"\nname = \"T\"\nkind = \"text\"\n{body}");
    let (f, warnings) = parse_format(&text, "test").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap()
}

/// Tokenizes `lines` from `Normal`; returns per line the `(element, text)` pairs, checking INV-TOK-1..3 and
/// that reference mode agrees.
fn run(f: &CompiledFormat, lines: &[&str]) -> Vec<Vec<(String, String)>> {
    let g = f.grammar();
    let mut fast = Tokenizer::new(g);
    let mut slow = Tokenizer::reference(g);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut state = LineState::Normal;
    let mut out = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let oa = fast.tokenize_line(line, k as u32, state, &mut a);
        let ob = slow.tokenize_line(line, k as u32, state, &mut b);
        assert_eq!(a, b, "reference mode differs on {line:?}");
        assert_eq!(oa.end_state, ob.end_state);
        check_invariants(line, &a);
        out.push(
            a.iter()
                .map(|t| {
                    (
                        g.elements().name(t.element).to_owned(),
                        t.text(line).to_owned(),
                    )
                })
                .collect(),
        );
        state = oa.end_state;
    }
    out
}

fn check_invariants(line: &str, tokens: &[Token]) {
    let mut pos = 0u32;
    for (k, t) in tokens.iter().enumerate() {
        assert!(t.end > t.start, "INV-TOK-1 {tokens:?}");
        assert_eq!(t.start, pos, "INV-TOK-2 {tokens:?}");
        assert!(line.is_char_boundary(t.start as usize) && line.is_char_boundary(t.end as usize));
        assert_eq!(t.item.is_none(), t.element == ElementId::EVERYTHING_ELSE);
        if k > 0 {
            assert!(
                !(t.item.is_none() && tokens[k - 1].item.is_none()),
                "INV-TOK-3 {tokens:?}"
            );
        }
        pos = t.end;
    }
    assert_eq!(pos as usize, line.len(), "INV-TOK-2 coverage {tokens:?}");
}

fn one(f: &CompiledFormat, line: &str) -> Vec<(String, String)> {
    run(f, &[line]).remove(0)
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(e, t)| ((*e).to_owned(), (*t).to_owned()))
        .collect()
}

const STRINGS: &str = r#"
[[grammar]]
element = "String"
kind = "delimited"
from = '"'
to = '"'
escape = '\'
stop_at_eol = true
"#;

#[test]
fn escape_table() {
    let c = format(STRINGS);
    assert_eq!(
        one(&c, r#"s = "a\"b" + c"#),
        pairs(&[
            ("Everything Else", "s = "),
            ("String", r#""a\"b""#),
            ("Everything Else", " + c")
        ])
    );
    assert_eq!(
        one(&c, r#""a\\" x"#),
        pairs(&[("String", r#""a\\""#), ("Everything Else", " x")])
    );
    assert_eq!(one(&c, r#""abc"#), pairs(&[("String", r#""abc"#)]));
    let sql = format(
        "[[grammar]]\nelement = \"String\"\nkind = \"delimited\"\nfrom = \"'\"\nto = \"'\"\nescape = \"'\"\n",
    );
    assert_eq!(one(&sql, "'it''s'"), pairs(&[("String", "'it''s'")]));
    assert_eq!(
        one(&sql, "'abc' x"),
        pairs(&[("String", "'abc'"), ("Everything Else", " x")])
    );
    let vb = format(
        "[[grammar]]\nelement = \"String\"\nkind = \"delimited\"\nfrom = '\"'\nto = '\"'\nescape = '\"'\n",
    );
    assert_eq!(
        one(&vb, r#""say ""hi""""#),
        pairs(&[("String", r#""say ""hi""""#)])
    );
}

const C_LIKE: &str = r#"
[[grammar]]
element = "Comment"
kind = "delimited"
from = "//"
stop_at_eol = true

[[grammar]]
element = "Comment"
kind = "delimited"
from = "/*"
to = "*/"
"#;

#[test]
fn multi_line_delimited_with_d13_trimming() {
    let c = format(C_LIKE);
    let out = run(&c, &["x /* a", "   b */ y"]);
    assert_eq!(
        out[0],
        pairs(&[("Everything Else", "x "), ("Comment", "/* a")])
    );
    assert_eq!(
        out[1],
        pairs(&[
            ("Everything Else", "   "),
            ("Comment", "b */"),
            ("Everything Else", " y")
        ])
    );
    assert_eq!(
        one(&c, "x = 1; // c   "),
        pairs(&[
            ("Everything Else", "x = 1; "),
            ("Comment", "// c"),
            ("Everything Else", "   ")
        ])
    );
    // A line inside the comment: leading and trailing whitespace stay outside (D-13); a blank line is EE.
    let out = run(&c, &["/* start  ", "   middle  ", "   ", "", "end */"]);
    assert_eq!(
        out[0],
        pairs(&[("Comment", "/* start"), ("Everything Else", "  ")])
    );
    assert_eq!(
        out[1],
        pairs(&[
            ("Everything Else", "   "),
            ("Comment", "middle"),
            ("Everything Else", "  ")
        ])
    );
    assert_eq!(out[2], pairs(&[("Everything Else", "   ")]));
    assert!(out[3].is_empty());
    assert_eq!(out[4], pairs(&[("Comment", "end */")]));
}

#[test]
fn line_states() {
    let c = format(C_LIKE);
    let g = c.grammar();
    assert!(g.has_multiline());
    let mut t = Tokenizer::new(g);
    let mut out = Vec::new();
    let o = t.tokenize_line("a /* b", 0, LineState::Normal, &mut out);
    assert_eq!(o.end_state, LineState::InDelimited { item: 1 });
    assert_eq!(o.regex_errors, 0);
    let o = t.tokenize_line("c */ d", 1, o.end_state, &mut out);
    assert_eq!(o.end_state, LineState::Normal);
    // States that do not fit the grammar are treated as Normal (T9).
    for bogus in [
        LineState::InDelimited { item: 0 },
        LineState::InDelimited { item: 99 },
        LineState::InLines {
            item: 1,
            remaining: 3,
        },
    ] {
        let o = t.tokenize_line("x */ y", 0, bogus, &mut out);
        assert_eq!(o.end_state, LineState::Normal);
        assert_eq!(out.len(), 1, "{bogus:?}: {out:?}");
    }
    assert_eq!(
        serde_json::to_string(&LineState::InLines {
            item: 2,
            remaining: 5
        })
        .unwrap(),
        r#"{"state":"in_lines","item":2,"remaining":5}"#
    );
}

#[test]
fn empty_line_and_zero_length_regex() {
    let c = format(
        "[[grammar]]\nelement = \"Z\"\nkind = \"basic\"\ntext = 'x*'\nregex = true\n\
         [[grammar]]\nelement = \"W\"\nkind = \"basic\"\ntext = '\\b'\nregex = true\n",
    );
    assert!(one(&c, "").is_empty());
    // `x*` matches empty everywhere except at an x: an empty match is no match (INV-TOK-1).
    assert_eq!(
        one(&c, "abxxc"),
        pairs(&[
            ("Everything Else", "ab"),
            ("Z", "xx"),
            ("Everything Else", "c")
        ])
    );
}

#[test]
fn columns() {
    let c = format(
        "[[grammar]]\nelement = \"Seq\"\nkind = \"columns\"\nfrom_column = 1\nto_column = 3\n\
         [[grammar]]\nelement = \"Tail\"\nkind = \"columns\"\nfrom_column = 6\n",
    );
    assert_eq!(
        one(&c, "123456789"),
        pairs(&[("Seq", "123"), ("Everything Else", "45"), ("Tail", "6789")])
    );
    // Past the end of the line: the token is clamped; a to-EOL column item trims trailing whitespace.
    assert_eq!(one(&c, "1"), pairs(&[("Seq", "1")]));
    assert_eq!(
        one(&c, "12345x  "),
        pairs(&[
            ("Seq", "123"),
            ("Everything Else", "45"),
            ("Tail", "x"),
            ("Everything Else", "  ")
        ])
    );
    assert_eq!(
        one(&c, "1234      "),
        pairs(&[("Seq", "123"), ("Everything Else", "4      ")])
    );
    // Columns count chars, not bytes.
    assert_eq!(
        one(&c, "ééé45é"),
        pairs(&[("Seq", "ééé"), ("Everything Else", "45"), ("Tail", "é")])
    );
}

/// A Columns item starts a field: no token of another item crosses its first column, an end-of-line token
/// ends (trimmed, D-13) at the boundary, the scan goes on in the field, and a multi-line construct cut at the
/// boundary still continues on the next line (§2.10.2; wave-2 review: COBOL columns 73–80).
#[test]
fn columns_are_field_boundaries() {
    let c = format(
        r#"
[[grammar]]
element = "Block"
kind = "delimited"
from = "/*"
to = "*/"

[[grammar]]
element = "Area"
kind = "columns"
from_column = 11
to_column = 14

[[grammar]]
element = "String"
kind = "delimited"
from = '"'
to = '"'
stop_at_eol = true

[[grammar]]
element = "Word"
kind = "list"
tokens = ["ab"]

[[grammar]]
element = "Id"
kind = "basic"
text = '[a-z]+'
regex = true

[[grammar]]
element = "Comment"
kind = "basic"
text = '#.*'
regex = true
"#,
    );
    // An identifier reaching column 10 stops there; columns 11–14 are the Area.
    assert_eq!(
        one(&c, "x abcdefghWXYZ"),
        pairs(&[
            ("Id", "x"),
            ("Everything Else", " "),
            ("Id", "abcdefgh"),
            ("Area", "WXYZ"),
        ])
    );
    // An unterminated string ends at the boundary minus trailing blanks; the closing quote in the Area does
    // not count.
    assert_eq!(
        one(&c, "\"abc      WX\"Z"),
        pairs(&[
            ("String", "\"abc"),
            ("Everything Else", "      "),
            ("Area", "WX\"Z"),
        ])
    );
    // Word boundaries and `.*` stop at the boundary too: `ab` is a whole word there.
    assert_eq!(
        one(&c, "        abWXYZ"),
        pairs(&[
            ("Everything Else", "        "),
            ("Word", "ab"),
            ("Area", "WXYZ")
        ])
    );
    assert_eq!(
        one(&c, "x # note  ABCD and more"),
        pairs(&[
            ("Id", "x"),
            ("Everything Else", " "),
            ("Comment", "# note  "),
            ("Area", "ABCD"),
            ("Everything Else", " "),
            ("Id", "and"),
            ("Everything Else", " "),
            ("Id", "more"),
        ])
    );
    // A block comment cut at the boundary continues on the next line; the Area is tokenised meanwhile.
    let lines = run(&c, &["a /* open WXYZ", "  end */ b"]);
    assert_eq!(
        lines[0],
        pairs(&[
            ("Id", "a"),
            ("Everything Else", " "),
            ("Block", "/* open"),
            ("Everything Else", " "),
            ("Area", "WXYZ"),
        ])
    );
    assert_eq!(
        lines[1],
        pairs(&[
            ("Everything Else", "  "),
            ("Block", "end */"),
            ("Everything Else", " "),
            ("Id", "b"),
        ])
    );
    // Lines shorter than the boundary are unaffected; columns count chars.
    assert_eq!(one(&c, "\"ééééééééé"), pairs(&[("String", "\"ééééééééé")]));
    assert_eq!(
        one(&c, "éé \"ééééééWX"),
        pairs(&[
            ("Everything Else", "éé "),
            ("String", "\"éééééé"),
            ("Area", "WX"),
        ])
    );
}

/// A fancy-regex runtime-limit hit never hides a later match on the same line, and the fast paths count the
/// same hits as the reference algorithm: the unanchored search that hit the limit is not counted, the anchored
/// attempts are (wave-2 review: the error used to be cached as "no match for the rest of the line").
#[test]
fn runtime_limit_errors_never_hide_later_matches() {
    let text = "schema_version = 1\nid = \"t\"\nname = \"T\"\nkind = \"text\"\n[[grammar]]\nelement = \"X\"\n\
                kind = \"basic\"\ntext = '(a*)*\\1b'\nregex = true\n";
    let (f, _) = parse_format(text, "t").unwrap();
    let limits = RegexLimits {
        size_limit: 8 << 20,
        backtrack_limit: 20_000,
    };
    let c = CompiledFormat::compile(Arc::new(f), &limits).unwrap();
    let g = c.grammar();
    let line = format!("{} b", "a".repeat(30));
    let (mut fast, mut slow) = (Tokenizer::new(g), Tokenizer::reference(g));
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let oa = fast.tokenize_line(&line, 0, LineState::Normal, &mut a);
    let ob = slow.tokenize_line(&line, 0, LineState::Normal, &mut b);
    assert_eq!(a, b);
    assert_eq!(oa, ob);
    assert!(oa.regex_errors > 0);
    assert_eq!(a.last().map(|t| t.text(&line)), Some("b"));
    assert_eq!(a.last().map(|t| t.item), Some(Some(0)));
}

#[test]
fn lines_items() {
    let c = format(
        "[[grammar]]\nelement = \"Header\"\nkind = \"lines\"\nheading = \"PAGE\"\nnext_lines = 2\n\
         or_line_one = true\n",
    );
    let out = run(&c, &["title  ", " x ", "y", "z", "a PAGE 2", "", "w"]);
    assert_eq!(
        out[0],
        pairs(&[("Header", "title"), ("Everything Else", "  ")])
    );
    assert_eq!(
        out[1],
        pairs(&[
            ("Everything Else", " "),
            ("Header", "x"),
            ("Everything Else", " ")
        ])
    );
    assert_eq!(out[2], pairs(&[("Header", "y")]));
    assert_eq!(out[3], pairs(&[("Everything Else", "z")]));
    assert_eq!(out[4], pairs(&[("Header", "a PAGE 2")]));
    assert!(out[5].is_empty());
    assert_eq!(out[6], pairs(&[("Header", "w")]));
    // Without or_line_one, line 1 is ordinary.
    let c = format(
        "[[grammar]]\nelement = \"Header\"\nkind = \"lines\"\nheading = 'pa+ge'\nregex = true\n\
         match_case = false\n",
    );
    let out = run(&c, &["title", "PAAGE 1"]);
    assert_eq!(out[0], pairs(&[("Everything Else", "title")]));
    assert_eq!(out[1], pairs(&[("Header", "PAAGE 1")]));
}

const KEYWORDS: &str = r##"
[[grammar]]
element = "Keyword"
kind = "list"
tokens = ["in", "int", "<=", "<", "#include", "@end", "non-sealed"]

[[grammar]]
element = "Identifier"
kind = "basic"
text = '[^\W\d]\w*'
regex = true
"##;

#[test]
fn lists_longest_match_and_word_boundaries() {
    let c = format(KEYWORDS);
    assert_eq!(
        one(&c, "int x"),
        pairs(&[
            ("Keyword", "int"),
            ("Everything Else", " "),
            ("Identifier", "x")
        ])
    );
    assert_eq!(one(&c, "into"), pairs(&[("Identifier", "into")]));
    assert_eq!(
        one(&c, "a<=b"),
        pairs(&[("Identifier", "a"), ("Keyword", "<="), ("Identifier", "b")])
    );
    assert_eq!(
        one(&c, "#include x"),
        pairs(&[
            ("Keyword", "#include"),
            ("Everything Else", " "),
            ("Identifier", "x")
        ])
    );
    assert_eq!(
        one(&c, "#includes"),
        pairs(&[("Everything Else", "#"), ("Identifier", "includes")])
    );
    assert_eq!(
        one(&c, "x@end"),
        pairs(&[("Identifier", "x"), ("Keyword", "@end")])
    );
    assert_eq!(
        one(&c, "non-sealed class"),
        pairs(&[
            ("Keyword", "non-sealed"),
            ("Everything Else", " "),
            ("Identifier", "class")
        ])
    );
    assert_eq!(
        one(&c, "xnon-sealed"),
        pairs(&[
            ("Identifier", "xnon"),
            ("Everything Else", "-"),
            ("Identifier", "sealed")
        ])
    );
    // Unicode identifiers and word boundaries: "in" inside "iné" is not a keyword.
    assert_eq!(one(&c, "inéz"), pairs(&[("Identifier", "inéz")]));
    assert_eq!(
        one(&c, "größe in"),
        pairs(&[
            ("Identifier", "größe"),
            ("Everything Else", " "),
            ("Keyword", "in")
        ])
    );
    assert_eq!(
        one(&c, "日本 in"),
        pairs(&[
            ("Identifier", "日本"),
            ("Everything Else", " "),
            ("Keyword", "in")
        ])
    );
}

#[test]
fn word_chars_join_words() {
    let c = format(
        "[text]\nword_chars = \"-\"\n[elements]\nKeyword = { case_sensitive = false }\n\
         [[grammar]]\nelement = \"Keyword\"\nkind = \"list\"\nmatch_case = false\n\
         tokens = [\"END\", \"END-IF\", \"IF\", \"OF\"]\n\
         [[grammar]]\nelement = \"Identifier\"\nkind = \"basic\"\ntext = '[A-Za-z][A-Za-z0-9-]*'\nregex = true\n",
    );
    assert_eq!(
        one(&c, "end-if."),
        pairs(&[("Keyword", "end-if"), ("Everything Else", ".")])
    );
    assert_eq!(
        one(&c, "END-OF-FILE"),
        pairs(&[("Identifier", "END-OF-FILE")])
    );
    assert_eq!(
        one(&c, "IF END"),
        pairs(&[
            ("Keyword", "IF"),
            ("Everything Else", " "),
            ("Keyword", "END")
        ])
    );
    assert!(!c.grammar().elements().case_sensitive(ElementId(1)));
}

#[test]
fn regex_from_and_to() {
    let c = format(
        "[[grammar]]\nelement = \"Pod\"\nkind = \"delimited\"\nfrom = '^=[A-Za-z]\\w*'\nto = '^=cut\\b.*'\n\
         regex = true\n",
    );
    let out = run(&c, &["x =head1", "=head1 NAME", "text =cut", "=cut", "y"]);
    assert_eq!(out[0], pairs(&[("Everything Else", "x =head1")]));
    assert_eq!(out[1], pairs(&[("Pod", "=head1 NAME")]));
    assert_eq!(out[2], pairs(&[("Pod", "text =cut")]));
    assert_eq!(out[3], pairs(&[("Pod", "=cut")]));
    assert_eq!(out[4], pairs(&[("Everything Else", "y")]));
}

#[test]
fn first_match_in_list_order_wins() {
    let c = format(
        "[[grammar]]\nelement = \"A\"\nkind = \"basic\"\ntext = \"ab\"\n\
         [[grammar]]\nelement = \"B\"\nkind = \"basic\"\ntext = \"abc\"\n\
         [[grammar]]\nelement = \"C\"\nkind = \"basic\"\ntext = 'b\\w+'\nregex = true\n",
    );
    assert_eq!(
        one(&c, "abc bcd"),
        pairs(&[("A", "ab"), ("Everything Else", "c "), ("C", "bcd")])
    );
}

#[test]
fn case_insensitive_items() {
    let c = format(
        "[[grammar]]\nelement = \"Rem\"\nkind = \"delimited\"\nfrom = \"rem \"\nmatch_case = false\n\
         [[grammar]]\nelement = \"K\"\nkind = \"list\"\nmatch_case = false\ntokens = [\"kelvin\"]\n",
    );
    assert_eq!(one(&c, "REM hello"), pairs(&[("Rem", "REM hello")]));
    // KELVIN SIGN folds to 'k' (simple 1:1 folding, D-07).
    assert_eq!(one(&c, "\u{212A}ELVIN"), pairs(&[("K", "\u{212A}ELVIN")]));
}

#[test]
fn regex_runtime_errors_are_counted_as_no_match() {
    let text = "schema_version = 1\nid = \"t\"\nname = \"T\"\nkind = \"text\"\n[[grammar]]\nelement = \"X\"\n\
                kind = \"basic\"\ntext = '(a*)*\\1b'\nregex = true\n";
    let (f, _) = parse_format(text, "t").unwrap();
    let limits = RegexLimits {
        size_limit: 8 << 20,
        backtrack_limit: 50,
    };
    let c = CompiledFormat::compile(Arc::new(f), &limits).unwrap();
    let mut t = Tokenizer::new(c.grammar());
    let mut out = Vec::new();
    let line = "a".repeat(30);
    let o = t.tokenize_line(&line, 0, LineState::Normal, &mut out);
    assert!(o.regex_errors >= 1);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].item, None);
}

#[test]
fn token_dump_format() {
    let c = format(C_LIKE);
    let mut t = Tokenizer::new(c.grammar());
    let mut tokens = Vec::new();
    let line = "a\t// x\\y";
    t.tokenize_line(line, 0, LineState::Normal, &mut tokens);
    let mut out = String::new();
    write_token_dump(&mut out, 1, line, &tokens, c.grammar().elements());
    assert_eq!(
        out,
        "L1:0-2\tEverything Else\ta\\t\nL1:2-8\tComment\t// x\\\\y\n"
    );
    assert_eq!(escape_token_text("\u{1}\r\n"), "\\u{1}\\r\\n");
}

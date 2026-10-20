//! The content and verdict rows of the exit-code table §2.21 (D-01, D-02, D-03).

use hikasa_core::content::{ContentKind, ContentResult};
use hikasa_core::error::{ErrorKind, ErrorReport};
use hikasa_core::exit::{CompareVerdict, ExitCode};
use pretty_assertions::assert_eq;

fn error_result() -> ContentResult {
    ContentResult::Error {
        error: ErrorReport {
            kind: ErrorKind::NotFound,
            message: "x: not found".into(),
            spec: None,
            causes: vec![],
        },
    }
}

#[test]
fn quick_and_text_compare_rows() {
    // `quick`, `text compare`: BinarySame / RulesSame / UnimportantOnly / UnimportantOnly with
    // `--ignore-unimportant` / ImportantDifferences / BinaryDifferent → 1 / 2 / 12 / 2 / 13 / 11.
    let rows = [
        (ContentResult::BinarySame, false, 1),
        (ContentResult::RulesSame, false, 2),
        (ContentResult::UnimportantOnly, false, 12),
        (ContentResult::UnimportantOnly, true, 2),
        (ContentResult::ImportantDifferences, false, 13),
        (ContentResult::BinaryDifferent, false, 11),
    ];
    for (result, iu, code) in rows {
        assert_eq!(
            ExitCode::for_content(&result, iu).code(),
            code,
            "{result:?} iu={iu}"
        );
    }
}

#[test]
fn ignore_unimportant_changes_only_the_unimportant_row() {
    for (result, code) in [
        (ContentResult::BinarySame, 1),
        (ContentResult::RulesSame, 2),
        (ContentResult::ImportantDifferences, 13),
        (ContentResult::BinaryDifferent, 11),
        (error_result(), 100),
    ] {
        assert_eq!(
            ExitCode::for_content(&result, true).code(),
            code,
            "{result:?}"
        );
        assert_eq!(
            ExitCode::for_content(&result, false).code(),
            code,
            "{result:?}"
        );
    }
}

#[test]
fn size_crc_and_binary_methods() {
    // `quick --size`: sizes equal / different → 1 / 11; `quick --crc`, `quick --binary`: BinarySame /
    // BinaryDifferent → 1 / 11.
    assert_eq!(
        ExitCode::for_content(&ContentResult::BinarySame, false),
        ExitCode::BinarySame
    );
    assert_eq!(
        ExitCode::for_content(&ContentResult::BinaryDifferent, false),
        ExitCode::BinaryDifferent
    );
}

#[test]
fn any_error_of_a_file_comparison_is_100() {
    assert_eq!(
        ExitCode::for_content(&error_result(), false),
        ExitCode::UnknownError
    );
    assert_eq!(ExitCode::for_kind(ContentKind::Error, true).code(), 100);
}

#[test]
fn folder_compare_rows() {
    // `folder compare`: verdict NoDifferences / UnimportantOnly / Differences / Incomplete → 2 / 12 / 13 / 100.
    let rows = [
        (CompareVerdict::NoDifferences, 2),
        (CompareVerdict::UnimportantOnly, 12),
        (CompareVerdict::Differences, 13),
        (CompareVerdict::Incomplete, 100),
    ];
    for (verdict, code) in rows {
        assert_eq!(ExitCode::for_verdict(verdict).code(), code, "{verdict:?}");
    }
}

#[test]
fn for_content_equals_for_kind() {
    let results = [
        ContentResult::BinarySame,
        ContentResult::RulesSame,
        ContentResult::UnimportantOnly,
        ContentResult::ImportantDifferences,
        ContentResult::BinaryDifferent,
        error_result(),
    ];
    for result in &results {
        for iu in [false, true] {
            assert_eq!(
                ExitCode::for_content(result, iu),
                ExitCode::for_kind(result.kind(), iu)
            );
        }
    }
}

#[test]
fn cli_004_numbers() {
    let expected = [
        (ExitCode::Success, 0),
        (ExitCode::BinarySame, 1),
        (ExitCode::RulesSame, 2),
        (ExitCode::BinaryDifferent, 11),
        (ExitCode::Similar, 12),
        (ExitCode::RulesDifferent, 13),
        (ExitCode::Conflicts, 14),
        (ExitCode::UnknownError, 100),
        (ExitCode::ConflictsNotWritten, 101),
        (ExitCode::CannotWaitForGui, 102),
        (ExitCode::GuiNotFound, 103),
        (ExitCode::ScriptLoadError, 105),
        (ExitCode::ScriptSyntaxError, 106),
        (ExitCode::LoadFailed, 107),
    ];
    for (code, number) in expected {
        assert_eq!(code.code(), number);
        assert_eq!(ExitCode::from_code(number), Some(code));
        assert_eq!(u8::from(code), number);
    }
    assert_eq!(ExitCode::from_code(104), None);
}

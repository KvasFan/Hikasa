//! Vocabulary parsers reachable from the service API (T11: one parser for the CLI and the GUI).
//!
//! Every vocabulary lives with the crate that owns its meaning: `--display` (folders:
//! `FolderDisplayFilter::parse`; text: `hikasa_report::DisplayMode::parse`), `--folders`
//! (`hikasa_report::parse_folder_mode`), `--criteria` (`Criteria::parse_list`), `--unimportant`
//! (`UnimportantItem::parse_list`), the report words (`TextLayout::parse`, `PatchStyle::parse`, `Wrap::parse`,
//! `FolderLayout::parse`, `FolderColumns::parse_list`, `OutputFormat::parse`) and the legacy `gui open` switches
//! ([`super::gui::normalize_legacy_args`], [`super::gui::parse_open_args`]). This module only adapts the error
//! type of the `--folders` parser to [`ApiError`].

use hikasa_folder::FolderMode;
pub use hikasa_report::folder_mode_word;

use super::ApiError;

/// Parses `--folders` (the folder mode of the display filters, FC-055): `always` (Always Show Folders),
/// `structure` (Compare Files and Folder Structure), `files-only` (Only Compare Files), and the long forms
/// hikasa-report accepts. Delegates to `hikasa_report::parse_folder_mode`; an unknown word is
/// `ApiError::Report(ReportError::InvalidOption)` (kind `invalid_input`, exit 100).
pub fn parse_folder_mode(word: &str) -> Result<FolderMode, ApiError> {
    Ok(hikasa_report::parse_folder_mode(word)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_mode_words() {
        for mode in FolderMode::ALL {
            assert_eq!(parse_folder_mode(folder_mode_word(mode)).unwrap(), mode);
        }
        assert_eq!(
            parse_folder_mode(" Always_Show_Folders ").unwrap(),
            FolderMode::AlwaysShowFolders
        );
        assert_eq!(
            parse_folder_mode("ONLY-FILES").unwrap(),
            FolderMode::OnlyFiles
        );
        let err = parse_folder_mode("tree").unwrap_err();
        assert_eq!(err.kind(), hikasa_core::ErrorKind::InvalidInput);
    }
}

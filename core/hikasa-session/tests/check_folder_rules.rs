//! M2 W9c (D-249): `Service::check_folder_rules`, the rule tabs' OK. Good rules pass with the warnings the comparison
//! would give; a mask that does not compile, an offset out of range and a schema from the future are refused with the
//! comparison's own message.

mod common;

use hikasa_core::WarningCode;
use hikasa_folder::{ArchiveHandling, FolderRules, TzOffset};

#[test]
fn a9_23_check_folder_rules_is_the_compile_step() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = common::service(tmp.path());
    assert_eq!(svc.check_folder_rules(&FolderRules::default()).unwrap(), []);

    let mut rules = FolderRules::default();
    rules.filters.names.include_files = vec!["*.rs".into()];
    rules.handling.archives = ArchiveHandling::AsFiles;
    rules.open_archives = vec!["a.zip".into()];
    rules
        .format_overrides
        .insert("no-such-format".into(), false);
    let codes: Vec<WarningCode> = svc
        .check_folder_rules(&rules)
        .unwrap()
        .into_iter()
        .map(|w| w.code)
        .collect();
    assert_eq!(
        codes,
        [WarningCode::SwitchIgnored, WarningCode::UnknownConfigKey]
    );

    let mut bad = FolderRules::default();
    bad.filters.names.exclude_folders = vec!["[a".into()];
    let err = svc.check_folder_rules(&bad).unwrap_err();
    assert_eq!(
        hikasa_core::Classify::kind(&err),
        hikasa_core::ErrorKind::InvalidInput,
        "{err}"
    );
    let mut offset = FolderRules::default();
    offset.criteria.timestamps.offset = Some(TzOffset {
        side: hikasa_core::Side::Left,
        hours: 13,
    });
    let err = svc.check_folder_rules(&offset).unwrap_err();
    assert!(err.to_string().contains("-12..=12"), "{err}");
    let mut future = FolderRules::default();
    future.schema_version = 99;
    assert!(svc.check_folder_rules(&future).is_err());
}

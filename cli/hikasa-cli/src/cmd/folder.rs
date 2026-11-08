//! `hikasa folder compare <left> <right>` (§3.3): `Service::folder_prepare` + `Service::folder_run`, then the
//! report through REPORT ([`super::report::run_folder`]; the status list by default) or `hikasa.folder_compare.v1`
//! (`FolderCompareResponse::json`, display `differences` with the folder structure by default). Rules precedence
//! (§3.5): defaults ← `--rules-file` ← `--criteria` ← the individual flags (the service applies them in that
//! order). `--gui` sends the request to the GUI instead (D-50).

use std::collections::BTreeMap;

use hikasa_folder::{
    AttribFilter, CaseMode, Change, DateFilter, DosAttrs, FolderError, FolderRulesPatch,
    SizeFilter, UnixTypeFilter,
};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{FOLDER_COMPARE_SCHEMA, GUI_OPEN_SCHEMA};
use hikasa_session::api::{
    ApiOp, FolderCompareRequest, GuiOpenRequest, ViewWords, parse_archive_handling,
    parse_content_method,
};

use super::report::{Shape, run_folder};
use super::{Ctx, gui, importance_patch, tri};
use crate::args::{FolderCompareArgs, FolderRuleArgs};

/// Runs `folder compare`.
pub fn run(ctx: &mut Ctx<'_>, args: &FolderCompareArgs) -> u8 {
    let op = ApiOp::FolderCompare;
    let request = match request(ctx, args) {
        Ok(request) => request,
        Err(e) => {
            let (schema, op) = if args.gui {
                (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: args.wait })
            } else {
                (FOLDER_COMPARE_SCHEMA, op)
            };
            return ctx.fail(schema, &e, op, &[]);
        }
    };
    if args.gui {
        let words = ViewWords {
            display: args.report.display.clone(),
            context_lines: None,
            folders: args.report.folders.clone(),
            suppress_filters: args.report.suppress_filters,
            expand_all: args.expand_all,
        };
        let open = GuiOpenRequest::for_folder(&request, &words, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    run_folder(ctx, &args.report, request, "list", Shape::Compare)
}

/// A filter's flag: `none` is off, anything else its expression, parsed by hikasa-folder.
fn filter_word<T>(
    word: Option<&str>,
    parse: impl Fn(&str) -> Result<T, FolderError>,
) -> Result<Option<Change<T>>, ApiError> {
    match word {
        None => Ok(None),
        Some("none") => Ok(Some(Change::Clear)),
        Some(expr) => parse(expr)
            .map(|filter| Some(Change::Set(filter)))
            .map_err(ApiError::from),
    }
}

/// `--format-on`, `--format-off`: the last word for an id wins over the file's (off after on).
fn format_words(rule: &FolderRuleArgs) -> Option<BTreeMap<String, bool>> {
    let on = rule.format_on.iter().map(|id| (id.clone(), true));
    let off = rule.format_off.iter().map(|id| (id.clone(), false));
    let words: BTreeMap<String, bool> = on.chain(off).collect();
    (!words.is_empty()).then_some(words)
}

/// The request of the flags (§1.9.2 mapping): every individual flag becomes one `FolderRulesPatch` field (the
/// unix flags one field each, §1.8.7 item 19); filter expressions are parsed by hikasa-folder (`--cutoff` with
/// the clock's zone), attribute letters by hikasa-vfs, the content method by the session.
fn request(ctx: &Ctx<'_>, args: &FolderCompareArgs) -> Result<FolderCompareRequest, ApiError> {
    let mut request = rules_request(ctx, &args.rule)?;
    request.left = args.left.clone();
    request.right = args.right.clone();
    request.ignore_unimportant = args.ignore_unimportant;
    Ok(request)
}

/// The request of the rule flags, without the sides: one owner of the flags of `folder compare` and `folder op`.
pub fn rules_request(
    ctx: &Ctx<'_>,
    rule: &FolderRuleArgs,
) -> Result<FolderCompareRequest, ApiError> {
    let zone = ctx.service.clock().now().time_zone().clone();
    let patch = FolderRulesPatch {
        size: tri(rule.size, rule.no_size),
        timestamps_enabled: tri(rule.timestamps, rule.no_timestamps),
        tolerance_secs: rule.tolerance,
        ignore_dst: rule.ignore_dst.then_some(true),
        ignore_timezone: rule.ignore_timezone.then_some(true),
        filename_case: rule.filename_case.then_some(true),
        dos_attributes: rule
            .attributes
            .as_deref()
            .map(DosAttrs::parse_letters)
            .transpose()?,
        unix_permissions: rule.permissions.then_some(true),
        unix_owner: rule.owner.then_some(true),
        unix_group: rule.group.then_some(true),
        content_method: rule
            .content
            .as_deref()
            .map(parse_content_method)
            .transpose()?,
        skip_if_quick_same: rule.skip_if_quick_same.then_some(true),
        override_quick: rule.no_override_quick.then_some(false),
        case: if rule.case_sensitive {
            Some(CaseMode::Sensitive)
        } else if rule.case_insensitive {
            Some(CaseMode::Insensitive)
        } else {
            None
        },
        align_extensions: rule.align_extensions.then_some(true),
        align_normalization: rule.align_normalization.then_some(true),
        name_filter: rule.filters.clone(),
        exclude_protected: rule.include_protected.then_some(false),
        size_filter: rule
            .size_filter
            .as_deref()
            .map(SizeFilter::parse)
            .transpose()?
            .map(Change::Set),
        date_filter: rule
            .cutoff
            .as_deref()
            .map(|expr| DateFilter::parse(expr, &zone))
            .transpose()?
            .map(Change::Set),
        follow_symlinks: rule.follow_symlinks.then_some(true),
        flatten: rule.flatten.then_some(true),
        scan_orphan_folders: rule.no_scan_orphans.then_some(false),
        archives: rule
            .archives
            .as_deref()
            .map(parse_archive_handling)
            .transpose()?,
        open_archives: (!rule.open_archive.is_empty()).then(|| rule.open_archive.clone()),
        attrib_filter: filter_word(rule.attrib_filter.as_deref(), AttribFilter::parse)?,
        unix_type_filter: filter_word(rule.unix_type_filter.as_deref(), UnixTypeFilter::parse)?,
        auto_scan_subfolders: rule.no_auto_scan.then_some(false),
        expand_on_load: rule.expand_on_load.then_some(true),
        expand_differences_only: rule.expand_differences.then_some(true),
        ignored: (!rule.ignored.is_empty()).then(|| rule.ignored.clone()),
        format_overrides: format_words(rule),
        ..FolderRulesPatch::default()
    };
    Ok(FolderCompareRequest {
        left: std::ffi::OsString::new(),
        right: std::ffi::OsString::new(),
        rules_file: rule.rules_file.clone(),
        criteria: rule.criteria.clone(),
        patch,
        text_rules_file: rule.text_rules_file.clone(),
        text_patch: importance_patch(&rule.importance, ctx.matches)?,
        ignore_unimportant: false,
        base_rules: None,
        base_text_rules: None,
    })
}

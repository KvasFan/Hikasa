//! `hikasa settings get|set|unset|list|restore-defaults` (M2 W1; contract §1.1.6, D-71, D-73, D-77): the
//! `Service::settings_*` DTOs as JSON, or the `api::human` lines. The service of these commands reads
//! `options.toml` leniently, so a bad value is a warning and `settings set` can repair it. Exit 0, or 100 on
//! error.

use hikasa_core::{ExitCode, Warning};
use hikasa_session::ApiError;
use std::collections::{BTreeMap, BTreeSet};

use hikasa_session::api::dto::{
    SETTINGS_EXPORT_SCHEMA, SETTINGS_GET_SCHEMA, SETTINGS_IMPORT_SCHEMA, SETTINGS_LIST_SCHEMA,
    SETTINGS_RESTORE_SCHEMA, SETTINGS_SET_SCHEMA,
};
use hikasa_session::api::{ApiOp, human};
use hikasa_session::package::{Category, ExportRequest, ImportChoices, parse_item};
use hikasa_session::settings::{FileId, parse_key};

use super::Ctx;
use crate::args::{RestoreDefaultsArgs, SettingsCmd, SettingsExportArgs, SettingsImportArgs};
use crate::output::stderr_line;

/// Exit code of a successful `settings` command.
const OK: u8 = ExitCode::Success.code();

/// The operation of the exit codes (every failure is 100).
const OP: ApiOp = ApiOp::Other;

/// The warnings of a call that the startup warnings do not already hold (the service read `options.toml` too).
fn own_warnings(ctx: &Ctx<'_>, warnings: &[Warning]) -> Vec<Warning> {
    let startup = ctx.service.startup_warnings();
    warnings
        .iter()
        .filter(|w| !startup.contains(w))
        .cloned()
        .collect()
}

/// Runs a `settings` subcommand.
pub fn run(ctx: &mut Ctx<'_>, cmd: &SettingsCmd) -> u8 {
    match cmd {
        SettingsCmd::Get { key } => {
            let schema = SETTINGS_GET_SCHEMA;
            match parse_key(key).and_then(|k| ctx.service.settings_get(&k)) {
                Ok(dto) => {
                    let own = own_warnings(ctx, &dto.warnings);
                    if ctx.out.json() {
                        ctx.json_success(schema, OK, &dto, &own, OP)
                    } else {
                        let written = ctx.out.bytes(human::settings_get_text(&dto).as_bytes());
                        ctx.human_done(written, OK, schema, &own, OP)
                    }
                }
                Err(e) => ctx.fail(schema, &e, OP, &[]),
            }
        }
        SettingsCmd::Set { key, value, force } => {
            let result = parse_key(key).and_then(|k| ctx.service.settings_set(&k, value, *force));
            set_done(ctx, result)
        }
        SettingsCmd::Unset { key } => {
            let result = parse_key(key).and_then(|k| ctx.service.settings_unset(&k));
            set_done(ctx, result)
        }
        SettingsCmd::List { prefix, changed } => {
            let schema = SETTINGS_LIST_SCHEMA;
            let result = prefix
                .as_deref()
                .map(parse_key)
                .transpose()
                .and_then(|p| ctx.service.settings_list(p.as_ref(), *changed));
            match result {
                Ok(dto) => {
                    let own = own_warnings(ctx, &dto.warnings);
                    if ctx.out.json() {
                        ctx.json_success(schema, OK, &dto, &own, OP)
                    } else {
                        let written = ctx.out.lines(human::settings_list_lines(&dto));
                        if !ctx.out.quiet() {
                            for conflict in &dto.conflicts {
                                stderr_line(&format!(
                                    "warning: {}",
                                    human::shortcut_conflict_text(conflict)
                                ));
                            }
                        }
                        ctx.human_done(written, OK, schema, &own, OP)
                    }
                }
                Err(e) => ctx.fail(schema, &e, OP, &[]),
            }
        }
        SettingsCmd::RestoreDefaults(args) => restore(ctx, args),
        SettingsCmd::Export(args) => export(ctx, args),
        SettingsCmd::Import(args) => import(ctx, args),
    }
}

/// The `--item` names per category.
type Items = BTreeMap<Category, Vec<String>>;

/// Parses `--category` and `--item` words.
fn categories_and_items(
    categories: &[String],
    items: &[String],
) -> Result<(Vec<Category>, Items), ApiError> {
    let categories = categories
        .iter()
        .map(|c| Category::parse(c))
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_category: BTreeMap<Category, Vec<String>> = BTreeMap::new();
    for item in items {
        let (category, name) = parse_item(item)?;
        by_category.entry(category).or_default().push(name);
    }
    Ok((categories, by_category))
}

/// `settings export` (SES-060).
fn export(ctx: &mut Ctx<'_>, args: &SettingsExportArgs) -> u8 {
    let schema = SETTINGS_EXPORT_SCHEMA;
    let result =
        categories_and_items(&args.category, &args.item).and_then(|(categories, items)| {
            ctx.service
                .settings_export(&ExportRequest { categories, items }, &args.output)
        });
    match result {
        Ok(dto) => {
            if ctx.out.json() {
                ctx.json_success(schema, OK, &dto, &[], OP)
            } else {
                let written = ctx.out.lines(human::settings_export_lines(&dto));
                ctx.human_done(written, OK, schema, &[], OP)
            }
        }
        Err(e) => ctx.fail(schema, &e, OP, &[]),
    }
}

/// `settings import` (SES-061; T12: `--dry-run` prints the plan, `--yes` or `--silent` executes it).
fn import(ctx: &mut Ctx<'_>, args: &SettingsImportArgs) -> u8 {
    let result =
        categories_and_items(&args.category, &args.item).and_then(|(categories, items)| {
            let delete_existing = args
                .delete_existing
                .iter()
                .map(|c| Category::parse(c))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let choices = ImportChoices {
                categories,
                items,
                delete_existing,
                force: args.force,
            };
            ctx.service
                .settings_import(&args.path, &choices, args.dry_run)
        });
    import_done(ctx, result)
}

/// The end of an import (also `gui open <package> --silent`, D-88): exit 100 when an item failed.
pub fn import_done(
    ctx: &mut Ctx<'_>,
    result: Result<hikasa_session::api::SettingsImportDto, ApiError>,
) -> u8 {
    let schema = SETTINGS_IMPORT_SCHEMA;
    match result {
        Ok(dto) => {
            let code = if dto.ok() {
                OK
            } else {
                ExitCode::UnknownError.code()
            };
            if ctx.out.json() {
                ctx.json_success(schema, code, &dto, &dto.warnings, OP)
            } else {
                let written = ctx.out.lines(human::settings_import_lines(&dto));
                ctx.human_done(written, code, schema, &dto.warnings, OP)
            }
        }
        Err(e) => ctx.fail(schema, &e, OP, &[]),
    }
}

/// The end of `set` and `unset`.
fn set_done(
    ctx: &mut Ctx<'_>,
    result: Result<hikasa_session::api::SettingsSetDto, ApiError>,
) -> u8 {
    let schema = SETTINGS_SET_SCHEMA;
    match result {
        Ok(dto) => {
            let own = own_warnings(ctx, &dto.warnings);
            if ctx.out.json() {
                ctx.json_success(schema, OK, &dto, &own, OP)
            } else {
                let written = ctx.out.lines(human::settings_set_lines(&dto));
                ctx.human_done(written, OK, schema, &own, OP)
            }
        }
        Err(e) => ctx.fail(schema, &e, OP, &[]),
    }
}

/// `settings restore-defaults` (D-77): without `--dry-run` it needs `--yes`.
fn restore(ctx: &mut Ctx<'_>, args: &RestoreDefaultsArgs) -> u8 {
    let schema = SETTINGS_RESTORE_SCHEMA;
    if !args.dry_run && !args.yes {
        let e = ApiError::InvalidArgument(
            "restore-defaults deletes settings files: add --yes, or --dry-run to see the plan"
                .into(),
        );
        return ctx.fail(schema, &e, OP, &[]);
    }
    if !args.category.is_empty() {
        let result = args
            .category
            .iter()
            .map(|c| Category::parse(c))
            .collect::<Result<Vec<_>, _>>()
            .and_then(|categories| {
                ctx.service.settings_restore_categories(
                    &categories,
                    args.delete_sessions,
                    args.delete_workspaces,
                    args.dry_run,
                )
            });
        return match result {
            Ok(dto) => {
                if ctx.out.json() {
                    ctx.json_success(schema, OK, &dto, &[], OP)
                } else {
                    let written = ctx.out.lines(human::settings_restore_lines(&dto));
                    ctx.human_done(written, OK, schema, &[], OP)
                }
            }
            Err(e) => ctx.fail(schema, &e, OP, &[]),
        };
    }
    let files = if args.all {
        Ok(FileId::every())
    } else {
        args.file
            .iter()
            .map(|f| {
                FileId::parse(f).ok_or_else(|| {
                    ApiError::InvalidArgument(format!(
                        "unknown settings file {f:?} (options, state, commands, theme, themes, \
                         themes.<scheme>, filters, openwith, archives)"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()
    };
    match files.and_then(|files| ctx.service.settings_restore_defaults(&files, args.dry_run)) {
        Ok(dto) => {
            if ctx.out.json() {
                ctx.json_success(schema, OK, &dto, &[], OP)
            } else {
                let written = ctx.out.lines(human::settings_restore_lines(&dto));
                ctx.human_done(written, OK, schema, &[], OP)
            }
        }
        Err(e) => ctx.fail(schema, &e, OP, &[]),
    }
}

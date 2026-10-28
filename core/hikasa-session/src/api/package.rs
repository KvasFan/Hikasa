//! `hikasa settings export|import` and `settings restore-defaults --category` (M2 W2; contract §1.2.6–§1.2.7,
//! D-86, D-87): the [`Service`] calls and `hikasa.settings_import.v1`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use super::settings::{RestoreFileDto, SettingsRestoreDto};
use super::{ApiError, Service};
use crate::jobs::run_guarded;
use crate::package::{
    Category, CategoryCounts, ExportDto, ExportRequest, ImportChoices, ImportPlan, ItemResult,
    PlanAction, execute_import, export, item_of, plan_import, plan_restore,
};

/// One item of an import plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanItemDto {
    /// The category.
    pub category: Category,
    /// The item name.
    pub name: String,
    /// The target file.
    pub path: String,
    /// `add`, `replace` (a collision) or `skip_locked` (a locked session kept).
    pub action: PlanAction,
}

/// The plan of `hikasa.settings_import.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanDto {
    /// The items, in manifest order.
    pub items: Vec<PlanItemDto>,
    /// The files deleted first.
    pub deletes: Vec<String>,
    /// Per category: added, replaced, deleted.
    pub summary: BTreeMap<Category, CategoryCounts>,
}

impl PlanDto {
    /// The DTO of a plan.
    pub fn of(plan: &ImportPlan) -> PlanDto {
        PlanDto {
            items: plan
                .items
                .iter()
                .map(|i| PlanItemDto {
                    category: i.category,
                    name: i.name.clone(),
                    path: i.target.display().to_string(),
                    action: i.action,
                })
                .collect(),
            deletes: plan
                .deletes
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            summary: plan.summary.clone(),
        }
    }
}

/// `hikasa.settings_import.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsImportDto {
    /// Only the plan.
    pub dry_run: bool,
    /// The plan.
    pub plan: PlanDto,
    /// Per item (and per failed delete): written or not (empty in a dry run; none for a skipped item).
    pub results: Vec<ItemResult>,
    /// Warnings of the plan (`session_locked`).
    #[serde(skip)]
    pub warnings: Vec<hikasa_core::Warning>,
}

impl SettingsImportDto {
    /// True when nothing failed (exit 0; else 100).
    pub fn ok(&self) -> bool {
        self.results.iter().all(|r| r.ok)
    }
}

impl Service {
    /// `settings export` (SES-060): the package of the chosen categories and items, written atomically to
    /// `out`, with the clock's time in the manifest.
    pub fn settings_export(
        &self,
        request: &ExportRequest,
        out: &Path,
    ) -> Result<ExportDto, ApiError> {
        run_guarded(|| export(self.config(), request, &self.clock().now(), self.env(), out))
    }

    /// `settings import` (SES-061, T12): the plan (everything validated before anything is written) and, unless
    /// `dry_run`, its execution with per-item results.
    pub fn settings_import(
        &self,
        package: &Path,
        choices: &ImportChoices,
        dry_run: bool,
    ) -> Result<SettingsImportDto, ApiError> {
        run_guarded(|| {
            let plan = plan_import(self.config(), package, choices)?;
            let results = if dry_run {
                Vec::new()
            } else {
                execute_import(self.config(), &plan).results
            };
            Ok(SettingsImportDto {
                dry_run,
                plan: PlanDto::of(&plan),
                results,
                warnings: plan.warnings.clone(),
            })
        })
    }

    /// `settings restore-defaults --category` (D-87): the deletes of [`plan_restore`] and, unless `dry_run`,
    /// their execution; one entry per file, named by its category.
    pub fn settings_restore_categories(
        &self,
        categories: &[Category],
        delete_sessions: bool,
        delete_workspaces: bool,
        dry_run: bool,
    ) -> Result<SettingsRestoreDto, ApiError> {
        run_guarded(|| {
            let plan = plan_restore(
                self.config(),
                categories,
                delete_sessions,
                delete_workspaces,
            );
            let failed: Vec<String> = if dry_run {
                Vec::new()
            } else {
                execute_import(self.config(), &plan)
                    .results
                    .into_iter()
                    .filter(|r| !r.ok)
                    .map(|r| r.name)
                    .collect()
            };
            if let Some(first) = failed.first() {
                return Err(ApiError::InvalidArgument(format!(
                    "could not delete {first}"
                )));
            }
            let files = plan
                .deletes
                .iter()
                .map(|path| {
                    let category = path
                        .strip_prefix(&self.config().dir)
                        .ok()
                        .and_then(|p| p.to_str())
                        .map(|p| p.replace('\\', "/"))
                        .and_then(|p| item_of(&p))
                        .map_or_else(String::new, |(c, _)| c.as_str().to_owned());
                    RestoreFileDto {
                        file: category,
                        path: path.display().to_string(),
                        existed: true,
                        deleted: !dry_run,
                    }
                })
                .collect();
            Ok(SettingsRestoreDto { dry_run, files })
        })
    }
}

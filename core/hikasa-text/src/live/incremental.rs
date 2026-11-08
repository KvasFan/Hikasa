//! The exact incremental recompare (normative, §1.10.11).

use hikasa_core::progress::Ctl;

use super::LiveKeys;
use super::recompare::{PathCounters, RecompareJob};
use crate::compare::TextComparison;
use crate::error::TextError;

/// The incremental path: `Ok(None)` when the full compare must run.
pub(crate) fn run(
    _job: &RecompareJob,
    _pool: Option<&rayon::ThreadPool>,
    _ctl: &Ctl<'_>,
    _path: &mut PathCounters,
) -> Result<Option<(TextComparison, LiveKeys)>, TextError> {
    Ok(None)
}

//! Simulation soundness readings of the band table (SPEC_0033): which
//! completions are neither strict-high nor covered by a typed trace
//! exception, and the package that owns each for triage.

use super::{BandLabel, BandRow, BandTable, ExitReason};

impl BandRow {
    /// Whether this model simulated without strict-high parity and without a
    /// typed trace exception. A failed or unattempted simulation is not a
    /// completion, and an excluded row carries a typed exception; every other
    /// non-high row, including a pointwise non-identifiable one with no typed
    /// row, is a completion the gate cannot certify.
    pub fn is_unexcepted_non_high(&self) -> bool {
        if self.band == BandLabel::High {
            return false;
        }
        match self.exit_reason {
            Some(ExitReason::SimFailed | ExitReason::NotAttempted | ExitReason::Excluded) => false,
            Some(
                ExitReason::RumocaTraceMissing
                | ExitReason::ReferenceMissing
                | ExitReason::TraceMissingSideUnrecorded
                | ExitReason::ComparatorFailed
                | ExitReason::NoComparableSamples
                | ExitReason::TraceNonidentifiable
                | ExitReason::NotCompared,
            )
            | None => true,
        }
    }
}

impl BandTable {
    /// Models that simulated but are neither strict-high nor carried by a
    /// typed trace exception, in table order. The soundness rule is that this
    /// set is empty; the baseline owns the shrink-only roster of the models
    /// still in it (SPEC_0033).
    pub fn unexcepted_non_high_rows(&self) -> impl Iterator<Item = &BandRow> {
        self.rows.iter().filter(|row| row.is_unexcepted_non_high())
    }
}

/// The package that owns a model for triage: its first three name segments
/// (`Modelica.Electrical.Analog`, `Modelica.Mechanics.MultiBody`). Parity work
/// is split by these packages, so roster listings group by them.
pub fn triage_package(model_name: &str) -> &str {
    match model_name.match_indices('.').nth(2) {
        Some((index, _)) => &model_name[..index],
        None => model_name,
    }
}

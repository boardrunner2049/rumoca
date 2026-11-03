//! Bookkeeping for one candidate pass: the direct choice by pass order and
//! the structural analysis of a holonomic candidate.

use rumoca_ir_dae as dae;

use super::*;

/// The choices one direct pass has made so far, each with its candidate's
/// position in the pass order.
#[derive(Default)]
pub(super) struct PassChoice {
    /// The first strictly reducing candidate.
    pub(super) reduced: Option<(usize, DirectStateConstraint, usize, DemotionStep)>,
    /// The last residue-holding candidate.
    pub(super) held: Option<(usize, DirectStateConstraint, usize, DemotionStep)>,
    /// The first candidate refused for a discarded initial value.
    pub(super) blocked: Option<(usize, DiscardedInitialValue)>,
}

impl PassChoice {
    /// Record one attempt at pass position `index`; a sorted attempt ends the
    /// pass with its round.
    pub(super) fn record(
        &mut self,
        index: usize,
        residue: usize,
        attempt: DirectAttempt,
    ) -> Option<DemotionRound> {
        match attempt {
            DirectAttempt::Sorted {
                rebuilt,
                manifold,
                structural,
            } => Some(DemotionRound {
                step: Some(DemotionStep::Sorted {
                    dae: rebuilt,
                    manifold,
                    structural,
                }),
                blocked: None,
            }),
            DirectAttempt::Accepted {
                candidate,
                residue: next,
                step,
            } if next < residue => {
                if self
                    .reduced
                    .as_ref()
                    .is_none_or(|(first, ..)| index < *first)
                {
                    self.reduced = Some((index, candidate, next, step));
                }
                None
            }
            DirectAttempt::Accepted {
                candidate,
                residue: next,
                step,
            } => {
                if self.held.as_ref().is_none_or(|(last, ..)| index > *last) {
                    self.held = Some((index, candidate, next, step));
                }
                None
            }
            DirectAttempt::Blocked(discarded) => {
                if self
                    .blocked
                    .as_ref()
                    .is_none_or(|(first, _)| index < *first)
                {
                    self.blocked = Some((index, discarded));
                }
                None
            }
            DirectAttempt::Rejected => None,
        }
    }
}

/// The structural analysis of a holonomic candidate's rebuilt system.
///
/// The replacement rewrites its own owner. A promoted algebraic changes role
/// only, keeping its place in the dense variable order and so its columns,
/// and it changes the rows of exactly the owners that read it. Every other
/// owner's rows are reused from the pass model's incidence.
pub(super) fn holonomic_analysis(
    rebuilt: &dae::Dae,
    constraint: &HolonomicConstraint,
    reuse: Option<&crate::incidence::ReusableIncidence>,
    bounds: &super::demotion_bounds::DemotionRowBounds,
) -> Result<PreparedStructuralAnalysis, StructuralError> {
    let touched = reuse.map(|base| {
        let mut mask = vec![false; base.owner_count()];
        let promoted = constraint
            .lifted_algebraic
            .map_or(&[][..], |algebraic| bounds.owners(algebraic));
        for &owner in promoted.iter().chain([&constraint.owner_ordinal]) {
            if let Some(entry) = mask.get_mut(owner) {
                *entry = true;
            }
        }
        mask
    });
    match touched {
        Some(touched) => structural_analysis_capturing(rebuilt, reuse, Some(&touched)).0,
        None => structural_analysis(rebuilt),
    }
}

/// Debug builds reconstruct every holonomic candidate skipped after a
/// reduction and check the row-count proof: none of them may sort.
#[cfg(debug_assertions)]
pub(super) fn check_unsortable(
    source: &ReductionSource<'_>,
    (residue, prior_manifold, stated, reuse): (
        usize,
        &[ManifoldConstraint],
        &[u32],
        Option<&crate::incidence::ReusableIncidence>,
    ),
    skipped: Vec<HolonomicConstraint>,
) -> Result<(), StructuralError> {
    for constraint in skipped {
        let attempt = super::holonomic_attempt::attempt_holonomic_candidate(
            source,
            residue,
            prior_manifold,
            stated,
            constraint,
            reuse,
            &mut (),
        )?;
        assert!(
            !matches!(attempt, super::HolonomicAttempt::Sorted { .. }),
            "a holonomic candidate the row count excludes cannot sort"
        );
    }
    Ok(())
}

/// Debug builds reconstruct every candidate the residue bound deferred and
/// check the bound: none of them may reduce the residue.
#[cfg(debug_assertions)]
pub(super) fn check_deferred(
    source: &ReductionSource<'_>,
    (residue, stated, prior_manifold, reuse): (
        usize,
        &[u32],
        &[ManifoldConstraint],
        Option<&crate::incidence::ReusableIncidence>,
    ),
    candidates: &[DirectStateConstraint],
    deferred: &[usize],
) -> Result<(), StructuralError> {
    for &index in deferred {
        let attempt = attempt_direct_candidate(
            source,
            residue,
            stated,
            &candidates[index],
            prior_manifold,
            reuse,
            &mut (),
        )?;
        assert!(
            !matches!(attempt, DirectAttempt::Sorted { .. })
                && !matches!(&attempt, DirectAttempt::Accepted { residue: next, .. } if *next < residue),
            "a candidate the residue bound excludes cannot reduce"
        );
    }
    Ok(())
}

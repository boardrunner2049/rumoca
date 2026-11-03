//! A demotion a reduction may not take, and why.
//!
//! Two source facts forbid demoting a state even when doing so would reduce
//! the system: a `fixed = true` start no surviving equation states (MLS 3.6
//! §8.6), and a `reinit` of that state (MLS 3.6 §8.3.6, which makes the state
//! an implicit `stateSelect = StateSelect.always`, SPEC_0022 EQN-032). A
//! reduction records the first such candidate and keeps looking; only when no
//! other reduction exists is the refusal reported, in place of the singularity
//! it hides behind.

use rumoca_core::Span;
use rumoca_ir_dae as dae;

use super::observation::{AttemptOutcome, StoppedOutcome};
use crate::StructuralError;

/// The rule a refused demotion would have broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BlockedReason {
    /// The rebuilt system no longer states this `fixed = true` start.
    StatedInitialValue,
    /// A `reinit` targets this state, so it must remain a state.
    Reinitialized,
}

/// One demotion candidate a reduction refused for a source-stated reason.
#[derive(Clone)]
pub(super) struct BlockedDemotion {
    /// The variable, named as the model declares it.
    pub(super) variable: String,
    /// That variable's declaration.
    pub(super) span: Span,
    pub(super) reason: BlockedReason,
}

impl BlockedDemotion {
    /// The refusal demoting state ordinal `state` would meet because a
    /// `reinit` targets it, or `None` when no event action reinitializes it.
    ///
    /// Reconstruction cannot express such a demotion at all - the rebuilt
    /// event action would name a variable that is no longer a state - so this
    /// is asked before a candidate is rebuilt, not read back from the result.
    pub(super) fn reinitialized(view: dae::DaeView<'_>, state: u32) -> Option<Self> {
        let reinitialized = (0..view.event_action_count())
            .filter_map(|index| view.event_action(view.event_action_id(index)?))
            .any(|action| {
                matches!(
                    action.operation(),
                    dae::EventActionOperation::Reinitialize { state: target, .. }
                        if target.index() == state
                )
            });
        if !reinitialized {
            return None;
        }
        let variable = view.variable(view.variable_id(state as usize)?)?;
        Some(Self {
            variable: variable.name().as_str().to_string(),
            span: variable.declaration().span(),
            reason: BlockedReason::Reinitialized,
        })
    }

    /// The attempt outcome a candidate refused for this reason records.
    pub(super) fn attempt_outcome(&self) -> AttemptOutcome<'_> {
        let (variable, span) = (self.variable.as_str(), self.span);
        match self.reason {
            BlockedReason::StatedInitialValue => {
                AttemptOutcome::WouldDiscardInitial { variable, span }
            }
            BlockedReason::Reinitialized => {
                AttemptOutcome::WouldDemoteReinitialized { variable, span }
            }
        }
    }

    /// The final disposition a reduction stopped by this refusal records.
    pub(super) fn stopped_outcome(&self) -> StoppedOutcome<'_> {
        let (variable, span) = (self.variable.as_str(), self.span);
        match self.reason {
            BlockedReason::StatedInitialValue => StoppedOutcome::DiscardsInitial { variable, span },
            BlockedReason::Reinitialized => StoppedOutcome::DemotesReinitialized { variable, span },
        }
    }

    /// The diagnostic reported when this refusal was the only way forward.
    pub(super) fn into_error(self) -> StructuralError {
        let Self {
            variable,
            span,
            reason,
        } = self;
        match reason {
            BlockedReason::StatedInitialValue => {
                StructuralError::DroppedStatedInitialValue { variable, span }
            }
            BlockedReason::Reinitialized => {
                StructuralError::DemotedReinitializedState { variable, span }
            }
        }
    }
}

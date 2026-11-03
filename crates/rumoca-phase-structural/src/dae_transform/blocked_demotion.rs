//! A demotion a reduction may not take, and why.
//!
//! Two source facts forbid demoting a state even when doing so would reduce
//! the system: a `fixed = true` start no surviving equation states (MLS 3.6
//! §8.6), and a `reinit` of that state (MLS 3.6 §8.3.6, which makes the state
//! an implicit `stateSelect = StateSelect.always`, SPEC_0022 EQN-032). A
//! reduction records the first such candidate the round would otherwise take
//! and keeps looking; only when no other reduction exists is the refusal
//! reported, in place of the singularity it hides behind.
//!
//! A refused `reinit` demotion keeps the probe system the reduction built
//! without that `reinit`. The refusal is reported only when that probe
//! reduces to a sortable system; otherwise the model is singular whatever the
//! `reinit` does, and the probe's own failure is the diagnostic.

use std::rc::Rc;

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
    /// For a refused `reinit` demotion, the system that demotion builds with
    /// the `reinit` left out.
    pub(super) probe: Option<Rc<dae::Dae>>,
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
            probe: None,
        })
    }

    /// The refusal a reduced system meets when it keeps a `reinit` target in a
    /// retained state-manifold row, or `None` when no row couples one.
    ///
    /// Such a row keeps the target a state only nominally: after the event
    /// the runtime projects the states back onto the manifold by a
    /// minimum-norm correction that moves the reinitialized value as well,
    /// so two `reinit`s of coupled states would silently settle on their
    /// average. The target cannot be an independent state there, which is
    /// exactly what MLS 3.6 section 8.3.6 requires of it.
    pub(super) fn reinitialized_in_manifold(
        model: &dae::Dae,
        manifold: &[super::ManifoldConstraint],
    ) -> Option<Self> {
        model.inspect(|view| {
            manifold_states(view, manifold)
                .into_iter()
                .find_map(|state| Self::reinitialized(view, state))
        })
    }

    /// Keep the probe system the refused demotion built.
    pub(super) fn with_probe(self, probe: dae::Dae) -> Self {
        Self {
            probe: Some(Rc::new(probe)),
            ..self
        }
    }

    /// The failure that makes this refusal beside the point: the probe system
    /// does not reduce either, so the model is singular regardless of it.
    pub(super) fn probe_failure(&self) -> Option<StructuralError> {
        let probe = self.probe.as_deref()?;
        super::reduce_for_solve_with_observer(probe, &mut ()).err()
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
            ..
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

/// Every state ordinal a retained manifold row reads, in row order.
fn manifold_states(view: dae::DaeView<'_>, manifold: &[super::ManifoldConstraint]) -> Vec<u32> {
    let mut cache = rumoca_eval_dae::ScalarCoordinateProjectionCache::default();
    let mut states = Vec::new();
    let expressions = manifold
        .iter()
        .filter_map(|entry| view.expression_id(entry.expression as usize));
    for expression in expressions {
        let scalars = view
            .expression(expression)
            .and_then(|value| value.value_type().scalar_count())
            .unwrap_or(0);
        let mut record = |coordinate: dae::CoordinateView<'_>, _| {
            if let dae::CoordinateView::State(state) = coordinate {
                states.push(state.index());
            }
        };
        for scalar in 0..scalars {
            let _ = rumoca_eval_dae::for_each_scalar_coordinate_cached(
                view,
                expression,
                scalar,
                None,
                &mut cache,
                &mut record,
            );
        }
    }
    states
}

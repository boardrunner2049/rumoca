//! Exact clock semantics in the public interchange vocabulary.
use super::*;

fn rational(value: rumoca_core::ClockRational) -> RbcClockRational {
    RbcClockRational {
        numerator: value.numerator().to_string(),
        denominator: value.denominator().to_string(),
    }
}

pub(super) fn export_clocks(view: dae::DaeView<'_>, ctx: &mut Ctx<'_>) -> Vec<RbcClock> {
    (0..view.clock_count())
        .filter_map(|index| {
            let id = view.clock_id(index)?;
            let clock = view.clock(id)?;
            let node = match clock.operation() {
                dae::ClockOperation::Periodic(schedule) => RbcClockNode::Periodic {
                    period: rational(schedule.period()),
                    phase: rational(schedule.phase()),
                    anchor: match schedule.anchor() {
                        rumoca_core::ClockPhaseAnchor::Absolute => RbcClockAnchor::Absolute,
                        rumoca_core::ClockPhaseAnchor::SimulationStart => {
                            RbcClockAnchor::SimulationStart
                        }
                    },
                },
                dae::ClockOperation::Shifted {
                    base,
                    counter,
                    condition,
                } => RbcClockNode::Shifted {
                    base: ClockId(base.index()),
                    counter,
                    condition: ConditionId(condition.index()),
                },
                dae::ClockOperation::Triggered(condition) => RbcClockNode::Triggered {
                    condition: ConditionId(condition.index()),
                },
            };
            Some(RbcClock {
                id: ClockId(index as u32),
                node,
                provenance: ctx.provenance(clock.provenance()),
            })
        })
        .collect()
}

pub(super) fn export_ownerships(
    view: dae::DaeView<'_>,
    ctx: &mut Ctx<'_>,
) -> Vec<RbcClockOwnership> {
    (0..view.clock_ownership_count())
        .filter_map(|index| {
            let id = view.clock_ownership_id(index)?;
            let owner = view.clock_ownership(id)?;
            Some(RbcClockOwnership {
                variable: VariableId(owner.variable().index()),
                clock: ClockId(owner.clock().index()),
                sampled: owner.sampled(),
                provenance: ctx.provenance(owner.provenance()),
            })
        })
        .collect()
}

/// The derivation of a clock conversion's target clock.
pub(super) fn transfer_kind(kind: dae::ClockTransferKind) -> RbcClockTransferKind {
    match kind {
        dae::ClockTransferKind::SubSample { factor } => RbcClockTransferKind::SubSample { factor },
        dae::ClockTransferKind::SuperSample { factor } => {
            RbcClockTransferKind::SuperSample { factor }
        }
        dae::ClockTransferKind::ShiftSample {
            counter,
            resolution,
        } => RbcClockTransferKind::ShiftSample {
            counter,
            resolution,
        },
        dae::ClockTransferKind::BackSample {
            counter,
            resolution,
        } => RbcClockTransferKind::BackSample {
            counter,
            resolution,
        },
    }
}

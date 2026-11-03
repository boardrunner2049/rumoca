//! Reissue checked clock construction; never infer schedules from source text.
use super::*;
use rumoca_core::{ClockLattice, ClockRational, PeriodicClockSchedule};

fn rational(
    value: &RbcClockRational,
    ctx: &Rebuild<'_>,
) -> Result<ClockRational, dae::DaeConstructionError> {
    let parse = |text: &str| {
        text.parse::<i128>()
            .map_err(|_| ctx.unsupported("clock rational requires a decimal 128-bit integer"))
    };
    ClockRational::new(parse(&value.numerator)?, parse(&value.denominator)?)
        .map_err(|error| ctx.unsupported(error.to_string()))
}

pub(super) fn rebuild<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    variables: &[VariableSlot<'dae>],
    conditions: &[dae::ConditionId<'dae>],
) -> Result<RebuiltClocks<'dae>, dae::DaeConstructionError> {
    let mut ids = Vec::with_capacity(ctx.model.clocks.len());
    let mut periodic = Vec::with_capacity(ctx.model.clocks.len());
    for clock in &ctx.model.clocks {
        let at = ctx.provenance(clock.provenance)?;
        let id = construction.clocks(|owner| match &clock.node {
            RbcClockNode::Periodic {
                period,
                phase,
                anchor,
            } => {
                let lattice = ClockLattice::new(rational(period, ctx)?, rational(phase, ctx)?)
                    .map_err(|error| ctx.unsupported(error.to_string()))?;
                let schedule = match anchor {
                    RbcClockAnchor::Absolute => PeriodicClockSchedule::absolute(lattice),
                    RbcClockAnchor::SimulationStart => {
                        PeriodicClockSchedule::simulation_start_relative(lattice)
                    }
                }
                .map_err(|error| ctx.unsupported(error.to_string()))?;
                owner
                    .scheduled(schedule, at)
                    .map(|id| (id.into(), Some(id)))
            }
            RbcClockNode::Shifted {
                base,
                counter,
                condition,
            } => owner
                .shifted(
                    resolve(&ids, base.0, "clock", ctx)?,
                    *counter,
                    resolve(conditions, condition.0, "condition", ctx)?,
                    at,
                )
                .map(|id| (id, None)),
            RbcClockNode::Triggered { condition } => owner
                .triggered(resolve(conditions, condition.0, "condition", ctx)?, at)
                .map(|id| (id, None)),
        })?;
        ids.push(id.0);
        periodic.push(id.1);
    }
    for ownership in &ctx.model.clock_ownerships {
        let at = ctx.provenance(ownership.provenance)?;
        let clock = resolve(&ids, ownership.clock.0, "clock", ctx)?;
        let variable = resolve(variables, ownership.variable.0, "variable", ctx)?;
        construction.clocks(|owner| match (variable, ownership.sampled) {
            (VariableSlot::DiscreteReal(id), false) => owner.own_discrete_real(clock, id, at),
            (VariableSlot::DiscreteReal(id), true) => {
                owner.own_sampled_discrete_real(clock, id, at)
            }
            (VariableSlot::DiscreteValue(id), false) => owner.own_discrete_value(clock, id, at),
            (VariableSlot::DiscreteValue(id), true) => {
                owner.own_sampled_discrete_value(clock, id, at)
            }
            _ => Err(ctx.unsupported("clock ownership requires a discrete variable")),
        })?;
    }
    Ok(RebuiltClocks { ids, periodic })
}

/// Clocks by artifact index, and the periodic identity of those that are
/// periodic (an `interval(c)` coordinate needs that one).
pub(super) struct RebuiltClocks<'dae> {
    pub(super) ids: Vec<dae::ClockId<'dae>>,
    pub(super) periodic: Vec<Option<dae::PeriodicClockId<'dae>>>,
}

/// The DAE's clock-conversion derivation for an artifact's.
pub(super) fn transfer_kind(kind: RbcClockTransferKind) -> dae::ClockTransferKind {
    match kind {
        RbcClockTransferKind::SubSample { factor } => dae::ClockTransferKind::SubSample { factor },
        RbcClockTransferKind::SuperSample { factor } => {
            dae::ClockTransferKind::SuperSample { factor }
        }
        RbcClockTransferKind::ShiftSample {
            counter,
            resolution,
        } => dae::ClockTransferKind::ShiftSample {
            counter,
            resolution,
        },
        RbcClockTransferKind::BackSample {
            counter,
            resolution,
        } => dae::ClockTransferKind::BackSample {
            counter,
            resolution,
        },
    }
}

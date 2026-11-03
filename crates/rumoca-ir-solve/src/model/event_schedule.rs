//! The event iteration schedule (SPEC_0044 ME-EVENT-006).
//!
//! MLS Appendix B fixes what one event iteration computes; this schedule fixes
//! how every executor walks it: the sub-step order of the three nested fixed
//! points, their iteration cap, and the coupled Newton solve a stalled discrete
//! settle hands over to. The linked runtime and the generated C both execute
//! the schedule they are given; neither keeps its own order or cap.
//!
//! A schedule is valid by construction: its fields are private, and both the
//! standard schedule and a deserialized one pass [`EventIterationSchedule::new`].

use serde::{Deserialize, Serialize};

/// One sub-step of an event pass, the outer fixed point. Each pass fixes
/// `pre` from the previous pass (Appendix B); the pass converges when no step
/// changed the coordinate and the event iteration plan is settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum EventPassStep {
    /// Apply the located roots' relation overrides.
    RelationOverrides,
    /// Run the runtime assignments to their fixed point.
    RuntimeAssignments,
    /// Project the continuous algebraic coordinate.
    AlgebraicProjection,
    /// Refresh relation memory from the current coordinate, then rerun the
    /// runtime assignments when it changed, so discrete consumers read the
    /// refreshed side.
    RelationRefresh,
    /// Run the relation pass to its fixed point under this pass's `pre`.
    RelationSettle,
}

/// One sub-step of a relation pass, the middle fixed point: condition
/// equations settle with the discrete equations under one fixed `pre`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum RelationPassStep {
    /// Run the discrete settle pass to its fixed point.
    DiscreteSettle,
    /// Refresh relation memory and end the relation pass when it is unchanged.
    SettledIfRelationsUnchanged,
    /// Project the continuous algebraic coordinate.
    AlgebraicProjection,
    /// Run the runtime assignments to their fixed point.
    RuntimeAssignments,
}

/// One sub-step of a discrete settle pass, the inner Picard fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum SettleStep {
    /// Evaluate the active discrete owners once.
    DiscreteOwners,
    /// Run the runtime assignments to their fixed point.
    RuntimeAssignments,
    /// End the settle when no earlier step of this pass changed the
    /// coordinate: the caller's projected coordinate is then still certified.
    SettledIfUnchanged,
    /// Project the continuous algebraic coordinate.
    AlgebraicProjection,
}

/// The coupled Newton solve a discrete settle hands a stalled iteration to
/// (SOLVE-C57, ME-EVENT-007).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CoupledNewtonPolicy {
    iteration_cap: usize,
    line_search_halvings: usize,
}

impl CoupledNewtonPolicy {
    /// A policy with a positive iteration cap; `None` for a zero cap.
    pub const fn new(iteration_cap: usize, line_search_halvings: usize) -> Option<Self> {
        if iteration_cap == 0 {
            return None;
        }
        Some(Self {
            iteration_cap,
            line_search_halvings,
        })
    }

    /// Newton iterations before the solve reports non-convergence.
    pub const fn iteration_cap(&self) -> usize {
        self.iteration_cap
    }

    /// Step halvings the line search tries before accepting a full step.
    pub const fn line_search_halvings(&self) -> usize {
        self.line_search_halvings
    }
}

/// Why a candidate schedule was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventScheduleError {
    ZeroCap,
    EventPassMustEndWithRelationSettle,
    RelationPassMustSettleOnce,
    SettlePassMustStartWithOwners,
    SettlePassMustCheckOnce,
    SurfaceNeedsBothSides,
}

impl std::fmt::Display for EventScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ZeroCap => "an event iteration cap must be positive",
            Self::EventPassMustEndWithRelationSettle => {
                "an event pass must run exactly one relation settle, as its last step"
            }
            Self::RelationPassMustSettleOnce => {
                "a relation pass must run one discrete settle before one relation check"
            }
            Self::SettlePassMustStartWithOwners => {
                "a settle pass must start by evaluating the discrete owners"
            }
            Self::SettlePassMustCheckOnce => {
                "a settle pass must check convergence exactly once, after a changing step"
            }
            Self::SurfaceNeedsBothSides => "a relation surface needs two passes, one on each side",
        })
    }
}

impl std::error::Error for EventScheduleError {}

/// The event iteration schedule a component carries.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "EventScheduleWire")]
pub struct EventIterationSchedule {
    event_pass: Vec<EventPassStep>,
    relation_pass: Vec<RelationPassStep>,
    settle_pass: Vec<SettleStep>,
    fixed_point_cap: usize,
    stalled_settle: CoupledNewtonPolicy,
    relation_surface_window: usize,
    mode_search_relations: usize,
}

#[derive(Deserialize)]
struct EventScheduleWire {
    event_pass: Vec<EventPassStep>,
    relation_pass: Vec<RelationPassStep>,
    settle_pass: Vec<SettleStep>,
    fixed_point_cap: usize,
    stalled_settle: CoupledNewtonPolicy,
    relation_surface_window: usize,
    mode_search_relations: usize,
}

impl TryFrom<EventScheduleWire> for EventIterationSchedule {
    type Error = EventScheduleError;

    fn try_from(wire: EventScheduleWire) -> Result<Self, Self::Error> {
        Self::new(
            wire.event_pass,
            wire.relation_pass,
            wire.settle_pass,
            wire.fixed_point_cap,
            wire.stalled_settle,
        )?
        .with_relation_surface_window(wire.relation_surface_window)
        .map(|schedule| schedule.with_mode_search_relations(wire.mode_search_relations))
    }
}

impl Default for EventIterationSchedule {
    fn default() -> Self {
        Self::standard()
    }
}

impl EventIterationSchedule {
    /// Construct a schedule, refusing one whose passes cannot terminate or
    /// carry an unreachable convergence check.
    pub fn new(
        event_pass: Vec<EventPassStep>,
        relation_pass: Vec<RelationPassStep>,
        settle_pass: Vec<SettleStep>,
        fixed_point_cap: usize,
        stalled_settle: CoupledNewtonPolicy,
    ) -> Result<Self, EventScheduleError> {
        if fixed_point_cap == 0 || stalled_settle.iteration_cap == 0 {
            return Err(EventScheduleError::ZeroCap);
        }
        let settles = |step: &EventPassStep| *step == EventPassStep::RelationSettle;
        if event_pass.last() != Some(&EventPassStep::RelationSettle)
            || event_pass.iter().filter(|step| settles(step)).count() != 1
        {
            return Err(EventScheduleError::EventPassMustEndWithRelationSettle);
        }
        let position = |wanted: RelationPassStep| {
            let mut found = relation_pass
                .iter()
                .enumerate()
                .filter(|(_, s)| **s == wanted);
            match (found.next(), found.next()) {
                (Some((index, _)), None) => Some(index),
                _ => None,
            }
        };
        match (
            position(RelationPassStep::DiscreteSettle),
            position(RelationPassStep::SettledIfRelationsUnchanged),
        ) {
            (Some(settle), Some(check)) if settle < check => {}
            _ => return Err(EventScheduleError::RelationPassMustSettleOnce),
        }
        if settle_pass.first() != Some(&SettleStep::DiscreteOwners) {
            return Err(EventScheduleError::SettlePassMustStartWithOwners);
        }
        // Changes accumulate over a pass, so a check after the first one can
        // never fire: the pass only reaches it when something changed.
        if settle_pass
            .iter()
            .filter(|step| **step == SettleStep::SettledIfUnchanged)
            .count()
            != 1
        {
            return Err(EventScheduleError::SettlePassMustCheckOnce);
        }
        Ok(Self {
            event_pass,
            relation_pass,
            settle_pass,
            fixed_point_cap,
            stalled_settle,
            relation_surface_window: 0,
            mode_search_relations: 0,
        })
    }

    /// The schedule every component carries today: MLS Appendix B with the
    /// relation overrides applied first, relation memory refreshed from the
    /// projected coordinate before discrete consumers read it, and a
    /// projection only after a discrete change.
    ///
    /// Built directly rather than through [`Self::new`]: the literal is fixed,
    /// and a test proves [`Self::new`] accepts it.
    pub fn standard() -> Self {
        Self {
            event_pass: vec![
                EventPassStep::RelationOverrides,
                EventPassStep::RuntimeAssignments,
                EventPassStep::AlgebraicProjection,
                EventPassStep::RuntimeAssignments,
                EventPassStep::RelationRefresh,
                EventPassStep::RelationSettle,
            ],
            relation_pass: vec![
                RelationPassStep::DiscreteSettle,
                RelationPassStep::SettledIfRelationsUnchanged,
                RelationPassStep::AlgebraicProjection,
                RelationPassStep::RuntimeAssignments,
            ],
            settle_pass: vec![
                SettleStep::DiscreteOwners,
                SettleStep::RuntimeAssignments,
                SettleStep::SettledIfUnchanged,
                SettleStep::AlgebraicProjection,
                SettleStep::RuntimeAssignments,
            ],
            fixed_point_cap: 32,
            stalled_settle: CoupledNewtonPolicy {
                iteration_cap: 32,
                line_search_halvings: 16,
            },
            relation_surface_window: 8,
            mode_search_relations: 4,
        }
    }

    pub fn event_pass(&self) -> &[EventPassStep] {
        &self.event_pass
    }

    pub fn relation_pass(&self) -> &[RelationPassStep] {
        &self.relation_pass
    }

    pub fn settle_pass(&self) -> &[SettleStep] {
        &self.settle_pass
    }

    /// Passes each of the three fixed points may take before it has stalled.
    pub const fn fixed_point_cap(&self) -> usize {
        self.fixed_point_cap
    }

    pub const fn stalled_settle(&self) -> &CoupledNewtonPolicy {
        &self.stalled_settle
    }

    /// The passes an iteration looks back over for a repeated state: a pass
    /// that returns to the discrete values and relation memory of one of them
    /// with the coordinate unchanged has entered a cycle, which the mode
    /// search of [`Self::mode_search_relations`] resolves (ME-EVENT-008). Zero
    /// disables cycle detection, so a cycle runs to the fixed-point cap.
    pub const fn relation_surface_window(&self) -> usize {
        self.relation_surface_window
    }

    /// Relations an iteration that entered a cycle may search jointly for a
    /// consistent mode: the cycling relations and every relation reading a
    /// coordinate one of them reads. Each assignment of their sides is
    /// projected, and the first whose relations all agree with the sides
    /// they were given is taken, which is how an MLS Appendix B iteration
    /// crosses a fold that its passes alone cannot reach. When every mode was
    /// searched and none is consistent, each state of the cycle is a fixed
    /// point on a surface where the relations' expression is continuous, so
    /// the current side is kept and counted as a relation-surface settle. A
    /// cycle with more relations than this, or with none, is refused.
    pub const fn mode_search_relations(&self) -> usize {
        self.mode_search_relations
    }

    /// Set [`Self::mode_search_relations`].
    #[must_use]
    pub const fn with_mode_search_relations(mut self, relations: usize) -> Self {
        self.mode_search_relations = relations;
        self
    }

    /// Set [`Self::relation_surface_window`]. A cycle visits both sides of a
    /// relation, so a nonzero window below two is refused.
    pub fn with_relation_surface_window(
        mut self,
        window: usize,
    ) -> Result<Self, EventScheduleError> {
        if window == 1 {
            return Err(EventScheduleError::SurfaceNeedsBothSides);
        }
        self.relation_surface_window = window;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn newton() -> CoupledNewtonPolicy {
        CoupledNewtonPolicy {
            iteration_cap: 4,
            line_search_halvings: 2,
        }
    }

    #[test]
    fn the_standard_schedule_is_well_formed() {
        let standard = EventIterationSchedule::standard();
        assert_eq!(
            EventIterationSchedule::new(
                standard.event_pass().to_vec(),
                standard.relation_pass().to_vec(),
                standard.settle_pass().to_vec(),
                standard.fixed_point_cap(),
                *standard.stalled_settle(),
            )
            .and_then(|schedule| schedule.with_relation_surface_window(8))
            .map(|schedule| schedule.with_mode_search_relations(4)),
            Ok(standard)
        );
    }

    #[test]
    fn the_standard_schedule_survives_serialization() {
        let standard = EventIterationSchedule::standard();
        let text = serde_json::to_string(&standard).expect("serializes");
        let back: EventIterationSchedule = serde_json::from_str(&text).expect("deserializes");
        assert_eq!(back, standard);
    }

    #[test]
    fn a_second_settle_check_is_refused_as_unreachable() {
        let standard = EventIterationSchedule::standard();
        let mut settle = standard.settle_pass().to_vec();
        settle.push(SettleStep::SettledIfUnchanged);
        assert_eq!(
            EventIterationSchedule::new(
                standard.event_pass().to_vec(),
                standard.relation_pass().to_vec(),
                settle,
                8,
                newton(),
            ),
            Err(EventScheduleError::SettlePassMustCheckOnce)
        );
    }

    #[test]
    fn malformed_passes_are_refused() {
        let standard = EventIterationSchedule::standard();
        let build = |event: Vec<EventPassStep>, relation: Vec<RelationPassStep>, cap| {
            EventIterationSchedule::new(
                event,
                relation,
                standard.settle_pass().to_vec(),
                cap,
                newton(),
            )
        };
        let relation = standard.relation_pass().to_vec();
        assert_eq!(
            build(standard.event_pass().to_vec(), relation.clone(), 0),
            Err(EventScheduleError::ZeroCap)
        );
        assert_eq!(
            build(
                vec![
                    EventPassStep::RelationSettle,
                    EventPassStep::RuntimeAssignments
                ],
                relation,
                8
            ),
            Err(EventScheduleError::EventPassMustEndWithRelationSettle)
        );
        assert_eq!(
            build(
                standard.event_pass().to_vec(),
                vec![
                    RelationPassStep::SettledIfRelationsUnchanged,
                    RelationPassStep::DiscreteSettle
                ],
                8
            ),
            Err(EventScheduleError::RelationPassMustSettleOnce)
        );
        let text = serde_json::to_string(&standard)
            .expect("serializes")
            .replace("\"fixed_point_cap\":32", "\"fixed_point_cap\":0");
        assert!(serde_json::from_str::<EventIterationSchedule>(&text).is_err());
    }
}

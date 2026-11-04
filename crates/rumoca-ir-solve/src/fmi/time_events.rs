//! Admission proof for time events in generated C (SPEC_0044 ME-EVENT-002).
//!
//! A time event is an instant the component announces before it is reached:
//! FMI Model Exchange reports it as `nextEventTime` from the discrete-state
//! update, and Co-Simulation stops its substep integration exactly there.
//! Every instant comes from a Solve IR owner the linked runtime's stop
//! schedule also reads (rumoca-solver runtime/schedule.rs and
//! runtime/solve_events.rs): the static instants, the dynamic deadline rows
//! and named deadline slots, and the parameter a
//! direct time root compares `time` against, whose instant the schedule
//! announces instead of searching it ([`crate::RootSearchRole::AnnouncedTime`]).
//! The component evaluates the event at its right limit, the representable
//! time after the instant, as the runtime does for a scheduled stop that
//! follows the current values, so each time relation takes its post-event
//! side, and every read at the instant after the event keeps that right limit
//! until integration moves on (component.rs `public_time_eval_time`).
//!
//! The MLS §3.7.5 `sample(start, interval)` clocks of the Solve clock
//! partition are time events too (SPEC_0044 ME-EVENT-010): each announces its
//! next unconsumed tick `phase + k * period`, the instant evaluated from the
//! tick counter `k` rather than accumulated, and a tick is consumed exactly
//! once, by the event at its instant ([`super::sample_events`]).
//!
//! Periodic sample roots (`events.scheduled_root_conditions`) are refused: no
//! Solve lowering produces them (`sample()` lowers to periodic clock
//! schedules), so the C profile has no executed path that could show their
//! arm-then-clear relation memory is right.

use serde::Serialize;

use crate::{RootSearchPlan, RootSearchRole, SolveProblem};

/// The Solve IR time-event owners the generated component schedules.
#[derive(Debug, Serialize)]
pub(super) struct TimeEventProfile {
    /// Static instants, ascending, tolerance-coincident instants merged.
    instants: Vec<f64>,
    /// The parameter each direct time root compares `time` against.
    announced: Vec<usize>,
    /// Number of dynamic deadline rows (`solve.events.dynamic_time_event_rhs`).
    deadlines: usize,
    /// Parameters that hold a named dynamic deadline.
    named_deadlines: Vec<usize>,
    /// The `initial()` flag parameter, set only during the initial event.
    initial_flag: Option<usize>,
    /// `rumoca_core::SCHEDULE_TIME_RELATIVE_TOLERANCE`: two instants within
    /// `tol * (1 + max(|a|, |b|))` are one instant.
    match_tolerance: f64,
    /// The periodic `sample(start, interval)` clocks, in Solve clock order.
    samples: Vec<super::sample_events::SampleClock>,
}

/// Admit the problem's time events, or say why the C profile cannot run one.
pub(super) fn validate(
    problem: &SolveProblem,
    search: &RootSearchPlan,
) -> Result<TimeEventProfile, &'static str> {
    let events = &problem.events;
    let tolerance = rumoca_core::SCHEDULE_TIME_RELATIVE_TOLERANCE;
    let mut instants = events
        .scheduled_time_events
        .iter()
        .copied()
        .filter(|instant| instant.is_finite())
        .collect::<Vec<_>>();
    instants.sort_by(f64::total_cmp);
    instants.dedup_by(|later, earlier| {
        (*later - *earlier).abs() <= tolerance * (1.0 + later.abs().max(earlier.abs()))
    });
    if !events.scheduled_root_conditions.is_empty() {
        return Err("the C profile does not execute periodic sample roots");
    }
    let announced = search
        .roles()
        .iter()
        .filter_map(|role| match role {
            RootSearchRole::AnnouncedTime { param_index, .. } => Some(*param_index),
            RootSearchRole::Search | RootSearchRole::Static => None,
        })
        .collect::<Vec<_>>();
    let layout = &problem.solve_layout;
    let named_deadlines = events
        .dynamic_time_event_names
        .iter()
        .map(|name| {
            layout
                .input_parameter_index(name)
                .or_else(|| layout.discrete_real_parameter_index(name))
                .or_else(|| layout.discrete_valued_parameter_index(name))
                .ok_or("a named dynamic time event names no runtime parameter")
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TimeEventProfile {
        instants,
        announced,
        deadlines: events.dynamic_time_event_rhs.output_count(),
        named_deadlines,
        initial_flag: layout.initial_event_parameter_index,
        match_tolerance: tolerance,
        samples: super::sample_events::clocks(problem)?,
    })
}

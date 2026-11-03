//! Admission proof for state events in generated C (SPEC_0044 ME-EVENT-002).
//!
//! The scalar event profile executes Appendix B event iteration over scalar
//! discrete rows: root relations with relation memory, discrete equations and
//! condition memories that read `pre` values following the current pass, and
//! unscheduled assertions. Everything the component decides comes from Solve
//! IR facts: the searched roots and their indicator table, the relation-memory
//! targets and zero domains, the event iteration schedule, and the pre-binding
//! lanes of each iteration run. Runtime, post-commit, and unclocked guarded
//! assignments run as their row blocks. Time events are scheduled by
//! [`super::time_events`]. Clocks, delays, structured updates, and event
//! transactions are refused here.

use serde::Serialize;

use crate::{
    DiscreteEventPreMode, DiscreteRowRole, EventIterationOwner, PreParamSource, RootSearchPlan,
    RootSearchRole, RootZeroDomain, ScalarSlot, SolveEventActionKind, SolveModel,
};

/// The Solve IR facts the generated component reads to execute events.
#[derive(Debug, Serialize)]
pub(super) struct ScalarEventProfile {
    /// Per root output: whether and how it is evaluated in Event Mode.
    roots: Vec<ScalarEventRoot>,
    /// Destination and source of every pre binding, in binding order.
    pre_bindings: Vec<PreLane>,
    /// The pre bindings (indices into `pre_bindings`) an event pass advances
    /// and whose settlement ends the iteration.
    iteration_lanes: Vec<usize>,
    /// The pre bindings (indices into `pre_bindings`) of the MLS §8.6 steady
    /// states `pre(m) = m`, which the initial event seeds from the value `m`
    /// settles at (initial_event.rs apply_steady_initial_event_update).
    steady_lanes: Vec<usize>,
    /// Per discrete output: its parameter target.
    discrete_targets: Vec<usize>,
    /// Per discrete output: whether it is a condition memory, the rows
    /// initialization seeds with `pre` following the current value.
    condition_memory_rows: Vec<bool>,
    /// Targets of the runtime assignments and of the post-commit
    /// assignments, in output order.
    runtime_targets: Vec<Slot>,
    /// Per guarded-assignment output, in program order: its target.
    guarded_targets: Vec<Slot>,
    /// Per guarded-assignment program, in issued order: its output count.
    /// Each program's outputs reach the pass's working state before the next
    /// program runs (discrete_rows.rs collect_guarded_discrete_row_values,
    /// SOLVE-C58).
    guarded_program_outputs: Vec<usize>,
    post_commit_targets: Vec<Slot>,
    /// The parameters actions read at their event-entry value.
    condition_memories: Vec<usize>,
    /// Per root output: the outputs reading a coordinate it reads, itself
    /// included (ME-EVENT-008 mode search).
    root_neighborhoods: Vec<Vec<usize>>,
    /// The event iteration schedule the component walks (ME-EVENT-006).
    schedule: crate::EventIterationSchedule,
    /// The time events the component announces and stops at.
    time_events: super::time_events::TimeEventProfile,
}

#[derive(Debug, Serialize)]
struct ScalarEventRoot {
    /// `search`, `announced_time`, or `static`.
    role: &'static str,
    /// The relation-memory parameter this root writes, if any.
    memory: Option<usize>,
    /// `positive`, `non_positive`, or `previous`.
    zero: &'static str,
    /// Whether the relation memory is refreshed from the algebraic
    /// coordinate after the event commit.
    algebraic_dependent: bool,
}

#[derive(Debug, Serialize)]
struct Slot {
    /// `y` or `p`.
    column: &'static str,
    index: usize,
}

fn slots(targets: &[ScalarSlot]) -> Result<Vec<Slot>, &'static str> {
    targets
        .iter()
        .map(|slot| match slot {
            ScalarSlot::Y { index, .. } => Ok(Slot {
                column: "y",
                index: *index,
            }),
            ScalarSlot::P { index, .. } => Ok(Slot {
                column: "p",
                index: *index,
            }),
            _ => Err("an assignment writes neither a solver coordinate nor a parameter"),
        })
        .collect()
}

#[derive(Debug, Serialize)]
struct PreLane {
    dest: usize,
    /// `y` or `p`.
    column: &'static str,
    source: usize,
}

/// The pre bindings (indices into `pre_bindings`) of the steady-state `pre`
/// slots, each of which must have one.
fn steady_lanes(steady: &[usize], pre_bindings: &[PreLane]) -> Result<Vec<usize>, &'static str> {
    let lanes = pre_bindings
        .iter()
        .enumerate()
        .filter(|(_, lane)| steady.contains(&lane.dest))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if lanes.len() != steady.len() {
        return Err("a steady-state initial equation names a pre slot without a pre binding");
    }
    Ok(lanes)
}

/// Admit the scalar event profile, or say why the model needs more.
pub(super) fn validate(model: &SolveModel) -> Result<ScalarEventProfile, &'static str> {
    let problem = &model.problem;
    refuse_unsupported_owners(model)?;
    let events = &problem.events;
    let discrete = &problem.discrete;
    let search = RootSearchPlan::derive(problem);
    let count = events.root_conditions.output_count();
    if search.len() != count
        || events.root_zero_domains.len() != count
        || events.root_relation_memory_targets.len() != count
        || events.root_relation_refresh_roles.len() != count
    {
        return Err("the root tables disagree on the root count");
    }
    let roots = (0..count)
        .map(|index| scalar_event_root(problem, &search, index))
        .collect::<Result<Vec<_>, _>>()?;
    let discrete_targets = discrete
        .update_targets
        .iter()
        .map(|slot| match slot {
            ScalarSlot::P { index, .. } => Ok(*index),
            _ => Err("a discrete row writes outside the parameters"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if discrete.row_roles.iter().any(|role| {
        !matches!(
            role,
            DiscreteRowRole::Equation | DiscreteRowRole::ConditionMemory
        )
    }) {
        return Err("the C profile executes only discrete equations and condition memories");
    }
    if discrete
        .pre_modes
        .iter()
        .any(|mode| *mode != DiscreteEventPreMode::FollowCurrent)
    {
        return Err("the C profile executes only discrete rows whose pre follows the current pass");
    }
    let (guarded_targets, guarded_program_outputs) = guarded_targets(discrete)?;
    let layout = &problem.solve_layout;
    let pre_bindings = layout
        .pre_param_bindings
        .iter()
        .map(|binding| match binding.source {
            PreParamSource::Y { index } => PreLane {
                dest: binding.dest_p_index,
                column: "y",
                source: index,
            },
            PreParamSource::P { index } => PreLane {
                dest: binding.dest_p_index,
                column: "p",
                source: index,
            },
        })
        .collect::<Vec<_>>();
    let mut iteration_lanes = Vec::new();
    for run in &discrete.event_iteration_plan.runs {
        // pre_params.rs event_iteration_plan_settled compares every
        // unclocked run's lanes alike; the clock-free guarded owners are
        // admitted by `guarded_targets`.
        if !matches!(
            run.owner,
            EventIterationOwner::ScalarRows { .. }
                | EventIterationOwner::GuardedAssignment { .. }
                | EventIterationOwner::Hold
        ) {
            return Err("the C profile iterates only scalar rows and guarded assignments");
        }
        let storage = layout
            .variable_storage_runs
            .get(run.variable)
            .ok_or("an event iteration run names no storage")?;
        let lanes = run.pre_binding_start..run.pre_binding_start + storage.scalar_count;
        if lanes.end > pre_bindings.len() {
            return Err("an event iteration run names a pre binding outside the layout");
        }
        iteration_lanes.extend(lanes);
    }
    let steady_lanes = steady_lanes(problem.initialization.steady_pre(), &pre_bindings)?;
    Ok(ScalarEventProfile {
        roots,
        pre_bindings,
        iteration_lanes,
        steady_lanes,
        condition_memory_rows: discrete
            .row_roles
            .iter()
            .map(|role| *role == DiscreteRowRole::ConditionMemory)
            .collect(),
        discrete_targets,
        runtime_targets: slots(&discrete.runtime_assignment_targets)?,
        guarded_targets,
        guarded_program_outputs,
        post_commit_targets: slots(&discrete.post_commit_assignment_targets)?,
        condition_memories: events.condition_memory_parameter_indices.clone(),
        schedule: discrete.event_iteration_plan.schedule.clone(),
        root_neighborhoods: crate::root_neighborhoods(&events.root_conditions)
            .map_err(|_| "a root condition's coordinate reads cannot be derived")?,
        time_events: super::time_events::validate(problem, &search)?,
    })
}

fn refuse_unsupported_owners(model: &SolveModel) -> Result<(), &'static str> {
    let problem = &model.problem;
    let events = &problem.events;
    let discrete = &problem.discrete;
    if crate::solve_has_runtime_events(problem) || crate::solve_has_clocks(problem) {
        return Err("the C profile cannot execute delays, terminal events, or clocks");
    }
    if events.actions.iter().any(|action| {
        !matches!(
            action.kind,
            SolveEventActionKind::Assert | SolveEventActionKind::Warning
        ) || action.clock_owner.is_some()
    }) {
        return Err("the C profile executes only unscheduled assertions");
    }
    if !discrete.structured_rhs.is_empty()
        || !discrete.structured_updates.is_empty()
        || !discrete.event_transactions.is_empty()
    {
        return Err("the C profile cannot execute structured or transactional discrete updates");
    }
    if problem
        .solve_layout
        .pre_param_bindings
        .iter()
        .any(|binding| binding.clock_schedule.is_some())
    {
        return Err("the C profile cannot execute clocked previous() history");
    }
    Ok(())
}

fn scalar_event_root(
    problem: &crate::SolveProblem,
    search: &RootSearchPlan,
    index: usize,
) -> Result<ScalarEventRoot, &'static str> {
    let events = &problem.events;
    let role = match search.roles()[index] {
        RootSearchRole::Search => "search",
        RootSearchRole::AnnouncedTime { .. } => "announced_time",
        RootSearchRole::Static => "static",
    };
    let memory = match events.root_relation_memory_targets[index] {
        None => None,
        Some(ScalarSlot::P { index, .. }) => Some(index),
        Some(_) => return Err("a relation memory lives outside the parameters"),
    };
    let zero = match events.root_zero_domains[index] {
        RootZeroDomain::Positive => "positive",
        RootZeroDomain::NonPositive => "non_positive",
        RootZeroDomain::Previous => "previous",
    };
    Ok(ScalarEventRoot {
        role,
        memory,
        zero,
        algebraic_dependent: events.root_relation_refresh_roles[index]
            == crate::RootRelationRefreshRole::AlgebraicDependent,
    })
}

/// The guarded assignments' targets, expanded from their compact ranges in
/// program and range order, and each program's output count; an owner under
/// a clock is refused.
fn guarded_targets(
    discrete: &crate::DiscreteSolveSystem,
) -> Result<(Vec<Slot>, Vec<usize>), &'static str> {
    let mut targets = Vec::new();
    let mut counts = Vec::new();
    for program in &discrete.guarded_assignments {
        let first = targets.len();
        if program.clock_owner().is_some() {
            return Err("the C profile cannot execute clocked guarded assignments");
        }
        for range in program.target_ranges() {
            let (column, base) = match range.base() {
                ScalarSlot::Y { index, .. } => ("y", index),
                ScalarSlot::P { index, .. } => ("p", index),
                _ => {
                    return Err(
                        "a guarded assignment writes neither a solver coordinate nor a parameter",
                    );
                }
            };
            targets.extend((0..range.count()).map(|offset| Slot {
                column,
                index: base + offset,
            }));
        }
        counts.push(targets.len() - first);
    }
    Ok((targets, counts))
}

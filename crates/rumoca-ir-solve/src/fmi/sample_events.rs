//! Admission proof for MLS §3.7.5 `sample(start, interval)` time events in
//! generated C (SPEC_0044 ME-EVENT-010).
//!
//! Phase DAE gives each `when sample(start, interval)` an exact periodic
//! schedule, and Solve lowers its body to clock-owned discrete rows and
//! guarded assignments listed in issued causal order by
//! `discrete.clock_partition_order` (SOLVE-C57). When that partition needs no
//! MLS §16 synchronous construct (`crate::solve_has_synchronous_clocks`), it
//! is a set of `when` clauses on periodic time events, and the C profile runs
//! it as the linked runtime does (discrete_rows.rs
//! `execute_clock_partition_steps`): once per tick, in the first pass of the
//! tick's event iteration, each step's outputs reaching the working state
//! before the next step runs, every `pre` read taking its event-entry lane.
//! Later passes never rerun a clock-owned producer.
//!
//! The ticks themselves are time events ([`super::time_events`]): tick `k` of
//! a clock is the instant `phase + k * period`, computed from the tick
//! counter rather than accumulated, so no drift builds up and every tick runs
//! exactly once however the importer's communication points fall.

use serde::Serialize;

use crate::{ClockPartitionStep, DiscreteSolveSystem, ScalarProgramBlock, SolveProblem};

/// One periodic sample clock the component schedules.
///
/// The exact lattice is carried as `phase = phase_units / units` and
/// `period = period_units / units` over one common denominator, each an
/// integer the C `double` holds exactly, so tick `k` is
/// `(phase_units + k * period_units) / units` rounded once, the instant the
/// linked runtime's `ClockLattice::tick_time_seconds` reports.
#[derive(Debug, Serialize)]
pub(super) struct SampleClock {
    /// Instant of tick zero in seconds; relative to the instance start time
    /// when `start_relative`.
    phase: f64,
    /// Seconds between ticks.
    period: f64,
    phase_units: f64,
    period_units: f64,
    units: f64,
    /// The schedule's phase is relative to the simulation start
    /// (`rumoca_core::ClockPhaseAnchor::SimulationStart`).
    start_relative: bool,
    /// The hidden activation parameter that holds `1` exactly while the clock
    /// ticks (SOLVE-C11).
    lane: usize,
    /// A parametric `sample(start, interval)`: the parameter slots whose
    /// initialization values are its start and interval. The lattice fields
    /// above are then unused.
    start_slot: Option<usize>,
    interval_slot: Option<usize>,
}

/// One producer of the sample partition, in issued order.
#[derive(Debug, Serialize)]
pub(super) struct SampleStep {
    /// `rows` (discrete rows of one `rhs` program), `guarded` (one guarded
    /// assignment program), or `intermediate` (one same-tick algebraic
    /// refresh row of `clock_partition_intermediates`).
    kind: &'static str,
    /// The clocks any of whose ticks runs the step.
    clocks: Vec<usize>,
    /// The `rhs`, guarded-assignment, or intermediate program index.
    program: usize,
    /// `intermediate`: the refreshed slot, `y` or `p`, and its index.
    column: &'static str,
    index: usize,
    /// `rows`: the discrete rows the program writes.
    rows: Vec<usize>,
    /// `guarded`: the first output in the guarded-target numbering.
    offset: usize,
    /// `guarded`: the program's output count.
    count: usize,
}

/// The clock-owned producers and the per-owner masks the unclocked passes
/// skip.
#[derive(Debug, Default, Serialize)]
pub(super) struct SamplePartition {
    steps: Vec<SampleStep>,
    /// Per `rhs` program: whether a sample clock owns its rows.
    clocked_programs: Vec<bool>,
    /// Per discrete row: whether a sample clock owns it.
    clocked_rows: Vec<bool>,
    /// Per guarded-assignment program: whether a sample clock owns it.
    clocked_guarded: Vec<bool>,
}

/// The sample clocks of `problem`, or why the C profile cannot schedule them.
pub(super) fn clocks(problem: &SolveProblem) -> Result<Vec<SampleClock>, &'static str> {
    let clocks = &problem.clocks;
    if clocks.periodic_event_schedules.len() != clocks.activation_parameter_indices.len() {
        return Err("the sample clocks and their activation lanes disagree");
    }
    if crate::solve_has_synchronous_clocks(problem) {
        return Err("the C profile cannot execute MLS §16 synchronous clock partitions");
    }
    clocks
        .periodic_event_schedules
        .iter()
        .zip(&clocks.activation_parameter_indices)
        .enumerate()
        .map(|(index, (schedule, &lane))| {
            if let Some(slots) = clocks.parametric_sample(index) {
                return Ok(SampleClock {
                    phase: 0.0,
                    period: 0.0,
                    phase_units: 0.0,
                    period_units: 0.0,
                    units: 0.0,
                    start_relative: false,
                    lane,
                    start_slot: Some(slots.start),
                    interval_slot: Some(slots.interval),
                });
            }
            let (phase, period) = (schedule.phase_seconds(), schedule.period_seconds());
            if !phase.is_finite() || !period.is_finite() || period <= 0.0 {
                return Err("a sample clock has no finite positive period");
            }
            let (phase_units, period_units, units) = exact_units(schedule.lattice())
                .ok_or("a sample clock's exact lattice exceeds the C double integer range")?;
            Ok(SampleClock {
                phase,
                period,
                phase_units,
                period_units,
                units,
                start_relative: schedule.anchor() == rumoca_core::ClockPhaseAnchor::SimulationStart,
                lane,
                start_slot: None,
                interval_slot: None,
            })
        })
        .collect()
}

/// The lattice over one common denominator, every term an integer of at most
/// 53 bits.
fn exact_units(lattice: rumoca_core::ClockLattice) -> Option<(f64, f64, f64)> {
    let (phase, period) = (lattice.phase(), lattice.period());
    let gcd = gcd(phase.denominator(), period.denominator());
    let units = phase
        .denominator()
        .checked_div(gcd)?
        .checked_mul(period.denominator())?;
    let phase_units = phase.numerator().checked_mul(units / phase.denominator())?;
    let period_units = period
        .numerator()
        .checked_mul(units / period.denominator())?;
    const EXACT: i128 = 1 << 53;
    [phase_units, period_units, units]
        .iter()
        .all(|value| value.abs() < EXACT)
        .then_some((phase_units as f64, period_units as f64, units as f64))
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.abs().max(1)
}

/// Admit the sample partition: every clock-owned producer is a scalar-row or
/// guarded-assignment step of `clock_partition_order`, and every `rhs`
/// program is wholly clocked or wholly unclocked.
pub(super) fn partition(
    discrete: &DiscreteSolveSystem,
    guarded_program_outputs: &[usize],
) -> Result<SamplePartition, &'static str> {
    let clocked_rows = discrete
        .clock_owners
        .iter()
        .map(Option::is_some)
        .collect::<Vec<_>>();
    let clocked_programs = clocked_programs(&discrete.rhs, &clocked_rows)?;
    let clocked_guarded = discrete
        .guarded_assignments
        .iter()
        .map(|program| program.clock_owner().is_some())
        .collect::<Vec<_>>();
    let mut offsets = Vec::with_capacity(guarded_program_outputs.len());
    let mut offset = 0usize;
    for count in guarded_program_outputs {
        offsets.push(offset);
        offset += count;
    }
    let mut steps = Vec::with_capacity(discrete.clock_partition_order.len());
    for step in &discrete.clock_partition_order {
        steps.push(match *step {
            ClockPartitionStep::ScalarRows { start_row, count } => {
                rows_step(discrete, start_row, count)?
            }
            ClockPartitionStep::GuardedAssignment { program_index } => {
                let clock = discrete
                    .guarded_assignments
                    .get(program_index)
                    .and_then(|program| program.clock_owner())
                    .ok_or("a guarded partition step names no clock-owned program")?;
                SampleStep {
                    kind: "guarded",
                    clocks: vec![clock.index()],
                    program: program_index,
                    column: "p",
                    index: 0,
                    rows: Vec::new(),
                    offset: offsets[program_index],
                    count: guarded_program_outputs[program_index],
                }
            }
            ClockPartitionStep::Intermediate { row } => intermediate_step(discrete, row)?,
            ClockPartitionStep::StructuredUpdate { .. }
            | ClockPartitionStep::EventTransaction { .. } => {
                return Err(
                    "the C profile runs only scalar-row, guarded, and intermediate sample steps",
                );
            }
        });
    }
    let covered_rows = steps
        .iter()
        .filter(|step| step.kind == "rows")
        .map(|step| step.rows.len())
        .sum::<usize>();
    let covered_guarded = steps.iter().filter(|step| step.kind == "guarded").count();
    if covered_rows != clocked_rows.iter().filter(|clocked| **clocked).count()
        || covered_guarded != clocked_guarded.iter().filter(|clocked| **clocked).count()
    {
        return Err("a clock-owned producer is missing from the clock partition order");
    }
    Ok(SamplePartition {
        steps,
        clocked_programs,
        clocked_rows,
        clocked_guarded,
    })
}

fn rows_step(
    discrete: &DiscreteSolveSystem,
    start_row: usize,
    count: usize,
) -> Result<SampleStep, &'static str> {
    let rows = (start_row..start_row + count).collect::<Vec<_>>();
    let clock = rows
        .first()
        .and_then(|row| discrete.clock_owners.get(*row).copied().flatten())
        .ok_or("a scalar partition step names an unclocked row")?;
    let program = discrete
        .rhs
        .program_index_for_output(start_row)
        .ok_or("a scalar partition step names no producing program")?;
    for &row in &rows {
        if discrete.clock_owners.get(row).copied().flatten() != Some(clock)
            || discrete.rhs.program_index_for_output(row) != Some(program)
        {
            return Err("a scalar partition step spans several clocks or programs");
        }
    }
    Ok(SampleStep {
        kind: "rows",
        clocks: vec![clock.index()],
        program,
        column: "p",
        index: 0,
        rows,
        offset: 0,
        count: 0,
    })
}

/// One same-tick algebraic refresh: the linked runtime writes it into the
/// tick's work state; the C profile writes the coordinate itself, which the
/// event iteration's own refresh then settles.
fn intermediate_step(
    discrete: &DiscreteSolveSystem,
    row: usize,
) -> Result<SampleStep, &'static str> {
    let block = &discrete.clock_partition_intermediates;
    let program = block
        .program_index_for_output(row)
        .ok_or("a clock-partition intermediate has no producing program")?;
    if crate::ScalarProgramBlock::program_output_count(&block.programs()[program]) != 1 {
        return Err("a clock-partition intermediate program writes several rows");
    }
    let (column, index) = match discrete.clock_partition_intermediate_targets.get(row) {
        Some(crate::ScalarSlot::Y { index, .. }) => ("y", *index),
        Some(crate::ScalarSlot::P { index, .. }) => ("p", *index),
        _ => return Err("a clock-partition intermediate refreshes neither y nor p"),
    };
    let clocks = discrete
        .clock_partition_intermediate_clocks
        .get(row)
        .map(|clocks| clocks.iter().map(|clock| clock.index()).collect::<Vec<_>>())
        .filter(|clocks| !clocks.is_empty())
        .ok_or("a clock-partition intermediate has no consumer clock")?;
    Ok(SampleStep {
        kind: "intermediate",
        clocks,
        program,
        column,
        index,
        rows: vec![row],
        offset: 0,
        count: 0,
    })
}

/// Per `rhs` program: whether its rows are clocked, refusing a program that
/// writes both clocked and unclocked rows.
fn clocked_programs(
    rhs: &ScalarProgramBlock,
    clocked_rows: &[bool],
) -> Result<Vec<bool>, &'static str> {
    let mut programs: Vec<Option<bool>> = vec![None; rhs.programs().len()];
    for (row, &clocked) in clocked_rows.iter().enumerate() {
        let program = rhs
            .program_index_for_output(row)
            .ok_or("a discrete row has no producing program")?;
        match programs[program] {
            Some(previous) if previous != clocked => {
                return Err("a discrete program writes clocked and unclocked rows");
            }
            _ => programs[program] = Some(clocked),
        }
    }
    Ok(programs
        .into_iter()
        .map(|clocked| clocked.unwrap_or(false))
        .collect())
}

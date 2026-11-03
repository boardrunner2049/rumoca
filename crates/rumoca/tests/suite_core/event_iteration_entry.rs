//! MLS §8.5 event iteration: an unclocked `when` fires on its conditions at the
//! event-iteration entry, while its values observe the other equations of the
//! same instant.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, SimSolverMode, simulate_dae};

fn simulate(source: &str, model: &str, solver_mode: SimSolverMode) -> SimResult {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("{model} compiles: {error:?}"),
    };
    let options = SimOptions {
        solver_mode,
        t_end: 1.0,
        dt: Some(0.01),
        ..Default::default()
    };
    let result = match simulate_dae(&compiled.dae, &options) {
        Ok(result) => result,
        Err(error) => panic!("{model} simulates: {error:?}"),
    };
    assert!(result.times.len() > 10, "{model} produced an output grid");
    result
}

/// The value `name` holds at the last output instant not after `time`.
fn value_at(result: &SimResult, name: &str, time: f64) -> f64 {
    let Some(column) = result.names.iter().position(|candidate| candidate == name) else {
        panic!("result has no column `{name}`");
    };
    let row = result
        .times
        .iter()
        .rposition(|candidate| *candidate <= time + 1.0e-12)
        .expect("an output instant precedes the probe");
    result.data[column][row]
}

fn assert_counts(source: &str, model: &str) {
    for solver_mode in [SimSolverMode::RkLike, SimSolverMode::Bdf] {
        let result = simulate(source, model, solver_mode);
        for (time, count) in [(0.1, 0.0), (0.3, 1.0), (0.5, 2.0), (0.7, 3.0), (0.9, 4.0)] {
            assert_eq!(
                value_at(&result, "count", time),
                count,
                "{model} {solver_mode:?} count at {time}"
            );
        }
    }
}

/// A `when` that reschedules its own trigger: the update of `nextTime` belongs
/// to the same instant, so it does not disarm the trigger that fired it.
#[test]
fn a_self_rescheduling_when_fires_every_period() {
    let source = r#"
model SelfRescheduling
  discrete Real nextTime(start = 0.2, fixed = true);
  discrete Real count(start = 0, fixed = true);
  Real clock_(start = 0, fixed = true);
equation
  der(clock_) = 1;
  when time >= nextTime then
    count = pre(count) + 1;
    nextTime = pre(nextTime) + 0.2;
  end when;
end SelfRescheduling;
"#;
    assert_counts(source, "SelfRescheduling");
}

/// The trigger that reads `nextTime` belongs to another `when`; it still sees
/// the entry value of the iteration in which `nextTime` is rescheduled.
#[test]
fn a_trigger_reads_the_iteration_entry_of_another_when() {
    let source = r#"
model SplitRescheduling
  discrete Real nextTime(start = 0.2, fixed = true);
  discrete Real count(start = 0, fixed = true);
  Real clock_(start = 0, fixed = true);
equation
  der(clock_) = 1;
  when time >= nextTime then
    nextTime = pre(nextTime) + 0.2;
  end when;
  when time >= nextTime then
    count = pre(count) + 1;
  end when;
end SplitRescheduling;
"#;
    assert_counts(source, "SplitRescheduling");
}

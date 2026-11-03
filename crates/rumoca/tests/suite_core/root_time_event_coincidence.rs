//! SPEC_0044 ME-EVENT-004: a state-event root located within the
//! `RootLocationPlan` roundoff of a scheduled time event coincides with it and
//! is handled in the time event's iteration, with post-time-event values; a
//! root clearly before the time event is its own event, located on the
//! event's left limit.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimSolverMode, simulate_dae_with_diagnostics};

struct Outcome {
    /// The first published time carrying the post-time-event threshold.
    threshold_moved_at: f64,
    ticks: f64,
}

/// `x` reaches the pre-event threshold `offset` before the time event at
/// `t = 1`, where the threshold moves out of reach.
fn run(offset: &str) -> Outcome {
    let source = format!(
        r#"
model Coincidence
  Real x(start = 0, fixed = true);
  Real thr;
  discrete Integer n(start = 0, fixed = true);
equation
  der(x) = 1;
  thr = if time < 1 then 1 - {offset} else 3;
  when x > thr then
    n = pre(n) + 1;
  end when;
end Coincidence;
"#
    );
    let compiled = Compiler::new()
        .model("Coincidence")
        .compile_str(&source, "root_time_event_coincidence.mo")
        .unwrap();
    let options = SimOptions {
        t_end: 1.5,
        solver_mode: SimSolverMode::Auto,
        ..SimOptions::default()
    };
    let result = simulate_dae_with_diagnostics(&compiled.dae, &options).unwrap();
    let column = |name: &str| result.names.iter().position(|n| n == name).unwrap();
    let (thr, n) = (column("thr"), column("n"));
    let moved = result.data[thr].iter().position(|v| *v == 3.0).unwrap();
    Outcome {
        threshold_moved_at: result.times[moved],
        ticks: *result.data[n].last().unwrap(),
    }
}

/// A root 3e-15 s before the time event is that event: it is handled at
/// `t = 1` in the time event's iteration, which sees the moved threshold, so
/// the `when` never fires.
#[test]
fn a_root_within_roundoff_of_a_time_event_is_handled_in_its_iteration() {
    let outcome = run("3e-15");
    assert_eq!(outcome.threshold_moved_at, 1.0);
    assert_eq!(outcome.ticks, 0.0);
}

/// A root 1e-6 s before the time event is its own event. The scan reads the
/// step's endpoint at the time event's left limit, so the crossing is not
/// hidden by the post-event threshold.
#[test]
fn a_root_clearly_before_a_time_event_is_its_own_event() {
    let outcome = run("1e-6");
    assert_eq!(outcome.threshold_moved_at, 1.0);
    assert_eq!(outcome.ticks, 1.0);
}

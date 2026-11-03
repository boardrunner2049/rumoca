//! `floor`, `ceil`, and `integer` generate events (MLS 3.7 §3.7.2).
//!
//! "div, ceil, floor, integer can only change values at events and will
//! trigger events as needed." A discrete target is recomputed only at event
//! instants, so an Integer defined from one of them must own the integer
//! crossings of its argument; without that owner it keeps its initial value.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package Steps
  model Integers
    Integer i = integer(u);
    Integer f = integer(floor(u));
    Integer c = integer(ceil(u));
    Real u = 3*time + 0.1;
  end Integers;
  model Assigned
    Integer n;
    Real u = 3*time;
  algorithm
    n := integer(u);
    n := n + 1;
  end Assigned;
end Steps;
"#;

fn simulate(model: &str) -> SimResult {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(MODELS, "Steps.mo")
        .expect("the model compiles");
    simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            dt: Some(0.25),
            ..SimOptions::default()
        },
    )
    .expect("the model simulates")
}

fn value_at(result: &SimResult, name: &str, time: f64) -> f64 {
    let column = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .expect("the result records the column");
    let row = result
        .times
        .iter()
        .position(|sample| (sample - time).abs() < 1e-9)
        .expect("the result has a sample at the time");
    result.data[column][row]
}

#[test]
fn integer_steps_update_discrete_targets_at_each_crossing() {
    // `u = 3*time + 0.1` is 0.85 at 0.25, 1.6 at 0.5, and 2.35 at 0.75.
    let result = simulate("Steps.Integers");
    for (time, floor) in [(0.25, 0.0), (0.5, 1.0), (0.75, 2.0)] {
        assert_eq!(value_at(&result, "i", time), floor, "integer at {time}");
        assert_eq!(value_at(&result, "f", time), floor, "floor at {time}");
        assert_eq!(value_at(&result, "c", time), floor + 1.0, "ceil at {time}");
    }
    let result = simulate("Steps.Assigned");
    assert_eq!(value_at(&result, "n", 0.25), 1.0);
    assert_eq!(value_at(&result, "n", 0.75), 3.0);
}

//! Relations in a model algorithm outside `when` generate events (MLS §8.5).
//!
//! A discrete target is recomputed only at event instants, so an algorithm
//! that assigns an Integer from a relation must own that relation's crossing,
//! exactly as the same expression in an equation does. Without the owner the
//! target keeps its initial value for the whole run.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package Relations
  model Statement "an if-statement condition"
    Integer n;
    Real u = 3*time;
  algorithm
    if u > 1 then
      n := 2;
    else
      n := 1;
    end if;
  end Statement;
  model Value "an if-expression in an assignment value"
    Integer n;
    Real u = 3*time;
  algorithm
    n := if u > 1 then 2 else 1;
  end Value;
  model Reassigned "a read of the target after its own definition"
    Integer n;
    Real u = 3*time;
  algorithm
    n := if u > 1 then 2 else 1;
    n := n + 1;
  end Reassigned;
end Relations;
"#;

fn simulate(model: &str) -> SimResult {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(MODELS, "Relations.mo")
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
fn an_algorithm_relation_updates_its_discrete_target_at_the_crossing() {
    // `u = 3*time` crosses 1 at t = 1/3.
    for (model, before, after) in [
        ("Relations.Statement", 1.0, 2.0),
        ("Relations.Value", 1.0, 2.0),
        ("Relations.Reassigned", 2.0, 3.0),
    ] {
        let result = simulate(model);
        assert_eq!(value_at(&result, "n", 0.25), before, "{model} before");
        assert_eq!(value_at(&result, "n", 0.5), after, "{model} after");
    }
}

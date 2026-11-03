//! `der(...)` in a conditional arm a structural selection never takes
//! (MLS 3.7 §8.3.4, SPEC_0040 DAE-C22).
//!
//! `Blocks.Interfaces.Adaptors.FlowToPotentialAdaptor` writes
//! `y1 = if use_pder then der(y) else 0` with `use_pder` an `Evaluate = true`
//! parameter. With `use_pder = false` the arm is not part of the flattened
//! system, so `y` is not a state: classifying it as one leaves a state
//! derivative no equation determines, and the system is structurally singular.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package DeadArms
  model Adaptor
    parameter Boolean use_pder = true annotation(Evaluate = true);
    input Real p;
    Real y;
    Real y1;
  equation
    y = p;
    y1 = if use_pder then der(y) else 0;
  end Adaptor;
  model Driven "an inductor current driven through a disabled derivative output"
    Adaptor a(use_pder = false);
    Real i(start = 0, fixed = true);
    Real v;
  equation
    a.p = i;
    der(i) = v;
    v = cos(time);
  end Driven;
  model Selected "the same adaptor with its derivative arm selected"
    Adaptor a(use_pder = true);
    Real i(start = 0, fixed = true);
  equation
    a.p = i;
    der(i) = cos(time);
  end Selected;
end DeadArms;
"#;

fn last(model: &str, name: &str) -> f64 {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(MODELS, "DeadArms.mo")
        .expect("the model compiles");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("the model simulates");
    let index = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .expect("the result records the column");
    *result.data[index].last().expect("a sample")
}

#[test]
fn a_derivative_in_an_unselected_arm_makes_no_state() {
    assert!((last("DeadArms.Driven", "a.y") - 1.0f64.sin()).abs() < 1e-5);
    assert!(last("DeadArms.Driven", "a.y1").abs() < 1e-12);
}

#[test]
fn a_selected_derivative_arm_still_differentiates() {
    assert!((last("DeadArms.Selected", "a.y1") - 1.0f64.cos()).abs() < 1e-5);
}

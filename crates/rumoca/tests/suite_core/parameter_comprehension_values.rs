//! Parameter values built from array constructors with iterators
//! (MLS 3.7 §10.4.2.1) and from conditionals whose arms differ in shape.
//!
//! `Blocks.Sources.BooleanTable` builds its `CombiTimeTable` matrix as
//! `if n > 0 then [table[1], 1.0; table, {mod(i + 1, 2.0) for i in 1:n}] else
//! [0.0, 1.0]`. Three facts make that value constructible:
//!
//! * a time-invariant `mod` needs no event owner (MLS §3.7.2 events occur only
//!   when the quotient changes), so a comprehension binder may appear in it;
//! * a binding conditional whose arms have different shapes is a structural
//!   selection evaluated at translation, since a value keeps one shape;
//! * the constant evaluator folds the comprehension, so the native table
//!   constructor receives a parameter-constant matrix.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package ParameterComprehension
  model ModBinder
    parameter Real v[3] = {mod(i + 1, 2.0) for i in 1:3};
    Real y;
  equation
    y = v[1] + 10*v[2] + 100*v[3] + time;
  end ModBinder;
  model ShapedSelection
    parameter Real t[:] = {1.25, 3.2, 4.0};
    parameter Integer n = size(t, 1);
    parameter Real m[:, 2] = if n > 0 then [t[1], 1.0; t, {mod(i + 1, 2.0) for i in 1:n}] else [0.0, 1.0];
    Real rows;
    Real y;
  equation
    rows = size(m, 1);
    y = m[1, 2] + 10*m[2, 2] + 100*m[3, 2] + 1000*m[4, 2] + m[4, 1] + time;
  end ShapedSelection;
end ParameterComprehension;
"#;

fn simulate(model: &str) -> SimResult {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(MODELS, "ParameterComprehension.mo")
        .expect("the model compiles");
    simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("the model simulates")
}

fn first(result: &SimResult, name: &str) -> f64 {
    let index = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .expect("the result records the column");
    result.data[index][0]
}

#[test]
fn a_time_invariant_mod_over_a_comprehension_binder_needs_no_event_owner() {
    // mod(2, 2) = 0, mod(3, 2) = 1, mod(4, 2) = 0.
    let result = simulate("ParameterComprehension.ModBinder");
    assert!((first(&result, "y") - 10.0).abs() < 1e-12);
}

#[test]
fn a_binding_conditional_with_differently_shaped_arms_selects_at_translation() {
    let result = simulate("ParameterComprehension.ShapedSelection");
    assert!((first(&result, "rows") - 4.0).abs() < 1e-12);
    // Column 2 is {1, mod(2,2), mod(3,2), mod(4,2)} = {1, 0, 1, 0}; m[4,1] = 4.
    assert!((first(&result, "y") - (1.0 + 100.0 + 4.0)).abs() < 1e-12);
}

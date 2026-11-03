//! Structural parameters (MLS 3.7 §10.1, §8.3.3; SPEC_0022 VAR-STRUCT).
//!
//! An array dimension and a for-equation range are fixed at translation, so an
//! ordinary parameter either reads is structural: DAE construction records it
//! evaluable, with every parameter its binding reads, and warns (WD001) at the
//! use. `Blocks.Continuous.Filter` indexes `x[nr + 2*i - 1]` with such a
//! parameter. An ordinary parameter no structure reads stays settable.

use rumoca::Compiler;
use rumoca_ir_dae as dae;
use rumoca_sim::{SimOptions, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package Structural
  model Dimension
    parameter Integer n = 2;
    parameter Real k = 3;
    Real x[n](each start = 1, each fixed = true);
  equation
    der(x) = -k*x;
  end Dimension;
  model Range
    parameter Integer m = 2;
    parameter Integer n = 3;
    Real x[3](each start = 1, each fixed = true);
  equation
    for i in 1:m loop
      der(x[i]) = -x[i];
    end for;
    for i in m + 1:n loop
      der(x[i]) = 0;
    end for;
  end Range;
  model DependentBinding
    parameter Integer order = 3;
    parameter Integer nr = if order > 2 then 1 else 0;
    parameter Integer na = order - nr;
    Real x[order](each start = 1, each fixed = true);
    parameter Real r[nr] = fill(2.0, nr);
  equation
    for i in 1:nr loop
      der(x[i]) = -r[i]*x[i];
    end for;
    for i in 1:na loop
      der(x[nr + i]) = -x[nr + i];
    end for;
  end DependentBinding;
  function half
    input Integer m;
    output Integer n;
  algorithm
    n := if m > 2 then 2*half(div(m, 2)) else 1;
  end half;
  model FinalBinding
    parameter Integer m = 8;
    final parameter Integer n = half(m);
    Real x[n](each start = 1, each fixed = true);
  equation
    der(x) = -x;
  end FinalBinding;
  block SizeOfInput
    input Real a[:] = {1, 2, 3};
    parameter Integer n = size(a, 1) - 1;
    Real b[n + 1];
  equation
    b = a*time;
  end SizeOfInput;
  model InputExtent
    SizeOfInput f(a = {1, 2, 3});
  end InputExtent;
  model UnfixedDimension
    parameter Integer n(fixed = false, start = 2);
    Real x[n](each start = 1, each fixed = true);
  initial equation
    n = 2;
  equation
    der(x) = -x;
  end UnfixedDimension;
  model UnevaluatedRange
    parameter Integer m = 2 annotation(Evaluate = false);
    Real x[3](each start = 1, each fixed = true);
  equation
    for i in 1:m loop
      der(x[i]) = -x[i];
    end for;
    for i in m + 1:3 loop
      der(x[i]) = 0;
    end for;
  end UnevaluatedRange;
end Structural;
"#;

fn compile(model: &str) -> std::sync::Arc<dae::Dae> {
    Compiler::new()
        .model(model)
        .compile_str(MODELS, "Structural.mo")
        .expect("the model compiles")
        .dae
}

fn evaluable(model: &dae::Dae) -> Vec<String> {
    let mut names = model.inspect(|view| {
        view.variables()
            .filter(|(_, variable)| variable.is_evaluable())
            .map(|(_, variable)| variable.name().to_string())
            .collect::<Vec<_>>()
    });
    names.sort();
    names
}

#[test]
fn a_dimension_parameter_is_structural_and_an_unrelated_one_stays_settable() {
    assert_eq!(evaluable(&compile("Structural.Dimension")), ["n"]);
}

#[test]
fn a_for_range_parameter_is_structural() {
    assert_eq!(evaluable(&compile("Structural.Range")), ["m", "n"]);
}

#[test]
fn a_structural_parameter_closes_over_its_binding_and_indexes_a_family() {
    let dae = compile("Structural.DependentBinding");
    assert_eq!(evaluable(&dae), ["na", "nr", "order"]);
    let result = simulate_dae_with_diagnostics(
        &dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("the indexed families simulate");
    let last = |name: &str| {
        let index = result
            .names
            .iter()
            .position(|candidate| candidate == name)
            .expect("the result records the column");
        *result.data[index].last().expect("a sample")
    };
    assert!((last("x[1]") - (-2.0f64).exp()).abs() < 1e-4);
    assert!((last("x[3]") - (-1.0f64).exp()).abs() < 1e-4);
}

#[test]
fn a_final_binding_carries_an_ordinary_parameter_into_an_extent() {
    // `n` is final, but its recursive binding reads the ordinary `m`, so the
    // dimension makes both structural rather than leaving the call to run.
    let dae = compile("Structural.FinalBinding");
    assert_eq!(evaluable(&dae), ["m", "n"]);
    let states = dae.inspect(|view| {
        view.variables()
            .filter(|(_, variable)| variable.role() == dae::VariableRole::State)
            .map(|(_, variable)| variable.scalar_count())
            .sum::<usize>()
    });
    assert_eq!(states, 4);
}

fn refusal(model: &str) -> String {
    let Err(error) = Compiler::new()
        .model(model)
        .compile_str(MODELS, "Structural.mo")
    else {
        panic!("{model} must be refused");
    };
    format!("{error:?}")
}

/// MLS 3.7 §4.5, §10.1: a `fixed = false` parameter is not evaluable, so an
/// extent reading it has no translation value.
#[test]
fn a_dimension_reading_a_fixed_false_parameter_is_refused() {
    let error = refusal("Structural.UnfixedDimension");
    assert!(
        error.contains("non-evaluable parameter `n`"),
        "unexpected refusal: {error}"
    );
}

/// MLS 3.7 §18.6, §8.3.3: `Evaluate = false` makes a parameter non-evaluable,
/// so a for-equation range reading it is refused rather than folded.
#[test]
fn a_for_range_reading_an_evaluate_false_parameter_is_refused() {
    let error = refusal("Structural.UnevaluatedRange");
    assert!(
        error.contains("non-evaluable parameter `m`"),
        "unexpected refusal: {error}"
    );
}

/// `size(a, 1)` reads only the translation-time shape of the input `a`, so a
/// dimension parameter bound to it is structural without reading `a`'s values.
#[test]
fn a_size_of_an_input_array_binds_a_structural_parameter() {
    assert_eq!(evaluable(&compile("Structural.InputExtent")), ["f.n"]);
}

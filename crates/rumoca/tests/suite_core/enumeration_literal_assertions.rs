//! Enumeration literals in assertion conditions (MLS 3.7 §4.9.5, §8.3.7).
//!
//! `E.lit` is an ordinary value expression, so an `assert` in an equation or
//! initial equation section may compare against it exactly as an equation may.
//! The assertion owners read the same expression roles that catalog the
//! literals: before, `assert(e <> E.A, ...)` failed with `ED008` while
//! `y = if e <> E.A then ...` resolved. `Blocks.Sources.BooleanTable` and
//! `Electrical.Analog.Sources.LightningImpulse` carry exactly this shape.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package EnumAssert
  type E = enumeration(A, B, C);
  model Equation
    parameter E e = EnumAssert.E.B;
    Real x(start = 1, fixed = true);
  equation
    assert(e <> EnumAssert.E.A, "e must not be A");
    der(x) = -x;
  end Equation;
  model Initial
    parameter E e = EnumAssert.E.B;
    Real x(start = 1, fixed = true);
  initial equation
    assert(e == EnumAssert.E.B, "e must be B");
  equation
    der(x) = -x;
  end Initial;
  model InitialAlgorithmValue
    parameter E e = EnumAssert.E.C;
    Real x(start = 1, fixed = true);
    discrete Real z;
  initial algorithm
    z := if e == EnumAssert.E.C then 3 else 1;
  equation
    der(x) = -z*x;
    when time > 0.5 then
      z = pre(z) + 1;
    end when;
  end InitialAlgorithmValue;
  model Violated
    parameter E e = EnumAssert.E.A;
    Real x(start = 1, fixed = true);
  equation
    assert(e <> EnumAssert.E.A, "enumeration assertion fired");
    der(x) = -x;
  end Violated;
end EnumAssert;
"#;

fn simulate(model: &str) -> Result<rumoca_sim::SimResult, String> {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(MODELS, "EnumAssert.mo")
    {
        Ok(compiled) => compiled,
        Err(error) => return Err(format!("{error:?}")),
    };
    simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .map_err(|error| format!("{error:?}"))
}

#[test]
fn equation_and_initial_assertions_resolve_enumeration_literals() {
    simulate("EnumAssert.Equation").expect("equation assertion over E.A simulates");
    simulate("EnumAssert.Initial").expect("initial assertion over E.B simulates");
}

#[test]
fn initial_algorithm_value_reads_an_enumeration_literal() {
    let result = simulate("EnumAssert.InitialAlgorithmValue")
        .expect("initial algorithm reading E.C simulates");
    let z = result
        .names
        .iter()
        .position(|name| name == "z")
        .expect("z is recorded");
    assert_eq!(result.data[z].first().copied(), Some(3.0));
}

#[test]
fn a_violated_enumeration_assertion_still_fires() {
    let error = simulate("EnumAssert.Violated").expect_err("the assertion is violated");
    assert!(
        error.contains("enumeration assertion fired"),
        "unexpected failure: {error}"
    );
}

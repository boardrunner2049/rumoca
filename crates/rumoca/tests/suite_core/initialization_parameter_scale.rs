//! Newton scale of `fixed = false` parameter unknowns (MLS 3.7 §4.8.1, §8.6).
//!
//! The double-exponential impulse of
//! `Modelica.Electrical.Analog.Sources.LightningImpulse` determines five
//! parameters of very different magnitude (a 1e-5 s time constant, an
//! amplitude factor near 1) from its initial equations. The system also has a
//! degenerate root, `tau1 = tau2` with `eta = 0`, which satisfies every row.
//! Scaled by a floor of 1, one Newton step moved `tau2` from its 1e-5 s guess
//! onto that manifold; scaled by its start guess, or by a declared `nominal`,
//! the projection stays on the branch the guesses name, which is OMC's.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae};

const MODELS: &str = r#"
package Impulse
  model Guessed
    parameter Real T1 = 10e-6;
    parameter Real T2 = 350e-6;
    parameter Real T0 = T2*T1/(T2 - T1)*log(T2/T1);
    parameter Real eta(fixed = false, start = 1);
    parameter Real T(fixed = false, start = T0);
    parameter Real T10(fixed = false, start = 0.01*T0);
    parameter Real tau1(fixed = false, start = T2);
    parameter Real tau2(fixed = false, start = T1);
    Real e = eta;
    Real a = tau1;
    Real b = tau2;
  initial equation
    exp(-T/tau1)/tau1 = exp(-T/tau2)/tau2;
    eta = exp(-T/tau1) - exp(-T/tau2);
    0.1*eta = exp(-T10/tau1) - exp(-T10/tau2);
    0.9*eta = exp(-(T10 + 0.8*T1)/tau1) - exp(-(T10 + 0.8*T1)/tau2);
    0.5*eta = exp(-(T10 - 0.1*T1 + T2)/tau1) - exp(-(T10 - 0.1*T1 + T2)/tau2);
  end Guessed;
  model Nominal
    extends Guessed(
      T(nominal = 1e-5),
      T10(nominal = 1e-7),
      tau1(nominal = 1e-4),
      tau2(nominal = 1e-5));
  end Nominal;
end Impulse;
"#;

fn initial(model: &str, name: &str) -> f64 {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(MODELS, "Impulse.mo")
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("{model} should compile: {error:?}"),
    };
    let options = SimOptions {
        t_end: 1e-4,
        ..SimOptions::default()
    };
    let result = match simulate_dae(&compiled.dae, &options) {
        Ok(result) => result,
        Err(error) => panic!("{model} should simulate: {error:?}"),
    };
    let Some(column) = result.names.iter().position(|candidate| candidate == name) else {
        panic!("{name} is recorded");
    };
    result.data[column][0]
}

fn assert_relative(actual: f64, expected: f64, what: &str) {
    assert!(
        ((actual - expected) / expected).abs() < 1e-5,
        "{what} = {actual}, expected {expected}"
    );
}

/// OMC 2026-07-21 from the same guesses: eta 0.95112484909122,
/// tau1 4.70106611735245e-4, tau2 4.063951610206661e-6.
fn assert_non_degenerate(model: &str) {
    assert_relative(initial(model, "e"), 0.951_124_849_091_22, "eta");
    assert_relative(initial(model, "a"), 4.701_066_117_352_45e-4, "tau1");
    assert_relative(initial(model, "b"), 4.063_951_610_206_661e-6, "tau2");
}

#[test]
fn a_parameter_unknown_is_scaled_by_its_start_guess() {
    assert_non_degenerate("Impulse.Guessed");
}

#[test]
fn a_declared_nominal_scales_a_parameter_unknown() {
    assert_non_degenerate("Impulse.Nominal");
}

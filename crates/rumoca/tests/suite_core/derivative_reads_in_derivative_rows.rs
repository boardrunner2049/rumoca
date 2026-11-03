//! A derivative row that reads another state's derivative (MLS 3.7 §3.7.4).
//!
//! `Electrical.Analog.Basic.OpAmpDetailed` limits its slew rate with
//! `der(v_source) = smooth(0, noEvent(if der(x) > sr then sr else ...))`, where
//! `der(x)` has its own explicit equation. Only a read of the row's own target
//! derivative takes part in isolating it; any other derivative the row reads
//! is that state's defined value, which the compiler substitutes exactly as it
//! does for an algebraic row.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package DerRead
  model SlewRate
    Real x(start = 0, fixed = true);
    Real y(start = 0, fixed = true);
  equation
    der(x) = cos(time);
    der(y) = smooth(0, noEvent(if der(x) > 0.5 then 0.5 else der(x)));
  end SlewRate;
  model Affine
    Real x(start = 0, fixed = true);
    Real y(start = 0, fixed = true);
  equation
    der(x) = cos(time);
    2*der(y) + der(x)*der(x) = 1;
  end Affine;
end DerRead;
"#;

fn final_y(model: &str) -> f64 {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(MODELS, "DerRead.mo")
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("{model} compiles: {error:?}"),
    };
    let options = SimOptions {
        t_end: 1.0,
        ..SimOptions::default()
    };
    let result: SimResult = match simulate_dae_with_diagnostics(&compiled.dae, &options) {
        Ok(result) => result,
        Err(error) => panic!("{model} simulates: {error:?}"),
    };
    let index = result.names.iter().position(|name| name == "y");
    let (Some(_), Some(last)) = (index, index.and_then(|i| result.data[i].last())) else {
        panic!("y is recorded with at least one sample");
    };
    *last
}

#[test]
fn an_explicit_row_reads_another_states_derivative() {
    // cos(t) > 0.5 until t = pi/3, so y(1) = 0.5.
    assert!((final_y("DerRead.SlewRate") - 0.5).abs() < 1e-5);
}

#[test]
fn an_affine_row_reads_another_states_derivative_in_its_offset() {
    // der(y) = sin(t)^2/2, so y(1) = 1/4 - sin(2)/8.
    let expected = 0.25 - 2.0f64.sin() / 8.0;
    assert!((final_y("DerRead.Affine") - expected).abs() < 1e-5);
}

//! A nanosecond-scale circuit resolves its first BDF steps.
//!
//! A junction charged through a resistor from zero settles in picoseconds, so
//! the first steps BDF needs lie far below an absolute `1e-13` floor. The
//! integrator's minimum step is roundoff in the run's own time scale, so the
//! model simulates, as it does with the explicit method.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimSolverMode, simulate_dae_with_diagnostics};

const JUNCTION: &str = "
model Junction
  parameter Real R = 50;
  parameter Real C = 0.4e-12;
  parameter Real Is = 1e-16;
  parameter Real Vt = 0.02585;
  Real v(start = 0, fixed = true);
equation
  C*der(v) = (0.7 - v)/R - Is*(exp(min(v/Vt, 40)) - 1);
end Junction;";

#[test]
fn a_picosecond_junction_simulates_with_bdf() {
    let compiled = Compiler::new()
        .model("Junction")
        .compile_str(JUNCTION, "Junction.mo")
        .expect("the model is well formed");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            solver_mode: SimSolverMode::Bdf,
            t_end: 1e-8,
            dt: Some(2e-11),
            ..SimOptions::default()
        },
    )
    .expect("the junction simulates");
    let v = result
        .names
        .iter()
        .position(|name| name == "v")
        .expect("v is recorded");
    let last = *result.data[v].last().expect("a sample");
    // The steady state balances the resistor current against the junction.
    let residual = (0.7 - last) / 50.0 - 1e-16 * ((last / 0.02585).exp() - 1.0);
    assert!(
        residual.abs() < 1e-6,
        "v(end) = {last}, residual {residual}"
    );
}

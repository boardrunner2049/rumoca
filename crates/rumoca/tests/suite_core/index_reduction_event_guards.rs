//! Index reduction through event-guarded conditionals, powers, `exp`, and
//! `log` (MLS 3.7 Appendix B, §3.7.1).
//!
//! `der(i)` of a current that a source defines needs the time derivative of
//! the source's expression. `Sources.SineCurrent` writes
//! `if time < startTime then 0 else I*sin(...)`: an event-owned relation keeps
//! its value between events (SPEC_0022 SIM-008), so the derivative is the
//! derivative of the selected branch. `Sources.LightningImpulse` writes
//! `(t/tau)^m*exp(-t/tau2)` with a parameter exponent: `d(a^n) = n*a^(n-1)*da`.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

const MODELS: &str = r#"
package Guards
  model TimeGuard
    parameter Real t0 = 0.1;
    Real v;
    Real i;
    Real w;
  equation
    if time < t0 then
      v = 0;
    else
      v = sin(time - t0);
    end if;
    v = 2*i;
    w = der(i);
  end TimeGuard;
  model NoEventGuard
    Real v;
    Real i;
    Real w;
  equation
    v = if noEvent(time < 0.1) then 0 else sin(time - 0.1);
    v = 2*i;
    w = der(i);
  end NoEventGuard;
  model Heidler
    parameter Integer m = 5;
    Real v;
    Real i;
    Real w;
  equation
    v = (time/0.5)^m/(1 + (time/0.5)^m)*exp(-time);
    v = 2*i;
    w = der(i);
  end Heidler;
  model Logarithm
    Real v;
    Real i;
    Real w;
  equation
    v = log(1 + time);
    v = 2*i;
    w = der(i);
  end Logarithm;
end Guards;
"#;

fn simulate(model: &str) -> Result<SimResult, String> {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(MODELS, "Guards.mo")
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

/// The recorded `w` against `expected(t)` at every sample away from `skip`.
fn assert_derivative(result: &SimResult, expected: impl Fn(f64) -> f64, skip: f64) {
    let column = |name: &str| {
        let index = result
            .names
            .iter()
            .position(|candidate| candidate == name)
            .expect("the result records the column");
        &result.data[index]
    };
    let w = column("w");
    for (t, value) in result.times.iter().zip(w) {
        if (t - skip).abs() < 1e-6 {
            continue;
        }
        let wanted = expected(*t);
        assert!(
            (value - wanted).abs() < 1e-6 * (1.0 + wanted.abs()),
            "w({t}) = {value}, expected {wanted}"
        );
    }
}

#[test]
fn an_event_guarded_conditional_differentiates_its_selected_branch() {
    let result = simulate("Guards.TimeGuard").expect("the time-guarded source reduces");
    assert_derivative(
        &result,
        |t| if t < 0.1 { 0.0 } else { 0.5 * (t - 0.1).cos() },
        0.1,
    );
}

#[test]
fn a_no_event_guard_keeps_no_event_surface_to_differentiate_across() {
    let error = simulate("Guards.NoEventGuard").expect_err("noEvent removes the event surface");
    assert!(error.contains("structurally singular"), "{error}");
}

#[test]
fn a_parameter_exponent_power_and_exp_differentiate_exactly() {
    let result = simulate("Guards.Heidler").expect("the Heidler shape reduces");
    assert_derivative(
        &result,
        |t| {
            let r = (t / 0.5).powi(5);
            let dr = if t == 0.0 { 0.0 } else { 5.0 * r / t };
            let f = r / (1.0 + r) * (-t).exp();
            0.5 * (dr / (1.0 + r).powi(2) * (-t).exp() - f)
        },
        -1.0,
    );
}

#[test]
fn log_differentiates_exactly() {
    let result = simulate("Guards.Logarithm").expect("the logarithm reduces");
    assert_derivative(&result, |t| 0.5 / (1.0 + t), -1.0);
}

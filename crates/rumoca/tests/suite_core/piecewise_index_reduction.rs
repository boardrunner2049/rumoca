//! Structural index reduction through piecewise definitions and invariant
//! powers (Pantelides differentiation of a state's defining equation).
//!
//! `d/dt (if c then a else b) = if c then da/dt else db/dt` on every interval
//! where the guard `c` is constant; MLS §8.5 makes a relation change only at
//! its event, so the guard is retained and only the branch values are
//! differentiated. `d/dt u^v = v*u^(v-1)*du/dt` for a time-invariant `v`.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae};

fn simulate(source: &str, model: &str) -> SimResult {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("{model} compiles: {error:?}"),
    };
    let options = SimOptions {
        t_end: 1.0,
        dt: Some(0.01),
        ..Default::default()
    };
    let result = match simulate_dae(&compiled.dae, &options) {
        Ok(result) => result,
        Err(error) => panic!("{model} simulates: {error:?}"),
    };
    assert!(result.times.len() > 10, "{model} produced an output grid");
    result
}

fn column<'result>(result: &'result SimResult, name: &str) -> &'result [f64] {
    let Some(index) = result.names.iter().position(|candidate| candidate == name) else {
        panic!("result has no column `{name}`");
    };
    &result.data[index]
}

/// A position profile defined piecewise in time is a state whose velocity and
/// acceleration follow from differentiating each branch twice.
#[test]
fn piecewise_state_definition_is_differentiated_per_branch() {
    let source = r#"
model Profile
  parameter Real a = 2;
  Real T1;
  Real s;
  Real sd;
  Real sdd;
equation
  T1 = sqrt(a) / 4;
  s = if time < T1 then a / 2 * time * time else a / 2 * T1 * T1 + a * T1 * (time - T1);
  sd = der(s);
  sdd = der(sd);
end Profile;
"#;
    let result = simulate(source, "Profile");
    let t1 = 2.0_f64.sqrt() / 4.0;
    for (index, time) in result.times.iter().enumerate() {
        if (time - t1).abs() < 0.02 {
            continue;
        }
        let (velocity, acceleration) = if *time < t1 {
            (2.0 * time, 2.0)
        } else {
            (2.0 * t1, 0.0)
        };
        let sd = column(&result, "sd")[index];
        let sdd = column(&result, "sdd")[index];
        assert!((sd - velocity).abs() < 1.0e-9, "sd at {time}: {sd}");
        assert!((sdd - acceleration).abs() < 1.0e-9, "sdd at {time}: {sdd}");
    }
}

/// A power with a parameter exponent differentiates through order two.
#[test]
fn invariant_exponent_power_is_differentiated_twice() {
    let source = r#"
model Power
  parameter Real c = 3;
  Real s;
  Real sd;
  Real sdd;
equation
  s = (1 + time) ^ c;
  sd = der(s);
  sdd = der(sd);
end Power;
"#;
    let result = simulate(source, "Power");
    for (index, time) in result.times.iter().enumerate() {
        let base = 1.0 + time;
        let sd = column(&result, "sd")[index];
        let sdd = column(&result, "sdd")[index];
        assert!(
            (sd - 3.0 * base * base).abs() < 1.0e-9,
            "sd at {time}: {sd}"
        );
        assert!((sdd - 6.0 * base).abs() < 1.0e-9, "sdd at {time}: {sdd}");
    }
}

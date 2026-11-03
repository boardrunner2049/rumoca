//! A `reinit` target stays a state through index reduction
//! (MLS 3.6 §8.3.6, SPEC_0022 EQN-032: reinit on `x` implies
//! `stateSelect = StateSelect.always` on `x`).
//!
//! Index reduction used to try demoting the reinitialized state like any other
//! candidate; reconstruction then had no state to hand the rebuilt event action
//! and the whole lowering failed with an internal structural-contract error.
//! A reinitialized state is now never demoted: when a twin state can be demoted
//! instead the model reduces and simulates, and when the model's own equations
//! determine the reinitialized variable algebraically (the FIRE library's
//! `DynamicDCEquivalent` left standalone, whose unconnected pins pin the winding
//! current to zero) the refusal is the typed `ES017` diagnostic.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

/// Two rigidly coupled inertias: `phi1 = phi2` leaves one state pair, and the
/// `reinit` of `w1` decides which pair it is. No start is `fixed`, so nothing
/// but the `reinit` keeps `w1` (tried first, by declaration order) a state.
const COUPLED: &str = r#"
model Coupled
  Real phi1(start = 1);
  Real w1(start = 0);
  Real phi2;
  Real w2;
  Real tau;
equation
  der(phi1) = w1;
  der(w1) = tau;
  der(phi2) = w2;
  2*der(w2) = -tau - phi2;
  phi1 = phi2;
  when time > 0.5 then
    reinit(w1, 0);
  end when;
end Coupled;
"#;

/// An inductor between two open pins: each pin's unconnected flow is zero
/// (`a = 0`, `b = 0`), so the equations fix the current `i` twice over and the
/// reduction can only proceed by demoting `i` - the shape of the FIRE
/// `DynamicDCEquivalent` component left standalone.
const PINNED: &str = r#"
model Pinned
  Boolean zero = time > 0.5;
  Real i(start = 0, fixed = true);
  Real v1;
  Real v2;
  Real a;
  Real b;
equation
  0.01*der(i) = v1 - v2 - i;
  a = i;
  a = 0;
  b = -i;
  b = 0;
  when zero then
    reinit(i, 0);
  end when;
end Pinned;
"#;

fn compiled(source: &str, model: &str) -> rumoca::CompilationResult {
    Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
        .unwrap_or_else(|error| panic!("{model} compiles: {error:?}"))
}

fn column<'a>(result: &'a SimResult, name: &str) -> &'a [f64] {
    let index = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("{name} is recorded"));
    &result.data[index]
}

#[test]
fn the_reinitialized_twin_survives_index_reduction() {
    let result = simulate_dae_with_diagnostics(
        &compiled(COUPLED, "Coupled").dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("Coupled reduces by demoting w2 and simulates");
    let (w1, w2) = (column(&result, "w1"), column(&result, "w2"));
    let mut reinitialized = false;
    for (sample, time) in result.times.iter().enumerate() {
        assert!((w1[sample] - w2[sample]).abs() < 1e-6, "w1 = w2 at {time}");
        if *time > 0.5 && !reinitialized {
            // The first sample after the event is the reinitialized value.
            assert!(
                w1[sample].abs() < 1e-3,
                "w1 = {} just after reinit",
                w1[sample]
            );
            reinitialized = true;
        }
    }
    assert!(reinitialized, "the run crosses the reinit event");
    assert!(
        w1[result.times.len() / 4] < -0.05,
        "w1 moved before the event"
    );
}

#[test]
fn a_reinitialized_variable_the_equations_fix_is_the_typed_refusal() {
    let error = simulate_dae_with_diagnostics(
        &compiled(PINNED, "Pinned").dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect_err("Pinned leaves `i` no freedom to remain a state");
    let text = format!("{error:?} {error}");
    assert!(
        text.contains("ES017") || text.contains("a `reinit` requires to remain a state"),
        "expected the reinitialized-state refusal, got: {text}"
    );
    assert!(
        !text.contains("invalid structural IR contract"),
        "the refusal is a model diagnostic, not a contract violation: {text}"
    );
}

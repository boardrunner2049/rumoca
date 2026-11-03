//! A `reinit` target stays a state through index reduction
//! (MLS 3.6 §8.3.6, SPEC_0022 EQN-032: reinit on `x` implies
//! `stateSelect = StateSelect.always` on `x`).
//!
//! Index reduction used to try demoting the reinitialized state like any other
//! candidate; reconstruction then had no state to hand the rebuilt event action
//! and the whole lowering failed with an internal structural-contract error
//! (FIRE `Systems.Propulsion.Motors.DynamicDCEquivalent`). A reinitialized
//! state is now never demoted: when a twin state can be demoted instead the
//! model reduces and simulates; when the demotion of the reinitialized state is
//! the only way to a sortable system the refusal is the typed `ES017`; and a
//! model that stays singular without the `reinit` still reports that
//! singularity (`ES010`) rather than a refusal that would hide it.

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

/// `i = 2*time` determines `i`, so the system is sortable only by demoting
/// `i`, which the `reinit` forbids. Without the `when` clause it simulates.
const DETERMINED: &str = r#"
model Determined
  Real i(start = 1);
  Real y;
equation
  der(i) = y;
  i = 2*time;
  when time > 0.5 then
    reinit(i, 0);
  end when;
end Determined;
"#;

/// An inductor between two open pins: each pin's unconnected flow is zero
/// (`a = 0`, `b = 0`), which fixes `i`, but nothing determines `v1` or `v2`.
/// The model is singular with or without its `reinit`, so demoting `i` is not
/// what stands between it and a sortable system.
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

/// `phi1 = phi2` couples `w1` and `w2`, and both are reinitialized to
/// contradictory values. Neither can be demoted, and keeping both under a
/// state-manifold projection would settle them on their average (2.5).
const BOTH_REINITIALIZED: &str = r#"
model BothReinitialized
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
    reinit(w2, 5);
  end when;
end BothReinitialized;
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

fn simulation_error(source: &str, model: &str) -> String {
    let error = simulate_dae_with_diagnostics(
        &compiled(source, model).dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect_err("the model is refused");
    let text = format!("{error:?} {error}");
    assert!(
        !text.contains("invalid structural IR contract") && !text.contains("state mapping"),
        "the refusal is a model diagnostic, not a contract violation: {text}"
    );
    text
}

fn is_reinit_refusal(text: &str) -> bool {
    text.contains("ES017") || text.contains("a `reinit` requires to remain a state")
}

#[test]
fn a_reinitialized_variable_the_equations_fix_is_the_typed_refusal() {
    let text = simulation_error(DETERMINED, "Determined");
    assert!(
        is_reinit_refusal(&text),
        "expected the reinitialized-state refusal, got: {text}"
    );
    let without_reinit = DETERMINED.replace(
        "  when time > 0.5 then\n    reinit(i, 0);\n  end when;\n",
        "",
    );
    assert_ne!(without_reinit, DETERMINED);
    simulate_dae_with_diagnostics(
        &compiled(&without_reinit, "Determined").dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("without its reinit the demotion of `i` sorts the model");
}

#[test]
fn a_model_singular_without_its_reinit_reports_the_singularity() {
    let text = simulation_error(PINNED, "Pinned");
    assert!(
        !is_reinit_refusal(&text),
        "the reinit refusal must not hide the singularity: {text}"
    );
    assert!(
        text.contains("structurally singular"),
        "expected the structural singularity, got: {text}"
    );
}

#[test]
fn coupled_reinitialized_states_are_refused_instead_of_projected() {
    let text = simulation_error(BOTH_REINITIALIZED, "BothReinitialized");
    assert!(
        is_reinit_refusal(&text),
        "expected the reinitialized-state refusal, got: {text}"
    );
}

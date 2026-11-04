//! Constructs the ModelicaCPSLibrary sensor models rely on.
//!
//! Each model is a reduced form of one the library compiles:
//! `Systems.Sensing.SampledDelivery` fills a delivery queue row by row inside a
//! `when`, and checks its start time in an initial equation;
//! `Systems.Sensing.GNSS.broadcastStates` sizes its outputs from a record-array
//! input and calls a multi-output function in a loop; `Tests.GnssEphemeris`
//! determines array `fixed = false` parameters from call statements in an
//! initial algorithm. Values are checked by simulation where a wrong lowering
//! would still compile.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae};

fn simulate(source: &str, model: &str, t_end: f64) -> Result<SimResult, String> {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
        .unwrap_or_else(|error| panic!("{model} must compile: {error:?}"));
    simulate_dae(
        &compiled.dae,
        &SimOptions {
            t_end,
            dt: Some(0.05),
            ..SimOptions::default()
        },
    )
    .map_err(|error| format!("{error:?}"))
}

fn last(result: &SimResult, name: &str) -> f64 {
    let index = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("simulation result has no column {name}: {:?}", result.names));
    *result.data[index]
        .last()
        .expect("the simulation reports at least one sample")
}

fn assert_close(result: &SimResult, name: &str, expected: f64) {
    let actual = last(result, name);
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0),
        "{name}: expected {expected}, got {actual}"
    );
}

/// `history[1,:] = u` assigns one row of a discrete array inside a `when`
/// (MLS §10.6.1: an array equation is the equations of its elements), and
/// `history[j,:] = pre(history[j-1,:])` shifts the queue.
#[test]
fn a_when_assigns_an_array_row_by_row() {
    const SOURCE: &str = r#"
model SliceQueue
  parameter Integer n = 2;
  parameter Integer latency = 1;
  Real u[n] = {time, 2*time};
  discrete Real history[latency + 1, n](each start = 0, each fixed = true);
  discrete Real y[n](each start = 0, each fixed = true);
equation
  when sample(0, 0.1) then
    history[1, :] = u;
    for j in 2:latency + 1 loop
      history[j, :] = pre(history[j - 1, :]);
    end for;
    y = history[latency + 1, :];
  end when;
end SliceQueue;
"#;
    let result = simulate(SOURCE, "SliceQueue", 0.35).expect("the queue simulates");
    // Last tick t = 0.3: the head holds u(0.3), the tail u(0.2).
    assert_close(&result, "history[1,1]", 0.3);
    assert_close(&result, "history[1,2]", 0.6);
    assert_close(&result, "y[1]", 0.2);
    assert_close(&result, "y[2]", 0.4);
}

/// An initial equation holds during initialization only (MLS §8.6), so
/// `assert(startTime >= time)` there restricts the start time and is not
/// violated once time passes `startTime`; a violated one still fails.
#[test]
fn an_initial_equation_assertion_is_checked_at_initialization_only() {
    const SOURCE: &str = r#"
model InitialTimeAssert
  parameter Real startTime = 0.1;
  parameter Real limit = 1;
  Real x(start = 0, fixed = true);
initial equation
  assert(startTime >= time, "starts after startTime");
  assert(limit > 0, "limit must be positive");
equation
  der(x) = 1;
end InitialTimeAssert;
"#;
    let result = simulate(SOURCE, "InitialTimeAssert", 0.3).expect("t > startTime is no violation");
    assert_close(&result, "x", 0.3);
    let violated = SOURCE
        .replace("parameter Real limit = 1;", "parameter Real limit = -1;")
        .replace("InitialTimeAssert", "ViolatedInitialAssert");
    let error = simulate(&violated, "ViolatedInitialAssert", 0.3)
        .expect_err("a violated initial assertion fails initialization");
    assert!(error.contains("limit must be positive"), "{error}");
}

/// `output Real y[size(r, 1), 2]` sizes an output from a record-array input
/// that record-parameter lowering splits into its fields, and `(q, w) :=
/// g(r[i], t)` binds outputs of a call inside a function loop.
#[test]
fn a_record_array_input_sizes_outputs_of_a_looping_multi_output_call() {
    const SOURCE: &str = r#"
package RecordShapes
  record Rec
    parameter Real a = 1;
    parameter Real b = 2;
  end Rec;
  function g
    input Rec e;
    input Real t;
    output Real p[2];
    output Real c;
  algorithm
    p := {e.a*t, e.b};
    c := e.a + e.b;
  end g;
  function f
    input Rec r[:];
    input Real t;
    output Real y[size(r, 1), 2];
    output Real z[size(r, 1)];
  protected
    Real q[2];
    Real w;
  algorithm
    for i in 1:size(r, 1) loop
      (q, w) := g(r[i], t);
      y[i, :] := q;
      z[i] := w;
    end for;
  end f;
  model M
    parameter Integer n = 2;
    parameter Rec recs[n](a = {1, 3});
    Real y[n, 2];
    Real z[n];
  equation
    (y, z) = f(recs, time);
  end M;
end RecordShapes;
"#;
    let result = simulate(SOURCE, "RecordShapes.M", 0.2).expect("the record shapes simulate");
    assert_close(&result, "y[1,1]", 0.2);
    assert_close(&result, "y[2,1]", 0.6);
    assert_close(&result, "y[2,2]", 2.0);
    assert_close(&result, "z[2]", 5.0);
}

/// Call statements in an initial algorithm determine array and Boolean
/// `fixed = false` parameters; an element read takes the element of the value
/// assigned before it, and a call in a loop reads the receivers' values.
#[test]
fn initial_algorithm_call_statements_determine_array_parameters() {
    const SOURCE: &str = r#"
package InitialCalls
  function f
    input Real u;
    output Real a[2];
    output Real b;
    output Boolean ok;
  algorithm
    a := {u, 2*u};
    b := 3*u;
    ok := u > 1;
  end f;
  model M
    parameter Real k = 2;
    parameter Real a[2](each fixed = false);
    parameter Real b(fixed = false);
    parameter Boolean ok(fixed = false);
    parameter Real c[3](each fixed = false);
    Real x(start = 0, fixed = true);
  initial algorithm
    (a, b, ok) := f(k);
    assert(ok and b > 5, "first call");
    c := {1, 2, a[1]};
    for j in 1:2 loop
      (a, b) := f(k + j);
    end for;
    assert(abs(b - 12) < 1e-12 and abs(a[2] - 8) < 1e-12, "loop call");
  equation
    der(x) = a[2] + b + c[3] + (if ok then 100 else 0);
  end M;
end InitialCalls;
"#;
    let result = simulate(SOURCE, "InitialCalls.M", 1.0).expect("the initial calls settle");
    // a = f(4).a = {4, 8}, b = 12, c[3] = f(2).a[1] = 2, ok = f(2).ok.
    assert_close(&result, "x", 122.0);
    let violated = SOURCE
        .replace("b > 5", "b > 7")
        .replace("InitialCalls", "ViolatedInitialCalls");
    let error = simulate(&violated, "ViolatedInitialCalls.M", 1.0)
        .expect_err("the first call's assertion reads its outputs");
    assert!(error.contains("first call"), "{error}");
}

//! MLS §12.9 external functions the native runtime executes: the MSL
//! `ModelicaExternalC` random generators (reference semantics transcribed from
//! `ModelicaRandom.c`) and `external "builtin"` math functions, which are the
//! builtin itself. The generator fixture also carries the function constructs
//! the MSL initial-state functions need: stepped loops over affine slices,
//! tuple receivers first defined inside a loop, `size(output, 1)` before the
//! output is assigned, a package constant several packages deep in a
//! dimension, and a multi-output call in a `when` clause.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae};

const FIXTURE: &str = include_str!("../fixtures/external_functions/Random.mo");

/// The `ModelicaRandom_xorshift128plus` states and draws of the MSL C source
/// from `Xorshift128plus.initialState(1, 2)`: the initial state, then after
/// the draws at t = 0.1 and t = 0.2.
pub(crate) const STATES: [[i64; 4]; 3] = [
    [2_049_721_705, -416_146_649, 2_001_124_099, 1_729_396_250],
    [2_001_124_099, 1_729_396_250, 1_908_300_452, 2_058_340_188],
    [1_908_300_452, 2_058_340_188, 870_918_268, -1_959_790_993],
];
pub(crate) const DRAWS: [f64; 2] = [0.979_244_670_934_116_3, 0.043_700_601_719_966_26];

fn simulate(model: &str, t_end: f64) -> rumoca_solver::SimResult {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(FIXTURE, "Random.mo")
        .unwrap_or_else(|error| panic!("{model} compiles: {error:?}"));
    simulate_dae(
        &compiled.dae,
        &SimOptions {
            t_end,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{model} simulates: {error}"))
}

/// The value of `name` at the last recorded point not after `time` (the
/// right limit at an event instant).
fn value_at(result: &rumoca_solver::SimResult, name: &str, time: f64) -> f64 {
    let column = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("{name} is recorded in {:?}", result.names));
    let index = result
        .times
        .iter()
        .rposition(|t| *t <= time + 1e-12)
        .expect("a recorded point");
    result.data[column][index]
}

#[test]
fn msl_random_generator_matches_the_c_source_bit_for_bit() {
    let result = simulate("RngTicks", 0.25);
    for (time, state) in [0.05, 0.15, 0.25].into_iter().zip(STATES) {
        for (element, expected) in state.into_iter().enumerate() {
            let value = value_at(&result, &format!("s[{}]", element + 1), time);
            assert_eq!(value, expected as f64, "s[{}] at {time}", element + 1);
        }
    }
    for (time, draw) in [0.15, 0.25].into_iter().zip(DRAWS) {
        assert_eq!(
            value_at(&result, "r", time).to_bits(),
            draw.to_bits(),
            "r at {time}"
        );
    }
}

#[test]
fn builtin_external_math_function_is_the_builtin() {
    let result = simulate("AtanEvents", 1.0);
    let angle = value_at(&result, "angle", 1.0);
    assert!(
        (angle - std::f64::consts::FRAC_PI_4).abs() < 1e-9,
        "angle = {angle}"
    );
    // The relation over the builtin generates its event: one crossing at
    // x = tan(0.5).
    assert_eq!(value_at(&result, "crossings", 1.0), 1.0);
    assert_eq!(value_at(&result, "past", 0.5), 0.0);
    assert_eq!(value_at(&result, "past", 1.0), 1.0);
}

#[test]
fn unsupported_external_is_refused_by_name() {
    let compiled = Compiler::new()
        .model("UserExternal")
        .compile_str(FIXTURE, "Random.mo")
        .expect("UserExternal compiles");
    let error = simulate_dae(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect_err("an external without a runtime implementation cannot execute");
    let rendered = error.to_string();
    assert!(
        rendered.contains("`myGain`") && rendered.contains("`my_gain`"),
        "{rendered}"
    );
}

const PURE_FUNCTIONS: &str = r#"
model WhenTuple
  function g
    input Integer s[2];
    output Real r;
    output Integer o[2];
  algorithm
    o := {s[2] + 1, s[1] + 3};
    r := s[1] / 10;
  end g;
  parameter Boolean on = true;
  discrete Integer s[2](start = {1, 2}, each fixed = true);
  discrete Real r(start = 0, fixed = true);
equation
  when sample(0, 0.1) then
    if on then
      (r, s) = g(pre(s));
    else
      r = 0;
      s = pre(s);
    end if;
  end when;
end WhenTuple;

model LoopReceivers
  function g
    input Integer s[2];
    output Real r;
    output Integer o[2];
  algorithm
    o := {s[2] + 1, s[1] + 3};
    r := 0;
  end g;
  function f
    input Integer n;
    output Integer x[n];
  protected
    Integer aux[2];
    Real r;
  algorithm
    x[1:2] := {1, 2};
    for i in 3:2:n loop
      (r, aux) := g(x[i-2:i-1]);
      x[i:i+1] := aux;
    end for;
  end f;
  discrete Integer y[4](start = f(4), each fixed = true);
equation
  when sample(0, 0.1) then
    y = pre(y) .+ 1;
  end when;
end LoopReceivers;
"#;

/// MLS §8.3.5: `(a, b) = f(...)` in a `when` clause assigns every receiver
/// from one evaluation of the call at the event.
#[test]
fn when_clause_tuple_call_assigns_every_receiver() {
    let compiled = Compiler::new()
        .model("WhenTuple")
        .compile_str(PURE_FUNCTIONS, "WhenTuple.mo")
        .unwrap_or_else(|error| panic!("WhenTuple compiles: {error:?}"));
    let result = simulate_dae(
        &compiled.dae,
        &SimOptions {
            t_end: 0.25,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("WhenTuple simulates: {error}"));
    // t = 0: g({1, 2}) = (0.1, {3, 4}); t = 0.1: g({3, 4}) = (0.3, {5, 6}).
    for (time, s1, s2, r) in [(0.05, 3.0, 4.0, 0.1), (0.15, 5.0, 6.0, 0.3)] {
        assert_eq!(value_at(&result, "s[1]", time), s1, "s[1] at {time}");
        assert_eq!(value_at(&result, "s[2]", time), s2, "s[2] at {time}");
        assert!(
            (value_at(&result, "r", time) - r).abs() < 1e-12,
            "r at {time}"
        );
    }
}

/// A stepped loop reading the affine slice `x[i-2:i-1]` the previous
/// iteration wrote, with a tuple receiver first defined inside the loop.
#[test]
fn stepped_loop_reads_slices_the_previous_iteration_defined() {
    let compiled = Compiler::new()
        .model("LoopReceivers")
        .compile_str(PURE_FUNCTIONS, "LoopReceivers.mo")
        .unwrap_or_else(|error| panic!("LoopReceivers compiles: {error:?}"));
    let result = simulate_dae(
        &compiled.dae,
        &SimOptions {
            t_end: 0.1,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("LoopReceivers simulates: {error}"));
    // f(4) = {1, 2, 3, 4}; the sample at t = 0 adds one.
    for (element, expected) in [2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
        assert_eq!(
            value_at(&result, &format!("y[{}]", element + 1), 0.05),
            expected
        );
    }
}

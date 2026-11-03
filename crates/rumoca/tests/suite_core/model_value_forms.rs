//! Model-scope value forms from MSL source blocks: enumeration literals in
//! assertions, array comprehensions in parameter bindings, conditional bindings
//! whose arms differ in size, and multi-result calls with discrete receivers.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae};

/// Compile and simulate `model`; a compile or simulation error is returned
/// as its debug text.
fn simulate(source: &str, model: &str) -> Result<SimResult, String> {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
    {
        Ok(compiled) => compiled,
        Err(error) => return Err(format!("{error:?}")),
    };
    let options = SimOptions {
        t_end: 1.0,
        dt: Some(0.1),
        ..Default::default()
    };
    match simulate_dae(&compiled.dae, &options) {
        Ok(result) => {
            assert!(result.times.len() > 5, "{model} produced an output grid");
            Ok(result)
        }
        Err(error) => Err(format!("{error:?}")),
    }
}

fn last(result: &SimResult, name: &str) -> f64 {
    let Some(index) = result.names.iter().position(|candidate| candidate == name) else {
        panic!("result has no column `{name}`");
    };
    *result.data[index]
        .last()
        .expect("a simulated column has samples")
}

const ASSERTION: &str = r#"
package Assertion
  type Mode = enumeration(Hold, Extrapolate, Periodic);
  block Checked
    parameter Mode mode = Mode.Hold;
    output Real y;
  equation
    assert(mode <> Assertion.Mode.Extrapolate, "unsuitable mode");
    y = time;
  end Checked;
  model Accepted
    Checked checked;
  end Accepted;
  model Rejected
    Checked checked(mode = Mode.Extrapolate);
  end Rejected;
end Assertion;
"#;

/// MLS §8.3.7 / §4.9.5: an assertion condition reads an enumeration literal
/// as an ordinary value and is checked at run time.
#[test]
fn assertion_condition_reads_enumeration_literals() {
    let accepted = simulate(ASSERTION, "Assertion.Accepted").expect("accepted mode simulates");
    assert!((last(&accepted, "checked.y") - 1.0).abs() < 1.0e-12);
    let rejected = simulate(ASSERTION, "Assertion.Rejected")
        .expect_err("the violated assertion stops the simulation");
    assert!(rejected.contains("unsuitable mode"), "{rejected}");
}

/// MLS §10.4.1: a comprehension in a parameter binding is a constant array;
/// its iterator is constant, so a quotient over it needs no event.
#[test]
fn parameter_binding_comprehension_evaluates() {
    let source = r#"
model Comprehension
  parameter Integer n = 3;
  parameter Real table[3, 2] = [{0.1, 0.2, 0.3}, {mod(i, 2.0) for i in 1:n}];
  Real y = table[3, 2] + table[2, 2] * time;
end Comprehension;
"#;
    let result = simulate(source, "Comprehension").expect("comprehension model simulates");
    assert!((last(&result, "y") - 1.0).abs() < 1.0e-12);
}

/// A conditional binding whose arms differ in size selects structure, so its
/// guard is decided at translation (MLS §4.5) instead of kept at run time.
#[test]
fn size_selecting_conditional_binding_is_decided_at_translation() {
    let source = r#"
model SizeSelection
  parameter Real t[:] = {0.5, 1.5};
  parameter Integer n = size(t, 1);
  parameter Real table[:, 2] = if n > 0 then [t[1], 0.0; t, {mod(i, 2.0) for i in 1:n}] else [0.0, 0.0];
  Real y = table[size(table, 1), 2] + size(table, 1) * time;
end SizeSelection;
"#;
    let result = simulate(source, "SizeSelection").expect("size-selecting model simulates");
    assert!((last(&result, "y") - 3.0).abs() < 1.0e-12);
}

/// MLS §12.4.3: a multi-result call equation may define discrete-valued
/// receivers outside any clock.
#[test]
fn multi_output_call_defines_discrete_value_receivers() {
    let source = r#"
model Receivers
  function copy
    input Integer s[3];
    output Real x;
    output Integer so[3];
  algorithm
    so := s;
    x := s[1];
  end copy;
  Real x;
  Integer so[3];
equation
  (x, so) = copy({23, 87, 187});
end Receivers;
"#;
    let result = simulate(source, "Receivers").expect("receiver model simulates");
    assert_eq!(last(&result, "x"), 23.0);
    assert_eq!(last(&result, "so[3]"), 187.0);
}

/// MLS §3.7.2: `div`, `rem`, and `mod` of Integer operands inside a function
/// body are exact Integer quotients with truncating and flooring signs.
#[test]
fn function_integer_quotients_are_exact() {
    let source = r#"
model Quotients
  function quotients
    input Integer a;
    input Integer sign;
    output Integer d;
    output Integer r;
    output Integer m;
  algorithm
    d := div(a, 2 * sign);
    r := rem(a, 2 * sign);
    m := mod(a, 2 * sign);
  end quotients;
  Integer a = if time < 10 then -7 else 0;
  Integer nd;
  Integer nr;
  Integer nm;
  Integer pd;
  Integer pr;
  Integer pm;
equation
  (nd, nr, nm) = quotients(a, 1);
  (pd, pr, pm) = quotients(-a, -1);
end Quotients;
"#;
    let result = simulate(source, "Quotients").expect("quotient model simulates");
    let expected = [
        ("nd", -3.0),
        ("nr", -1.0),
        ("nm", 1.0),
        ("pd", -3.0),
        ("pr", 1.0),
        ("pm", -1.0),
    ];
    for (name, value) in expected {
        assert_eq!(last(&result, name), value, "{name}");
    }
}

const ALGORITHM_ASSERTION: &str = r#"
package AlgorithmAssertion
  block Checked
    parameter Real interval = 1;
    output Real y;
  algorithm
    assert(interval > 0 or interval < 0, "the interval must be non-zero");
  equation
    y = time;
  end Checked;
  model Accepted
    Checked checked;
  end Accepted;
  model Rejected
    Checked checked(interval = 0);
  end Rejected;
end AlgorithmAssertion;
"#;

/// MLS §11.2.8.1: an algorithm of `assert` statements only is checked like the
/// assert equations it states.
#[test]
fn an_assertion_only_algorithm_is_checked_at_run_time() {
    let accepted = simulate(ALGORITHM_ASSERTION, "AlgorithmAssertion.Accepted")
        .expect("the accepted interval simulates");
    assert!((last(&accepted, "checked.y") - 1.0).abs() < 1.0e-12);
    let rejected = simulate(ALGORITHM_ASSERTION, "AlgorithmAssertion.Rejected")
        .expect_err("the violated assertion stops the simulation");
    assert!(
        rejected.contains("the interval must be non-zero"),
        "{rejected}"
    );
}

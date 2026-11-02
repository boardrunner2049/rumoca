//! Constructs the FIRE multicopter library (`fire_modelica_models`) relies on.
//!
//! Each model here is a reduced form of one the library compiles: record
//! arrays passed to mass-assembly functions, component arrays sized through a
//! vehicle's geometry record, landing-gear reductions over zero legs,
//! electrical drives with `der` of an angle difference and per-phase `when`
//! clauses, and SPI blocks reading the history of their inputs. Values are
//! checked by simulation where a wrong lowering would still compile.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

fn compile(source: &str, model: &str) -> rumoca::CompilationResult {
    Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
        .unwrap_or_else(|error| panic!("{model} must compile: {error:?}"))
}

fn simulate(source: &str, model: &str, t_end: f64) -> SimResult {
    let compiled = compile(source, model);
    simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{model} must simulate: {error:?}"))
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

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= 1e-6 * expected.abs().max(1.0),
        "{what}: expected {expected}, got {actual}"
    );
}

const RECORD_ARRAYS: &str = "\
package RecordArrays
  record R
    Real m = 0;
    Real v[3] = zeros(3);
  end R;
  function total
    input R parts[:];
    output Real y = 0;
  algorithm
    for i in 1:size(parts, 1) loop
      y := y + parts[i].m + parts[i].v[1];
    end for;
  end total;
  model Named
    parameter R a[2](m = {1, 2});
    Real y;
  equation
    y = total(a);
  end Named;
  model Literal
    parameter R c(m = 1, v = {1, 0, 0});
    parameter R d(m = 2);
    Real y;
  equation
    y = total({c, d});
  end Literal;
  model Concatenated
    parameter R c(m = 1);
    parameter R a[2](each m = 2);
    Real y;
  equation
    y = total(cat(1, {c}, a));
  end Concatenated;
  model Empty
    parameter R c(m = 3);
    parameter Integer n = 0;
    parameter R a[n];
    Real y;
  equation
    y = total(cat(1, {c}, a));
  end Empty;
  model Inner
    parameter Integer n = 2;
    parameter R a[n](each m = 1);
    parameter R c(m = 3);
    final parameter Real s = total(cat(1, {c}, a));
  end Inner;
  model Nested
    Inner first;
    Inner none(n = 0);
    Real y;
  equation
    y = first.s + 10 * none.s;
  end Nested;
end RecordArrays;
";

/// A record array reaches a function through every spelling the chassis mass
/// assembly uses: whole, as an array literal, concatenated with a scalar
/// record, empty, and inside sub-components of different sizes.
#[test]
fn record_arrays_pass_to_functions_in_every_form() {
    for (model, expected) in [
        ("RecordArrays.Named", 3.0),
        ("RecordArrays.Literal", 4.0),
        ("RecordArrays.Concatenated", 5.0),
        ("RecordArrays.Empty", 3.0),
        ("RecordArrays.Nested", 35.0),
    ] {
        let result = simulate(RECORD_ARRAYS, model, 0.1);
        assert_close(last(&result, "y"), expected, model);
    }
}

const FUNCTION_ASSERTIONS: &str = "\
package FunctionAssertions
  function triangular
    input Real x;
    output Real y = 0;
  algorithm
    for j in 1:3 loop
      for k in (j + 1):3 loop
        assert(x * j * k >= 0, \"negative\");
        y := y + j * k;
      end for;
    end for;
  end triangular;
  function guarded
    input Real x;
    output Real y = 0;
  algorithm
    if x > 1 then
      assert(x < 10, \"too large\");
      y := x;
    end if;
  end guarded;
  model Use
    Real y1;
    Real y2;
  equation
    y1 = triangular(time + 1);
    y2 = guarded(2);
  end Use;
end FunctionAssertions;
";

/// An assertion inside a dependent-range loop or an `if` of a function body.
#[test]
fn function_assertions_in_loops_and_branches() {
    let result = simulate(FUNCTION_ASSERTIONS, "FunctionAssertions.Use", 0.1);
    assert_close(last(&result, "y1"), 11.0, "triangular sum");
    assert_close(last(&result, "y2"), 2.0, "guarded value");
}

const PARAMETER_QUOTIENT: &str = "\
model ParameterQuotient
  parameter Integer n = 3;
  parameter Integer idx[4] = {1 + mod(i - 1, n) for i in 1:4};
  Real x(start = 0, fixed = true);
equation
  der(x) = idx[4] + mod(7, n) + div(7, n);
end ParameterQuotient;
";

/// `mod`/`div` of parameters need no event, even inside a comprehension.
#[test]
fn parameter_quotients_need_no_event() {
    let result = simulate(PARAMETER_QUOTIENT, "ParameterQuotient", 1.0);
    assert_close(last(&result, "x"), 4.0, "x(1)");
}

const COMPONENT_ARRAYS: &str = "\
package ComponentArrays
  record Geometry
    parameter Integer n = 4;
    parameter Real mount[n, 3] = {{i, 0, 0} for i in 1:n};
  end Geometry;
  record Hexa
    extends Geometry(n = 6);
  end Hexa;
  model Vehicle
    parameter Geometry geometry;
  end Vehicle;
  model Leg
    parameter Real r[3] = zeros(3);
    input Real v[3];
    output Real f;
  equation
    f = v[1] + v[2] + v[3] + r[1];
  end Leg;
  model Cycle
    Vehicle vehicle;
    Leg legs[vehicle.geometry.n](r = vehicle.geometry.mount / 2);
    Real total;
  equation
    legs.v = zeros(vehicle.geometry.n, 3);
    total = sum(legs.f);
  end Cycle;
  model Pair
    Cycle quad;
    Cycle hexa(vehicle(geometry = Hexa()));
  end Pair;
end ComponentArrays;
";

/// A component array sized by a record field of a sub-component, resized by
/// an outer modifier, with a non-`each` modifier distributed row by row and an
/// array equation over the elements' vector members.
#[test]
fn component_arrays_sized_through_a_sub_component() {
    let compiled = compile(COMPONENT_ARRAYS, "ComponentArrays.Pair");
    let names = compiled
        .flat
        .variables
        .keys()
        .map(|name| name.as_str().to_string())
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name == "quad.legs[4].f"));
    assert!(!names.iter().any(|name| name == "quad.legs[5].f"));
    assert!(names.iter().any(|name| name == "hexa.legs[6].f"));
    let result = simulate(COMPONENT_ARRAYS, "ComponentArrays.Pair", 0.1);
    assert_close(
        last(&result, "quad.total"),
        5.0,
        "quad: sum of i/2 over 4 legs",
    );
    assert_close(
        last(&result, "hexa.total"),
        10.5,
        "hexa: sum of i/2 over 6 legs",
    );
}

const REDUCTIONS: &str = "\
package Reductions
  model Leg
    input Real x;
    output Real f[3];
  equation
    f = {x, 2 * x, 3 * x};
  end Leg;
  model Sum
    parameter Integer n = 2;
    Leg legs[n];
    Real F[3];
  equation
    for i in 1:n loop
      legs[i].x = 1;
    end for;
    for j in 1:3 loop
      F[j] = sum(legs[i].f[j] for i in 1:n);
    end for;
  end Sum;
  model NoLegs
    parameter Integer n = 0;
    Leg legs[n];
    Real F[3];
    Real g;
  equation
    for j in 1:3 loop
      F[j] = sum(legs[i].f[j] for i in 1:n);
    end for;
    g = sum(i * 1.0 for i in 1:n) + product(i * 2.0 for i in 1:n);
  end NoLegs;
end Reductions;
";

/// Reductions inside a `for`-equation, over component arrays of any size
/// including zero (the landing gear with `enabled = false`).
#[test]
fn reductions_over_component_arrays_and_empty_domains() {
    let result = simulate(REDUCTIONS, "Reductions.Sum", 0.1);
    assert_close(last(&result, "F[3]"), 6.0, "F[3] over two legs");
    let result = simulate(REDUCTIONS, "Reductions.NoLegs", 0.1);
    assert_close(last(&result, "F[2]"), 0.0, "F[2] over no legs");
    assert_close(last(&result, "g"), 1.0, "empty sum plus empty product");
}

const DERIVATIVE_OF_EXPRESSION: &str = "\
model DerivativeOfExpression
  parameter Real k = 2;
  Real a(start = 1, fixed = true);
  Real b(start = 0, fixed = true);
  Real w;
equation
  der(a) = -a;
  der(b) = 1;
  w = der(k * (a - b)) + der(-a / k);
end DerivativeOfExpression;
";

/// `der` of a linear expression of states (`der(flange.phi - support.phi)`).
#[test]
fn derivative_of_a_linear_expression() {
    let result = simulate(DERIVATIVE_OF_EXPRESSION, "DerivativeOfExpression", 1.0);
    let a = (-1.0f64).exp();
    assert!(
        (last(&result, "w") - (2.0 * (-a - 1.0) + a / 2.0)).abs() < 1e-4,
        "w(1) = k*(der(a) - der(b)) - der(a)/k"
    );
}

const DISCRETE_ARRAYS: &str = "\
package DiscreteArrays
  block Selector
    input Real u;
    output Boolean sel[3];
  equation
    for j in 1:3 loop
      sel[j] = u > j;
    end for;
  end Selector;
  model Leg
    input Boolean on;
    Real i(start = 0, fixed = true);
  equation
    der(i) = if on then 1 else 0;
  end Leg;
  model Looped
    Selector c(u = time);
    Leg legs[3];
  equation
    for j in 1:3 loop
      legs[j].on = c.sel[j];
    end for;
  end Looped;
  model Vectorized
    Selector c(u = time);
    Leg legs[3];
  equation
    legs.on = c.sel;
  end Vectorized;
end DiscreteArrays;
";

/// Boolean connections between component arrays, looped and vectorized.
#[test]
fn discrete_equations_between_component_arrays() {
    for model in ["DiscreteArrays.Looped", "DiscreteArrays.Vectorized"] {
        let result = simulate(DISCRETE_ARRAYS, model, 4.0);
        for (leg, on_time) in [(1, 3.0), (2, 2.0), (3, 1.0)] {
            assert!(
                (last(&result, &format!("legs[{leg}].i")) - on_time).abs() < 1e-3,
                "{model}: leg {leg} accumulates its on time"
            );
        }
    }
}

const EVENTS: &str = "\
package Events
  model ElementWhen
    discrete Integer count[2](each start = 0, each fixed = true);
    Boolean high[2];
  equation
    for i in 1:2 loop
      high[i] = time > i;
      when change(high[i]) then
        count[i] = pre(count[i]) + 1;
      end when;
    end for;
  end ElementWhen;
  block Counter
    input Boolean u;
    output Integer n(start = 0, fixed = true);
  equation
    when change(u) then
      n = pre(n) + 1;
    end when;
  end Counter;
  block AlgorithmCounter
    input Real u;
    output Integer n(start = 0, fixed = true);
  algorithm
    when u > 0.5 then
      n := pre(n) + 1;
    end when;
  end AlgorithmCounter;
  model Counters
    AlgorithmCounter c[2];
  equation
    c[1].u = time;
    c[2].u = 2 * time;
  end Counters;
end Events;
";

/// Per-element `when` clauses, `change` of an external input, and
/// algorithms inside a component array (the PWM and SPI blocks).
#[test]
fn per_element_events_and_input_history() {
    let result = simulate(EVENTS, "Events.ElementWhen", 3.0);
    assert_close(last(&result, "count[1]"), 1.0, "count[1]");
    assert_close(last(&result, "count[2]"), 1.0, "count[2]");
    let counter = compile(EVENTS, "Events.Counter");
    assert!(
        counter
            .flat
            .variables
            .keys()
            .any(|name| name.as_str() == "u$history"),
        "change(u) of an external input reads a declared history"
    );
    let result = simulate(EVENTS, "Events.Counters", 1.0);
    assert_close(last(&result, "c[1].n"), 1.0, "c[1].n");
    assert_close(last(&result, "c[2].n"), 1.0, "c[2].n");
}

const FAMILY_BRANCH_CALL: &str = "\
model FamilyBranchCall
  function f
    input Real x;
    output Real y;
  algorithm
    y := 2 * x;
  end f;
  parameter Boolean useF = true;
  Real x[3](each start = 1, each fixed = true);
equation
  for i in 1:3 loop
    der(x[i]) = if useF then f(x[i]) else -x[i];
  end for;
end FamilyBranchCall;
";

/// `select-family-branches` choosing a branch that is a call result.
#[test]
fn a_selected_family_branch_may_be_a_call() {
    let result = simulate(FAMILY_BRANCH_CALL, "FamilyBranchCall", 0.5);
    assert!(
        (last(&result, "x[2]") - 1.0f64.exp()).abs() < 1e-3,
        "x = exp(2 t)"
    );
}

const RECORD_BINDING_PRECEDENCE: &str = "\
package RecordBinding
  record MP
    Real mass = 0;
    Real r[3] = zeros(3);
  end MP;
  model Assembly
    parameter MP aggregate(mass = 1);
    final parameter Real m = aggregate.mass;
  end Assembly;
  model Plant
    parameter MP aggregate(mass = 1.5);
    Assembly chassis(aggregate = aggregate);
    Real y;
  equation
    y = chassis.m;
  end Plant;
end RecordBinding;
";

/// An outer binding of a whole record overrides the declaration's own field
/// modifiers (MLS §7.2.4): the chassis receives the plant's mass budget.
#[test]
fn an_outer_record_binding_overrides_declared_field_modifiers() {
    let result = simulate(RECORD_BINDING_PRECEDENCE, "RecordBinding.Plant", 0.1);
    assert_close(last(&result, "y"), 1.5, "chassis mass");
}

const LOOP_ASSERTION_OVER_LOCAL: &str = "\
package LoopAssertion
  function largest
    input Real x[:];
    output Real y = 0;
  protected
    Real scaled[2];
  algorithm
    for i in 1:size(x, 1) loop
      scaled[1] := 2 * x[i];
      assert(scaled[1] >= 0, \"negative entry\");
      y := max(y, scaled[1]);
    end for;
  end largest;
  model Good
    Real y;
  equation
    y = largest({1, 3, 2});
  end Good;
  model Bad
    Real y;
  equation
    y = largest({1, -3, 2});
  end Bad;
end LoopAssertion;
";

/// An assertion over a value its loop iteration computed is still checked.
#[test]
fn a_loop_assertion_over_an_iteration_value_still_fires() {
    let result = simulate(LOOP_ASSERTION_OVER_LOCAL, "LoopAssertion.Good", 0.1);
    assert_close(last(&result, "y"), 6.0, "largest doubled entry");
    let compiled = compile(LOOP_ASSERTION_OVER_LOCAL, "LoopAssertion.Bad");
    let failure = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 0.1,
            ..SimOptions::default()
        },
    );
    assert!(failure.is_err(), "a negative entry must fail the assertion");
}

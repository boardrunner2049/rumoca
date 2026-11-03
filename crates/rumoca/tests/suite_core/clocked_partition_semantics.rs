//! MLS §16 clocked-partition semantics exercised by the `Modelica.Clocked`
//! library: inferred sub-sampling factors, left-limit sampling, vector and
//! multi-output clocked definitions, parameter-selected clocks, and clock
//! inference through conversions.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae};

fn simulate(source: &str, model: &str, t_end: f64, dt: f64) -> SimResult {
    let compiled = match Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("{model} compiles: {error:?}"),
    };
    let options = SimOptions {
        t_end,
        dt: Some(dt),
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

/// The value `name` holds at the last output instant not after `time`.
fn value_at(result: &SimResult, name: &str, time: f64) -> f64 {
    let index = result
        .times
        .iter()
        .rposition(|candidate| *candidate <= time + 1.0e-12)
        .expect("an output instant precedes the probe");
    column(result, name)[index]
}

/// MLS §16.5.2: `superSample(u)` without `factor` infers the factor from the
/// source and target partitions. One block class instantiated twice owns one
/// source span but two partitions, and each instance keeps its own factor.
#[test]
fn inferred_super_sample_factor_follows_each_instance_partition() {
    let source = r#"
model InferredFactor
  block Up
    input Real u;
    output Real y;
  protected
    Real dummy;
  equation
    when Clock() then
      dummy = u;
    end when;
    when Clock() then
      y = superSample(u);
    end when;
  end Up;
  block UpBy
    parameter Integer f = 3;
    input Real u;
    output Real y;
  equation
    when Clock() then
      y = superSample(u, f);
    end when;
  end UpBy;
  Clock c = Clock(1, 10);
  Real x;
  Up up1;
  Up up2;
  UpBy by3;
  UpBy by2(f = 2);
  Real s3;
  Real s2;
  Real h3;
  Real h2;
equation
  x = sample(time, c);
  up1.u = x;
  up2.u = x;
  by3.u = x;
  by2.u = x;
  s3 = up1.y + by3.y;
  s2 = up2.y + by2.y;
  h3 = hold(s3);
  h2 = hold(s2);
end InferredFactor;
"#;
    let result = simulate(source, "InferredFactor", 0.35, 0.005);
    for time in [0.05, 0.15, 0.25] {
        let base = (time * 10.0_f64).floor() / 10.0;
        for name in ["h3", "h2"] {
            let actual = value_at(&result, name, time);
            assert!(
                (actual - 2.0 * base).abs() < 1.0e-12,
                "{name} at {time}: expected {}, got {actual}",
                2.0 * base
            );
        }
    }
}

/// MLS §16.5.1: `sample(u)` is the left limit of `u` at the tick, so a clocked
/// value that feeds back through `hold` reads the value held before the tick
/// and forms no same-instant loop.
#[test]
fn sampling_a_discrete_value_reads_its_left_limit() {
    let source = r#"
model LeftLimit
  Clock c = Clock(1, 10);
  Integer y(start = 0);
  Integer s;
  Integer h;
equation
  h = hold(y);
  s = sample(h, c);
  y = s + 1;
end LeftLimit;
"#;
    let result = simulate(source, "LeftLimit", 0.35, 0.01);
    for (time, expected) in [(0.05, 1.0), (0.15, 2.0), (0.25, 3.0), (0.35, 4.0)] {
        let actual = value_at(&result, "h", time);
        assert_eq!(actual, expected, "h at {time}");
    }
}

/// MLS §8.5: equations active at one event instant are solved together, so an
/// unclocked `when` body that reads a variable it defines sees the new value.
#[test]
fn simultaneous_when_equations_read_the_instant_value() {
    let source = r#"
model Simultaneous
  discrete Real a(start = 0, fixed = true);
  discrete Real b(start = 0, fixed = true);
equation
  when {time >= 0.05, initial()} then
    a = 3 + time;
    b = a;
  end when;
end Simultaneous;
"#;
    let result = simulate(source, "Simultaneous", 0.1, 0.01);
    for time in [0.0, 0.02, 0.06, 0.09] {
        assert_eq!(
            value_at(&result, "a", time),
            value_at(&result, "b", time),
            "b must equal a at {time}"
        );
    }
    assert!((value_at(&result, "b", 0.09) - 3.05).abs() < 1.0e-12);
}

/// MLS §8.3.4: the rows of an if-equation are sets; branches that assign the
/// same variables in different orders are paired by variable.
#[test]
fn if_equation_rows_pair_by_assigned_variable() {
    let source = r#"
model TickPulse
  Clock c = Clock(1, 10);
  Integer counter(start = 0);
  Boolean started(start = false);
  Real h;
equation
  when c then
    if previous(started) then
      counter = if previous(counter) == 2 then 0 else previous(counter) + 1;
      started = previous(started);
    else
      started = previous(counter) >= 1;
      counter = if started then 0 else previous(counter) + 1;
    end if;
  end when;
  h = hold(counter);
end TickPulse;
"#;
    let result = simulate(source, "TickPulse", 0.55, 0.01);
    let expected = [1.0, 0.0, 1.0, 2.0, 0.0, 1.0];
    for (tick, expected) in expected.iter().enumerate() {
        let time = tick as f64 / 10.0 + 0.05;
        assert_eq!(value_at(&result, "h", time), *expected, "h at {time}");
    }
}

/// MLS §10.4.1: a range bound written over a settled parameter proves the same
/// compact range inside a clocked `when` body as in a plain equation.
#[test]
fn clocked_body_range_over_parameter_is_settled() {
    let source = r#"
model Fir
  parameter Integer n = 2;
  Clock c = Clock(1, 10);
  Real u;
  Real buffer[n + 1](each start = 0);
  Real y;
  Real h;
equation
  u = sample(time, c);
  when c then
    buffer = cat(1, {u}, previous(buffer[1:n]));
    y = sum(buffer);
  end when;
  h = hold(y);
end Fir;
"#;
    let result = simulate(source, "Fir", 0.35, 0.01);
    let actual = value_at(&result, "h", 0.35);
    assert!((actual - (0.3 + 0.2 + 0.1)).abs() < 1.0e-12, "h = {actual}");
}

/// MLS §16.5.1 samples a vector element-wise; each element is a clocked
/// discrete Real definition, not a continuous residual.
#[test]
fn vector_sample_defines_clocked_discrete_reals() {
    let source = r#"
model VectorSample
  Clock c = Clock(1, 10);
  Real u[2];
  Real y[2];
equation
  u = {time, 2 * time};
  y = sample(u, c);
end VectorSample;
"#;
    let result = simulate(source, "VectorSample", 0.25, 0.01);
    assert!((value_at(&result, "y[1]", 0.25) - 0.2).abs() < 1.0e-12);
    assert!((value_at(&result, "y[2]", 0.25) - 0.4).abs() < 1.0e-12);
}

/// MLS §12.4.3: a multi-result call equation defines discrete receivers the
/// same way it defines continuous ones.
#[test]
fn multi_output_call_defines_clocked_discrete_receivers() {
    let source = r#"
model MultiOutput
  function split
    input Real s;
    output Real x;
    output Integer n;
  algorithm
    x := 2 * s;
    n := integer(s);
  end split;
  Clock c = Clock(1, 2);
  Real u;
  Real x;
  Integer n;
  Real hx;
  Real hn;
equation
  u = sample(3 * time, c);
  (x, n) = split(u);
  hx = hold(x);
  hn = hold(n);
end MultiOutput;
"#;
    let result = simulate(source, "MultiOutput", 1.2, 0.05);
    assert!((value_at(&result, "hx", 1.1) - 6.0).abs() < 1.0e-12);
    assert_eq!(value_at(&result, "hn", 1.1), 3.0);
}

/// MLS §16.7: a Clock defined by a parameter-selected expression takes the
/// clock the parameter values select.
#[test]
fn parameter_selected_clock_expression_has_a_static_schedule() {
    let source = r#"
model SelectedClock
  parameter Boolean fast = false;
  Clock c;
  Real y;
  Real h;
equation
  if fast then
    c = Clock(1, 100);
  else
    c = subSample(Clock(1, 100), 10);
  end if;
  y = sample(time, c);
  h = hold(y);
end SelectedClock;
"#;
    let result = simulate(source, "SelectedClock", 0.25, 0.01);
    assert!((value_at(&result, "h", 0.25) - 0.2).abs() < 1.0e-12);
}

/// MLS §16.5.1 infers the clock of a `sample(u)` partition from the clock
/// relations it takes part in before any fallback to a unique model clock.
#[test]
fn sample_clock_is_inferred_through_a_super_sample_relation() {
    let source = r#"
model InferThroughConversion
  Clock fast = Clock(1, 50);
  Real slow;
  Real fastValue;
  Real reference;
  Real mixed;
  Real held;
equation
  slow = sample(time);
  fastValue = superSample(slow, 5);
  reference = sample(time, fast);
  mixed = fastValue + 0 * reference;
  held = hold(mixed);
end InferThroughConversion;
"#;
    let result = simulate(source, "InferThroughConversion", 0.25, 0.01);
    assert!((value_at(&result, "held", 0.25) - 0.2).abs() < 1.0e-12);
    assert!((value_at(&result, "held", 0.19) - 0.1).abs() < 1.0e-12);
}

/// MLS §16.10: `firstTick()` is true exactly at the first tick of the clock of
/// the partition it is read in, in an equation or a clocked `when` body.
#[test]
fn first_tick_is_true_only_at_the_first_tick() {
    let source = r#"
model FirstTick
  block Change
    input Boolean u;
    output Boolean y;
  equation
    if firstTick() then
      y = false;
    else
      y = not (u == previous(u));
    end if;
  end Change;
  Clock c = Clock(1, 10);
  Boolean s;
  Change change;
  Integer k(start = 0);
  Boolean changed;
  Integer count;
equation
  s = sample(time > 0.25, c);
  change.u = s;
  changed = hold(change.y);
  when c then
    k = if firstTick() then 100 else previous(k) + 1;
  end when;
  count = hold(k);
end FirstTick;
"#;
    let result = simulate(source, "FirstTick", 0.45, 0.01);
    for (tick, expected) in [100.0, 101.0, 102.0, 103.0, 104.0].iter().enumerate() {
        let time = tick as f64 / 10.0 + 0.05;
        assert_eq!(
            value_at(&result, "count", time),
            *expected,
            "count at {time}"
        );
        let changed = if tick == 3 { 1.0 } else { 0.0 };
        assert_eq!(
            value_at(&result, "changed", time),
            changed,
            "changed at {time}"
        );
    }
}

/// SOLVE-C57: a producer that reads another same-clock producer through a row
/// between two discrete Real coordinates observes it on the tick, so the two
/// are never fused into one entry read.
#[test]
fn connected_same_clock_producers_are_ordered_not_fused() {
    let source = r#"
model ConnectedFeedback
  block Controller
    input Real e;
    output Real y;
  protected
    Real x(start = 0);
  equation
    when Clock() then
      x = previous(x) + e;
      y = x + e;
    end when;
  end Controller;
  block Assign
    input Real u;
    output Real y;
    input Clock clock;
  equation
    when clock then
      y = u;
    end when;
  end Assign;
  Clock c = Clock(1, 10);
  Controller controller;
  Assign assign;
  Real s;
  Real u2;
  Real held;
equation
  s = sample(time, c);
  assign.clock = c;
  assign.u = s;
  u2 = assign.y;
  controller.e = 1 - u2;
  held = hold(controller.y);
end ConnectedFeedback;
"#;
    let result = simulate(source, "ConnectedFeedback", 0.35, 0.01);
    for (tick, expected) in [2.0, 2.8, 3.5, 4.1].iter().enumerate() {
        let time = tick as f64 / 10.0 + 0.05;
        let actual = value_at(&result, "held", time);
        assert!(
            (actual - expected).abs() < 1.0e-12,
            "held at {time}: {actual}"
        );
    }
}

/// MLS §16.8.1: an equation that reads a clocked value and contains `der()`
/// belongs to a discretized partition. Integrating it continuously would
/// silently replace the clock's discretization, so it is refused.
#[test]
fn discretized_partition_is_refused_before_simulation() {
    let source = r#"
model Discretized
  Clock c = Clock(0.1);
  Real u;
  Real x(start = 1, fixed = true);
  Real y;
equation
  u = sample(time, c);
  der(x) = -x + u;
  y = hold(x);
end Discretized;
"#;
    let Err(error) = Compiler::new()
        .model("Discretized")
        .compile_str(source, "Discretized.mo")
    else {
        panic!("a discretized partition is refused");
    };
    let message = format!("{error:?}");
    assert!(
        message.contains("discretized clocked partition"),
        "{message}"
    );
}

/// MLS §16.5 / §9.1: a clocked vector assignment `y = u` and the scalar
/// connections to its elements define each element of a discrete Real array
/// by its own row. Each element counter advances on every tick of its own
/// feedback loop through `previous`.
#[test]
fn array_elements_are_defined_by_their_own_rows() {
    let source = r#"
model ElementRows
  block Assign
    input Real u[2];
    output Real y[2];
    input Clock clock;
  equation
    when clock then
      y = u;
    end when;
  end Assign;
  block Delay
    input Real u;
    output Real y;
  equation
    y = previous(u);
  end Delay;
  Clock c = Clock(1, 10);
  Assign assign;
  Delay d1;
  Delay d2;
equation
  assign.clock = c;
  assign.u[1] = d1.y + 1;
  assign.u[2] = d2.y + 2;
  d1.u = assign.y[1];
  d2.u = assign.y[2];
end ElementRows;
"#;
    let result = simulate(source, "ElementRows", 1.0, 0.01);
    for (tick, time) in [(1.0, 0.0), (5.0, 0.45), (11.0, 1.0)] {
        assert_eq!(
            value_at(&result, "assign.y[1]", time),
            tick,
            "y[1] at {time}"
        );
        assert_eq!(
            value_at(&result, "assign.y[2]", time),
            2.0 * tick,
            "y[2] at {time}"
        );
        assert_eq!(
            value_at(&result, "d1.y", time),
            tick - 1.0,
            "d1.y at {time}"
        );
    }
}

//! MLS §3.7.5 `when sample(start, interval)` clauses are event-style `when`
//! clauses on a time event (SPEC_0044 ME-EVENT-010), not MLS §16 clock
//! partitions.
//!
//! Two clauses on one sample instant are one set of active equations, solved
//! together and sorted by data dependency (MLS §8.6): a clause that reads a
//! value another clause writes at the same tick — directly or through
//! algebraic element, slice, or `for`-loop equations — sees the new value
//! whatever the source order (the FIRE `SampledDelivery` queue reads the
//! `StatisticalSensor` draw of its own epoch). And a clause body reads any
//! variable, including a discrete one another event writes.

use rumoca_sim::{SimOptions, SimResult, simulate_dae};

fn simulate(source: &str, model: &str, t_end: f64) -> SimResult {
    let compiled = rumoca::Compiler::new()
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
    .unwrap_or_else(|error| panic!("{model} must simulate: {error:?}"))
}

/// The value of `name` in the last trace row at `time`, the settled value
/// after any event there.
fn settled(result: &SimResult, name: &str, time: f64) -> f64 {
    let column = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("no column {name}: {:?}", result.names));
    let row = result
        .times
        .iter()
        .rposition(|candidate| (candidate - time).abs() <= 1e-12)
        .unwrap_or_else(|| panic!("no trace row at {time}"));
    result.data[column][row]
}

/// The reading clause comes first in source order; `u` is determined by
/// element equations, a slice equation, and a `for` loop, none of them one
/// exact definition of the whole array.
const SAME_TICK_READ: &str = r#"
model SameTickRead
  discrete Real a(start = 0, fixed = true);
  discrete Real b(start = 0, fixed = true);
  discrete Real c[3](each start = 0, each fixed = true);
  Real u[2];
  Real w[3];
equation
  when sample(0, 0.1) then
    b = u[1] + u[2];
    c = w;
  end when;
  u[1] = 2*a;
  u[2] = a;
  w[1:2] = {a, 10*a};
  for i in 3:3 loop
    w[i] = u[1] + 1;
  end for;
  when sample(0, 0.1) then
    a = pre(a) + 1;
  end when;
end SameTickRead;
"#;

#[test]
fn a_sample_clause_reads_the_value_another_clause_writes_at_the_same_tick() {
    let result = simulate(SAME_TICK_READ, "SameTickRead", 0.35);
    // Tick k (t = 0.1 k) sets a = k + 1; the reading clause sees that value.
    for (time, a) in [(0.0, 1.0), (0.1, 2.0), (0.2, 3.0), (0.3, 4.0)] {
        assert_eq!(settled(&result, "a", time), a, "a at {time}");
        assert_eq!(settled(&result, "b", time), 3.0 * a, "b at {time}");
        assert_eq!(settled(&result, "c[1]", time), a, "c[1] at {time}");
        assert_eq!(settled(&result, "c[2]", time), 10.0 * a, "c[2] at {time}");
        assert_eq!(
            settled(&result, "c[3]", time),
            2.0 * a + 1.0,
            "c[3] at {time}"
        );
    }
}

/// A `when sample` body reads discrete variables a time event and the
/// initial values own; neither belongs to the sample instant.
const FOREIGN_DISCRETE_READ: &str = r#"
model ForeignDiscreteRead
  discrete Real x(start = 1, fixed = true);
  discrete Real h[2](each start = 5, each fixed = true);
  discrete Real y(start = 0, fixed = true);
  discrete Real z[2](each start = 0, each fixed = true);
equation
  when time > 0.15 then
    x = 2;
  end when;
  when sample(0, 0.1) then
    y = x;
    z = {h[2], x + y};
  end when;
end ForeignDiscreteRead;
"#;

#[test]
fn a_sample_clause_reads_discretes_other_events_own() {
    let result = simulate(FOREIGN_DISCRETE_READ, "ForeignDiscreteRead", 0.25);
    assert_eq!(settled(&result, "y", 0.1), 1.0);
    assert_eq!(settled(&result, "z[1]", 0.1), 5.0);
    assert_eq!(settled(&result, "z[2]", 0.1), 2.0);
    assert_eq!(settled(&result, "y", 0.2), 2.0);
    assert_eq!(settled(&result, "z[2]", 0.2), 4.0);
}

/// The FIRE `PartialSampledSensor` delivery queue: a component's record
/// parameter shares its name with a record parameter of the enclosing model.
const SHADOWED_RECORD_QUEUE: &str = r#"
package ShadowedRecordQueue
  record P
    parameter Integer latency = 0;
    parameter Real period = 0.1;
  end P;
  model Sensor
    parameter P p;
    discrete Integer acquired(start = 0, fixed = true);
    discrete Real history[p.latency + 1, 2](each start = 0, each fixed = true);
    discrete Real delivered[2](each start = 0, each fixed = true);
  equation
    when sample(0, p.period) then
      acquired = pre(acquired) + 1;
      history[1, :] = {time, 2*time};
      for j in 2:p.latency + 1 loop
        history[j, :] = pre(history[j - 1, :]);
      end for;
      delivered = if acquired > p.latency then history[p.latency + 1, :] else pre(delivered);
    end when;
  end Sensor;
  model Top
    parameter P p(latency = 2);
    Sensor s;
  end Top;
end ShadowedRecordQueue;
"#;

#[test]
fn a_sample_queue_beside_a_same_named_outer_record_simulates() {
    let result = simulate(SHADOWED_RECORD_QUEUE, "ShadowedRecordQueue.Top", 0.25);
    // Latency 0: each tick delivers its own acquisition.
    assert_eq!(settled(&result, "s.delivered[1]", 0.2), 0.2);
    assert_eq!(settled(&result, "s.delivered[2]", 0.2), 0.4);
}

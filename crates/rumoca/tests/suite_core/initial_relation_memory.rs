//! MLS §8.6 (SPEC_0022 SIM-011, SPEC_0044 ME-EVENT-011): at initialization a
//! relation takes the value of its operands, not the declaration seed of its
//! relation memory, so an equation the relation deselects is never evaluated;
//! an input written across a relation between steps moves it before the next
//! read.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimSolverMode, SimulationSession, simulate_dae_with_diagnostics};

/// `parallel` reads the algebraic `den`, which the parameter `d = 0` makes
/// zero: the seed `false` of its relation memory would divide by zero. The
/// relation `x < 0` reads the algebraic the first one selects, as the FIRE
/// range sensor's status does.
const SOURCE: &str = r#"
model InitialRelation
  parameter Real d = 0;
  Real den;
  Boolean parallel;
  Real x;
  Real s(start = 0, fixed = true);
  Integer status;
equation
  den = d * (1 + 0 * time);
  parallel = abs(den) < 1e-9;
  x = if parallel then 0 else 1 / den;
  status = if parallel or x < 0 then 2 else 1;
  der(s) = 1 + x;
end InitialRelation;

model InputRelation
  input Real u(start = 1);
  Real s(start = 0, fixed = true);
  Real y;
equation
  y = if u > 0.5 then sqrt(u - 0.5) else 0;
  der(s) = y;
end InputRelation;

model InputLatch
  input Real gap(start = 1);
  Boolean contact;
  Integer hits(start = 0, fixed = true);
equation
  contact = gap <= 0;
  when contact then
    hits = pre(hits) + 1;
  end when;
end InputLatch;
"#;

fn check_initial_relation(solver_mode: SimSolverMode) {
    let compiled = Compiler::new()
        .model("InitialRelation")
        .compile_str(SOURCE, "initial_relation_memory.mo")
        .expect("compile the initial-relation model");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 0.5,
            dt: Some(0.1),
            rtol: 1e-8,
            atol: 1e-8,
            solver_mode,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{solver_mode:?}: initialization failed: {error:?}"));
    let column = |name: &str| {
        &result.data[result
            .names
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or_else(|| panic!("no `{name}` in {:?}", result.names))]
    };
    let status = column("status");
    for (k, (time, (x, s))) in result
        .times
        .iter()
        .zip(column("x").iter().zip(column("s")))
        .enumerate()
    {
        assert_eq!(*x, 0.0, "{solver_mode:?}: x at t={time}");
        assert_eq!(status[k], 2.0, "{solver_mode:?}: status at t={time}");
        assert!(
            (s - time).abs() < 1e-6,
            "{solver_mode:?}: s={s} at t={time}"
        );
    }
}

#[test]
fn bdf_initial_relation_takes_its_operand_value() {
    check_initial_relation(SimSolverMode::Bdf);
}

#[test]
fn rk_initial_relation_takes_its_operand_value() {
    check_initial_relation(SimSolverMode::RkLike);
}

/// The input crosses `u > 0.5` between steps: each read after the write sees
/// the side the written value selects, never `sqrt` of a negative argument.
#[test]
fn session_input_written_across_a_relation_moves_it() {
    let compiled = Compiler::new()
        .model("InputRelation")
        .compile_str(SOURCE, "initial_relation_memory.mo")
        .expect("compile the input-relation model");
    let mut session = SimulationSession::new(
        &compiled.dae,
        SimOptions {
            initial_inputs: vec![("u".into(), 1.0)],
            ..SimOptions::default()
        },
    )
    .expect("start the input-relation session");
    let half = 0.5_f64.sqrt();
    let mut integral = 0.0;
    for (u, y) in [
        (1.0, half),
        (0.0, 0.0),
        (1.0, half),
        (0.0, 0.0),
        (2.5, 2.0_f64.sqrt()),
    ] {
        session.set_input("u", u).expect("write u");
        let read = session.get("y").expect("read y").expect("y is a variable");
        assert!(
            (read - y).abs() < 1e-12,
            "u = {u}: y = {read}, expected {y}"
        );
        session.step(0.1).expect("step across the written input");
        integral += 0.1 * y;
    }
    let s = session.get("s").expect("read s").expect("s is a variable");
    assert!((s - integral).abs() < 1e-6, "s = {s}, expected {integral}");
}

/// A `when` on a relation of an input fires once per written crossing: the
/// write is an event whose `pre` is the coordinate before it.
#[test]
fn session_input_crossing_fires_its_when_edge_once() {
    let compiled = Compiler::new()
        .model("InputLatch")
        .compile_str(SOURCE, "initial_relation_memory.mo")
        .expect("compile the input-latch model");
    let mut session = SimulationSession::new(
        &compiled.dae,
        SimOptions {
            initial_inputs: vec![("gap".into(), 1.0)],
            ..SimOptions::default()
        },
    )
    .expect("start the input-latch session");
    for (gap, contact, hits) in [
        (-1.0, 1.0, 1.0),
        (-2.0, 1.0, 1.0),
        (1.0, 0.0, 1.0),
        (-0.5, 1.0, 2.0),
    ] {
        session.set_input("gap", gap).expect("write gap");
        let read = |session: &SimulationSession, name: &str| {
            session.get(name).expect("read").expect("a variable")
        };
        let written = (read(&session, "contact"), read(&session, "hits"));
        assert_eq!(written, (contact, hits), "after gap = {gap}");
        session.step(0.1).expect("step");
        let stepped = (read(&session, "contact"), read(&session, "hits"));
        assert_eq!(stepped, (contact, hits), "stepped gap = {gap}");
    }
}

//! The accepted steps and located events a simulation records for its speed
//! report row.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, reset_step_counts, simulate_dae_with_diagnostics, step_counts};

const BOUNCE: &str = "
model Bounce
  Real h(start = 1, fixed = true);
  Real v(start = 0, fixed = true);
equation
  der(h) = v;
  der(v) = -9.81;
  when h < 0 then
    reinit(v, -0.8 * pre(v));
  end when;
end Bounce;
";

/// A falling ball integrates with accepted steps and bounces at located
/// events, and the counts start from zero at each reset.
#[test]
fn a_bouncing_ball_records_its_steps_and_events() {
    let compiled = match Compiler::new()
        .model("Bounce")
        .compile_str(BOUNCE, "Bounce.mo")
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("compile Bounce: {error:#}"),
    };
    reset_step_counts();
    let options = SimOptions {
        t_end: 1.5,
        ..Default::default()
    };
    if let Err(error) = simulate_dae_with_diagnostics(&compiled.dae, &options) {
        panic!("simulate Bounce: {error:#}");
    }
    let counts = step_counts();
    assert!(counts.solver_steps > 10, "{counts:?}");
    assert!(counts.root_hits >= 2, "{counts:?}");
    reset_step_counts();
    assert_eq!(step_counts(), rumoca_sim::HotpathStatsSnapshot::default());
}

/// A chain of explicit algebraic assignments issues exact assignment
/// schedules, and the interpreter proves and prepares each one when its
/// runtime is built, before any refresh runs.
#[test]
fn every_issued_schedule_is_prepared_when_the_runtime_is_built() {
    const CHAIN: &str = "
model Chain
  Real m(start = 1, fixed = true);
  Real level;
  Real h;
equation
  level = m / 1000;
  h = 4000 * level;
  der(m) = -0.1 * m;
end Chain;
";
    let compiled = match Compiler::new()
        .model("Chain")
        .compile_str(CHAIN, "Chain.mo")
    {
        Ok(compiled) => compiled,
        Err(error) => panic!("compile Chain: {error:#}"),
    };
    let model = match rumoca_sim::lower_dae_for_simulation(&compiled.dae, &SimOptions::default()) {
        Ok(model) => model,
        Err(error) => panic!("lower Chain: {error:#}"),
    };
    let issued = model
        .problem
        .continuous
        .refresh_owners
        .exact_assignment_schedules()
        .len();
    assert!(issued > 0, "the chain issues an exact assignment schedule");
    let runtime = rumoca_solver::SolveRuntime::new(&model).expect("the runtime builds");
    assert_eq!(runtime.interpreted_schedule_count(), issued);
}

//! The warm-start rule of each executor class (SPEC_0044 ME-PROJ-005): an
//! importer-driven component refreshes from the committed seed, so its result
//! is a function of (t, x, p, relation memory, committed seed); an
//! integrator-driven run keeps the integrator's warm start, which its fixed
//! evaluation order makes deterministic.

use super::{constant_delay_model, fixture_instance_config, nonlinear_right_limit_seed_model};
use crate::fmi_me::{MeInstanceConfig, MeModelSource, MeTime, SolveMeKernel};
use rumoca_ir_solve as solve;

fn continuous_kernel(model: &solve::SolveModel, config: &MeInstanceConfig) -> SolveMeKernel {
    let mut model = model.clone();
    model.problem.continuous.refresh_owners =
        rumoca_eval_solve::refresh_plan::build_continuous_refresh_owners(&mut model.problem)
            .expect("fixture refresh owners construct");
    let mut kernel = SolveMeKernel::instantiate(MeModelSource::fixture(&model), config).unwrap();
    kernel.enter_initialization_mode().unwrap();
    kernel.exit_initialization_mode().unwrap();
    kernel.update_discrete_states().unwrap();
    kernel.enter_continuous_time_mode().unwrap();
    kernel
}

fn importer() -> MeInstanceConfig {
    fixture_instance_config().importer_driven()
}

/// Evaluate the derivative after a sequence of trial states, then at `x`.
fn derivative_after(kernel: &mut SolveMeKernel, trials: &[f64], x: f64) -> u64 {
    let mut out = Vec::new();
    for &trial in trials {
        kernel.set_continuous_states(&[trial]).unwrap();
        kernel.get_continuous_state_derivatives(&mut out).unwrap();
    }
    kernel.set_continuous_states(&[x]).unwrap();
    kernel.get_continuous_state_derivatives(&mut out).unwrap();
    out[0].to_bits()
}

#[test]
fn importer_trial_histories_from_one_commit_give_bit_identical_refreshes() {
    // `a² = x` settles by Newton; a warm start carried over from the last
    // trial reaches `sqrt(5)` along a different path for each history.
    let model = nonlinear_right_limit_seed_model();
    let histories: [&[f64]; 3] = [&[], &[4.1], &[3.9, 4.4, 6.5]];
    let bits = histories
        .iter()
        .map(|trials| derivative_after(&mut continuous_kernel(&model, &importer()), trials, 5.0))
        .collect::<Vec<_>>();
    assert!(bits.windows(2).all(|pair| pair[0] == pair[1]), "{bits:?}");

    // After a completed step the accepted point's refresh is the next seed,
    // whatever the importer evaluated before or after completing the step.
    let after_step = |before: &[f64], after: &[f64]| {
        let mut kernel = continuous_kernel(&model, &importer());
        derivative_after(&mut kernel, before, 4.2);
        kernel.set_time(MeTime::at(0.1)).unwrap();
        kernel.set_continuous_states(&[4.2]).unwrap();
        assert!(
            !kernel
                .completed_integrator_step(true)
                .unwrap()
                .enter_event_mode
        );
        derivative_after(&mut kernel, after, 5.0)
    };
    assert_eq!(after_step(&[], &[]), after_step(&[3.0, 4.0], &[4.9, 7.0]));
}

/// The integrator-driven class keeps its warm start: one fixed evaluation
/// order gives one result, bit for bit, on every run.
#[test]
fn integrator_driven_runs_repeat_bit_for_bit() {
    let model = nonlinear_right_limit_seed_model();
    let run = || {
        let mut kernel = continuous_kernel(&model, &fixture_instance_config());
        derivative_after(&mut kernel, &[3.9, 4.4, 6.5], 5.0)
    };
    assert_eq!(run(), run());
}

/// A delay-bearing component resolves the accepted point's seed before it
/// borrows its delay scratch, so a derivative query after a completed step
/// does not re-enter that scratch.
#[test]
fn a_delay_component_resolves_the_seed_before_its_delay_scratch() {
    let mut kernel = continuous_kernel(&constant_delay_model(), &importer());
    kernel.set_time(MeTime::at(0.05)).unwrap();
    assert!(
        !kernel
            .completed_integrator_step(true)
            .unwrap()
            .enter_event_mode
    );
    let mut out = Vec::new();
    kernel.get_continuous_state_derivatives(&mut out).unwrap();
    kernel.set_time(MeTime::at(0.07)).unwrap();
    kernel.get_continuous_state_derivatives(&mut out).unwrap();
}

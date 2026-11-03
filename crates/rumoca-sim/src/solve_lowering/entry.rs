use rumoca_ir_dae as dae;
use rumoca_ir_solve as solve;
use rumoca_solver::SimOptions;

use super::diagnostics::SimulationDiagnosticError;

pub fn lower_dae_for_simulation(
    model: &dae::Dae,
    opts: &SimOptions,
) -> Result<solve::SolveModel, SimulationDiagnosticError> {
    lower_dae_for_simulation_with_stage_timing_and_param_overrides(
        model,
        opts,
        &std::collections::HashMap::new(),
        |_| {},
    )
    .map(|(model, _)| model)
}

/// Lower a model whose inputs a host writes on every dispatch.
///
/// `lower_dae_for_simulation` refuses to build runtime vectors for an input
/// nobody drives, because integrating one would publish a trajectory computed
/// from a value the model never stated. GPU preparation is the case where the
/// driver provably exists: `prepare_gpu_simulation` hands the browser the
/// input's `P` slot index and `packages/rumoca-web/runtime/rumoca_gpu.js`
/// writes that slot from the live input source before each dispatch. What the
/// prepared payload still has to state is the value the slot holds until the
/// first write, and MLS §4.4.2.1 names it: the input's `start` attribute —
/// written on the declaration where the modeler chose it (`input Real
/// throttle(start = 0)`, as every shipped model in `examples/interactive`
/// does) and otherwise the predefined type's own attribute default (MLS §4.9),
/// moved into the declaration's `min`/`max` range when that excludes it. The
/// rule is `rumoca_phase_solve::fmi::host_input_start_values`, shared with
/// FMI export, so an input starts from the value its FMU publishes.
///
/// The seed travels the same override channel a runtime provider uses, so an
/// input carrying a binding keeps its binding and headless simulation
/// (`lower_dae_for_simulation`) keeps refusing an undriven input.
pub fn lower_dae_for_gpu_preparation(
    model: &dae::Dae,
    opts: &SimOptions,
) -> Result<solve::SolveModel, SimulationDiagnosticError> {
    lower_dae_with_host_driven_inputs(model, opts)
}

/// Lower for a host that writes every input before each evaluation.
///
/// GPU preparation and FMI export share this: both hand the kernel to a driver
/// that owns the input values, so the pre-write value is the input's `start`
/// (declared, else its type's default) rather than a refusal.
pub(super) fn lower_dae_with_host_driven_inputs(
    model: &dae::Dae,
    opts: &SimOptions,
) -> Result<solve::SolveModel, SimulationDiagnosticError> {
    let host_driven_seeds = host_driven_input_seeds(model)?;
    lower_dae_for_simulation_with_stage_timing_and_param_overrides(
        model,
        opts,
        &host_driven_seeds,
        |_| {},
    )
    .map(|(model, _)| model)
}

/// Pre-write value of every host-driven input scalar, by scalar name.
///
/// Inputs that carry a binding are left out: the binding is already a checked
/// default the runtime vectors evaluate. Every other input is seeded with its
/// `start`: the declared one, or, when the declaration has none, the start its
/// predefined type defines (MLS §4.9: `0`, `false`, or the first enumeration
/// literal). That value is not invented here: it is the declaration's start,
/// and FMI requires every input to publish one. Only headless simulation,
/// which has no host to write the input, refuses an undriven input.
pub(super) fn host_driven_input_seeds(
    model: &dae::Dae,
) -> Result<std::collections::HashMap<String, f64>, SimulationDiagnosticError> {
    Ok(host_driven_input_starts(model)?.into_iter().collect())
}

/// The pre-write value of every unbound top-level input scalar, in declaration
/// order: the value FMI export and GPU preparation seed it with, and the value
/// a standalone run that holds free inputs constant holds it at.
///
/// # Errors
///
/// Fails when a declared `start`, `min` or `max` of an input does not evaluate
/// to one numeric value per scalar.
pub fn host_driven_input_starts(
    model: &dae::Dae,
) -> Result<Vec<(String, f64)>, SimulationDiagnosticError> {
    model.inspect(|view| {
        let mut seeds = Vec::new();
        for (_, variable) in view
            .variables()
            .filter(|(_, variable)| variable.role() == dae::VariableRole::Input)
        {
            seed_host_driven_input(view, variable, &mut seeds)?;
        }
        Ok(seeds)
    })
}

fn seed_host_driven_input<'dae>(
    view: dae::DaeView<'dae>,
    variable: dae::VariableView<'dae>,
    seeds: &mut Vec<(String, f64)>,
) -> Result<(), SimulationDiagnosticError> {
    if variable.binding().is_some() {
        return Ok(());
    }
    // A String input has no numeric start, declared or default, and seeds
    // nothing: the host supplies it, and its default stays its own.
    let values = rumoca_phase_solve::fmi::host_input_start_values(view, variable)
        .map_err(fmi_metadata_error)?;
    let Some(values) = values else {
        return Ok(());
    };
    for scalar in 0..variable.scalar_count() {
        let name = variable.scalar_name(scalar).ok_or_else(|| {
            preparation_error(
                format!(
                    "checked input `{}` has no scalar name at ordinal {scalar}",
                    variable.name()
                ),
                variable,
            )
        })?;
        // One start expression may cover every scalar of an array input
        // (`input Real u[3](start = 0)`), exactly as the runtime vectors
        // broadcast a scalar value across the declared shape.
        let value = match values.as_slice() {
            [single] => *single,
            many => *many.get(scalar).ok_or_else(|| {
                preparation_error(
                    format!(
                        "start value for input `{}` contains {} scalars; expected {}",
                        variable.name(),
                        many.len(),
                        variable.scalar_count()
                    ),
                    variable,
                )
            })?,
        };
        seeds.push((name, value));
    }
    Ok(())
}

fn preparation_error(
    message: String,
    variable: dae::VariableView<'_>,
) -> SimulationDiagnosticError {
    SimulationDiagnosticError::RuntimePreparation {
        message,
        span: Some(variable.declaration().span()),
    }
}

pub(crate) fn lower_dae_for_simulation_with_stage_timing_and_param_overrides(
    model: &dae::Dae,
    opts: &SimOptions,
    parameter_overrides: &std::collections::HashMap<String, f64>,
    begin_stage: impl FnMut(&'static str),
) -> Result<(solve::SolveModel, crate::BuildSimulationTimings), SimulationDiagnosticError> {
    let (lowered, timings) = lower_correlated_for_simulation_with_stage_timing_and_param_overrides(
        model,
        opts,
        parameter_overrides,
        begin_stage,
    )?;
    Ok((lowered.into_model(), timings))
}

/// Lower while retaining the phase-owned DAE/Solve correlation until the
/// caller consumes it into the runtime FMI component.
pub(crate) fn lower_correlated_for_simulation_with_stage_timing_and_param_overrides<'source>(
    model: &'source dae::Dae,
    opts: &SimOptions,
    parameter_overrides: &std::collections::HashMap<String, f64>,
    mut begin_stage: impl FnMut(&'static str),
) -> Result<
    (
        rumoca_phase_solve::LoweredSolveModel<'source>,
        crate::BuildSimulationTimings,
    ),
    SimulationDiagnosticError,
> {
    let mut overrides = parameter_overrides.clone();
    overrides.extend(super::overrides::initial_input_values(model, opts)?);
    let lowered = rumoca_phase_solve::lower_solve_model(model, &overrides, |stage| match stage {
        rumoca_phase_solve::SolveModelLoweringStage::Programs => begin_stage("ir_solve"),
        rumoca_phase_solve::SolveModelLoweringStage::RuntimeValues => {
            begin_stage("runtime_vectors");
        }
    })
    .map_err(model_lowering_error)?;
    report_unlocalizable_guards(&lowered.model().problem.continuous.unlocalizable_guards);
    let timings = crate::BuildSimulationTimings {
        ir_solve_seconds: lowered.program_seconds() + lowered.runtime_value_seconds(),
        ir_solve_structural_dae_seconds: lowered.runtime_value_seconds(),
        ir_solve_lower_seconds: lowered.program_seconds(),
        ..crate::BuildSimulationTimings::default()
    };
    Ok((lowered, timings))
}

/// Warn, before any integration, about every ES016 fact of the Solve model
/// (SPEC_0044 ME-EVENT-008): a block whose own unknowns a relation under
/// `noEvent` switches fails with a typed fold error where its branch ends.
pub(super) fn report_unlocalizable_guards(guards: &[solve::UnlocalizableGuard]) {
    for guard in guards {
        eprintln!("warning[ES016]: {}", guard.warning());
    }
}

fn fmi_metadata_error(
    error: rumoca_phase_solve::fmi::FmiLoweringError,
) -> SimulationDiagnosticError {
    SimulationDiagnosticError::RuntimePreparation {
        message: error.to_string(),
        span: error.span(),
    }
}

pub(super) fn model_lowering_error(
    error: rumoca_phase_solve::SolveModelLoweringError,
) -> SimulationDiagnosticError {
    match error {
        rumoca_phase_solve::SolveModelLoweringError::Lower(error) => {
            SimulationDiagnosticError::SolveLowering(error)
        }
        rumoca_phase_solve::SolveModelLoweringError::RuntimeValues { message, span } => {
            SimulationDiagnosticError::RuntimePreparation { message, span }
        }
        rumoca_phase_solve::SolveModelLoweringError::InvalidOverride { message } => {
            SimulationDiagnosticError::InvalidOverride { message }
        }
    }
}

//! Native execution of checked public RBC programs on the existing ME/RK45 host.
use rumoca_ir_solve as solve;
#[cfg(feature = "solver-rk45")]
use rumoca_solver::fmi_me::{MeModelArtifact, PublicationObserver, session::MeRetainedComponent};
use solve::execution as ir;
#[cfg(feature = "solver-rk45")]
use std::path::Path;

pub fn lower(
    model: &rumoca_ir_solve::SolveModel,
    observed: &[(u32, u32, String)],
) -> Result<ir::NumericalProgram, String> {
    rumoca_phase_solve::execution::export(model, observed)
}

/// Check an artifact against the program derived for it.
///
/// The derived program is a parameter, not a field: v2 does not serialize one
/// (D1), so the caller derives it and hands it in.
pub fn check(
    artifact: &ir::ExecutionArtifact,
    numerical: &ir::NumericalProgram,
) -> Result<(), String> {
    ir::validate(artifact)?;
    rumoca_phase_solve::execution::reconstruct(numerical)?;
    Ok(())
}

/// Executes the authored host program against a derived numerical program.
#[cfg(feature = "solver-rk45")]
pub fn run(
    artifact: &ir::ExecutionArtifact,
    numerical: &ir::NumericalProgram,
    options: &crate::SimOptions,
    root: &Path,
) -> Result<crate::SimResult, String> {
    check(artifact, numerical)?;
    let component = rumoca_phase_solve::execution::reconstruct(numerical)?;
    let observer = NativePublication(rumoca_eval_solve::execution::CsvExecution::start(
        artifact.clone(),
        numerical.clone(),
        root,
    )?);
    let artifact = MeModelArtifact::new(component);
    let options_host = crate::me_backend::batch_options(&artifact.root_location(), options)
        .map_err(|e| e.to_string())?;
    let mut cursor = rumoca_solver::fmi_me::driver::batch_output_cursor(&options_host)
        .map_err(|e| e.to_string())?;
    let retained = MeRetainedComponent::instantiate(
        artifact.source(),
        &crate::me_backend::instance_config("rbc-execution", options).map_err(|e| e.to_string())?,
        None,
    )
    .map_err(|e| e.to_string())?;
    let mut host = retained
        .into_lease_with_observer(options_host, Some(Box::new(observer)))
        .map_err(|e| e.to_string())?;
    if host.is_terminated() {
        host.finish_publication().map_err(|e| e.to_string())?;
        return Ok(host.finish());
    }
    let plugin = crate::me_backend::plugin_for_host(
        &host,
        options,
        crate::me_backend::IntegratorFactory {
            method: "rk45",
            build: rumoca_solver_rk45::model_exchange_integrator,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut session = host.into_session(plugin).map_err(|e| e.to_string())?;
    session
        .run_to_stop(&mut cursor)
        .map_err(|e| e.to_string())?;
    session.finish_publication().map_err(|e| e.to_string())?;
    Ok(session.finish())
}

/// Diagnostic mode executes the same edited instructions through the reference
/// interpreter. Failure evidence is written even when no physical sample exists.
#[cfg(feature = "solver-rk45")]
pub fn run_with_domain_diagnostics(
    artifact: &ir::ExecutionArtifact,
    numerical: &ir::NumericalProgram,
    options: &crate::SimOptions,
    root: &Path,
) -> Result<crate::SimResult, String> {
    if root.exists()
        || artifact
            .program
            .sinks
            .iter()
            .any(|sink| sink.filename == "domain-diagnostics.json")
    {
        return Err(
            "domain diagnostics require a new directory and unreserved sink filename".into(),
        );
    }
    let scope = rumoca_eval_solve::domain_diagnostics::Scope::start()?;
    let solver_scope = rumoca_solver::diagnostics::Scope::start()?;
    let mut options = options.clone();
    options.execution_policy = rumoca_solver::SimExecutionPolicy::Interpreter;
    let result = run(artifact, numerical, &options, root);
    let mut evidence = scope.finish();
    evidence["solver"] = solver_scope.finish();
    // An invalid destination/program may fail before a trace directory exists.
    // Do not create or reuse it behind the normal exclusive resource boundary.
    if root.is_dir() {
        let output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("domain-diagnostics.json"))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(output, &evidence).map_err(|e| e.to_string())?;
    }
    result
}

/// Same domain-evidence boundary for ordinary equation simulations. Unsupported
/// tensor/call/assignment-prefix evaluations remain counted as coverage gaps.
#[cfg(feature = "solver-rk45")]
pub fn simulate_equations_with_domain_diagnostics(
    dae: &rumoca_ir_dae::Dae,
    options: &crate::SimOptions,
    destination: &Path,
) -> Result<crate::SimResult, String> {
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| e.to_string())?;
    let scope = rumoca_eval_solve::domain_diagnostics::Scope::start()?;
    let solver_scope = rumoca_solver::diagnostics::Scope::start()?;
    let mut options = options.clone();
    options.execution_policy = rumoca_solver::SimExecutionPolicy::Interpreter;
    let result = crate::simulate_with_diagnostics(dae, &options).map_err(|e| e.to_string());
    let mut evidence = scope.finish();
    evidence["solver"] = solver_scope.finish();
    serde_json::to_writer_pretty(output, &evidence).map_err(|e| e.to_string())?;
    result
}

#[cfg(feature = "solver-rk45")]
struct NativePublication(rumoca_eval_solve::execution::CsvExecution);
#[cfg(feature = "solver-rk45")]
impl PublicationObserver for NativePublication {
    fn publish(
        &mut self,
        time: f64,
        phase: &str,
        names: &[String],
        values: &[f64],
    ) -> Result<(), String> {
        self.0.publish(time, phase, names, values)
    }
    fn finish(&mut self) -> Result<(), String> {
        self.0.finish()
    }
}

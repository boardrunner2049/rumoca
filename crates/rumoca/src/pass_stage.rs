//! The in-compiler bitcode pass stage: `DAE -> RBC -> [passes] -> DAE`.
//!
//! Runs unless `--pass none` / `--pass O0` skips it. The
//! DAE the frontend produced is exported, rewritten by
//! [`rumoca_bitcode::passes`], and rebuilt through the DAE's checked
//! constructors, so the rest of the compiler -- solve, simulation, code
//! generation -- consumes the rewritten model exactly as it would the
//! original. See docs/design/minimal-frontend.md.

use std::sync::Arc;

use rumoca_compile::compile::{Dae, FlatModel};

use crate::CompilerError;

/// Where the stage stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Export,
    Pass,
    Rebuild,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Export => "export",
            Self::Pass => "pass",
            Self::Rebuild => "rebuild",
        }
    }
}

/// Rewrite `dae` through the named passes; return the rebuilt model and the
/// artifact it was rebuilt from.
pub(crate) fn apply(
    dae: &Dae,
    flat: &FlatModel,
    model_name: &str,
    passes: &[String],
    verbose: bool,
) -> Result<(Arc<Dae>, rumoca_bitcode::RbcFile), CompilerError> {
    run(dae, flat, model_name, passes, verbose).map_err(|(stage, error)| failed(stage, error))
}

/// [`apply`] for the group a plain compile runs without being asked.
///
/// A model the bitcode profile cannot carry -- export or rebuild refuses it,
/// as for a recursive function body, whose artifact body stays elided --
/// keeps the frontend's DAE, with a warning, so ordinary native compilation
/// is not restricted to the profile. A pass that fails on a model the profile
/// does carry is still an error: that is a defect, not a limit.
pub(crate) fn apply_implicit(
    dae: &Dae,
    flat: &FlatModel,
    model_name: &str,
    passes: &[String],
    verbose: bool,
) -> Result<Option<(Arc<Dae>, rumoca_bitcode::RbcFile)>, CompilerError> {
    match run(dae, flat, model_name, passes, verbose) {
        Ok(rewritten) => Ok(Some(rewritten)),
        Err((Stage::Pass, error)) => Err(failed(Stage::Pass, error)),
        Err((stage, error)) => {
            eprintln!(
                "[rumoca] warning: default passes skipped for {model_name}: the model is outside \
                 the bitcode profile ({}: {error}); compiled without them",
                stage.name()
            );
            Ok(None)
        }
    }
}

fn failed(stage: Stage, error: String) -> CompilerError {
    CompilerError::BitcodePassError(format!("{}: {error}", stage.name()))
}

fn run(
    dae: &Dae,
    flat: &FlatModel,
    model_name: &str,
    passes: &[String],
    verbose: bool,
) -> Result<(Arc<Dae>, rumoca_bitcode::RbcFile), (Stage, String)> {
    let mut file = rumoca_bitcode::export(
        dae,
        Some(flat),
        model_name,
        &rumoca_bitcode::ExportOptions::default(),
    )
    .map_err(|error| (Stage::Export, error.to_string()))?;
    let reports = rumoca_bitcode::passes::run(&mut file, passes)
        .map_err(|error| (Stage::Pass, error.to_string()))?;
    if verbose {
        for report in &reports {
            eprintln!(
                "[rumoca] pass {}: {} rewrite(s), {} expression(s) dropped",
                report.pass, report.rewrites, report.removed_expressions
            );
        }
    }
    let rebuilt =
        rumoca_bitcode::import(&file).map_err(|error| (Stage::Rebuild, error.to_string()))?;
    Ok((Arc::new(rebuilt), file))
}

//! Execution view of the retained state-manifold projection.
//!
//! Index reduction keeps the lower-order constraints it differentiated away
//! (`ContinuousSolveSystem::manifold_residual`), and the linked ME kernel
//! projects accepted states back onto them (`rumoca-solver`
//! `runtime::projection::manifold`). The generated component runs the same
//! damped minimum-norm Newton projection; this module reads the checked
//! residual, its forward directional derivative, and the block plan once and
//! records them for the C kernel (`templates/fmi3/me_projection.jinja`,
//! `manifold_kernel`).

use std::sync::Arc;

use minijinja::Value;
use rumoca_eval_solve::projection_policy as policy;
use rumoca_ir_solve as solve;
use serde::Serialize;

use super::super::scalar_program_plan::ScalarProgramPlan;
use crate::errors::CodegenError;

/// One constraint block: pool offsets of its residual rows and of the state
/// coordinates it corrects.
#[derive(Serialize)]
struct ManifoldBlockRecord {
    nrows: usize,
    ncols: usize,
    rows: usize,
    y: usize,
}

fn refuse(reason: &str) -> CodegenError {
    CodegenError::dae_preparation_failed(
        format!("unsupported-feature:algebraic_projection: the state-manifold projection {reason}"),
        None,
    )
}

/// Refuse a directional program that reads a seed the component does not
/// hold: one lane over the solver-Y and parameter coordinates.
fn check_seeds(operations: &[solve::LinearOp], seed_len: usize) -> Result<(), CodegenError> {
    for operation in operations {
        let outside = match operation {
            solve::LinearOp::LoadSeed { index, .. } => *index >= seed_len,
            solve::LinearOp::TensorLoad {
                seed_start: Some(start),
                count,
                ..
            } => start.checked_add(*count).is_none_or(|end| end > seed_len),
            solve::LinearOp::LoadIndexedSeed { .. } => true,
            _ => false,
        };
        if outside {
            return Err(refuse(
                "reads a seed outside the solver-Y and parameter lanes",
            ));
        }
    }
    Ok(())
}

/// The blocks of the plan, checked against the residual and the state prefix
/// exactly as the linked kernel validates them before projecting.
fn block_records(
    plan: &solve::AlgebraicProjectionPlan,
    residual_len: usize,
    state_count: usize,
    pool: &mut Vec<usize>,
) -> Result<(Vec<ManifoldBlockRecord>, usize), CodegenError> {
    let mut rows_seen = vec![false; residual_len];
    let mut states_seen = vec![false; state_count];
    let mut widest = 1;
    let mut records = Vec::with_capacity(plan.blocks.len());
    for block in &plan.blocks {
        let (nrows, ncols) = (block.rows.len(), block.y_indices.len());
        if nrows == 0 || nrows > ncols {
            return Err(refuse("has a block with more rows than state coordinates"));
        }
        for &row in &block.rows {
            if rows_seen
                .get_mut(row)
                .is_none_or(|seen| std::mem::replace(seen, true))
            {
                return Err(refuse("repeats or misses a residual row"));
            }
        }
        for &state in &block.y_indices {
            if states_seen
                .get_mut(state)
                .is_none_or(|seen| std::mem::replace(seen, true))
            {
                return Err(refuse("corrects a coordinate outside the states"));
            }
        }
        widest = widest.max(nrows * ncols);
        let rows = pool.len();
        pool.extend(&block.rows);
        let y = pool.len();
        pool.extend(&block.y_indices);
        records.push(ManifoldBlockRecord {
            nrows,
            ncols,
            rows,
            y,
        });
    }
    if rows_seen.iter().any(|seen| !seen) {
        return Err(refuse("leaves a retained residual row outside every block"));
    }
    Ok((records, widest))
}

/// No manifold view: index reduction retained no constraint.
fn no_retained_manifold() -> Result<Option<Value>, CodegenError> {
    Ok(Option::None)
}

/// The manifold view, or `None` when index reduction retained no constraint.
fn manifold_value(
    problem: &solve::SolveProblem,
    artifacts: &solve::SolveArtifacts,
    seed_len: usize,
) -> Result<Option<Value>, CodegenError> {
    let continuous = &problem.continuous;
    let plan = &continuous.manifold_projection_plan;
    if plan.is_empty() {
        return no_retained_manifold();
    }
    let residual = rumoca_eval_solve::to_scalar_program_block(&continuous.manifold_residual)?;
    let directional =
        rumoca_eval_solve::to_scalar_program_block(&artifacts.continuous.manifold_jacobian_v)?;
    let residual_len = residual.output_count();
    if directional.output_count() != residual_len {
        return Err(refuse(
            "has a directional derivative of a different extent than its residual",
        ));
    }
    for program in directional.programs() {
        check_seeds(program, seed_len)?;
    }
    let state_count = problem.solve_layout.state_scalar_count();
    let mut pool = Vec::new();
    let (blocks, widest) = block_records(plan, residual_len, state_count, &mut pool)?;
    let widest_cols = blocks.iter().map(|block| block.ncols).max().unwrap_or(1);
    // One block frame: the full residual, the directional output, the block
    // residual, Jacobian, row and variable scales, the Newton correction and
    // the snapshot, then the dense or SVD solve (two square frames of the
    // widest block plus its right-hand side and solution).
    let doubles = 2 * residual_len + 3 * widest + 6 * widest_cols * widest_cols + 8 * widest_cols;
    Ok(Some(minijinja::context! {
        rows => residual_len,
        residual => Value::from_object(ScalarProgramPlan::new(Arc::new(residual))?),
        jvp => Value::from_object(ScalarProgramPlan::new(Arc::new(directional))?),
        blocks => Value::from_serialize(&blocks),
        pool => pool,
        max_iters => policy::MANIFOLD_PROJECTION_MAX_ITERS,
        doubles => doubles,
        sizes => 2 * widest_cols,
    }))
}

/// The manifold view with the arena its frame needs, (doubles, sizes), zero
/// without one.
pub(super) fn manifold_view(
    problem: &solve::SolveProblem,
    artifacts: &solve::SolveArtifacts,
    seed_len: usize,
) -> Result<(Option<Value>, (usize, usize)), CodegenError> {
    let Some(view) = manifold_value(problem, artifacts, seed_len)? else {
        return Ok((None, (0, 0)));
    };
    let extent = |name: &str| -> Result<usize, CodegenError> {
        Ok(view.get_attr(name)?.as_usize().unwrap_or(0))
    };
    let work = (extent("doubles")?, extent("sizes")?);
    Ok((Some(view), work))
}

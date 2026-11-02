//! Split the continuous prefix off an event algorithm (MLS §11.1.2).
//!
//! ThermoSysPro's `ConvAD` writes
//!
//! ```modelica
//! algorithm
//!   qInterval := (maxval - minval)/2^bits;
//!   when sample(SampleOffset, SampleInterval) then
//!     uBound := ...;
//!     y.signal := qInterval*floor(abs(uBound/qInterval) + 0.5)*sign(uBound);
//!   end when;
//! ```
//!
//! An algorithm owner is either declarative (continuous targets) or an event
//! transaction (discrete targets), so the mixed section was refused. Its
//! leading top-level assignments are, however, independent of the rest: when
//! each writes a continuous coordinate that nothing else in the section
//! writes, and reads none of the section's other targets, it computes the same
//! value whether it runs as the first statement of the section or as its own
//! algorithm, and every later read of it in the section sees exactly that
//! value. Such a prefix is moved into a separate algorithm before analysis;
//! the remainder keeps its event meaning.

use super::*;
use std::borrow::Cow;

pub(super) fn split_continuous_prefixes(flat: &flat::Model) -> Cow<'_, flat::Model> {
    let splits = flat
        .algorithms
        .iter()
        .map(|algorithm| continuous_prefix_len(flat, algorithm))
        .collect::<Vec<_>>();
    if splits.iter().all(|split| *split == 0) {
        return Cow::Borrowed(flat);
    }
    let mut normalized = flat.clone();
    normalized.algorithms.clear();
    for (algorithm, split) in flat.algorithms.iter().zip(splits) {
        if split == 0 {
            normalized.algorithms.push(algorithm.clone());
            continue;
        }
        let (prefix, rest) = algorithm.statements.split_at(split);
        for statements in [prefix, rest] {
            let mut part = algorithm.clone();
            part.statements = statements.to_vec();
            normalized.algorithms.push(part);
        }
    }
    Cow::Owned(normalized)
}

/// Number of leading statements that form an independent continuous prefix of
/// an algorithm that also contains a `when` statement (0 when there is none).
fn continuous_prefix_len(flat: &flat::Model, algorithm: &flat::Algorithm) -> usize {
    let statements = &algorithm.statements;
    let Some(first_when) = statements
        .iter()
        .position(|statement| matches!(statement, rumoca_core::Statement::When { .. }))
    else {
        return 0;
    };
    let mut assignments = Vec::new();
    for statement in &statements[..first_when] {
        let rumoca_core::Statement::Assignment { comp, value, .. } = statement else {
            break;
        };
        if comp.parts().is_empty()
            || !rumoca_core::component_ref_element_subscripts(comp).is_empty()
        {
            break;
        }
        assignments.push((comp, value));
    }
    let prefix = assignments.len();
    if prefix == 0 || prefix == statements.len() {
        return 0;
    }
    let mut rest_targets = HashSet::new();
    collect_written(&statements[prefix..], &mut rest_targets);
    let mut prefix_targets = HashSet::new();
    for (comp, value) in assignments {
        let target = rumoca_core::component_ref_to_base_reference(comp)
            .var_name()
            .clone();
        let continuous = flat.variables.get(&target).is_some_and(|variable| {
            variable.dims.is_empty()
                && matches!(
                    variable.variability,
                    Variability::Continuous(_) | Variability::Empty
                )
        });
        let mut reads = Vec::new();
        value.collect_var_refs(&mut reads);
        if !continuous
            || rest_targets.contains(&target)
            || !prefix_targets.insert(target)
            || reads.iter().any(|read| rest_targets.contains(read))
        {
            return 0;
        }
    }
    prefix
}

fn collect_written(statements: &[rumoca_core::Statement], written: &mut HashSet<VarName>) {
    for statement in statements {
        match statement {
            rumoca_core::Statement::Assignment { comp, .. } => {
                written.insert(
                    rumoca_core::component_ref_to_base_reference(comp)
                        .var_name()
                        .clone(),
                );
            }
            rumoca_core::Statement::FunctionCall { outputs, .. } => {
                written.extend(outputs.iter().flatten().map(|output| output.to_var_name()));
            }
            rumoca_core::Statement::For { equations, .. } => collect_written(equations, written),
            rumoca_core::Statement::While { block, .. } => collect_written(&block.stmts, written),
            rumoca_core::Statement::If {
                cond_blocks,
                else_block,
                ..
            } => {
                for block in cond_blocks {
                    collect_written(&block.stmts, written);
                }
                if let Some(statements) = else_block {
                    collect_written(statements, written);
                }
            }
            rumoca_core::Statement::When { blocks, .. } => {
                for block in blocks {
                    collect_written(&block.stmts, written);
                }
            }
            _ => {}
        }
    }
}

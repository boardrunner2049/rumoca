//! Cyclic parameter bindings of the instantiated model (MLS §8.6).
//!
//! Resolve checks a class's own *final* bindings for cycles, because a
//! non-final binding is only a default that any modification replaces
//! (IBPSA `PlugFlowPipeDiscretized` declares `totLen = sum(segLen)` and
//! `segLen = fill(totLen/nSeg, nSeg)` and every use sets one of them). Whether
//! such a cycle survives is a property of the instance, so it is proven here,
//! over the bindings the Flat model actually carries.
use super::*;
use rumoca_core::DefId;

/// Reject a cycle among the parameter and constant bindings the translation
/// time evaluation could not settle.
///
/// Only bindings left unevaluated can be cyclic: an evaluated binding has a
/// value, so every binding it reads was settled first. Edges are exact Flat
/// coordinate names, so a binding that reads one element of an array through
/// a subscript still forms an edge to the whole array, as MLS §8.6 orders
/// whole variables. A read whose resolved declaration differs from the
/// coordinate's own declaration names something else under the same spelling
/// and forms no edge.
pub(super) fn reject_cyclic_parameter_bindings(
    flat: &flat::Model,
    context: &EvalContext,
) -> Result<(), ToDaeError> {
    // Declaration order keeps the reported cycle deterministic.
    let pending: Vec<(&VarName, &Expression, Span)> = flat
        .variables
        .iter()
        .filter(|(_, variable)| {
            matches!(
                variable.variability,
                Variability::Constant(_) | Variability::Parameter(_)
            ) && variable.fixed_uniform() != Some(false)
                && context.instance_value(variable.instance_id).is_none()
        })
        .filter_map(|(name, variable)| {
            let binding = variable.binding.as_ref()?;
            Some((name, binding, variable.source_span))
        })
        .collect();
    let index: HashMap<&VarName, (usize, Option<DefId>)> = pending
        .iter()
        .enumerate()
        .map(|(ordinal, (name, _, _))| {
            let declaration = flat
                .variables
                .get(*name)
                .and_then(|variable| variable.component_ref.as_ref())
                .map(rumoca_core::ComponentReference::target_def_id);
            (*name, (ordinal, declaration))
        })
        .collect();
    let edges: Vec<Vec<usize>> = pending
        .iter()
        .map(|(_, binding, _)| {
            let mut reads = Vec::new();
            collect_pending_reads(binding, &index, &mut reads, flat, context);
            reads
        })
        .collect();
    let mut finished = vec![false; pending.len()];
    for start in 0..pending.len() {
        let mut path = Vec::new();
        if let Some(cycle) = find_cycle(start, &edges, &mut finished, &mut path) {
            let names: Vec<&str> = cycle.iter().map(|node| pending[*node].0.as_str()).collect();
            return Err(ToDaeError::cyclic_parameter_binding(
                names.join(" -> "),
                pending[start].2,
            ));
        }
    }
    Ok(())
}

fn collect_pending_reads(
    expression: &Expression,
    index: &HashMap<&VarName, (usize, Option<DefId>)>,
    reads: &mut Vec<usize>,
    flat: &flat::Model,
    context: &EvalContext,
) {
    if let Expression::If {
        branches,
        else_branch,
        ..
    } = expression
    {
        let evaluable = evaluable_parameters(flat);
        let mut decided = true;
        for (condition, value) in branches {
            let mut names = Vec::new();
            condition.collect_var_refs(&mut names);
            if !names.iter().all(|name| evaluable.contains(name)) {
                decided = false;
                break;
            }
            let Ok(EvalValue::Bool(selected)) = eval_expr(condition, context) else {
                decided = false;
                break;
            };
            if selected {
                collect_pending_reads(value, index, reads, flat, context);
                return;
            }
        }
        if decided {
            collect_pending_reads(else_branch, index, reads, flat, context);
            return;
        }
    }
    if let Expression::VarRef { name, .. } = expression
        && let Some((ordinal, declaration)) = index.get(name.var_name())
        && same_declaration(name.target_def_id(), *declaration)
        && !reads.contains(ordinal)
    {
        reads.push(*ordinal);
    }
    for child in expression_children(expression) {
        collect_pending_reads(child, index, reads, flat, context);
    }
}

fn same_declaration(read: Option<DefId>, declared: Option<DefId>) -> bool {
    match (read, declared) {
        (Some(read), Some(declared)) => read == declared,
        _ => true,
    }
}

/// Depth-first search returning the cycle `a, b, a` reachable from `node`.
fn find_cycle(
    node: usize,
    edges: &[Vec<usize>],
    finished: &mut [bool],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    if finished.get(node).copied().unwrap_or(true) {
        return None;
    }
    if let Some(start) = path.iter().position(|entry| *entry == node) {
        let mut cycle = path[start..].to_vec();
        cycle.push(node);
        return Some(cycle);
    }
    path.push(node);
    for next in edges.get(node).into_iter().flatten() {
        if let Some(cycle) = find_cycle(*next, edges, finished, path) {
            return Some(cycle);
        }
    }
    path.pop();
    if let Some(done) = finished.get_mut(node) {
        *done = true;
    }
    None
}

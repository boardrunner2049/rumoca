//! `prune-functions`: drop functions no equation can reach.

use super::*;

/// The call graph walked from the model's own expressions; everything the
/// walk does not reach is removed, bodies and all.
///
/// The frontend does this during lowering (`prune_unreachable_functions`),
/// which is why it has to decide reachability before any call is folded:
/// folding first makes a pure call vanish and its callee then looks
/// unreachable although the source calls it (TOOLBUG-029). As a pass it
/// runs on the artifact the frontend produced, where that ordering is
/// simply the order of the pipeline.
pub(super) struct PruneFunctions;

impl Pass for PruneFunctions {
    fn name(&self) -> &'static str {
        "prune-functions"
    }
    fn description(&self) -> &'static str {
        "drop functions no equation, attribute or event can reach"
    }
    fn run(&self, model: &mut RbcModel) -> Result<usize, PassError> {
        let reachable = reachable_functions(model)?;
        let removed = reachable.iter().filter(|kept| !**kept).count();
        if removed == 0 {
            return Ok(0);
        }
        // A removed function's body is the only thing referring to its
        // expressions, so set the bodies aside before deciding which
        // expressions stay.
        for (function, kept) in model.functions.iter_mut().zip(&reachable) {
            if !kept {
                function.body = RbcFunctionBody::ElidedModelica;
                function.folds.clear();
                function.calls.clear();
                function.derivatives.clear();
            }
        }
        let live = live_expressions(model)?;
        crate::link::renumber(model, &compaction(&live), &compaction(&reachable))?;
        Ok(removed)
    }
}

fn reachable_functions(model: &RbcModel) -> Result<Vec<bool>, PassError> {
    // Roots are what the model uses outside any function body.
    let mut outside = model.clone();
    for function in &mut outside.functions {
        function.body = RbcFunctionBody::ElidedModelica;
        function.folds.clear();
    }
    let live = live_expressions(&outside)?;
    let mut reachable = vec![false; model.functions.len()];
    let mut pending: Vec<usize> = model
        .expressions
        .iter()
        .zip(&live)
        .filter(|(_, live)| **live)
        .filter_map(|(expression, _)| match expression.node {
            RbcExprNode::Call { function, .. } => Some(function.0 as usize),
            _ => None,
        })
        .collect();
    while let Some(function) = pending.pop() {
        let Some(slot) = reachable.get_mut(function) else {
            continue;
        };
        if std::mem::replace(slot, true) {
            continue;
        }
        for derivative in &model.functions[function].derivatives {
            pending.push(derivative.target.0 as usize);
            if let Some((source, _)) = derivative.previous {
                pending.push(source.0 as usize);
            }
        }
        pending.extend(
            model.functions[function]
                .calls
                .iter()
                .map(|callee| callee.0 as usize),
        );
    }
    Ok(reachable)
}

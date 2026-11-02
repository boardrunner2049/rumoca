//! `select-family-branches`: an `if` inside an equation family whose
//! conditions the parameter values decide becomes the branch it takes.
//!
//! DAE construction used to make this choice itself, lowering only the
//! selected arm of a structured body's top-level `if`. It is an optimization
//! -- the family is the same set of equations either way at the declared
//! values -- so it is a pass (docs/design/minimal-frontend.md), deciding with
//! the environment construction decided with: every parameter and constant
//! whose binding settles, except a `fixed = false` parameter, whose value
//! only initialization determines.
//!
//! Scope is the bodies of equation families, initial ones included, and
//! anything they reach. A materialized family packs its per-point bodies into
//! an array, so each point's `if` is reached through it. An `if` with a
//! condition the evaluator cannot settle is left whole.

use std::collections::{HashMap, HashSet};

use super::evaluate::{self, Value};
use super::parameter_bindings::{Tunable, Values};
use super::*;

pub(super) struct SelectFamilyBranches;

impl Pass for SelectFamilyBranches {
    fn name(&self) -> &'static str {
        "select-family-branches"
    }
    fn description(&self) -> &'static str {
        "replace an `if` in an equation family by the branch parameter values select"
    }
    fn run(&self, model: &mut RbcModel) -> Result<usize, PassError> {
        let values = Values::settle(model, Tunable::Included);
        let mut pending: Vec<ExprId> = model
            .equation_families
            .iter()
            .chain(&model.initial_equation_families)
            .flat_map(|family| family.bodies.iter().copied())
            .collect();
        let mut seen = HashSet::new();
        let mut selected = 0;
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let index = id.0 as usize;
            if let Some(branch) = taken_branch(model, index, &values)
                && let Some(node) =
                    copied_node(&model.expressions[branch.0 as usize].node, branch, id)
            {
                model.expressions[index].node = node;
                selected += 1;
            }
            pending.extend(crate::build::references(id, &model.expressions[index].node));
        }
        Ok(selected)
    }
}

/// The taken branch's node as it reads from the conditional's slot `at`.
///
/// Operands are shared, so most nodes copy as they are. A call result owns
/// its evaluation: a single-output call (its own `owner`) becomes the owner
/// of the copy, one more evaluation of the same pure call. A projection of a
/// multi-output call names the call's first projection as owner and cannot
/// be detached from it, so that branch is not selected.
fn copied_node(node: &RbcExprNode, branch: ExprId, at: ExprId) -> Option<RbcExprNode> {
    let mut node = node.clone();
    if let RbcExprNode::Call { owner, .. } = &mut node {
        if *owner != branch {
            return None;
        }
        *owner = at;
    }
    Some(node)
}

/// The branch a conditional takes under `values`, when every condition up
/// to it settles and it has the conditional's own type.
fn taken_branch(
    model: &RbcModel,
    index: usize,
    values: &HashMap<VariableId, Value>,
) -> Option<ExprId> {
    let expression = model.expressions.get(index)?;
    let RbcExprNode::Conditional { branches, fallback } = &expression.node else {
        return None;
    };
    let mut taken = *fallback;
    for branch in branches {
        match evaluate::expression_with_parameters(model, branch.condition, values)? {
            Value::Boolean(true) => {
                taken = branch.value;
                break;
            }
            Value::Boolean(false) => {}
            _ => return None,
        }
    }
    (model.expressions.get(taken.0 as usize)?.value_type == expression.value_type).then_some(taken)
}

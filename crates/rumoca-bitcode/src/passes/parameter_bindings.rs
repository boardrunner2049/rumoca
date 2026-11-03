//! `fold-parameter-bindings`: a derived parameter's declaration binding
//! becomes the value it evaluates to.
//!
//! `parameter Real d = k * 10` with `k = 2` keeps `d = k * 10` through
//! lowering: the dependency on `k` is a fact about the model, and an
//! analysis that asks which knob reaches a denominator needs it. This pass
//! is the optimization the frontend used to apply during instantiation
//! (`fold_parameter_declaration_bindings`): it writes `d = 20.0`, the value
//! a solver wants, when optimization is asked for.
//!
//! Scope is declaration bindings of parameters and constants, not bindings a
//! modifier supplied, and never a `fixed = false` parameter, whose binding is
//! not its value. A binding is folded only when the evaluator settles it from
//! literals, constants and *non-tunable* parameters (structural, `Evaluate`,
//! or themselves derived from such): a tunable parameter can be changed on
//! the artifact without recompiling, and a binding that reads one must keep
//! reading it -- galec's `Recalibrate` recomputes it from the new value.
//! Anything the evaluator does not model leaves the binding as written. Only scalars are written back: the artifact has no
//! array literal, and an `Array` of element literals is not cheaper than the
//! binding it would replace.

use std::collections::HashMap;

use super::evaluate::{self, Value};
use super::pure_calls::literal;
use super::*;

pub(super) struct FoldParameterBindings;

impl Pass for FoldParameterBindings {
    fn name(&self) -> &'static str {
        "fold-parameter-bindings"
    }
    fn description(&self) -> &'static str {
        "replace a parameter's declaration binding by the scalar it evaluates to"
    }
    fn run(&self, model: &mut RbcModel) -> Result<usize, PassError> {
        let values = Values::settle(model);
        let mut folded = 0;
        for index in 0..model.variables.len() {
            let variable = &model.variables[index];
            if !foldable(variable) {
                continue;
            }
            let Some(binding) = variable.binding else {
                continue;
            };
            let expression = &model.expressions[binding.0 as usize];
            if matches!(expression.node, RbcExprNode::Literal { .. }) {
                continue;
            }
            // The variable's own tunability does not matter: overriding it
            // replaces the binding either way. What it reads does.
            let Some(value) = evaluate::expression_with_parameters(model, binding, &values) else {
                continue;
            };
            let Some(scalar) = model
                .types
                .get(expression.value_type.0 as usize)
                .filter(|ty| ty.dimensions.is_empty())
                .map(|ty| ty.scalar)
            else {
                continue;
            };
            let Some(value) = literal(scalar, &value) else {
                continue;
            };
            let id = ExprId(model.expressions.len() as u32);
            model.expressions.push(RbcExpr {
                id,
                value_type: expression.value_type,
                node: RbcExprNode::Literal { value },
                provenance: expression.provenance,
            });
            let variable = &mut model.variables[index];
            if variable.start == Some(binding) {
                variable.start = Some(id);
            }
            variable.binding = Some(id);
            if let Some(contract) = variable.contract.as_mut() {
                contract.binding_depends_on.clear();
            }
            folded += 1;
        }
        Ok(folded)
    }
}

/// A declaration binding of a parameter or constant that is its value.
fn foldable(variable: &RbcVariable) -> bool {
    matches!(variable.role, RbcRole::Parameter | RbcRole::Constant)
        && variable.fixed != Some(false)
        && variable
            .contract
            .as_ref()
            .is_none_or(|contract| !contract.binding_from_modification)
}

/// The value of every parameter and constant whose binding settles from
/// literals, constants and non-tunable parameters. A tunable parameter has
/// no value here: it may change on the artifact (MLS §4.5).
pub(super) struct Values<'m> {
    model: &'m RbcModel,
    settled: HashMap<VariableId, Value>,
    /// Variables being evaluated, so a cyclic binding stays unsettled.
    visiting: std::collections::HashSet<VariableId>,
    /// Variables known not to settle.
    unsettled: std::collections::HashSet<VariableId>,
}

impl<'m> Values<'m> {
    pub(super) fn settle(model: &'m RbcModel) -> HashMap<VariableId, Value> {
        let mut values = Values {
            model,
            settled: HashMap::new(),
            visiting: Default::default(),
            unsettled: Default::default(),
        };
        for variable in &model.variables {
            values.value(variable.id);
        }
        values.settled
    }

    fn value(&mut self, id: VariableId) {
        if self.settled.contains_key(&id)
            || self.unsettled.contains(&id)
            || !self.visiting.insert(id)
        {
            return;
        }
        let value = self.evaluate(id);
        self.visiting.remove(&id);
        match value {
            Some(value) => {
                self.settled.insert(id, value);
            }
            None => {
                self.unsettled.insert(id);
            }
        }
    }

    fn evaluate(&mut self, id: VariableId) -> Option<Value> {
        let variable = self.model.variables.get(id.0 as usize)?;
        if !matches!(variable.role, RbcRole::Parameter | RbcRole::Constant)
            || variable.fixed == Some(false)
            || (variable.role == RbcRole::Parameter && variable.tunable)
        {
            return None;
        }
        let binding = variable.binding?;
        for read in parameter_reads(self.model, binding) {
            self.value(read);
        }
        evaluate::expression_with_parameters(self.model, binding, &self.settled)
    }
}

/// Parameters an expression reads, operands closed.
fn parameter_reads(model: &RbcModel, root: ExprId) -> Vec<VariableId> {
    let mut seen = std::collections::HashSet::new();
    let mut pending = vec![root];
    let mut reads = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(expression) = model.expressions.get(id.0 as usize) else {
            continue;
        };
        if let RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::Parameter { variable },
        } = expression.node
        {
            reads.push(variable);
        }
        pending.extend(crate::build::references(id, &expression.node));
    }
    reads
}

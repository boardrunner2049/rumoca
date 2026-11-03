//! Differentiability admissions beyond the elementary operation table, shared
//! by the first-order and the holonomic preflight: a call followed through a
//! straight-line body and the earlier definitions that body reads, the
//! `smooth`/`noEvent` annotations, and a power whose exponent varies in time.
//!
//! The differentiator follows exactly these paths, so each admission here
//! discharges an `unreachable!` there.

use rumoca_eval_dae::FunctionCallContext;
use rumoca_ir_dae as dae;

use super::super::builtin_profiles::is_differentiable_varying_power;
use super::super::parameter_conditionals::{has_smooth_guards, values};
use super::{
    DifferentiationFacts, HolonomicProofWalk, VisitMarks, is_differentiable_builtin,
    is_differentiable_in_context, operation_is_differentiable,
};

/// The first-order preflight of one node, after the selected-branch,
/// supplied-derivative, call-result, zero and time-invariance checks of
/// `is_differentiable_in_context`.
pub(super) fn is_differentiable_node<'dae>(
    view: dae::DaeView<'dae>,
    facts: &DifferentiationFacts,
    expression: dae::ExprId<'dae>,
    node: dae::ExpressionView<'dae>,
    demoted: dae::StateId<'dae>,
    visited: &mut VisitMarks,
    context: &FunctionCallContext<'dae>,
) -> bool {
    match node.operation() {
        dae::ExpressionOperation::Call { .. } => {
            let Some((result, nested)) = context.straight_line_call_result(view, expression) else {
                return false;
            };
            is_differentiable_in_context(view, facts, result, demoted, visited, &nested)
        }
        // An earlier definition of the same function body.
        dae::ExpressionOperation::FunctionValue { definition, .. } => {
            is_differentiable_in_context(view, facts, definition.rhs(), demoted, visited, context)
        }
        dae::ExpressionOperation::Binary { operator, lhs, rhs }
            if is_differentiable_varying_power(view, operator, lhs, rhs, 1) =>
        {
            is_differentiable_in_context(view, facts, lhs, demoted, visited, context)
                && is_differentiable_in_context(view, facts, rhs, demoted, visited, context)
        }
        operation => operation_is_differentiable(view, facts, operation, demoted, visited, context),
    }
}

/// Admit exactly the pure builtins whose first derivative has a closed DAE
/// representation. The checked DAE constructor already proves builtin arity
/// and tensor shapes; this walk proves only that each value operand can itself
/// be differentiated under the current function-call substitution.
/// `smooth(p, e)` and `noEvent(e)` annotate `e`, whose derivative they take.
pub(super) fn builtin_is_differentiable<'dae>(
    view: dae::DaeView<'dae>,
    facts: &DifferentiationFacts,
    builtin: dae::PureBuiltin,
    arguments: dae::ExpressionOperands<'dae>,
    demoted: dae::StateId<'dae>,
    visited: &mut VisitMarks,
    context: &FunctionCallContext<'dae>,
) -> bool {
    if is_annotation(builtin) {
        let Some(value) = annotated_value(arguments) else {
            return false;
        };
        if is_differentiable_in_context(view, facts, value, demoted, visited, context) {
            return true;
        }
        let Some(operands) = smooth_conditional(view, context, builtin, arguments, 1) else {
            return false;
        };
        return values(operands).all(|value| {
            is_differentiable_in_context(view, facts, value, demoted, visited, context)
        });
    }
    is_differentiable_builtin(builtin, 1)
        && arguments.iter().all(|argument| {
            is_differentiable_in_context(view, facts, argument, demoted, visited, context)
        })
}

impl<'dae> HolonomicProofWalk<'_, 'dae> {
    /// The holonomic preflight of one node, after the selected-branch,
    /// supplied-derivative, call-result, zero and time-invariance checks of
    /// `can_differentiate_order_scoped`.
    pub(super) fn can_differentiate_node(
        &mut self,
        expression_id: dae::ExprId<'dae>,
        expression: dae::ExpressionView<'dae>,
        order: u8,
        on_residual: bool,
    ) -> bool {
        match expression.operation() {
            dae::ExpressionOperation::Call { .. } => {
                let Some((result, nested)) = self
                    .function_context
                    .straight_line_call_result(self.view, expression_id)
                else {
                    return false;
                };
                let previous = std::mem::replace(&mut self.function_context, nested);
                let differentiable = self.can_differentiate_order(result, order, on_residual);
                self.function_context = previous;
                differentiable
            }
            // An earlier definition of the same function body.
            dae::ExpressionOperation::FunctionValue { definition, .. } => {
                self.can_differentiate_order(definition.rhs(), order, on_residual)
            }
            dae::ExpressionOperation::Builtin { builtin, arguments } if is_annotation(builtin) => {
                self.can_differentiate_annotated(builtin, arguments, order, on_residual)
            }
            dae::ExpressionOperation::Binary { operator, lhs, rhs }
                if is_differentiable_varying_power(self.view, operator, lhs, rhs, order) =>
            {
                self.can_differentiate_order(lhs, order, on_residual)
                    && self.can_differentiate_order(rhs, order, on_residual)
            }
            _ => self.can_differentiate_operation(expression, order, on_residual),
        }
    }

    fn can_differentiate_annotated(
        &mut self,
        builtin: dae::PureBuiltin,
        arguments: dae::ExpressionOperands<'dae>,
        order: u8,
        on_residual: bool,
    ) -> bool {
        let Some(value) = annotated_value(arguments) else {
            return false;
        };
        if self.can_differentiate_order(value, order, on_residual) {
            return true;
        }
        let Some(operands) =
            smooth_conditional(self.view, &self.function_context, builtin, arguments, order)
        else {
            return false;
        };
        values(operands).all(|value| self.can_differentiate_order(value, order, on_residual))
    }
}

/// `smooth(p, e)` and `noEvent(e)` (MLS §3.7.4) only annotate `e`.
fn is_annotation(builtin: dae::PureBuiltin) -> bool {
    matches!(
        builtin,
        dae::PureBuiltin::Smooth | dae::PureBuiltin::NoEvent
    )
}

/// The annotated expression: the last operand of `smooth` or `noEvent`.
fn annotated_value<'dae>(arguments: dae::ExpressionOperands<'dae>) -> Option<dae::ExprId<'dae>> {
    arguments.get(arguments.len().checked_sub(1)?)
}

/// The operands of a conditional that is the value of `smooth(p, if ...)`
/// when `p` is an Integer literal of at least `order` and every guard is one
/// `has_smooth_guards` admits under that declared continuity.
fn smooth_conditional<'dae>(
    view: dae::DaeView<'dae>,
    context: &FunctionCallContext<'dae>,
    builtin: dae::PureBuiltin,
    arguments: dae::ExpressionOperands<'dae>,
    order: u8,
) -> Option<dae::ExpressionOperands<'dae>> {
    if builtin != dae::PureBuiltin::Smooth {
        return None;
    }
    let continuity = match view.expression(arguments.get(0)?)?.operation() {
        dae::ExpressionOperation::Literal(dae::DaeLiteral::Integer(continuity)) => *continuity,
        _ => return None,
    };
    if continuity < i64::from(order) {
        return None;
    }
    let dae::ExpressionOperation::Conditional(operands) =
        view.expression(arguments.get(1)?)?.operation()
    else {
        return None;
    };
    has_smooth_guards(view, context, operands).then_some(operands)
}

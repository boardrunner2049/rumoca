//! Conditional guards stay fixed during continuous-time differentiation.
//!
//! MLS §3.6.5 selects one branch of `if c1 then e1 elseif ... else eN` at each
//! instant. A guard that is piecewise constant in time selects the same branch
//! on every interval between the instants where it changes, so on each such
//! interval `d/dt (if c then a else b) = if c then da/dt else db/dt`; the guard
//! is retained, never differentiated. Parameters and literals are constant; a
//! relation is piecewise constant because MLS §8.5 makes it change only at its
//! event, and a discrete or condition coordinate changes only at events; Boolean
//! operators preserve the property.
//!
//! A relation in a function body owns no event, so a body relation is admitted
//! only when, instantiated at the call site, it reads nothing but `time` and
//! time-invariant values: it then switches at instants fixed by `time` alone,
//! independent of the solution, and the selected branch's derivative is the
//! derivative of the body on each side. A body relation over a state or an
//! algebraic keeps the refusal of an eventless relation.

use rumoca_eval_dae::FunctionCallContext;
use rumoca_ir_dae as dae;

use super::equalities::is_time_invariant;

pub(super) fn is_guard(index: usize, length: usize) -> bool {
    index.is_multiple_of(2) && index + 1 < length
}

pub(super) fn values(
    operands: dae::ExpressionOperands<'_>,
) -> impl Iterator<Item = dae::ExprId<'_>> {
    operands
        .iter()
        .enumerate()
        .filter_map(move |(index, value)| (!is_guard(index, operands.len())).then_some(value))
}

pub(super) fn has_parameter_guards<'dae>(
    view: dae::DaeView<'dae>,
    context: &FunctionCallContext<'dae>,
    operands: dae::ExpressionOperands<'dae>,
) -> bool {
    operands.iter().enumerate().all(|(index, operand)| {
        !is_guard(index, operands.len()) || piecewise_constant_guard(view, context, operand)
    })
}

/// The guards of a conditional that is the value of `smooth(p, ...)` with `p`
/// at least the derivative order (MLS §3.7.4.2): besides a piecewise-constant
/// guard, a relation between `time` and a time-invariant value. That relation
/// switches exactly once, at a known instant, whether or not it owns the time
/// event `smooth` lets a tool omit, and the declared continuity makes the
/// selected branch's derivative the derivative on both sides of the switch.
pub(super) fn has_smooth_guards<'dae>(
    view: dae::DaeView<'dae>,
    context: &FunctionCallContext<'dae>,
    operands: dae::ExpressionOperands<'dae>,
) -> bool {
    operands.iter().enumerate().all(|(index, operand)| {
        !is_guard(index, operands.len())
            || piecewise_constant_guard(view, context, operand)
            || is_time_threshold(view, operand)
    })
}

fn is_time_threshold<'dae>(view: dae::DaeView<'dae>, expression: dae::ExprId<'dae>) -> bool {
    let Some(node) = view.expression(expression) else {
        return false;
    };
    let dae::ExpressionOperation::Binary {
        operator:
            dae::BinaryOperator::Less
            | dae::BinaryOperator::LessEqual
            | dae::BinaryOperator::Greater
            | dae::BinaryOperator::GreaterEqual,
        lhs,
        rhs,
    } = node.operation()
    else {
        return false;
    };
    let is_time = |operand: dae::ExprId<'dae>| {
        view.expression(operand).is_some_and(|operand| {
            matches!(
                operand.operation(),
                dae::ExpressionOperation::Coordinate(dae::CoordinateView::Time)
            )
        })
    };
    (is_time(lhs) && is_time_invariant(view, rhs)) || (is_time(rhs) && is_time_invariant(view, lhs))
}

fn piecewise_constant_guard<'dae>(
    view: dae::DaeView<'dae>,
    context: &FunctionCallContext<'dae>,
    expression: dae::ExprId<'dae>,
) -> bool {
    if invariant_guard(view, context, expression) {
        return true;
    }
    let context = context.scoped_to_expression(view, expression);
    let Some(node) = view.expression(expression) else {
        return false;
    };
    match node.operation() {
        dae::ExpressionOperation::Coordinate(
            dae::CoordinateView::DiscreteValue(_)
            | dae::CoordinateView::PreDiscreteValue(_)
            | dae::CoordinateView::Condition(_),
        ) => true,
        dae::ExpressionOperation::Unary {
            operator: dae::UnaryOperator::Not,
            operand,
        } => piecewise_constant_guard(view, &context, operand),
        dae::ExpressionOperation::Binary {
            operator: dae::BinaryOperator::And | dae::BinaryOperator::Or,
            lhs,
            rhs,
        } => {
            piecewise_constant_guard(view, &context, lhs)
                && piecewise_constant_guard(view, &context, rhs)
        }
        dae::ExpressionOperation::Binary {
            operator:
                dae::BinaryOperator::Equal
                | dae::BinaryOperator::NotEqual
                | dae::BinaryOperator::Less
                | dae::BinaryOperator::LessEqual
                | dae::BinaryOperator::Greater
                | dae::BinaryOperator::GreaterEqual,
            ..
        } => {
            if context.is_empty() {
                relation_owns_event(view, expression, node.provenance())
            } else {
                is_time_driven_instance(view, &context, expression)
            }
        }
        _ => false,
    }
}

/// Whether `rebuild_instantiated` rebuilds `expression` at the call site
/// (every function-body operation it reaches is one that rebuild
/// instantiates) and every whole-model operand it reaches is `is_time_driven`.
fn is_time_driven_instance<'dae>(
    view: dae::DaeView<'dae>,
    context: &FunctionCallContext<'dae>,
    expression: dae::ExprId<'dae>,
) -> bool {
    let context = context.scoped_to_expression(view, expression);
    if let Some(branch) = context.selected_branch(view, expression) {
        return is_time_driven_instance(view, &context, branch);
    }
    let Some(node) = view.expression(expression) else {
        return false;
    };
    if node.function_scope().is_none() && node.binder_domain().is_none() {
        return is_time_driven(view, expression);
    }
    let instantiable =
        |operand: dae::ExprId<'dae>| is_time_driven_instance(view, &context, operand);
    match node.operation() {
        dae::ExpressionOperation::Literal(_)
        | dae::ExpressionOperation::Coordinate(dae::CoordinateView::Binder(_)) => true,
        dae::ExpressionOperation::Coordinate(dae::CoordinateView::FunctionParameter(parameter)) => {
            context
                .parameter_argument(parameter)
                .is_some_and(instantiable)
        }
        dae::ExpressionOperation::Unary { operand, .. } => instantiable(operand),
        dae::ExpressionOperation::Binary { lhs, rhs, .. } => instantiable(lhs) && instantiable(rhs),
        dae::ExpressionOperation::Array(operands)
        | dae::ExpressionOperation::Record(operands)
        | dae::ExpressionOperation::Conditional(operands)
        | dae::ExpressionOperation::Builtin {
            arguments: operands,
            ..
        }
        | dae::ExpressionOperation::Call {
            arguments: operands,
            ..
        } => operands.iter().all(instantiable),
        dae::ExpressionOperation::Field { base, field } => {
            match context.projected_field(view, base, field) {
                Some((projected, nested)) => is_time_driven_instance(view, &nested, projected),
                None => instantiable(base),
            }
        }
        dae::ExpressionOperation::Index { base, subscripts } => {
            instantiable(base)
                && subscripts.iter().all(|subscript| match subscript {
                    dae::SubscriptView::Whole { .. } => true,
                    dae::SubscriptView::Index { expression, .. }
                    | dae::SubscriptView::Slice { expression, .. } => instantiable(expression),
                })
        }
        dae::ExpressionOperation::FunctionValue { definition, .. } => {
            instantiable(definition.rhs())
        }
        _ => false,
    }
}

/// A whole-model value built only from `time` and time-invariant values.
fn is_time_driven<'dae>(view: dae::DaeView<'dae>, expression: dae::ExprId<'dae>) -> bool {
    if is_time_invariant(view, expression) {
        return true;
    }
    let Some(node) = view.expression(expression) else {
        return false;
    };
    match node.operation() {
        dae::ExpressionOperation::Coordinate(dae::CoordinateView::Time) => true,
        dae::ExpressionOperation::Unary { operand, .. } => is_time_driven(view, operand),
        dae::ExpressionOperation::Binary { lhs, rhs, .. } => {
            is_time_driven(view, lhs) && is_time_driven(view, rhs)
        }
        dae::ExpressionOperation::Builtin { arguments, .. } => arguments
            .iter()
            .all(|argument| is_time_driven(view, argument)),
        _ => false,
    }
}

/// MLS §8.5 fixes a relation between its events only when the relation owns
/// one: a scalar or structured root, or a time event. A relation under
/// `noEvent`, or inside `smooth` without an owned root, owns none.
fn relation_owns_event<'dae>(
    view: dae::DaeView<'dae>,
    expression: dae::ExprId<'dae>,
    provenance: dae::DaeProvenance,
) -> bool {
    view.relations()
        .any(|(_, relation)| relation.expression() == expression)
        || view
            .structured_roots()
            .any(|(_, root)| root.expression() == expression)
        || view
            .time_events()
            .any(|(_, event)| event.provenance().span() == provenance.span())
}

fn invariant_guard<'dae>(
    view: dae::DaeView<'dae>,
    context: &FunctionCallContext<'dae>,
    expression: dae::ExprId<'dae>,
) -> bool {
    let context = context.scoped_to_expression(view, expression);
    let Some(node) = view.expression(expression) else {
        return false;
    };
    match node.operation() {
        dae::ExpressionOperation::Literal(_)
        | dae::ExpressionOperation::Coordinate(dae::CoordinateView::Parameter(_)) => true,
        dae::ExpressionOperation::Coordinate(dae::CoordinateView::FunctionParameter(parameter)) => {
            context
                .parameter_argument(parameter)
                .is_some_and(|argument| invariant_guard(view, &context, argument))
        }
        dae::ExpressionOperation::Unary { operand, .. } => invariant_guard(view, &context, operand),
        dae::ExpressionOperation::Binary { lhs, rhs, .. } => {
            invariant_guard(view, &context, lhs) && invariant_guard(view, &context, rhs)
        }
        dae::ExpressionOperation::Index { base, subscripts } => {
            invariant_guard(view, &context, base)
                && subscripts.iter().all(|subscript| match subscript {
                    dae::SubscriptView::Whole { .. } => true,
                    dae::SubscriptView::Index { expression, .. }
                    | dae::SubscriptView::Slice { expression, .. } => {
                        invariant_guard(view, &context, expression)
                    }
                })
        }
        dae::ExpressionOperation::Array(operands)
        | dae::ExpressionOperation::Builtin {
            arguments: operands,
            ..
        } => operands
            .iter()
            .all(|operand| invariant_guard(view, &context, operand)),
        _ => false,
    }
}

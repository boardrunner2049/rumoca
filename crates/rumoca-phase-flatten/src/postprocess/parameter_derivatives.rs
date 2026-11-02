//! Fold `der(...)` of time-invariant operands to zero (MLS §3.7.4.2).
//!
//! MLS §3.7.4.2 `der(expr)`: "For Real parameters and constants the result is
//! a zero scalar or array of the same size as the variable." A parameter or
//! constant is not a state, so the canonical DAE has no derivative coordinate
//! for it; the operator is folded here, once, where the variability of every
//! flat variable is still known. Package constants that were substituted by
//! value leave `der(<literal>)`, which folds the same way.
//!
//! Only operands whose result shape is known exactly are folded: a whole
//! variable, or a variable selected by scalar subscripts. A slice keeps the
//! call, and DAE validation reports it with a source diagnostic.
//!
//! `der(expr)` of a linear expression is first distributed over it (MLS
//! §3.7.4.2 defines `der` of any Real expression): `der(a - b)` is
//! `der(a) - der(b)` and `der(k * x)` is `k * der(x)` for a time-invariant
//! `k`. Each operand is then a variable whose derivative the DAE owns, or a
//! time-invariant one this pass folds. Anything else keeps the call.

use super::*;
use rumoca_core::ExpressionRewriter;

pub(crate) fn fold_time_invariant_derivatives(flat: &mut flat::Model) {
    let invariant = TimeInvariantVars::build(flat);
    let mut folder = DerivativeFolder {
        invariant: &invariant,
    };
    for equation in flat
        .equations
        .iter_mut()
        .chain(flat.initial_equations.iter_mut())
    {
        equation.residual = folder.rewrite_expression(&equation.residual);
    }
    for family in flat
        .structured_equations
        .iter_mut()
        .chain(flat.initial_structured_equations.iter_mut())
    {
        let Some(template) = family.template.as_mut() else {
            continue;
        };
        for body in &mut template.body {
            *body = folder.rewrite_expression(body);
        }
    }
    for variable in flat.variables.values_mut() {
        if let Some(binding) = &variable.binding {
            variable.binding = Some(folder.rewrite_expression(binding));
        }
    }
}

/// Declared extents of every parameter or constant flat variable.
struct TimeInvariantVars {
    dims: rustc_hash::FxHashMap<String, Vec<i64>>,
}

impl TimeInvariantVars {
    fn build(flat: &flat::Model) -> Self {
        let dims = flat
            .variables
            .iter()
            .filter(|(_, variable)| {
                matches!(
                    variable.variability,
                    rumoca_core::Variability::Parameter(_) | rumoca_core::Variability::Constant(_)
                )
            })
            .map(|(name, variable)| (name.as_str().to_string(), variable.dims.clone()))
            .collect();
        Self { dims }
    }
}

struct DerivativeFolder<'a> {
    invariant: &'a TimeInvariantVars,
}

impl DerivativeFolder<'_> {
    /// `der(argument)` distributed one level over a linear `argument`.
    fn distribute(&self, argument: &Expression, span: rumoca_core::Span) -> Option<Expression> {
        use rumoca_core::{OpBinary, OpUnary};
        let der = |operand: &Expression| {
            Box::new(Expression::BuiltinCall {
                function: rumoca_core::BuiltinFunction::Der,
                args: vec![operand.clone()],
                span,
            })
        };
        Some(match argument {
            Expression::Binary {
                op: op @ (OpBinary::Add | OpBinary::Sub | OpBinary::AddElem | OpBinary::SubElem),
                lhs,
                rhs,
                span,
            } => Expression::Binary {
                op: op.clone(),
                lhs: der(lhs),
                rhs: der(rhs),
                span: *span,
            },
            Expression::Unary {
                op: op @ (OpUnary::Minus | OpUnary::DotMinus),
                rhs,
                span,
            } => Expression::Unary {
                op: op.clone(),
                rhs: der(rhs),
                span: *span,
            },
            Expression::Unary {
                op: OpUnary::Plus | OpUnary::DotPlus,
                rhs,
                ..
            } => *der(rhs),
            Expression::Binary {
                op: op @ (OpBinary::Mul | OpBinary::MulElem),
                lhs,
                rhs,
                span,
            } if self.is_time_invariant(lhs) => Expression::Binary {
                op: op.clone(),
                lhs: lhs.clone(),
                rhs: der(rhs),
                span: *span,
            },
            Expression::Binary {
                op: op @ (OpBinary::Mul | OpBinary::MulElem | OpBinary::Div | OpBinary::DivElem),
                lhs,
                rhs,
                span,
            } if self.is_time_invariant(rhs) => Expression::Binary {
                op: op.clone(),
                lhs: der(lhs),
                rhs: rhs.clone(),
                span: *span,
            },
            _ => return None,
        })
    }

    /// An expression over literals and parameters/constants only.
    fn is_time_invariant(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Literal { .. } => true,
            Expression::VarRef {
                name, subscripts, ..
            } => {
                self.invariant.dims.contains_key(name.as_str())
                    && subscripts.iter().all(|subscript| match subscript {
                        rumoca_core::Subscript::Expr { expr, .. } => self.is_time_invariant(expr),
                        _ => true,
                    })
            }
            Expression::Unary { rhs, .. } => self.is_time_invariant(rhs),
            Expression::Binary { lhs, rhs, .. } => {
                self.is_time_invariant(lhs) && self.is_time_invariant(rhs)
            }
            _ => false,
        }
    }

    /// The exact result extents of `der(argument)` when the operand is time
    /// invariant, or `None` when the operator must stay.
    fn invariant_extents(&self, argument: &Expression) -> Option<Vec<i64>> {
        match argument {
            Expression::Literal {
                value: rumoca_core::Literal::Real(_) | rumoca_core::Literal::Integer(_),
                ..
            } => Some(Vec::new()),
            Expression::VarRef {
                name, subscripts, ..
            } => {
                let dims = self.invariant.dims.get(name.as_str())?;
                if subscripts.is_empty() {
                    return Some(dims.clone());
                }
                let scalar =
                    subscripts.len() == dims.len() && subscripts.iter().all(is_scalar_subscript);
                scalar.then(Vec::new)
            }
            _ => None,
        }
    }
}

fn is_scalar_subscript(subscript: &rumoca_core::Subscript) -> bool {
    match subscript {
        rumoca_core::Subscript::Index { .. } => true,
        rumoca_core::Subscript::Colon { .. } => false,
        rumoca_core::Subscript::Expr { expr, .. } => !matches!(
            expr.as_ref(),
            Expression::Range { .. } | Expression::Array { .. }
        ),
    }
}

/// A Real zero of the given extents, as an explicit array literal.
fn zero_of_extents(extents: &[i64], span: rumoca_core::Span) -> Option<Expression> {
    let Some((&first, rest)) = extents.split_first() else {
        return Some(Expression::Literal {
            value: rumoca_core::Literal::Real(0.0),
            span,
        });
    };
    let count = usize::try_from(first).ok().filter(|count| *count > 0)?;
    let element = zero_of_extents(rest, span)?;
    Some(Expression::Array {
        elements: vec![element; count],
        is_matrix: false,
        span,
    })
}

impl ExpressionRewriter for DerivativeFolder<'_> {
    fn rewrite_expression(&mut self, expr: &Expression) -> Expression {
        if let Expression::BuiltinCall {
            function: rumoca_core::BuiltinFunction::Der,
            args,
            span,
        } = expr
            && let [argument] = args.as_slice()
            && let Some(distributed) = self.distribute(argument, *span)
        {
            return self.rewrite_expression(&distributed);
        }
        if let Expression::BuiltinCall {
            function: rumoca_core::BuiltinFunction::Der,
            args,
            span,
        } = expr
            && let [argument] = args.as_slice()
            && let Some(zero) = self
                .invariant_extents(argument)
                .and_then(|extents| zero_of_extents(&extents, *span))
        {
            return zero;
        }
        self.walk_expression(expr)
    }
}

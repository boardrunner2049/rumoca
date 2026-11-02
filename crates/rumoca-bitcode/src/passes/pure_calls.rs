//! `fold-pure-calls`: evaluate calls whose value is settled at translation.

use super::evaluate::{self, Value};
use super::*;

/// Replace a call to a carried Modelica function, made with literal
/// arguments from a parameter's or constant's attributes, by the literal it
/// returns.
///
/// This is the frontend's `fold_pure_constant_calls` as a pass, restricted
/// the same way: only where a value is settled at translation time -- the
/// binding, start, min, max and nominal of parameters and constants.
/// Applied to ordinary equations it would fold a call away whenever its
/// arguments happened to be literal, which is what once deleted declared
/// functions from the compiler's artifact (TOOLBUG-029).
///
/// A body is foldable only if it and everything it calls are carried
/// Modelica: an external body is where impurity lives (MLS §12.3), and the
/// evaluator refuses it. Every result of a call must be a scalar the
/// artifact can write as a literal, because a call and its projections are
/// replaced together or not at all.
pub(super) struct FoldPureCalls;

impl Pass for FoldPureCalls {
    fn name(&self) -> &'static str {
        "fold-pure-calls"
    }
    fn description(&self) -> &'static str {
        "evaluate calls with literal arguments in parameter and constant attributes"
    }
    fn run(&self, model: &mut RbcModel) -> Result<usize, PassError> {
        let settled = settled_expressions(model);
        let mut folded = 0;
        let heads: Vec<usize> = settled
            .iter()
            .enumerate()
            .filter_map(|(index, settled)| settled.then_some(index))
            .collect();
        for head in heads {
            let RbcExprNode::Call {
                owner,
                function,
                arguments,
                ..
            } = &model.expressions[head].node
            else {
                continue;
            };
            if owner.0 as usize != head {
                continue;
            }
            let Some(values) = arguments
                .iter()
                .map(|argument| evaluate::expression(model, *argument))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let Some(results) = evaluate::call(model, *function, &values) else {
                continue;
            };
            let Some(replacements) = replacements(model, ExprId(head as u32), &results) else {
                continue;
            };
            for (index, literal) in replacements {
                model.expressions[index].node = RbcExprNode::Literal { value: literal };
                folded += 1;
            }
        }
        Ok(folded)
    }
}

/// Expressions reachable from parameter and constant attributes.
fn settled_expressions(model: &RbcModel) -> Vec<bool> {
    let mut settled = vec![false; model.expressions.len()];
    let mut pending: Vec<ExprId> = model
        .variables
        .iter()
        .filter(|variable| matches!(variable.role, RbcRole::Parameter | RbcRole::Constant))
        .flat_map(|variable| {
            [
                variable.binding,
                variable.start,
                variable.min,
                variable.max,
                variable.nominal,
            ]
        })
        .flatten()
        .collect();
    while let Some(id) = pending.pop() {
        let Some(slot) = settled.get_mut(id.0 as usize) else {
            continue;
        };
        if std::mem::replace(slot, true) {
            continue;
        }
        pending.extend(crate::build::references(
            id,
            &model.expressions[id.0 as usize].node,
        ));
    }
    settled
}

/// The literal for the head and each projection of one call, or `None` if
/// any of them is not a scalar the artifact can write.
fn replacements(
    model: &RbcModel,
    head: ExprId,
    results: &[Value],
) -> Option<Vec<(usize, RbcLiteral)>> {
    model
        .expressions
        .iter()
        .enumerate()
        .filter_map(|(index, expression)| match expression.node {
            RbcExprNode::Call { owner, output, .. } if owner == head => Some((index, output)),
            _ => None,
        })
        .map(|(index, output)| {
            let value = results.get(output as usize)?;
            let scalar = model
                .types
                .get(model.expressions[index].value_type.0 as usize)?
                .scalar;
            Some((index, literal(scalar, value)?))
        })
        .collect()
}

pub(super) fn literal(scalar: RbcScalar, value: &Value) -> Option<RbcLiteral> {
    Some(match (scalar, value) {
        (RbcScalar::Real, Value::Real(value)) => RbcLiteral::Real { value: *value },
        (RbcScalar::Real, Value::Integer(value)) => RbcLiteral::Real {
            value: *value as f64,
        },
        (RbcScalar::Integer, Value::Integer(value)) => RbcLiteral::Integer { value: *value },
        (RbcScalar::Boolean, Value::Boolean(value)) => RbcLiteral::Boolean { value: *value },
        _ => return None,
    })
}

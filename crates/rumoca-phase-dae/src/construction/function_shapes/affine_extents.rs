//! Extents of compact ranges whose bounds share one symbolic offset.
//!
//! MLS §10.4.3 sizes `j:k` as `k - j + 1`. A slice such as `state[i-2:i-1]`
//! inside `for i in ...` has bounds no scope proves a value for, yet its
//! cardinality is exact: both bounds are the loop index plus a constant, so
//! the index cancels. This module decides that cancellation for Integer
//! affine bounds (sums of `coefficient * name` plus a constant) and nothing
//! wider; anything else stays unproven.

use std::collections::BTreeMap;

use super::*;

/// `coefficient * name` terms plus a constant.
#[derive(Debug, Default, PartialEq, Eq)]
struct Affine {
    terms: BTreeMap<String, i64>,
    constant: i64,
}

impl Affine {
    fn constant(value: i64) -> Self {
        Self {
            terms: BTreeMap::new(),
            constant: value,
        }
    }

    fn add(mut self, other: Self, sign: i64) -> Option<Self> {
        for (name, coefficient) in other.terms {
            let entry = self.terms.entry(name).or_insert(0);
            *entry = entry.checked_add(coefficient.checked_mul(sign)?)?;
        }
        self.terms.retain(|_, coefficient| *coefficient != 0);
        self.constant = self
            .constant
            .checked_add(other.constant.checked_mul(sign)?)?;
        Some(self)
    }

    fn scale(mut self, factor: i64) -> Option<Self> {
        for coefficient in self.terms.values_mut() {
            *coefficient = coefficient.checked_mul(factor)?;
        }
        self.terms.retain(|_, coefficient| *coefficient != 0);
        self.constant = self.constant.checked_mul(factor)?;
        Some(self)
    }
}

/// The affine form of an Integer expression: proven values fold to constants,
/// unsubscripted scalar names stay symbolic.
fn affine(expression: &Expression, values: &ShapeEnvironment) -> Option<Affine> {
    if let Ok(value) = evaluate_shape_integer(expression, values) {
        return Some(Affine::constant(value));
    }
    match expression {
        Expression::VarRef {
            name, subscripts, ..
        } if subscripts.is_empty() => Some(Affine {
            terms: BTreeMap::from([(name.as_str().to_string(), 1)]),
            constant: 0,
        }),
        Expression::Unary {
            op: OpUnary::Plus,
            rhs,
            ..
        } => affine(rhs, values),
        Expression::Unary {
            op: OpUnary::Minus,
            rhs,
            ..
        } => affine(rhs, values)?.scale(-1),
        Expression::Binary { op, lhs, rhs, .. } => affine_binary(op, lhs, rhs, values),
        _ => None,
    }
}

fn affine_binary(
    op: &OpBinary,
    lhs: &Expression,
    rhs: &Expression,
    values: &ShapeEnvironment,
) -> Option<Affine> {
    match op {
        OpBinary::Add | OpBinary::AddElem => affine(lhs, values)?.add(affine(rhs, values)?, 1),
        OpBinary::Sub | OpBinary::SubElem => affine(lhs, values)?.add(affine(rhs, values)?, -1),
        OpBinary::Mul | OpBinary::MulElem => {
            if let Ok(factor) = evaluate_shape_integer(lhs, values) {
                affine(rhs, values)?.scale(factor)
            } else {
                let factor = evaluate_shape_integer(rhs, values).ok()?;
                affine(lhs, values)?.scale(factor)
            }
        }
        _ => None,
    }
}

/// `end - start` when both bounds are affine in the same names with the same
/// coefficients, so the difference is one exact Integer.
pub(super) fn affine_bound_difference(
    start: &Expression,
    end: &Expression,
    values: &ShapeEnvironment,
) -> Option<i64> {
    let difference = affine(end, values)?.add(affine(start, values)?, -1)?;
    difference.terms.is_empty().then_some(difference.constant)
}

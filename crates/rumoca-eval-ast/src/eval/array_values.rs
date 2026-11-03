//! Array-valued parameters and array comprehensions in compile-time
//! evaluation (MLS §10.4.1 array constructors with iterators, §10.3.4
//! reductions).
//!
//! Structural parameters are often reductions over array parameters, e.g.
//! `dimension = sum({if A > 0 then 1 else 0 for A in AArray})` with
//! `AArray = {ATotExt, ATotWin}` and `ATotExt = sum(AExt)`. The context keeps
//! the element values of array parameters whose binding is evaluable
//! (`real_arrays`), and a comprehension is evaluated by substituting each
//! iterator value into its body.

use super::{
    TypeCheckEvalContext, eval_integer_with_scope, eval_real_with_scope, lookup_with_scope,
};
use rumoca_core::{OpUnary, Span, Token};
use rumoca_ir_ast::{Expression, ForIndex, TerminalType};
use std::sync::Arc;

/// Upper bound on comprehension size evaluated element-wise at compile time.
const MAX_COMPREHENSION_ELEMENTS: usize = 100_000;

/// Element values of a one-dimensional Real array expression.
pub fn eval_real_array_with_scope(
    expr: &Expression,
    ctx: &TypeCheckEvalContext,
    scope: &str,
) -> Option<Vec<f64>> {
    match expr {
        Expression::Array {
            elements,
            kind: rumoca_core::ArrayConstructor::Array,
            ..
        } => elements
            .iter()
            .map(|element| eval_real_with_scope(element, ctx, scope))
            .collect(),
        Expression::ComponentReference(cr)
            if cr
                .parts
                .iter()
                .all(|part| part.subs.as_ref().is_none_or(Vec::is_empty)) =>
        {
            let path = super::component_reference_path(cr);
            lookup_with_scope(&path, scope, &ctx.real_arrays).cloned()
        }
        Expression::Parenthesized { inner, .. } => eval_real_array_with_scope(inner, ctx, scope),
        Expression::ArrayComprehension {
            expr: body,
            indices,
            filter: None,
            ..
        } => comprehension_elements(body, indices, ctx, scope)?
            .iter()
            .map(|element| eval_real_with_scope(element, ctx, scope))
            .collect(),
        _ => None,
    }
}

/// Integer elements of an array comprehension `{e for i in r}`.
pub(super) fn eval_integer_comprehension(
    body: &Expression,
    indices: &[ForIndex],
    ctx: &TypeCheckEvalContext,
    scope: &str,
) -> Option<Vec<i64>> {
    comprehension_elements(body, indices, ctx, scope)?
        .iter()
        .map(|element| eval_integer_with_scope(element, ctx, scope))
        .collect()
}

/// The comprehension body instantiated for every iterator value (one index;
/// the result is one-dimensional).
fn comprehension_elements(
    body: &Expression,
    indices: &[ForIndex],
    ctx: &TypeCheckEvalContext,
    scope: &str,
) -> Option<Vec<Expression>> {
    let [index] = indices else {
        return None;
    };
    let values = iterator_values(&index.range, ctx, scope)?;
    if values.len() > MAX_COMPREHENSION_ELEMENTS {
        return None;
    }
    let ident = index.ident.text.as_ref();
    Some(
        values
            .into_iter()
            .map(|value| substitute(body, ident, &value))
            .collect(),
    )
}

/// Iterator values as literal expressions: an Integer range, or the elements
/// of an evaluable Real array (`for A in AArray`).
fn iterator_values(
    range: &Expression,
    ctx: &TypeCheckEvalContext,
    scope: &str,
) -> Option<Vec<Expression>> {
    if let Expression::Range {
        start, step, end, ..
    } = range
    {
        let start = eval_integer_with_scope(start, ctx, scope)?;
        let step = match step {
            Some(step) => eval_integer_with_scope(step, ctx, scope)?,
            None => 1,
        };
        let end = eval_integer_with_scope(end, ctx, scope)?;
        if step == 0 {
            return None;
        }
        let count = if (step > 0 && end < start) || (step < 0 && end > start) {
            0
        } else {
            usize::try_from((end - start) / step + 1).ok()?
        };
        if count > MAX_COMPREHENSION_ELEMENTS {
            return None;
        }
        let span = range.span();
        return Some(
            (0..count)
                .map(|k| integer_literal(start + step * k as i64, span))
                .collect(),
        );
    }
    let span = range.span();
    Some(
        eval_real_array_with_scope(range, ctx, scope)?
            .into_iter()
            .map(|value| real_literal(value, span))
            .collect(),
    )
}

fn literal(terminal_type: TerminalType, text: String, span: Span) -> Expression {
    Expression::Terminal {
        terminal_type,
        token: Token {
            text: Arc::from(text),
            ..Token::default()
        },
        span,
    }
}

fn negated(value: Expression, span: Span) -> Expression {
    Expression::Unary {
        op: OpUnary::Minus,
        rhs: Arc::new(value),
        span,
    }
}

fn integer_literal(value: i64, span: Span) -> Expression {
    let magnitude = literal(
        TerminalType::UnsignedInteger,
        value.unsigned_abs().to_string(),
        span,
    );
    if value < 0 {
        negated(magnitude, span)
    } else {
        magnitude
    }
}

fn real_literal(value: f64, span: Span) -> Expression {
    let magnitude = literal(
        TerminalType::UnsignedReal,
        format!("{:?}", value.abs()),
        span,
    );
    if value.is_sign_negative() && value != 0.0 {
        negated(magnitude, span)
    } else {
        magnitude
    }
}

/// Replace the iterator `ident` by `value` in the expression forms a
/// structural comprehension body uses; other forms are kept as written.
fn substitute(expr: &Expression, ident: &str, value: &Expression) -> Expression {
    let sub = |inner: &Arc<Expression>| Arc::new(substitute(inner, ident, value));
    match expr {
        Expression::ComponentReference(cr)
            if cr.parts.len() == 1
                && cr.parts[0].ident.text.as_ref() == ident
                && cr.parts[0].subs.as_ref().is_none_or(Vec::is_empty) =>
        {
            value.clone()
        }
        Expression::Binary { op, lhs, rhs, span } => Expression::Binary {
            op: op.clone(),
            lhs: sub(lhs),
            rhs: sub(rhs),
            span: *span,
        },
        Expression::Unary { op, rhs, span } => Expression::Unary {
            op: op.clone(),
            rhs: sub(rhs),
            span: *span,
        },
        Expression::Parenthesized { inner, span } => Expression::Parenthesized {
            inner: sub(inner),
            span: *span,
        },
        Expression::If {
            branches,
            else_branch,
            span,
        } => Expression::If {
            branches: branches
                .iter()
                .map(|(cond, then)| {
                    (
                        substitute(cond, ident, value),
                        substitute(then, ident, value),
                    )
                })
                .collect(),
            else_branch: sub(else_branch),
            span: *span,
        },
        Expression::FunctionCall {
            comp,
            args,
            is_partial_application,
            span,
        } => Expression::FunctionCall {
            comp: comp.clone(),
            args: args
                .iter()
                .map(|arg| substitute(arg, ident, value))
                .collect(),
            is_partial_application: *is_partial_application,
            span: *span,
        },
        _ => expr.clone(),
    }
}

/// `v[k]` of a one-dimensional array parameter with known element values.
pub(super) fn lookup_real_element(
    expr: &Expression,
    ctx: &TypeCheckEvalContext,
    scope: &str,
) -> Option<f64> {
    let Expression::ComponentReference(cr) = expr else {
        return None;
    };
    let (last, leading) = cr.parts.split_last()?;
    if leading
        .iter()
        .any(|part| part.subs.as_ref().is_some_and(|s| !s.is_empty()))
    {
        return None;
    }
    let [rumoca_ir_ast::Subscript::Expression(index)] = last.subs.as_deref()? else {
        return None;
    };
    let index = usize::try_from(eval_integer_with_scope(index, ctx, scope)?).ok()?;
    let mut path = String::new();
    for part in &cr.parts {
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(part.ident.text.as_ref());
    }
    let values = lookup_with_scope(&path, scope, &ctx.real_arrays)?;
    values.get(index.checked_sub(1)?).copied()
}

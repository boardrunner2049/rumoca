use super::*;
use rumoca_ir_ast::TerminalType;

/// Try to infer array dimensions from a binding expression.
pub fn infer_dimensions_from_binding(
    expr: &Expression,
    ctx: &(impl DimensionInferenceContext + ?Sized),
) -> Option<Vec<usize>> {
    infer_dimensions_from_binding_with_scope(expr, ctx, "")
}

/// Try to infer array dimensions from a binding expression with scope context.
///
/// The scope is used for resolving component references. For example, when
/// evaluating `combiTimeTable.table` with binding `table`, the scope is
/// the parent component path so we can resolve `table` correctly.
pub fn infer_dimensions_from_binding_with_scope(
    expr: &Expression,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<Vec<usize>> {
    match expr {
        Expression::Terminal { .. } => Some(Vec::new()),

        Expression::Array { elements, kind, .. } => infer_array_dims(elements, *kind, ctx, scope),

        Expression::FunctionCall { comp, args, .. } => {
            let func_name = comp
                .parts
                .iter()
                .map(|p| p.ident.text.as_ref())
                .collect::<Vec<_>>()
                .join(".");
            infer_dims_from_func_with_scope(&func_name, args, ctx, scope)
        }

        Expression::Range {
            start, step, end, ..
        } => infer_range_len_numeric(start, step.as_deref(), end, ctx, scope).map(|n| vec![n]),

        Expression::ComponentReference(cr) => {
            let indexed_path = cr.to_string();
            if let Some(dims) = ctx.lookup_dimensions(&indexed_path, scope) {
                return Some(dims);
            }

            let unindexed_path = cr
                .parts
                .iter()
                .map(|p| p.ident.text.as_ref())
                .collect::<Vec<_>>()
                .join(".");
            let Some(base_dims) = ctx.lookup_dimensions(&unindexed_path, scope) else {
                return (ctx.is_declared_scalar_reference(cr)
                    || ctx.scalar_value_known(&unindexed_path, scope))
                .then(Vec::new);
            };
            apply_component_subscripts_to_dims(base_dims, cr, ctx, scope)
        }

        Expression::Parenthesized { inner, .. } => {
            infer_dimensions_from_binding_with_scope(inner, ctx, scope)
        }

        // Handle if-expressions by checking branch consistency or evaluating condition.
        Expression::If {
            branches,
            else_branch,
            ..
        } => infer_dims_from_if_with_scope(branches, else_branch, ctx, scope),

        // Binary expressions: element-wise and regular ops preserve shape.
        Expression::Binary { op, lhs, rhs, .. } => {
            infer_dims_from_binary_with_scope(op, lhs, rhs, ctx, scope)
        }

        // Unary expressions (`-A`, `not A`) preserve shape.
        Expression::Unary { rhs, .. } => infer_dimensions_from_binding_with_scope(rhs, ctx, scope),

        // FieldAccess: `base.field` resolves as a full path in scope.
        Expression::FieldAccess { base, field, .. } => {
            if let Some((comp, args, output)) = function_output_base(base) {
                let func_name = comp
                    .parts
                    .iter()
                    .map(|p| p.ident.text.as_ref())
                    .collect::<Vec<_>>()
                    .join(".");
                return ctx
                    .infer_user_function_field_dimensions(&func_name, args, output, field, scope);
            }
            let base_path = extract_simple_component_path(base)?;
            let full_path = format!("{base_path}.{field}");
            ctx.lookup_dimensions(&full_path, scope)
        }

        // ArrayComprehension: `{expr for i in range}` -> `[range_len, inner_dims...]`.
        Expression::ArrayComprehension {
            expr: inner_expr,
            indices,
            ..
        } => infer_dims_from_array_comprehension(inner_expr, indices, ctx, scope),

        _ => None,
    }
}

/// Split a record-projection base into the called function, its arguments
/// and the selected output: `f(a)` (first output) or `f(a).out`.
fn function_output_base(
    base: &Expression,
) -> Option<(
    &rumoca_ir_ast::ComponentReference,
    &[Expression],
    Option<&str>,
)> {
    match base {
        Expression::FunctionCall { comp, args, .. } => Some((comp, args.as_slice(), None)),
        Expression::FieldAccess { base, field, .. } => match base.as_ref() {
            Expression::FunctionCall { comp, args, .. } => {
                Some((comp, args.as_slice(), Some(field.as_str())))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Apply component-reference subscripts to a base dimension vector.
///
/// MLS §10.1: scalar indexing consumes one dimension (`a[i]` -> scalar from `[n]`),
/// while range/colon indexing preserves that dimension (`a[2:4]`, `a[:]`).
fn apply_component_subscripts_to_dims(
    mut dims: Vec<usize>,
    cr: &rumoca_ir_ast::ComponentReference,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<Vec<usize>> {
    // `pos` tracks the dimension the next subscript applies to. Scalar
    // indexing removes that dimension (so the cursor stays put, now pointing
    // at the following dimension); slice/colon indexing keeps it and advances
    // the cursor. This positional walk is what lets `a[:, i]` on `[3, 4]`
    // drop dimension 1 and yield `[3]` rather than removing dimension 0.
    let mut pos = 0usize;
    let mut unknown = false;
    for part in &cr.parts {
        let Some(subs) = &part.subs else { continue };
        for sub in subs {
            if pos >= dims.len() {
                return Some(dims);
            }
            apply_subscript_to_dims(sub, &mut dims, &mut pos, &mut unknown, ctx, scope);
            if unknown {
                return None;
            }
        }
    }
    Some(dims)
}

/// Replace every `end` in a slice expression with the indexed extent.
fn substitute_end(expr: &Expression, extent: usize) -> Expression {
    match expr {
        Expression::Terminal {
            terminal_type: TerminalType::End,
            token,
            span,
        } => Expression::Terminal {
            terminal_type: TerminalType::UnsignedInteger,
            token: rumoca_core::Token {
                text: Arc::from(extent.to_string()),
                ..token.clone()
            },
            span: *span,
        },
        Expression::Range {
            start,
            step,
            end,
            span,
        } => Expression::Range {
            start: Arc::new(substitute_end(start, extent)),
            step: step.as_ref().map(|s| Arc::new(substitute_end(s, extent))),
            end: Arc::new(substitute_end(end, extent)),
            span: *span,
        },
        Expression::Binary { op, lhs, rhs, span } => Expression::Binary {
            op: op.clone(),
            lhs: Arc::new(substitute_end(lhs, extent)),
            rhs: Arc::new(substitute_end(rhs, extent)),
            span: *span,
        },
        Expression::Unary { op, rhs, span } => Expression::Unary {
            op: op.clone(),
            rhs: Arc::new(substitute_end(rhs, extent)),
            span: *span,
        },
        Expression::Parenthesized { inner, span } => Expression::Parenthesized {
            inner: Arc::new(substitute_end(inner, extent)),
            span: *span,
        },
        other => other.clone(),
    }
}

fn apply_subscript_to_dims(
    sub: &Subscript,
    dims: &mut Vec<usize>,
    pos: &mut usize,
    unknown: &mut bool,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) {
    match sub {
        // MLS §10.5: `end` inside a slice denotes the extent of the dimension
        // being indexed, so `a[2:end]` on `[n]` has `n - 1` elements. An
        // unevaluable slice leaves the dimension unknown instead of silently
        // keeping the full extent.
        Subscript::Expression(expr) if matches!(expr, Expression::Range { .. }) => {
            let bound = substitute_end(expr, dims[*pos]);
            let Some(len) = infer_range_length(&bound, ctx, scope) else {
                *unknown = true;
                return;
            };
            dims[*pos] = len;
            *pos += 1;
        }
        // Scalar indexing consumes the dimension at the cursor.
        Subscript::Expression(_) => {
            dims.remove(*pos);
        }
        // `:` keeps the current dimension unchanged.
        Subscript::Range { .. } | Subscript::Empty => {
            *pos += 1;
        }
    }
}

fn extract_simple_component_path(expr: &Expression) -> Option<String> {
    rumoca_ir_ast::expression_component_path(expr).map(|path| path.to_flat_string())
}

fn infer_dims_from_array_comprehension(
    inner_expr: &Expression,
    indices: &[rumoca_ir_ast::ForIndex],
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<Vec<usize>> {
    if indices.is_empty() {
        return None;
    }
    let range = &indices[0].range;
    let outer_len = infer_range_length(range, ctx, scope)?;
    let mut dims = vec![outer_len];
    if let Some(inner_dims) = infer_dimensions_from_binding_with_scope(inner_expr, ctx, scope) {
        dims.extend(inner_dims);
    }
    Some(dims)
}

fn infer_range_length(
    range: &Expression,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<usize> {
    if let Expression::Range {
        start, step, end, ..
    } = range
    {
        infer_range_len_numeric(start, step.as_deref(), end, ctx, scope)
    } else {
        ctx.eval_integer(range, scope)
            .and_then(|n| usize::try_from(n).ok())
    }
}

fn infer_dims_from_binary_with_scope(
    op: &OpBinary,
    lhs: &Expression,
    rhs: &Expression,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<Vec<usize>> {
    let lhs_dims = infer_dimensions_from_binding_with_scope(lhs, ctx, scope);
    let rhs_dims = infer_dimensions_from_binding_with_scope(rhs, ctx, scope);

    match op {
        // MLS §10.6.7–8: scalar powers and square matrix powers retain rank.
        OpBinary::Exp => match (lhs_dims, rhs_dims) {
            (Some(base), Some(exponent))
                if exponent.is_empty()
                    && (base.is_empty() || (base.len() == 2 && base[0] == base[1])) =>
            {
                Some(base)
            }
            _ => None,
        },
        // Matrix multiply: `[m,n] * [n,p]` -> `[m,p]`.
        OpBinary::Mul => match (&lhs_dims, &rhs_dims) {
            (Some(ld), Some(rd)) if ld.len() == 2 && rd.len() == 2 => Some(vec![ld[0], rd[1]]),
            (Some(ld), Some(rd)) if ld.len() == 2 && rd.len() == 1 => Some(vec![ld[0]]),
            (Some(ld), None) => Some(ld.clone()),
            (None, Some(rd)) => Some(rd.clone()),
            (Some(ld), Some(rd)) if ld.is_empty() => Some(rd.clone()),
            (Some(ld), Some(rd)) if rd.is_empty() => Some(ld.clone()),
            _ => lhs_dims.or(rhs_dims),
        },
        OpBinary::Add
        | OpBinary::Sub
        | OpBinary::AddElem
        | OpBinary::SubElem
        | OpBinary::MulElem
        | OpBinary::DivElem
        | OpBinary::ExpElem => lhs_dims.or(rhs_dims),
        OpBinary::Div => lhs_dims.or(rhs_dims),
        _ => None,
    }
}

fn infer_dims_from_if_with_scope(
    branches: &[(Expression, Expression)],
    else_branch: &Expression,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<Vec<usize>> {
    if let Some(dims) = try_eval_if_condition_with_scope(branches, else_branch, ctx, scope) {
        return Some(dims);
    }

    let else_dims = infer_dimensions_from_binding_with_scope(else_branch, ctx, scope)?;
    if all_branches_consistent_with_scope(branches, &else_dims, ctx, scope) {
        Some(else_dims)
    } else {
        None
    }
}

fn try_eval_if_condition_with_scope(
    branches: &[(Expression, Expression)],
    else_branch: &Expression,
    ctx: &(impl DimensionInferenceContext + ?Sized),
    scope: &str,
) -> Option<Vec<usize>> {
    for (cond, then_expr) in branches {
        match ctx.eval_boolean(cond, scope) {
            Some(true) => return infer_dimensions_from_binding_with_scope(then_expr, ctx, scope),
            Some(false) => continue,
            None => return None,
        }
    }
    infer_dimensions_from_binding_with_scope(else_branch, ctx, scope)
}

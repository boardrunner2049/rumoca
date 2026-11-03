//! Reductions over array comprehensions (`sum(e for i in r)`), MLS §10.3.4.

use super::*;

/// Expand `sum(e for i in r)` / `product(...)` into a fold of the body values.
///
/// MLS §10.3.4 defines a reduction expression as applying the operator to the
/// *values* of `e`. When `e` is an array, `+` is element-wise (§10.6.3), so
/// `sum(e for i in 1:2)` is `e[i:=1] + e[i:=2]` and keeps `e`'s shape.
///
/// Expanding it to `sum({e1, e2})` instead is a different expression: an array
/// of two 3-vectors is a 2x3 matrix, and `sum` of a matrix is correctly a
/// scalar. That silently turned `M_b = sum(cross(..) + .. for idx in 1:4)`
/// from a 3-vector into a scalar and failed as a shape mismatch — the one
/// place the distinction is visible, rather than where it was lost.
pub(super) fn expand_reduction_over_comprehension(
    ctx: &Context,
    comp: &ast::ComponentReference,
    args: &[ast::Expression],
    prefix: &ast::QualifiedName,
    span: rumoca_core::Span,
) -> Result<Option<ast::Expression>, FlattenError> {
    let name = comp
        .parts
        .last()
        .map(|part| part.ident.text.as_ref())
        .unwrap_or_default();
    let op = match name {
        "sum" => rumoca_core::OpBinary::Add,
        "product" => rumoca_core::OpBinary::Mul,
        _ => return Ok(None),
    };
    let [
        ast::Expression::ArrayComprehension {
            expr: body,
            indices,
            filter,
            ..
        },
    ] = args
    else {
        return Ok(None);
    };

    let mut index_ranges: Vec<(String, Vec<i64>)> = Vec::new();
    for index in indices {
        let values = expand_range_indices(ctx, &index.range, prefix, span)?;
        index_ranges.push((index.ident.text.to_string(), values));
    }
    let env = ArrayComprehensionExpansionEnv {
        ctx,
        prefix,
        span,
        index_ranges: &index_ranges,
    };
    let mut elements = Vec::new();
    expand_array_comprehension_recursive(
        &env,
        body.as_ref().clone(),
        filter.as_ref().map(|f| f.as_ref().clone()),
        0,
        &mut elements,
    )?;

    // An empty domain has no identity element to produce here, so leave it for
    // the normal path to report rather than inventing 0 or 1.
    let mut folded = match elements.first() {
        Some(first) => first.clone(),
        None => return Ok(None),
    };
    for element in elements.into_iter().skip(1) {
        folded = ast::Expression::Binary {
            op: op.clone(),
            lhs: Arc::new(folded),
            rhs: Arc::new(element),
            span,
        };
    }
    Ok(Some(folded))
}

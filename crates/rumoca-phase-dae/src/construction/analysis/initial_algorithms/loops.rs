//! `for` statements of an initial algorithm, unrolled over their ranges.
//!
//! MLS 3.7 §11.2.2 evaluates a `for` range once, before the first iteration.
//! An initial algorithm runs once, so a range whose bounds read only literals,
//! constants, and evaluable parameters has the same iterations at every run
//! the compiled model can make. Such a loop is exactly its unrolled sequence:
//! each iteration is the body with the index replaced by its value, replayed
//! in order through the section's ordinary grammar, so an indexed target such
//! as `y[i]` becomes the whole coordinate `y[2]`. A range that reads a settable
//! parameter could change between runs and is refused, as is a range that is
//! not an Integer range.

use super::*;
use rumoca_core::StatementRewriter;

impl Replay<'_> {
    pub(super) fn unrolled(
        &mut self,
        indices: &[rumoca_core::ForIndex],
        body: &[rumoca_core::Statement],
        span: Span,
        guard: Option<&Expression>,
        values: &mut ReplayValues,
    ) -> Result<(), ToDaeError> {
        let Some((index, rest)) = indices.split_first() else {
            return self.statements(body, guard, values);
        };
        let (start, step, end) = evaluable_range(self.flat, self.shapes, &index.range, span)?;
        let mut current = start;
        while (step > 0 && current <= end) || (step < 0 && current >= end) {
            let mut binding = IndexBinding {
                name: &index.ident,
                value: current,
                span,
            };
            let rest = binding.rewrite_for_indices(rest);
            let body = binding.rewrite_statements(body);
            self.unrolled(&rest, &body, span, guard, values)?;
            let Some(next) = current.checked_add(step) else {
                break;
            };
            current = next;
        }
        Ok(())
    }
}

/// The exact `(start, step, end)` of a range whose bounds read only literals,
/// constants, and evaluable parameters.
fn evaluable_range(
    flat: &flat::Model,
    shapes: &ShapeEnvironment,
    range: &Expression,
    span: Span,
) -> Result<(i64, i64, i64), ToDaeError> {
    let Expression::Range {
        start, step, end, ..
    } = range
    else {
        return Err(unsupported(
            "an initial algorithm `for` iterates over an Integer range",
            span,
        ));
    };
    let bound = |expression: &Expression| {
        reads_only_evaluable(flat, shapes, expression)
            .then(|| shapes.proven_integer_bounds(expression))
            .flatten()
            .and_then(|(lower, upper)| (lower == upper).then_some(lower))
            .ok_or_else(|| {
                unsupported(
                    "an initial algorithm `for` range must read only literals, constants, and \
                     evaluable parameters, so its iterations are fixed at translation",
                    span,
                )
            })
    };
    let step = step.as_deref().map(bound).transpose()?.unwrap_or(1);
    if step == 0 {
        return Err(unsupported(
            "an initial algorithm `for` range has a zero step",
            span,
        ));
    }
    Ok((bound(start)?, step, bound(end)?))
}

fn reads_only_evaluable(
    flat: &flat::Model,
    shapes: &ShapeEnvironment,
    expression: &Expression,
) -> bool {
    let evaluable = shapes.evaluable();
    ast_reads(expression).iter().all(|name| {
        evaluable.is_some_and(|evaluable| evaluable.contains(name))
            || flat
                .variables
                .get(name)
                .is_some_and(|variable| matches!(variable.variability, Variability::Constant(_)))
    })
}

fn ast_reads(expression: &Expression) -> Vec<VarName> {
    let mut reads = Vec::new();
    expression.collect_var_refs(&mut reads);
    reads
}

/// Replaces one loop index by its iteration value; a nested loop that rebinds
/// the same name keeps its own binding.
struct IndexBinding<'name> {
    name: &'name str,
    value: i64,
    span: Span,
}

impl ExpressionRewriter for IndexBinding<'_> {
    fn rewrite_var_ref_expression(
        &mut self,
        name: &rumoca_core::Reference,
        subscripts: &[Subscript],
        span: Span,
    ) -> Expression {
        if subscripts.is_empty() && name.var_name().as_str() == self.name {
            return Expression::Literal {
                value: rumoca_core::Literal::Integer(self.value),
                span: self.span,
            };
        }
        Expression::VarRef {
            name: name.clone(),
            subscripts: self.rewrite_subscripts(subscripts),
            span,
        }
    }
}

impl StatementRewriter for IndexBinding<'_> {
    fn rewrite_statement(&mut self, statement: &rumoca_core::Statement) -> rumoca_core::Statement {
        if let rumoca_core::Statement::For { indices, .. } = statement
            && indices.iter().any(|index| index.ident == self.name)
        {
            return statement.clone();
        }
        self.walk_statement(statement)
    }
}

//! Records of the if-equation branch selections flatten makes by evaluating
//! a parameter guard at translation (SPEC_0040 DAE-C22).
//!
//! Such a selection fixes each parameter its evaluated conditions read at its
//! translation-time value, so DAE construction marks those parameters
//! evaluable and warns at the if-equation. Flatten records the component
//! references the conditions read, with the flat names each can denote; DAE
//! analysis keeps the innermost one the model declares.

use rumoca_ir_ast as ast;
use rumoca_ir_flat as flat;

use super::build_qualified_name;
use crate::{Context, FlattenError};

/// MLS 3.7 sections 4.5 and 18.6: a `fixed = false` or `Evaluate = false`
/// parameter is not evaluable, so a for-equation range reading one has no
/// translation value (section 8.3.3); refuse it before expansion reads it.
pub(super) fn refuse_non_evaluable_range(
    ctx: &Context,
    indices: &[ast::ForIndex],
    prefix: &ast::QualifiedName,
    span: rumoca_core::Span,
) -> Result<(), FlattenError> {
    let Some(parameter) = indices.iter().find_map(|index| {
        crate::boolean_eval::non_evaluable_parameter_read(ctx, &index.range, prefix)
    }) else {
        return Ok(());
    };
    Err(FlattenError::unsupported_equation(
        format!(
            "for-equation range reads non-evaluable parameter `{parameter}` (fixed = false or \
             Evaluate = false); MLS 3.7 section 8.3.3 evaluates the range at translation"
        ),
        span,
    ))
}

/// The record of a structural use whose evaluated expressions are `conditions`.
pub(crate) fn parameter_branch_selection<'a>(
    kind: flat::StructuralParameterUse,
    conditions: impl IntoIterator<Item = &'a ast::Expression>,
    prefix: &ast::QualifiedName,
    span: rumoca_core::Span,
) -> flat::ParameterBranchSelection {
    let mut references = Vec::new();
    for condition in conditions {
        collect_references(condition, prefix, &mut references);
    }
    flat::ParameterBranchSelection {
        span,
        kind,
        references,
    }
}

fn collect_references(
    expression: &ast::Expression,
    prefix: &ast::QualifiedName,
    references: &mut Vec<Vec<String>>,
) {
    match expression {
        ast::Expression::ComponentReference(reference) => {
            references.push(scoped_candidates(prefix, reference));
        }
        ast::Expression::Binary { lhs, rhs, .. } => {
            collect_references(lhs, prefix, references);
            collect_references(rhs, prefix, references);
        }
        ast::Expression::Unary { rhs, .. } => collect_references(rhs, prefix, references),
        ast::Expression::Parenthesized { inner, .. } => {
            collect_references(inner, prefix, references);
        }
        ast::Expression::FunctionCall { comp, args, .. } => {
            // `size(a, k)` and `ndims(a)` read only the shape of `a`, which is
            // fixed at translation whatever its values (MLS §10.1).
            let shape_query = matches!(comp.to_string().as_str(), "size" | "ndims");
            for argument in args.iter().skip(usize::from(shape_query)) {
                collect_references(argument, prefix, references);
            }
        }
        ast::Expression::Range {
            start, step, end, ..
        } => {
            collect_references(start, prefix, references);
            if let Some(step) = step {
                collect_references(step, prefix, references);
            }
            collect_references(end, prefix, references);
        }
        ast::Expression::Array { elements, .. } => {
            for element in elements {
                collect_references(element, prefix, references);
            }
        }
        ast::Expression::If {
            branches,
            else_branch,
            ..
        } => {
            for (condition, value) in branches {
                collect_references(condition, prefix, references);
                collect_references(value, prefix, references);
            }
            collect_references(else_branch, prefix, references);
        }
        _ => {}
    }
}

/// The flat names `reference` can denote from `prefix`: qualified by the
/// whole prefix first, then by each enclosing scope.
fn scoped_candidates(
    prefix: &ast::QualifiedName,
    reference: &ast::ComponentReference,
) -> Vec<String> {
    (0..=prefix.parts.len())
        .rev()
        .map(|depth| {
            let scope = ast::QualifiedName {
                parts: prefix.parts[..depth].to_vec(),
            };
            build_qualified_name(&scope, reference)
        })
        .collect()
}

/// Whether the branches of an if-equation are structurally equal: the same
/// number of simple equations and, per position, the same variables and
/// called operators (parameters and constants aside). Only then can its
/// parameter guard stay a run-time branch (SPEC_0040 DAE-C22); any other
/// parameter guard is selected at translation.
pub(super) fn branches_structurally_equal(
    ctx: &crate::Context,
    cond_blocks: &[ast::EquationBlock],
    else_block: &Option<Vec<ast::Equation>>,
    prefix: &ast::QualifiedName,
    span: rumoca_core::Span,
) -> Result<bool, crate::errors::FlattenError> {
    let empty = Vec::new();
    let mut signature = None;
    for equations in cond_blocks
        .iter()
        .map(|block| &block.eqs)
        .chain(std::iter::once(else_block.as_ref().unwrap_or(&empty)))
    {
        let branch = super::expand_to_simple_equations(ctx, equations, prefix, span)?
            .iter()
            .map(|equation| {
                let mut names = equation_structure(ctx, &equation.lhs, prefix);
                names.extend(equation_structure(ctx, &equation.rhs, prefix));
                names
            })
            .collect::<Vec<_>>();
        match &signature {
            None => signature = Some(branch),
            Some(first) if *first != branch => return Ok(false),
            Some(_) => {}
        }
    }
    Ok(true)
}

fn equation_structure(
    ctx: &crate::Context,
    expression: &ast::Expression,
    prefix: &ast::QualifiedName,
) -> std::collections::BTreeSet<String> {
    let mut names = ast::collect_component_refs(expression)
        .iter()
        .map(ToString::to_string)
        .filter(|name| !crate::boolean_eval::names_parameter_or_constant(ctx, name, prefix))
        .collect::<std::collections::BTreeSet<_>>();
    let calls = std::cell::RefCell::new(Vec::new());
    ast::contains_function_call(expression, &|function, _| {
        calls.borrow_mut().push(format!("{function}()"));
        false
    });
    names.extend(calls.into_inner());
    names
}

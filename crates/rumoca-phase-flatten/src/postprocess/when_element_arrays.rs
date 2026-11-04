//! Give each element of an array a `when` assigns element-wise its own
//! variable.
//!
//! `when time > i then x[i] = time; end when` inside `for i` defines `x[1]`
//! and `x[2]` in separate `when`-equations (MLS §8.3.5). The canonical DAE
//! owns a discrete coordinate whole, so an array whose elements different
//! `when`-equations define has no single owner. The elements are separate
//! variables in fact: each is assigned only by its own `when`, holds its own
//! value between events and has its own `pre` (MLS §3.7.5).
//!
//! The pass replaces such an array `x[n]` by scalars `x[1]` ... `x[n]` (the
//! names the `when` targets already spell) and rewrites every reference: a
//! literal element becomes its scalar, the whole array becomes the array of
//! them, and anything else indexes that array. An array some `when` assigns
//! whole is left alone.
//!
//! A slice target `x[1,:] = u` is first split into its element assignments
//! `x[1,k] = u[k]` (MLS §10.6.1: an array equation is the equations of its
//! elements), so a `when` that fills an array row by row assigns it element
//! by element too.

use super::*;
use rumoca_core::{FallibleExpressionRewriter, FallibleStatementRewriter};
use std::collections::HashMap;

pub(crate) fn scalarize_when_element_arrays(flat: &mut flat::Model) -> Result<(), FlattenError> {
    split_slice_targets(flat);
    let arrays = element_assigned_arrays(flat);
    if arrays.is_empty() {
        return Ok(());
    }
    let mut rewriter = ElementReferences { arrays: &arrays };
    crate::functions::rewrite_model_expressions(flat, &mut rewriter)?;
    for (name, extents) in &arrays {
        scalarize(flat, name, extents);
    }
    Ok(())
}

/// Arrays with literal extents that some `when` assigns element by element
/// and none assigns whole.
fn element_assigned_arrays(flat: &flat::Model) -> HashMap<rumoca_core::VarName, Vec<i64>> {
    let mut targets = HashSet::new();
    for chain in &flat.when_chains {
        for branch in chain.branches() {
            collect_targets(&branch.equations, &mut targets);
        }
    }
    let mut arrays = HashMap::new();
    for target in &targets {
        if flat.variables.contains_key(target) {
            continue;
        }
        let Some((base, _)) = split_element(target.as_str()) else {
            continue;
        };
        let base = rumoca_core::VarName::new(base);
        let Some(variable) = flat.variables.get(&base) else {
            continue;
        };
        if variable.dims.is_empty()
            || variable.dims.iter().any(|extent| *extent <= 0)
            || targets.contains(&base)
        {
            continue;
        }
        arrays.insert(base, variable.dims.clone());
    }
    arrays
}

fn collect_targets(equations: &[flat::WhenEquation], targets: &mut HashSet<rumoca_core::VarName>) {
    for equation in equations {
        match equation {
            flat::WhenEquation::Assign { target, .. } => {
                targets.insert(target.clone());
            }
            flat::WhenEquation::Conditional {
                branches,
                else_branch,
                ..
            } => {
                for (_, branch) in branches {
                    collect_targets(branch, targets);
                }
                if let Some(branch) = else_branch {
                    collect_targets(branch, targets);
                }
            }
            _ => {}
        }
    }
}

/// Replace each `when` assignment to a slice `x[i,:]` of an array with literal
/// extents by the assignments of its elements.
fn split_slice_targets(flat: &mut flat::Model) {
    let variables = &flat.variables;
    for chain in &mut flat.when_chains {
        for branch in chain.branches_mut() {
            split_slices_in(&mut branch.equations, variables);
        }
    }
}

fn split_slices_in(
    equations: &mut Vec<flat::WhenEquation>,
    variables: &flat::VarNameIndexMap<flat::Variable>,
) {
    let mut split = Vec::with_capacity(equations.len());
    for mut equation in equations.drain(..) {
        if let flat::WhenEquation::Conditional {
            branches,
            else_branch,
            ..
        } = &mut equation
        {
            for (_, branch) in branches.iter_mut() {
                split_slices_in(branch, variables);
            }
            if let Some(branch) = else_branch {
                split_slices_in(branch, variables);
            }
        }
        match slice_elements(&equation, variables) {
            Some(elements) => split.extend(elements),
            None => split.push(equation),
        }
    }
    *equations = split;
}

/// The element assignments of a slice assignment; `None` when `equation` is
/// not one or its extents are not literal.
fn slice_elements(
    equation: &flat::WhenEquation,
    variables: &flat::VarNameIndexMap<flat::Variable>,
) -> Option<Vec<flat::WhenEquation>> {
    let flat::WhenEquation::Assign {
        target,
        value,
        span,
        origin,
    } = equation
    else {
        return None;
    };
    let (base, subscripts) = target.as_str().strip_suffix(']')?.rsplit_once('[')?;
    let subscripts = subscripts
        .split(',')
        .map(|subscript| match subscript.trim() {
            ":" => Some(None),
            index => index.parse::<i64>().ok().map(Some),
        })
        .collect::<Option<Vec<Option<i64>>>>()?;
    let dims = &variables.get(&rumoca_core::VarName::new(base))?.dims;
    if dims.len() != subscripts.len()
        || !subscripts.iter().any(Option::is_none)
        || dims.iter().any(|extent| *extent <= 0)
    {
        return None;
    }
    let free = subscripts
        .iter()
        .zip(dims)
        .filter(|(subscript, _)| subscript.is_none())
        .map(|(_, extent)| *extent)
        .collect::<Vec<_>>();
    let elements = index_tuples(&free)
        .into_iter()
        .map(|picks| {
            let mut free_picks = picks.iter();
            let indices = subscripts
                .iter()
                .map(|subscript| subscript.or_else(|| free_picks.next().copied()))
                .collect::<Option<Vec<i64>>>()
                .expect("one pick per colon");
            flat::WhenEquation::Assign {
                target: rumoca_core::VarName::new(element_name(base, &indices)),
                value: slice_value_element(value, &picks, *span),
                span: *span,
                origin: origin.clone(),
            }
        })
        .collect();
    Some(elements)
}

/// The element `picks` (one index per dimension) of an array-valued `value`:
/// pushed into a component reference's colons, through `pre`/`edge`/`change`,
/// an `if` and a literal array; otherwise the indexed expression.
fn slice_value_element(
    value: &rumoca_core::Expression,
    picks: &[i64],
    span: rumoca_core::Span,
) -> rumoca_core::Expression {
    use rumoca_core::{Expression, Subscript};
    let index = |value: i64| Subscript::Index { value, span };
    match value {
        Expression::VarRef {
            name, subscripts, ..
        } if subscripts.is_empty() => Expression::VarRef {
            name: name.clone(),
            subscripts: picks.iter().copied().map(index).collect(),
            span,
        },
        Expression::VarRef {
            name, subscripts, ..
        } if subscripts
            .iter()
            .filter(|subscript| matches!(subscript, Subscript::Colon { .. }))
            .count()
            == picks.len() =>
        {
            let mut picks = picks.iter().copied();
            let subscripts = subscripts
                .iter()
                .map(|subscript| match subscript {
                    Subscript::Colon { .. } => picks.next().map_or(subscript.clone(), index),
                    other => other.clone(),
                })
                .collect();
            Expression::VarRef {
                name: name.clone(),
                subscripts,
                span,
            }
        }
        Expression::BuiltinCall {
            function:
                function @ (rumoca_core::BuiltinFunction::Pre
                | rumoca_core::BuiltinFunction::Edge
                | rumoca_core::BuiltinFunction::Change),
            args,
            ..
        } if args.len() == 1 => Expression::BuiltinCall {
            function: *function,
            args: vec![slice_value_element(&args[0], picks, span)],
            span,
        },
        Expression::If {
            branches,
            else_branch,
            ..
        } => Expression::If {
            branches: branches
                .iter()
                .map(|(condition, branch)| {
                    (condition.clone(), slice_value_element(branch, picks, span))
                })
                .collect(),
            else_branch: Box::new(slice_value_element(else_branch, picks, span)),
            span,
        },
        Expression::Array {
            elements,
            kind: rumoca_core::ArrayConstructor::Array,
            ..
        } if picks
            .first()
            .and_then(|&first| usize::try_from(first - 1).ok())
            .is_some_and(|offset| offset < elements.len()) =>
        {
            let element = &elements[(picks[0] - 1) as usize];
            match &picks[1..] {
                [] => element.clone(),
                rest => slice_value_element(element, rest, span),
            }
        }
        other => Expression::Index {
            base: Box::new(other.clone()),
            subscripts: picks.iter().copied().map(index).collect(),
            span,
        },
    }
}

/// `x[1,2]` -> (`x`, [1, 2]).
fn split_element(name: &str) -> Option<(&str, Vec<i64>)> {
    let (base, indices) = name.strip_suffix(']')?.rsplit_once('[')?;
    let indices = indices
        .split(',')
        .map(|index| index.trim().parse().ok())
        .collect::<Option<Vec<i64>>>()?;
    Some((base, indices))
}

fn element_name(base: &str, indices: &[i64]) -> String {
    let indices = indices.iter().map(i64::to_string).collect::<Vec<_>>();
    format!("{base}[{}]", indices.join(","))
}

/// Every index tuple of `extents`, row-major.
fn index_tuples(extents: &[i64]) -> Vec<Vec<i64>> {
    extents.iter().fold(vec![Vec::new()], |tuples, extent| {
        tuples
            .into_iter()
            .flat_map(|prefix| {
                (1..=*extent).map(move |index| {
                    let mut tuple = prefix.clone();
                    tuple.push(index);
                    tuple
                })
            })
            .collect()
    })
}

struct ElementReferences<'a> {
    arrays: &'a HashMap<rumoca_core::VarName, Vec<i64>>,
}

impl ElementReferences<'_> {
    fn element_reference(
        &self,
        array: &rumoca_core::Reference,
        indices: &[i64],
        span: rumoca_core::Span,
    ) -> rumoca_core::Expression {
        let rendered = element_name(array.as_str(), indices);
        let name = match array.component_ref() {
            Some(component) => {
                let mut parts = component.parts().to_vec();
                if let Some(last) = parts.last_mut() {
                    last.subs = indices
                        .iter()
                        .map(|&value| rumoca_core::Subscript::Index { value, span })
                        .collect();
                }
                match component.with_replaced_parts(parts) {
                    Ok(component) => array.with_rewritten_component_reference(rendered, component),
                    Err(_) => array.with_var_name(rumoca_core::VarName::new(rendered)),
                }
            }
            None => array.with_var_name(rumoca_core::VarName::new(rendered)),
        };
        rumoca_core::Expression::VarRef {
            name,
            subscripts: Vec::new(),
            span,
        }
    }

    /// The whole array as nested array literals of its elements.
    fn whole(
        &self,
        array: &rumoca_core::Reference,
        extents: &[i64],
        prefix: &mut Vec<i64>,
        span: rumoca_core::Span,
        wrap: &impl Fn(rumoca_core::Expression) -> rumoca_core::Expression,
    ) -> rumoca_core::Expression {
        let Some((&extent, inner)) = extents.split_first() else {
            return wrap(self.element_reference(array, prefix, span));
        };
        let elements = (1..=extent)
            .map(|index| {
                prefix.push(index);
                let element = self.whole(array, inner, prefix, span, wrap);
                prefix.pop();
                element
            })
            .collect();
        rumoca_core::Expression::Array {
            elements,
            kind: rumoca_core::ArrayConstructor::Array,
            span,
        }
    }
}

impl FallibleExpressionRewriter for ElementReferences<'_> {
    type Error = FlattenError;

    fn rewrite_expression(
        &mut self,
        expression: &rumoca_core::Expression,
    ) -> Result<rumoca_core::Expression, FlattenError> {
        // `pre(x)` of the whole array is the array of the elements' `pre`.
        if let rumoca_core::Expression::BuiltinCall {
            function:
                function @ (rumoca_core::BuiltinFunction::Pre
                | rumoca_core::BuiltinFunction::Edge
                | rumoca_core::BuiltinFunction::Change),
            args,
            span,
        } = expression
            && let [
                rumoca_core::Expression::VarRef {
                    name, subscripts, ..
                },
            ] = args.as_slice()
            && subscripts.is_empty()
            && let Some(extents) = self.arrays.get(name.var_name())
        {
            let function = *function;
            let span = *span;
            return Ok(
                self.whole(name, extents, &mut Vec::new(), span, &|element| {
                    rumoca_core::Expression::BuiltinCall {
                        function,
                        args: vec![element],
                        span,
                    }
                }),
            );
        }
        let rumoca_core::Expression::VarRef {
            name,
            subscripts,
            span,
        } = expression
        else {
            return self.walk_expression(expression);
        };
        let Some(extents) = self.arrays.get(name.var_name()) else {
            return self.walk_expression(expression);
        };
        let literal = subscripts
            .iter()
            .map(|subscript| match subscript {
                rumoca_core::Subscript::Index { value, .. } => Some(*value),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        if let Some(indices) = literal
            && indices.len() == extents.len()
        {
            return Ok(self.element_reference(name, &indices, *span));
        }
        let whole = self.whole(name, extents, &mut Vec::new(), *span, &|element| element);
        if subscripts.is_empty() {
            return Ok(whole);
        }
        Ok(rumoca_core::Expression::Index {
            base: Box::new(whole),
            subscripts: self.rewrite_subscripts(subscripts)?,
            span: *span,
        })
    }
}

impl FallibleStatementRewriter for ElementReferences<'_> {}

fn scalarize(flat: &mut flat::Model, name: &rumoca_core::VarName, extents: &[i64]) {
    let Some(array) = flat.variables.shift_remove(name) else {
        return;
    };
    let type_name = flat.variable_type_names.shift_remove(name);
    for indices in index_tuples(extents) {
        let element = element_name(name.as_str(), &indices);
        let element_name = rumoca_core::VarName::new(&element);
        let instance_id = flat.materialize_instance(flat::InstanceRelation {
            owner: Some(array.instance_id),
            declaration: array
                .component_ref
                .as_ref()
                .map(rumoca_core::ComponentReference::target_def_id),
            indices: indices.clone().into_boxed_slice(),
            kind: flat::InstanceKind::Materialized,
        });
        let component_ref = array.component_ref.as_ref().and_then(|component| {
            let mut parts = component.parts().to_vec();
            parts.last_mut()?.subs = indices
                .iter()
                .map(|&value| rumoca_core::Subscript::Index {
                    value,
                    span: array.source_span,
                })
                .collect();
            component.with_replaced_parts(parts).ok()
        });
        let element_of = |expression: &Option<rumoca_core::Expression>| {
            expression
                .as_ref()
                .map(|expression| element_value(expression, &indices, array.source_span))
        };
        let variable = flat::Variable {
            instance_id,
            name: element_name.clone(),
            component_ref,
            dims: Vec::new(),
            binding: element_of(&array.binding),
            start: element_of(&array.start),
            min: element_of(&array.min),
            max: element_of(&array.max),
            nominal: element_of(&array.nominal),
            ..array.clone()
        };
        if let Some(type_name) = &type_name {
            flat.variable_type_names
                .insert(element_name.clone(), type_name.clone());
        }
        flat.add_variable(element_name, variable);
    }
}

/// One element of an array-valued attribute: a literal array's entry, a
/// scalar as itself (an `each` value), or the indexed expression.
fn element_value(
    expression: &rumoca_core::Expression,
    indices: &[i64],
    span: rumoca_core::Span,
) -> rumoca_core::Expression {
    match (expression, indices.split_first()) {
        (rumoca_core::Expression::Array { elements, .. }, Some((&first, rest)))
            if usize::try_from(first - 1).is_ok_and(|offset| offset < elements.len()) =>
        {
            element_value(&elements[(first - 1) as usize], rest, span)
        }
        (_, None) | (rumoca_core::Expression::Literal { .. }, _) => expression.clone(),
        _ => rumoca_core::Expression::Index {
            base: Box::new(expression.clone()),
            subscripts: indices
                .iter()
                .map(|&value| rumoca_core::Subscript::Index { value, span })
                .collect(),
            span,
        },
    }
}

//! Split a discrete-valued array equation into one equation per element.
//!
//! `legs.on = sel` over a component array collapses to
//! `{legs[1].on, legs[2].on} = sel`: one equation whose left side is an array
//! of separate Boolean variables. A discrete-valued equation defines one
//! coordinate by explicit assignment (MLS Appendix B), so each element is its
//! own equation `legs[k].on = sel[k]` (MLS §10.6.1, an array equation is the
//! equations of its elements).
//!
//! Only a left side that is a literal array of whole scalar variables of a
//! discrete type is split. A Real array equation keeps its compact form, which
//! the continuous partition owns as one array residual.
//!
//! The first element replaces the equation in place and the rest are appended,
//! so no row a structured family addresses moves.

use super::*;

pub(crate) fn split_discrete_array_equations(flat: &mut flat::Model) {
    let mut appended = Vec::new();
    let mut split = HashSet::new();
    for (row, equation) in flat.equations.iter_mut().enumerate() {
        let Some(mut elements) =
            discrete_element_equations(equation, &flat.variables, &flat.variable_type_names)
        else {
            continue;
        };
        *equation = elements.remove(0);
        appended.extend(elements);
        split.insert(row);
    }
    if split.is_empty() {
        return;
    }
    flat.equations.extend(appended);
    // A family whose template is the array form no longer describes its row;
    // the element equations are owned row by row.
    flat.structured_equations.retain(|family| {
        let rows = family
            .domain
            .scalar_count()
            .ok()
            .and_then(|points| points.checked_mul(family.equations_per_point))
            .map_or(1, |count| count.max(1));
        !(family.first_equation_index..family.first_equation_index.saturating_add(rows))
            .any(|row| split.contains(&row))
    });
}

fn discrete_element_equations(
    equation: &flat::Equation,
    variables: &flat::VarNameIndexMap<flat::Variable>,
    type_names: &flat::VarNameIndexMap<String>,
) -> Option<Vec<flat::Equation>> {
    let rumoca_core::Expression::Binary {
        op: rumoca_core::OpBinary::Sub,
        lhs,
        rhs,
        span,
    } = &equation.residual
    else {
        return None;
    };
    let rumoca_core::Expression::Array {
        elements,
        kind: rumoca_core::ArrayConstructor::Array,
        ..
    } = lhs.as_ref()
    else {
        return None;
    };
    if elements.len() < 2 || elements.len() != equation.scalar_count {
        return None;
    }
    let discrete = elements.iter().all(|element| {
        let rumoca_core::Expression::VarRef {
            name, subscripts, ..
        } = element
        else {
            return false;
        };
        subscripts.is_empty()
            && variables
                .get(name.var_name())
                .is_some_and(|variable| variable.dims.is_empty())
            && type_names.get(name.var_name()).is_some_and(|type_name| {
                matches!(type_name.as_str(), "Boolean" | "Integer" | "String")
            })
    });
    if !discrete {
        return None;
    }
    Some(
        elements
            .iter()
            .enumerate()
            .map(|(offset, element)| flat::Equation {
                residual: rumoca_core::Expression::Binary {
                    op: rumoca_core::OpBinary::Sub,
                    lhs: Box::new(element.clone()),
                    rhs: Box::new(element_of(rhs, offset, *span)),
                    span: *span,
                },
                span: equation.span,
                origin: equation.origin.clone(),
                scalar_count: 1,
            })
            .collect(),
    )
}

/// The `offset`th (zero-based) element of a one-dimensional array value.
fn element_of(
    array: &rumoca_core::Expression,
    offset: usize,
    span: rumoca_core::Span,
) -> rumoca_core::Expression {
    let index = rumoca_core::Subscript::Index {
        value: i64::try_from(offset + 1).expect("an array literal's length fits in i64"),
        span,
    };
    match array {
        rumoca_core::Expression::Array {
            elements,
            kind: rumoca_core::ArrayConstructor::Array,
            ..
        } if elements.len() > offset => elements[offset].clone(),
        rumoca_core::Expression::VarRef {
            name, subscripts, ..
        } if subscripts.is_empty() => rumoca_core::Expression::VarRef {
            name: name.clone(),
            subscripts: vec![index],
            span,
        },
        other => rumoca_core::Expression::Index {
            base: Box::new(other.clone()),
            subscripts: vec![index],
            span,
        },
    }
}

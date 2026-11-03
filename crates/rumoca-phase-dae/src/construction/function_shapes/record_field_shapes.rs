//! Proven shapes of the declared fields of record-typed function values.
//!
//! MLS §12.2: a record value's declared fields are readable through the joined
//! reference identity Flat renders, so each field carries its own proven shape
//! in the same environment as the value that declares it. Two record rules
//! feed a field's extents:
//!
//! * MLS §10.1: a field extent may read an earlier field of the same record
//!   (`Real y[:]; Real eta[size(y, 1)];`), so the sibling's own proven shape
//!   is readable under its bare field name while the later field resolves.
//! * MLS §12.4.4: a value declared with field modifiers
//!   (`output R r(y = {0, 1})`) carries them as the constructor default
//!   `R(y = {0, 1}, ...)`; that default is the entry value and fixes the extent
//!   of a flexible (`:`) field the body never resizes.

use super::*;

pub(super) fn bind_record_field_shapes(
    flat: &flat::Model,
    function: &rumoca_core::Function,
    values: &mut ShapeEnvironment,
) -> Result<(), ToDaeError> {
    for value in function
        .inputs
        .iter()
        .chain(&function.outputs)
        .chain(&function.locals)
    {
        bind_value_field_shapes(flat, value, values)?;
    }
    Ok(())
}

fn bind_value_field_shapes(
    flat: &flat::Model,
    value: &rumoca_core::FunctionParam,
    values: &mut ShapeEnvironment,
) -> Result<(), ToDaeError> {
    let defaults = constructor_default_args(value);
    let mut own_shapes: HashMap<VarName, Vec<(VarName, ValueShape)>> = HashMap::new();
    let mut direct_index = 0usize;
    for (path, parent, field) in record_field_projections(value, flat) {
        let mut shape = values.get(&parent).cloned().ok_or_else(|| {
            ToDaeError::unsupported_flat(
                "function shape proof",
                format!("record field `{path}` has no proven parent shape"),
                field.span,
            )
        })?;
        let entry = if parent.as_str() == value.name {
            direct_index += 1;
            defaults
                .and_then(|args| args.get(direct_index - 1))
                .filter(|_| has_flexible_axis(field))
                .and_then(|arg| call_free_expression_shape(arg, values))
        } else {
            None
        };
        let siblings = own_shapes
            .get(&parent)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let field_shape = resolve_field_shape(field, entry.as_ref(), siblings, values)?;
        own_shapes
            .entry(parent)
            .or_default()
            .push((VarName::new(&field.name), field_shape.clone()));
        shape.extend(field_shape);
        values.insert(path, shape);
    }
    Ok(())
}

/// Resolve one record field's declared shape with the record's earlier fields
/// (`siblings`, field name and own shape) readable under their bare names.
pub(super) fn resolve_field_shape(
    field: &rumoca_core::FunctionParam,
    actual: Option<&ValueShape>,
    siblings: &[(VarName, ValueShape)],
    values: &ShapeEnvironment,
) -> Result<ValueShape, ToDaeError> {
    if siblings.is_empty() || !has_expression_axis(field) {
        return resolve_declared_shape(field, actual, None, values);
    }
    let mut scoped = values.clone();
    for (name, sibling_shape) in siblings {
        scoped.insert(name.clone(), sibling_shape.clone());
    }
    resolve_declared_shape(field, actual, None, &scoped)
}

/// The positional arguments of a record-constructor default, in declared
/// field order (named arguments are already canonicalized by Flat).
fn constructor_default_args(value: &rumoca_core::FunctionParam) -> Option<&[Expression]> {
    match value.default.as_ref()? {
        Expression::FunctionCall {
            args,
            is_constructor: true,
            ..
        } if !args.iter().any(is_named_marker) => Some(args.as_slice()),
        _ => None,
    }
}

fn is_named_marker(argument: &Expression) -> bool {
    matches!(
        argument,
        Expression::FunctionCall { name, .. }
            if name.as_str().starts_with(rumoca_core::NAMED_FUNCTION_ARG_PREFIX)
    )
}

fn has_flexible_axis(field: &rumoca_core::FunctionParam) -> bool {
    field
        .shape_expr
        .iter()
        .any(|subscript| matches!(subscript, Subscript::Colon { .. }))
}

fn has_expression_axis(field: &rumoca_core::FunctionParam) -> bool {
    field
        .shape_expr
        .iter()
        .any(|subscript| matches!(subscript, Subscript::Expr { .. }))
}

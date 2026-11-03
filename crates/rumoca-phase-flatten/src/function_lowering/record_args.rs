//! Field projection through record-valued call arguments.
//!
//! A record argument of a decomposed function parameter is passed field by
//! field. An array of records (`{core, payload}`, `cat(1, {core}, arms)`, an
//! expanded record array) has no Flat coordinate of its own, so each field is
//! projected through the structure that builds it (MLS §§10.4.4, 10.6, 10.7).

use super::{
    expand_record_constructor_arg, expand_record_var_ref, record_field_access_source_span,
};
use crate::errors::FlattenError;
use rumoca_ir_flat as flat;
use std::collections::{HashMap, HashSet};

/// How names in a record-valued call argument resolve.
#[derive(Clone, Copy)]
pub(super) struct RecordArgContext<'a> {
    pub(super) local_record_params: Option<&'a HashSet<String>>,
    pub(super) aggregate_record_values: Option<&'a HashSet<String>>,
    pub(super) record_array_extents: &'a HashMap<String, Vec<i64>>,
    pub(super) empty_record_vectors: &'a HashSet<String>,
}

/// The extents of every expanded record array, read off the elements Flat
/// retains (`arms[1]`, `arms[2]` -> `arms: [2]`). An array whose elements are
/// not all present with literal indices is left out.
pub(super) fn record_array_extents(flat: &flat::Model) -> HashMap<String, Vec<i64>> {
    let mut extents: HashMap<String, (Vec<i64>, usize)> = HashMap::new();
    let mut unusable = HashSet::new();
    for record in flat.record_instances.values() {
        let parts = record.component_ref.parts();
        let Some((last, prefix)) = parts.split_last() else {
            continue;
        };
        if last.subs.is_empty() {
            continue;
        }
        let mut whole = prefix.to_vec();
        whole.push(rumoca_core::ComponentRefPart {
            subs: Vec::new(),
            ..last.clone()
        });
        let name = rumoca_core::ComponentPath::from_component_reference(&match record
            .component_ref
            .with_replaced_parts(whole)
        {
            Ok(reference) => reference,
            Err(_) => continue,
        })
        .to_flat_string();
        let indices = last
            .subs
            .iter()
            .map(|sub| match sub {
                rumoca_core::Subscript::Index { value, .. } if *value >= 1 => Some(*value),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        let Some(indices) = indices else {
            unusable.insert(name);
            continue;
        };
        let entry = extents
            .entry(name)
            .or_insert_with(|| (vec![0; indices.len()], 0));
        if entry.0.len() != indices.len() {
            entry.1 = usize::MAX;
            continue;
        }
        for (extent, index) in entry.0.iter_mut().zip(&indices) {
            *extent = (*extent).max(*index);
        }
        entry.1 = entry.1.saturating_add(1);
    }
    extents
        .into_iter()
        .filter(|(name, (extents, count))| {
            !unusable.contains(name)
                && extents.iter().try_fold(1usize, |product, &extent| {
                    product.checked_mul(extent as usize)
                }) == Some(*count)
        })
        .map(|(name, (extents, _))| (name, extents))
        .collect()
}

/// `array[indices]`: one element of an expanded record array.
fn record_array_element(
    array: &rumoca_core::Reference,
    indices: &[i64],
    span: rumoca_core::Span,
) -> Result<rumoca_core::Reference, FlattenError> {
    let missing = || {
        FlattenError::missing_resolved_class_metadata(
            array.as_str(),
            "record array element reference",
            span,
        )
    };
    let reference = array.component_ref().ok_or_else(missing)?;
    let mut parts = reference.parts().to_vec();
    let last = parts.last_mut().ok_or_else(missing)?;
    last.subs = indices
        .iter()
        .map(|&value| rumoca_core::Subscript::Index { value, span })
        .collect();
    let element = reference
        .with_replaced_parts(parts)
        .map_err(|_| missing())?;
    // The flat name may carry an enclosing instance prefix the scoped
    // component reference does not; extend the name the reference has.
    let rendered = format!(
        "{}[{}]",
        whole_array_name(array),
        indices
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(array
        .with_rewritten_component_reference(rendered, element)
        .without_resolved_function())
}

/// The flat name of the array `reference` names, without a trailing `[:]`.
fn whole_array_name(reference: &rumoca_core::Reference) -> &str {
    let name = reference.as_str();
    match name
        .strip_suffix(']')
        .and_then(|open| open.rsplit_once('['))
    {
        Some((base, subscripts))
            if subscripts
                .split(',')
                .all(|subscript| subscript.trim() == ":") =>
        {
            base
        }
        _ => name,
    }
}

/// Whether `subscripts` select every element (`[:, :]`).
fn selects_whole_array(subscripts: &[rumoca_core::Subscript]) -> bool {
    subscripts
        .iter()
        .all(|subscript| matches!(subscript, rumoca_core::Subscript::Colon { .. }))
}

/// What projecting one field out of a record-valued argument needs.
pub(super) struct RecordArgScope<'a> {
    pub(super) function_name: &'a str,
    pub(super) fields: &'a [rumoca_core::FunctionParam],
    pub(super) context: RecordArgContext<'a>,
}

impl RecordArgScope<'_> {
    /// `{array[1].field, ..., array[n].field}`, nested per dimension.
    fn project_elements(
        &self,
        array: &rumoca_core::Reference,
        extents: &[i64],
        prefix: &mut Vec<i64>,
        span: rumoca_core::Span,
        index: usize,
    ) -> Result<rumoca_core::Expression, FlattenError> {
        let Some((&extent, inner)) = extents.split_first() else {
            let element = record_array_element(array, prefix, span)?;
            return self.project(
                &rumoca_core::Expression::VarRef {
                    name: element,
                    subscripts: Vec::new(),
                    span,
                },
                index,
            );
        };
        let mut elements = Vec::with_capacity(extent as usize);
        for position in 1..=extent {
            prefix.push(position);
            elements.push(self.project_elements(array, inner, prefix, span, index)?);
            prefix.pop();
        }
        Ok(rumoca_core::Expression::Array {
            elements,
            kind: rumoca_core::ArrayConstructor::Array,
            span,
        })
    }

    /// The extents of the expanded record array `arg` names whole, if it does.
    ///
    /// Keyed by the occurrence, not the declaration: two instances of one
    /// class can size the same array differently, or not at all.
    pub(super) fn record_array_extents(&self, arg: &rumoca_core::Expression) -> Option<&Vec<i64>> {
        let rumoca_core::Expression::VarRef {
            name, subscripts, ..
        } = arg
        else {
            return None;
        };
        if !selects_whole_array(subscripts)
            || name
                .parts()
                .last()
                .is_some_and(|part| !selects_whole_array(&part.subs))
        {
            return None;
        }
        self.context
            .record_array_extents
            .get(whole_array_name(name))
    }

    /// Whether `operand` names a retained zero-length record vector whole.
    fn is_empty_record_vector(&self, operand: &rumoca_core::Expression) -> bool {
        let rumoca_core::Expression::VarRef {
            name, subscripts, ..
        } = operand
        else {
            return false;
        };
        selects_whole_array(subscripts)
            && self
                .context
                .empty_record_vectors
                .contains(whole_array_name(name))
    }

    /// `cat(k, a, b, ...).field` as `cat(k, a.field, b.field, ...)`.
    ///
    /// MLS §§10.4.4, 10.7: along the first axis an empty record vector
    /// contributes no element. Flat retains its typed container but no field
    /// of it (`a.m` of `R a[0]` is never instantiated), so it is dropped, not
    /// projected. `None` when every operand is empty: the field's element
    /// shape is then left to the typed owner rather than guessed.
    fn project_concatenation(
        &self,
        args: &[rumoca_core::Expression],
        index: usize,
    ) -> Result<Option<Vec<rumoca_core::Expression>>, FlattenError> {
        let Some((dimension, operands)) = args.split_first() else {
            return Ok(None);
        };
        let along_first = matches!(
            dimension,
            rumoca_core::Expression::Literal {
                value: rumoca_core::Literal::Integer(1),
                ..
            }
        );
        let mut projected = vec![dimension.clone()];
        for operand in operands {
            if !(along_first && self.is_empty_record_vector(operand)) {
                projected.push(self.project(operand, index)?);
            }
        }
        Ok((projected.len() > 1).then_some(projected))
    }

    /// `arg.field` kept as a projection: `arg` has no Flat coordinate of its own.
    fn field_access(
        &self,
        arg: &rumoca_core::Expression,
        index: usize,
    ) -> Result<rumoca_core::Expression, FlattenError> {
        let field = &self.fields[index];
        let source_span = record_field_access_source_span(arg, field)?;
        Ok(rumoca_core::Expression::FieldAccess {
            base: Box::new(arg.clone()),
            field: field.name.clone(),
            field_def_id: field.def_id.ok_or_else(|| {
                FlattenError::missing_resolved_class_metadata(
                    &field.name,
                    "record function field identity",
                    source_span,
                )
            })?,
            span: source_span,
        })
    }

    /// `arg.field` pushed through the structure that builds `arg`.
    ///
    /// An array of records (`{core, payload}`, `cat(1, {core}, arms)`, an
    /// `if` between two records) has no Flat coordinate of its own: its
    /// elements do. Projecting through the array constructor, `cat` and the
    /// branches leaves every element a field Flat owns (`core.mass`,
    /// `arms.mass`, a constructor's field argument), so the result is the
    /// array of that field (MLS §10.6, record arrays as arrays of fields).
    /// Anything else, such as a function's record result, keeps the
    /// projection as a `FieldAccess`.
    pub(super) fn project(
        &self,
        arg: &rumoca_core::Expression,
        index: usize,
    ) -> Result<rumoca_core::Expression, FlattenError> {
        use rumoca_core::Expression as E;
        Ok(match arg {
            E::Array {
                elements,
                kind,
                span,
            } => E::Array {
                elements: elements
                    .iter()
                    .map(|element| self.project(element, index))
                    .collect::<Result<_, _>>()?,
                kind: *kind,
                span: *span,
            },
            E::BuiltinCall {
                function: rumoca_core::BuiltinFunction::Cat,
                args,
                span,
            } if args.len() >= 2 => match self.project_concatenation(args, index)? {
                Some(args) => E::BuiltinCall {
                    function: rumoca_core::BuiltinFunction::Cat,
                    args,
                    span: *span,
                },
                None => self.field_access(arg, index)?,
            },
            // `records[:]` is `records`.
            E::Index {
                base, subscripts, ..
            } if selects_whole_array(subscripts) => self.project(base, index)?,
            E::If {
                branches,
                else_branch,
                span,
            } => E::If {
                branches: branches
                    .iter()
                    .map(|(condition, value)| Ok((condition.clone(), self.project(value, index)?)))
                    .collect::<Result<_, FlattenError>>()?,
                else_branch: Box::new(self.project(else_branch, index)?),
                span: *span,
            },
            // `records` names its elements, not an aggregate: `records[:].field`
            // is the spelling scalarization selects them through (see
            // `normalize_record_array_field_access_bindings`).
            // `records` names its elements, not an aggregate.
            E::VarRef { name, span, .. } if let Some(extents) = self.record_array_extents(arg) => {
                self.project_elements(name, extents, &mut Vec::new(), *span, index)?
            }
            E::FunctionCall {
                is_constructor: true,
                ..
            }
            | E::VarRef { .. } => {
                let mut projected = Vec::new();
                if !expand_record_constructor_arg(
                    self.function_name,
                    arg,
                    self.fields,
                    &mut projected,
                )? {
                    expand_record_var_ref(
                        arg,
                        self.fields,
                        self.context.local_record_params,
                        self.context.aggregate_record_values,
                        &mut projected,
                    )?;
                }
                let mut projected = projected.swap_remove(index);
                if let (
                    E::VarRef {
                        subscripts: element,
                        ..
                    },
                    E::VarRef { subscripts, .. },
                ) = (arg, &mut projected)
                    && subscripts.is_empty()
                {
                    subscripts.clone_from(element);
                }
                projected
            }
            _ => self.field_access(arg, index)?,
        })
    }
}

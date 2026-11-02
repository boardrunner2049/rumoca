//! Declare Real package constants instead of inlining their values.
//!
//! A reference such as `Modelica.Constants.pi` in an equation used to be
//! replaced by its number here, in the frontend. Replacing a named constant
//! by its value is an optimization, not lowering: the model means the same
//! with the name kept. So the frontend now declares the constant once, as a
//! model constant under its qualified name with its evaluated value as the
//! binding, and leaves the reference in place. The `inline-constants`
//! bitcode pass replaces the reference with the value when optimization is
//! asked for (docs/design/minimal-frontend.md).
//!
//! References are kept in equations, equation-family templates, assertion
//! conditions and variable attributes. Only Real scalars are declared, and
//! only constants owned
//! by their declaration. Integer, Boolean and enumeration constants select
//! branches, size arrays and index them, where lowering needs the literal;
//! a constant specialized per instance (a replaceable `Medium` package) has
//! no single qualified name. Those keep the frontend substitution.

use std::collections::HashSet;

use rumoca_core::FallibleExpressionRewriter;
use rumoca_ir_flat as flat;

use super::constant_expansion::SemanticConstantId;
use super::constant_lookup::resolve_source_constant;
use super::constant_substituter::substitute_known_constants_expr;
use crate::{Context, FlattenError};

/// One declared constant: its evaluated value and the reference that first
/// named it, whose provenance the declaration keeps.
struct Declared {
    value: rumoca_core::Expression,
    reference: rumoca_core::Reference,
    span: rumoca_core::Span,
}

pub(super) fn declare_real_package_constants(
    flat: &mut flat::Model,
    ctx: &Context,
) -> Result<(), FlattenError> {
    let live_vars: rustc_hash::FxHashSet<String> = flat
        .variables
        .keys()
        .map(|name| name.as_str().to_string())
        .collect();
    let mut declarer = Declarer {
        ctx,
        live_vars: &live_vars,
        declared: indexmap::IndexMap::new(),
    };
    for equation in flat
        .equations
        .iter_mut()
        .chain(flat.initial_equations.iter_mut())
    {
        equation.residual = declarer.rewrite_expression(&equation.residual)?;
    }
    for family in flat
        .structured_equations
        .iter_mut()
        .chain(flat.initial_structured_equations.iter_mut())
    {
        if let Some(template) = family.template.as_mut() {
            for body in &mut template.body {
                *body = declarer.rewrite_expression(body)?;
            }
        }
    }
    for assertion in flat
        .assert_equations
        .iter_mut()
        .chain(flat.initial_assert_equations.iter_mut())
    {
        assertion.condition = declarer.rewrite_expression(&assertion.condition)?;
    }
    // Attributes too: `parameter Real w = 2 * Modelica.Constants.pi` keeps
    // the name, and `inline-constants` with `fold-parameter-bindings` folds
    // it when optimization is asked for.
    for variable in flat.variables.values_mut() {
        for attribute in [
            &mut variable.binding,
            &mut variable.start,
            &mut variable.min,
            &mut variable.max,
            &mut variable.nominal,
        ]
        .into_iter()
        .flatten()
        {
            *attribute = declarer.rewrite_expression(attribute)?;
        }
    }
    let real = flat.predefined_scalar_type(flat.predefined_types.real);
    for (name, declared) in declarer.declared {
        let var_name = rumoca_core::VarName::new(&name);
        let instance_id = flat.materialize_instance(flat::InstanceRelation {
            owner: declared.reference.instance_id(),
            declaration: declared.reference.target_def_id(),
            indices: Box::default(),
            kind: flat::InstanceKind::Materialized,
        });
        let variable = flat::Variable {
            instance_id,
            name: var_name.clone(),
            component_ref: declared.reference.component_ref().cloned(),
            source_span: declared.span,
            type_id: real,
            variability: rumoca_core::Variability::Constant(rumoca_core::Token::default()),
            binding: Some(declared.value),
            is_primitive: true,
            is_protected: true,
            ..flat::Variable::empty_with_span(declared.span)
        };
        flat.variable_type_names
            .entry(var_name.clone())
            .or_insert_with(|| "Real".to_string());
        flat.add_variable(var_name, variable);
    }
    Ok(())
}

struct Declarer<'a> {
    ctx: &'a Context,
    live_vars: &'a rustc_hash::FxHashSet<String>,
    declared: indexmap::IndexMap<String, Declared>,
}

impl FallibleExpressionRewriter for Declarer<'_> {
    type Error = FlattenError;

    fn rewrite_expression(
        &mut self,
        expr: &rumoca_core::Expression,
    ) -> Result<rumoca_core::Expression, Self::Error> {
        if let rumoca_core::Expression::VarRef {
            name,
            subscripts,
            span,
        } = expr
            && subscripts.is_empty()
            && let Some(qualified) = self.declare(name, *span)?
        {
            return Ok(rumoca_core::Expression::VarRef {
                name: name.with_var_name(rumoca_core::VarName::new(qualified)),
                subscripts: Vec::new(),
                span: *span,
            });
        }
        self.walk_expression(expr)
    }
}

impl Declarer<'_> {
    /// The qualified name `reference` now names, when it is a Real package
    /// constant owned by its declaration.
    fn declare(
        &mut self,
        reference: &rumoca_core::Reference,
        span: rumoca_core::Span,
    ) -> Result<Option<String>, FlattenError> {
        if reference.is_generated() || self.live_vars.contains(reference.as_str()) {
            return Ok(None);
        }
        let Some((SemanticConstantId::Declaration(declaration), value)) =
            resolve_source_constant(reference, self.ctx)
        else {
            return Ok(None);
        };
        let Some(qualified) = self.ctx.target_def_names.get(&declaration) else {
            return Ok(None);
        };
        // A qualified name that is also a component path would alias it.
        if self.live_vars.contains(qualified.as_str()) {
            return Ok(None);
        }
        if self.declared.contains_key(qualified) {
            return Ok(Some(qualified.clone()));
        }
        // The value the frontend would have inlined, with any constants it
        // references resolved the same way.
        let value = substitute_known_constants_expr(
            value.clone(),
            self.ctx,
            self.live_vars,
            &HashSet::new(),
            "",
        )?;
        if !is_real_scalar_literal(&value) {
            return Ok(None);
        }
        self.declared.insert(
            qualified.clone(),
            Declared {
                value,
                reference: reference.clone(),
                span,
            },
        );
        Ok(Some(qualified.clone()))
    }
}

/// A finite Real literal: what the frontend would have inlined for a Real
/// scalar constant.
fn is_real_scalar_literal(value: &rumoca_core::Expression) -> bool {
    matches!(
        value,
        rumoca_core::Expression::Literal {
            value: rumoca_core::Literal::Real(number),
            ..
        } if number.is_finite()
    )
}

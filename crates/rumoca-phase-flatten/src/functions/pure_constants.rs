//! Prove, at translation, that no settled declaration binding indexes out of
//! bounds.
//!
//! Ordinary call folding belongs to the `fold-pure-calls` bitcode pass
//! (docs/design/minimal-frontend.md). Only the explicit fixed-input profile
//! specializes bindings here, before checked DAE construction. Default
//! compilation retains calls and performs the bounds proof alone.
use crate::FlattenError;
use rumoca_core::{Expression, ExpressionRewriter, Literal, Span, Variability};
use rumoca_eval_flat::constant::{EvalContext, EvalError, Value, eval_expr};
use rumoca_ir_flat as flat;

pub(crate) fn check_settled_binding_bounds(model: &flat::Model) -> Result<(), FlattenError> {
    check_bounds(model, &frozen_context(model))
}

fn frozen_context(model: &flat::Model) -> EvalContext {
    let mut context = EvalContext::new();
    for function in model.functions.values() {
        context.add_function(function.clone());
    }
    // Freeze is a whole fixed-parameter profile: a final/evaluated parameter
    // can still depend on a tunable parent, so no individual flag proves its
    // value immutable. Every fixed parameter must be non-tunable first.
    let fixed_parameters_frozen = model.variables.values().all(|variable| {
        !matches!(variable.variability, Variability::Parameter(_))
            || variable.fixed_uniform() == Some(false)
            || variable.evaluate
    });
    // A frozen parameter can depend on a later frozen parameter. Only insert
    // newly established values; the finite declaration set bounds this loop.
    loop {
        let mut changed = false;
        for (name, variable) in &model.variables {
            if !(matches!(variable.variability, Variability::Constant(_))
                || (matches!(variable.variability, Variability::Parameter(_))
                    && fixed_parameters_frozen
                    && variable.evaluate))
                || variable.fixed_uniform() == Some(false)
                || context.parameters.contains_key(name.as_str())
            {
                continue;
            }
            if let Some(binding) = &variable.binding
                && let Ok(value) = eval_expr(binding, &context)
            {
                context.parameters.insert(name.as_str().to_owned(), value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    context
}

fn check_bounds(model: &flat::Model, context: &EvalContext) -> Result<(), FlattenError> {
    // A declaration binding is evaluated unconditionally. Refuse only a typed
    // bounds proof here, never an unsupported body form or an unknown input.
    for variable in model.variables.values() {
        if let Some(binding) = &variable.binding
            && let Err(EvalError::IndexOutOfBounds { index, size, span }) =
                eval_expr(binding, context)
        {
            return Err(FlattenError::ConstantIndexOutOfBounds { index, size, span });
        }
    }
    Ok(())
}

/// Specialize only an explicitly requested fixed-parameter artifact. The
/// bounded evaluator refuses impure/external calls and unresolved inputs. A
/// refusal leaves the original expression for ordinary checked lowering;
/// neither a guessed result nor a runtime evaluator fallback is introduced.
pub(crate) fn specialize_frozen_bindings(model: &mut flat::Model) -> Result<(), FlattenError> {
    let context = frozen_context(model);
    check_bounds(model, &context)?;
    let mut folder = FrozenBindingFolder { context };
    for variable in model.variables.values_mut() {
        for expression in [
            &mut variable.binding,
            &mut variable.start,
            &mut variable.min,
            &mut variable.max,
            &mut variable.nominal,
        ]
        .into_iter()
        .flatten()
        {
            *expression = folder.rewrite_expression(expression);
        }
    }
    Ok(())
}

struct FrozenBindingFolder {
    context: EvalContext,
}

impl ExpressionRewriter for FrozenBindingFolder {
    fn rewrite_expression(&mut self, expression: &Expression) -> Expression {
        if let Expression::FunctionCall {
            is_constructor: false,
            span,
            ..
        } = expression
            && let Ok(value) = eval_expr(expression, &self.context)
            && let Some(literal) = literal_value(value, *span)
        {
            return literal;
        }
        self.walk_expression(expression)
    }
}

fn literal_value(value: Value, span: Span) -> Option<Expression> {
    let value = match value {
        Value::Real(value) if value.is_finite() => Literal::Real(value),
        Value::Integer(value) => Literal::Integer(value),
        Value::Bool(value) => Literal::Boolean(value),
        Value::String(value) => Literal::String(value),
        Value::Array(values) => {
            return Some(Expression::Array {
                elements: values
                    .into_iter()
                    .map(|value| literal_value(value, span))
                    .collect::<Option<_>>()?,
                kind: rumoca_core::ArrayConstructor::Array,
                span,
            });
        }
        _ => return None,
    };
    Some(Expression::Literal { value, span })
}

//! Calls of external functions whose body is one elementary math builtin.
//!
//! MLS §12.9 lets a function name its body `external "builtin"` (the language
//! reserved for functions the tool provides, MLS §12.9.1), and the MSL
//! declares `Modelica.Math.sin`, `atan2`, `exp`, ... that way. A C external
//! whose symbol is the C math library function of the same name, with the
//! same signature, means the same thing. A call of such a function *is* the
//! builtin applied to its arguments, so it is rewritten to the builtin here,
//! before any analysis: the call is then as pure, differentiable and
//! event-generating as the builtin, and no backend has to execute an opaque
//! foreign body for it. The declaration becomes unreachable and is pruned.

use std::collections::HashMap;

use crate::flat;
use rumoca_core::{BuiltinFunction, Expression, ExpressionRewriter, StatementRewriter, VarName};

/// Rewrite every call of an elementary external builtin to the builtin.
pub(crate) fn lower_elementary_external_builtins(flat: &mut flat::Model) {
    let builtins = flat
        .functions
        .iter()
        .filter_map(|(name, function)| {
            elementary_external_builtin(function).map(|builtin| (name.clone(), builtin))
        })
        .collect::<HashMap<_, _>>();
    if builtins.is_empty() {
        return;
    }
    super::model_rewrite::rewrite_model_expressions(flat, ElementaryBuiltins(&builtins));
}

#[derive(Clone, Copy)]
struct ElementaryBuiltins<'a>(&'a HashMap<VarName, BuiltinFunction>);

impl ExpressionRewriter for ElementaryBuiltins<'_> {
    fn rewrite_expression(&mut self, expr: &Expression) -> Expression {
        if let Expression::FunctionCall {
            name,
            args,
            is_constructor: false,
            span,
        } = expr
            && let Some(builtin) = self.0.get(name.var_name())
            && args.len() == builtin_arity(*builtin)
        {
            return Expression::BuiltinCall {
                function: *builtin,
                args: self.rewrite_expressions(args),
                span: *span,
            };
        }
        self.walk_expression(expr)
    }
}

impl StatementRewriter for ElementaryBuiltins<'_> {}

/// The builtin `function` is, when its whole body is the external
/// `y = f(u1, ..., un)` over scalar Real inputs in order.
fn elementary_external_builtin(function: &rumoca_core::Function) -> Option<BuiltinFunction> {
    let external = function.external.as_ref()?;
    if !matches!(external.language.as_str(), "builtin" | "C") {
        return None;
    }
    let symbol = external
        .function_name
        .as_deref()
        .unwrap_or_else(|| rumoca_core::top_level_last_segment(function.name.as_str()));
    let builtin = rumoca_core::elementary_math_builtin(symbol)?;
    let [output] = function.outputs.as_slice() else {
        return None;
    };
    let scalar_real = |param: &rumoca_core::FunctionParam| {
        param.shape_expr.is_empty() && param.type_name == "Real"
    };
    let passes_inputs_in_order = external.args.len() == function.inputs.len()
        && external
            .args
            .iter()
            .zip(&function.inputs)
            .all(|(argument, input)| is_plain_reference(argument, &input.name));
    (external.output_name.as_deref() == Some(output.name.as_str())
        && scalar_real(output)
        && function.inputs.iter().all(scalar_real)
        && passes_inputs_in_order
        && builtin_arity(builtin) == function.inputs.len())
    .then_some(builtin)
}

fn builtin_arity(builtin: BuiltinFunction) -> usize {
    if builtin == BuiltinFunction::Atan2 {
        2
    } else {
        1
    }
}

fn is_plain_reference(expression: &Expression, name: &str) -> bool {
    matches!(
        expression,
        Expression::VarRef { name: reference, subscripts, .. }
            if subscripts.is_empty() && reference.as_str() == name
    )
}

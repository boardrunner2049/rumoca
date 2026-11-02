//! Give a discrete external input the event history `pre`, `edge` and
//! `change` read.
//!
//! An input of the top-level model is supplied from outside, so the canonical
//! DAE has no left limit for it: nothing records its value at event entry.
//! `pre(u)` of a discrete-time input (MLS §3.7.5) is still defined -- a
//! Boolean, Integer, String or enumeration input changes only at events
//! (MLS §3.8.4) -- and is the value `u` had before the event.
//!
//! The pass declares one protected discrete variable per such input, defined
//! as the input itself (`u.history = u`, a declaration binding that holds at
//! every event iteration) and reads the history operators through it:
//! `pre(u)` becomes `pre(u.history)`, which the DAE owns like any other
//! discrete value's previous value. Inputs no history operator reads are left
//! alone.

use super::*;
use rumoca_core::{FallibleExpressionRewriter, FallibleStatementRewriter};
use std::collections::BTreeSet;

const HISTORY_SUFFIX: &str = "$history";

pub(crate) fn give_discrete_inputs_history(flat: &mut flat::Model) -> Result<(), FlattenError> {
    let inputs = discrete_external_inputs(flat);
    if inputs.is_empty() {
        return Ok(());
    }
    // Every expression the model lowers, algorithm `when` statements included:
    // the shared walk visits them in place, and the collector changes nothing.
    let mut collector = HistoryReads {
        inputs: &inputs,
        read: BTreeSet::new(),
    };
    crate::functions::rewrite_model_expressions(flat, &mut collector)?;
    if collector.read.is_empty() {
        return Ok(());
    }
    let read = collector.read;
    crate::functions::rewrite_model_expressions(flat, &mut HistoryRedirect { read: &read })?;
    for input in &read {
        declare_history(flat, input);
    }
    Ok(())
}

/// Inputs of the top-level model whose type is discrete-valued.
fn discrete_external_inputs(flat: &flat::Model) -> HashSet<rumoca_core::VarName> {
    flat.variables
        .iter()
        .filter(|(name, variable)| {
            matches!(variable.causality, rumoca_core::Causality::Input(_))
                && variable
                    .component_ref
                    .as_ref()
                    .and_then(|reference| reference.parts().first())
                    .is_some_and(|root| {
                        flat.top_level_input_components
                            .contains(root.ident.as_str())
                            || flat.top_level_connectors.contains(root.ident.as_str())
                    })
                && is_discrete_valued(flat, name, variable)
        })
        .map(|(name, _)| name.clone())
        .collect()
}

fn is_discrete_valued(
    flat: &flat::Model,
    name: &rumoca_core::VarName,
    variable: &flat::Variable,
) -> bool {
    flat.variable_type_names
        .get(name)
        .is_some_and(|type_name| matches!(type_name.as_str(), "Boolean" | "Integer" | "String"))
        || flat
            .effective_types
            .get(&variable.type_id)
            .is_some_and(|effective| flat.enumeration_types.contains(&effective.canonical_type()))
}

fn history_name(input: &rumoca_core::VarName) -> rumoca_core::VarName {
    rumoca_core::VarName::new(format!("{}{HISTORY_SUFFIX}", input.as_str()))
}

/// The input a history operator reads, when it reads one directly.
fn history_operand(
    expression: &rumoca_core::Expression,
) -> Option<(&rumoca_core::Reference, &[rumoca_core::Expression])> {
    let rumoca_core::Expression::BuiltinCall {
        function:
            rumoca_core::BuiltinFunction::Pre
            | rumoca_core::BuiltinFunction::Edge
            | rumoca_core::BuiltinFunction::Change,
        args,
        ..
    } = expression
    else {
        return None;
    };
    let [rumoca_core::Expression::VarRef { name, .. }, ..] = args.as_slice() else {
        return None;
    };
    Some((name, args))
}

struct HistoryReads<'a> {
    inputs: &'a HashSet<rumoca_core::VarName>,
    read: BTreeSet<rumoca_core::VarName>,
}

impl FallibleExpressionRewriter for HistoryReads<'_> {
    type Error = FlattenError;

    fn rewrite_expression(
        &mut self,
        expression: &rumoca_core::Expression,
    ) -> Result<rumoca_core::Expression, FlattenError> {
        if let Some((name, _)) = history_operand(expression)
            && self.inputs.contains(name.var_name())
        {
            self.read.insert(name.var_name().clone());
        }
        self.walk_expression(expression)
    }
}

impl FallibleStatementRewriter for HistoryReads<'_> {}

struct HistoryRedirect<'a> {
    read: &'a BTreeSet<rumoca_core::VarName>,
}

impl FallibleStatementRewriter for HistoryRedirect<'_> {}

impl FallibleExpressionRewriter for HistoryRedirect<'_> {
    type Error = FlattenError;

    fn rewrite_expression(
        &mut self,
        expression: &rumoca_core::Expression,
    ) -> Result<rumoca_core::Expression, FlattenError> {
        let rumoca_core::Expression::BuiltinCall {
            function,
            args,
            span,
        } = expression
        else {
            return self.walk_expression(expression);
        };
        let Some((name, _)) = history_operand(expression) else {
            return self.walk_expression(expression);
        };
        if !self.read.contains(name.var_name()) {
            return self.walk_expression(expression);
        }
        let mut args = args.clone();
        if let rumoca_core::Expression::VarRef { name, .. } = &mut args[0] {
            *name = history_reference(name);
        }
        Ok(rumoca_core::Expression::BuiltinCall {
            function: *function,
            args,
            span: *span,
        })
    }
}

/// `name` with its last component renamed to the history variable's.
fn history_reference(name: &rumoca_core::Reference) -> rumoca_core::Reference {
    let rendered = history_name(name.var_name());
    let Some(component) = name.component_ref() else {
        return name.with_var_name(rendered);
    };
    let mut parts = component.parts().to_vec();
    if let Some(last) = parts.last_mut() {
        last.ident.push_str(HISTORY_SUFFIX);
    }
    match component.with_replaced_parts(parts) {
        Ok(component) => name.with_rewritten_component_reference(rendered.as_str(), component),
        Err(_) => name.with_var_name(rendered),
    }
}

fn declare_history(flat: &mut flat::Model, input: &rumoca_core::VarName) {
    let Some(source) = flat.variables.get(input).cloned() else {
        return;
    };
    let name = history_name(input);
    let reference = rumoca_core::Reference::from_var_name(input.clone());
    let reference = match &source.component_ref {
        Some(component) => {
            reference.with_rewritten_component_reference(input.as_str(), component.clone())
        }
        None => reference,
    };
    let reference = reference.with_instance_id(source.instance_id);
    let instance_id = flat.materialize_instance(flat::InstanceRelation {
        owner: Some(source.instance_id),
        declaration: source
            .component_ref
            .as_ref()
            .map(rumoca_core::ComponentReference::target_def_id),
        indices: Box::default(),
        kind: flat::InstanceKind::Materialized,
    });
    let component_ref = source.component_ref.as_ref().and_then(|component| {
        let mut parts = component.parts().to_vec();
        parts.last_mut()?.ident.push_str(HISTORY_SUFFIX);
        component.with_replaced_parts(parts).ok()
    });
    let variable = flat::Variable {
        instance_id,
        name: name.clone(),
        component_ref,
        type_id: source.type_id,
        dims: source.dims.clone(),
        start: source.start.clone(),
        fixed: source.fixed,
        variability: rumoca_core::Variability::Discrete(rumoca_core::Token::default()),
        causality: rumoca_core::Causality::Empty,
        binding: Some(rumoca_core::Expression::VarRef {
            name: reference,
            subscripts: Vec::new(),
            span: source.source_span,
        }),
        is_primitive: true,
        is_protected: true,
        ..flat::Variable::empty_with_span(source.source_span)
    };
    if let Some(type_name) = flat.variable_type_names.get(input).cloned() {
        flat.variable_type_names.insert(name.clone(), type_name);
    }
    flat.add_variable(name, variable);
}

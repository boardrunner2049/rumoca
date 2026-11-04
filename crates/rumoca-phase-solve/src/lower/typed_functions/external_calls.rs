//! Pure-call owners whose body is a cataloged MLS §12.9 external function.
//!
//! The owner keeps the Modelica interface of the declaring function (its
//! parameters and results in declaration order) and its body is the one
//! `External` operation of the catalog entry its C symbol names. Any other
//! external body is refused here with its name and symbol.

use super::*;

/// The owner of an external function, or the named refusal.
pub(super) fn register_external_body<'dae>(
    table: &mut solve::SolvePureCallTableBuilder,
    view: dae::DaeView<'dae>,
    function: dae::FunctionView<'dae>,
    external: dae::ExternalFunctionView<'dae>,
    identity: solve::SolvePureCallIdentity,
    arithmetic: solve::SolveArithmeticProfile,
    provenance: rumoca_core::Span,
) -> Result<RegisteredCall<'dae>, solve::SolveProgramConstructionError> {
    let refused = || solve::SolveProgramConstructionError::UnsupportedExternalFunction {
        function: function.name().as_str().into(),
        language: external.language().as_str(),
        symbol: external.symbol().as_str().into(),
        provenance,
    };
    let catalog = dae::catalog_external(view, function).ok_or_else(refused)?;
    let input = single_leaf(view, function.parameter_types().iter().next(), arithmetic)
        .ok_or_else(refused)?;
    let mut outputs = Vec::with_capacity(2);
    for value_type in function.result_types().iter() {
        let leaf = single_leaf(view, Some(value_type), arithmetic).ok_or_else(refused)?;
        outputs.push(solve::SolvePureCallOutput::result(leaf));
    }
    let owner = table.add_owner(
        identity,
        vec![input],
        outputs,
        provenance,
        |builder, inputs, outputs| {
            let state = builder.load(inputs[0], provenance)?;
            let [draw, next] = builder.external(catalog.function, &[state], provenance)?[..] else {
                unreachable!("a generator returns its draw and its next state")
            };
            builder.store(outputs[catalog.draw_result], draw, provenance)?;
            builder.store(outputs[catalog.state_result()], next, provenance)
        },
    )?;
    let site = table
        .call_site(owner)
        .ok_or(solve::SolveProgramConstructionError::UnknownCallOwner { provenance })?;
    Ok(RegisteredCall {
        owner,
        site,
        result_ranges: vec![0..1, 1..2].into_boxed_slice(),
        result_leaf_count: 2,
        assertion_slots: std::sync::Arc::from(Vec::new()),
        assertions: Box::new([]),
    })
}

fn single_leaf<'dae>(
    view: dae::DaeView<'dae>,
    value_type: Option<dae::ValueTypeId<'dae>>,
    arithmetic: solve::SolveArithmeticProfile,
) -> Option<solve::SolveValueType> {
    let leaves = lower_value_type_leaves(view, value_type?, arithmetic).ok()?;
    let [leaf] = <[_; 1]>::try_from(leaves).ok()?;
    Some(leaf)
}

/// Whether a call of `function` reaches an MLS §12.9 external body, directly
/// or through the functions it calls.
///
/// Only the typed pure-call owner graph can invoke a foreign body (as its
/// cataloged `External` operation), so such a call is always lowered as a
/// typed pure call rather than inlined into a scalar program.
pub(crate) fn reaches_external_body<'dae>(
    view: dae::DaeView<'dae>,
    function: dae::FunctionId<'dae>,
) -> bool {
    let mut pending = vec![function];
    let mut seen = std::collections::HashSet::new();
    while let Some(function) = pending.pop() {
        if !seen.insert(function) {
            continue;
        }
        let Some(definition) = view.function(function) else {
            continue;
        };
        if definition.is_external() {
            return true;
        }
        let assertions = assertion_conditions(view, definition).unwrap_or_default();
        for call in nested_calls(view, definition, &assertions) {
            if let Some(dae::ExpressionOperation::Call { function, .. }) =
                view.expression(call).map(|node| node.operation())
            {
                pending.push(function);
            }
        }
    }
    false
}

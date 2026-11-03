//! DAE + Solve lowering into the checked FMI description of the problem.

use std::collections::HashMap;

use rumoca_core::Span;
use rumoca_ir_dae as dae;
use rumoca_ir_solve::fmi::{
    FmiCausality, FmiComponent, FmiComponentError, FmiDeclaredCausality, FmiVariability,
    FmiVariableInput,
};
use rumoca_ir_solve::{SolveVariableStorageRole, SolveVariableValueKind};
use serde::{Deserialize, Serialize};

use crate::{SolveModelLoweringError, lower_solve_model};

#[derive(Debug, thiserror::Error)]
pub enum FmiLoweringError {
    #[error("unsupported FMI scalar type `{kind:?}` for `{variable}`")]
    UnsupportedScalarType {
        variable: String,
        kind: dae::ScalarType,
        span: Span,
    },
    #[error("numeric FMI metadata for `{variable}` is invalid: {message}")]
    NumericMetadata {
        variable: String,
        message: String,
        span: Span,
    },
    #[error(
        "the declared start {start} of input `{variable}` lies outside its range \
         [{minimum}, {maximum}]; FMI requires min <= start <= max"
    )]
    InputStartOutOfRange {
        variable: String,
        start: f64,
        minimum: f64,
        maximum: f64,
        span: Span,
    },
    #[error(transparent)]
    SolveModel(#[from] SolveModelLoweringError),
    #[error(transparent)]
    Component(#[from] FmiComponentError),
    #[error("FMI component wire is invalid: {0}")]
    Wire(String),
}

impl FmiLoweringError {
    #[must_use]
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::UnsupportedScalarType { span, .. }
            | Self::NumericMetadata { span, .. }
            | Self::InputStartOutOfRange { span, .. } => Some(*span),
            Self::SolveModel(error) => error.source_span(),
            Self::Component(error) => error.span(),
            Self::Wire(_) => None,
        }
    }
}

/// Current FMI component wire schema; every other version is rejected.
///
/// 2 carries each variable's declared `input`/`output` prefix where it differs
/// from the exported causality.
pub const FMI_COMPONENT_SCHEMA_VERSION: u16 = 2;

/// Borrowed, canonical construction inputs for one correlated FMI component.
#[derive(Serialize)]
pub struct FmiComponentWireRef<'model> {
    schema_version: u16,
    solve_model: crate::SolveModelWireRef<'model>,
    variables: Vec<FmiVariableInput>,
}

/// Encode the still-correlated phase result without exposing either half as
/// an independently selectable runtime input.
pub fn fmi_component_wire<'wire>(
    lowered: &'wire crate::LoweredSolveModel<'_>,
) -> Result<FmiComponentWireRef<'wire>, FmiLoweringError> {
    let variables = lowered.prepared_dae().inspect(lower_variables)?;
    let solve_model = crate::solve_model_wire(lowered.model())
        .map_err(|error| FmiLoweringError::Wire(error.to_string()))?;
    Ok(FmiComponentWireRef {
        schema_version: FMI_COMPONENT_SCHEMA_VERSION,
        solve_model,
        variables,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FmiComponentWire {
    schema_version: u16,
    #[serde(deserialize_with = "crate::deserialize_solve_model")]
    solve_model: rumoca_ir_solve::SolveModel,
    variables: Vec<FmiVariableInput>,
}

/// Replay the canonical FMI wire through the same checked component
/// construction used by fresh lowering.
pub fn deserialize_fmi_component<'de, D>(deserializer: D) -> Result<FmiComponent, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let wire = FmiComponentWire::deserialize(deserializer)?;
    if wire.schema_version != FMI_COMPONENT_SCHEMA_VERSION {
        return Err(serde::de::Error::custom(format!(
            "unsupported FMI component schema {}; expected {}",
            wire.schema_version, FMI_COMPONENT_SCHEMA_VERSION
        )));
    }
    FmiComponent::construct(wire.solve_model, wire.variables).map_err(serde::de::Error::custom)
}

/// Project one checked DAE into one checked executable FMI component.
///
/// Every input here is a fact the Modelica declaration owns. The FMI-only
/// maximum-step-duration local of SPEC_0044 §8 is not one of them: the checked
/// component derives it from the delay partition of the kernel below, so this
/// lowering neither names it nor decides whether it exists.
pub fn lower_to_fmi_component(
    dae: &dae::Dae,
    overrides: &HashMap<String, f64>,
) -> Result<FmiComponent, FmiLoweringError> {
    let lowered = lower_solve_model(dae, overrides, |_| {})?;
    finish_fmi_component(lowered)
}

/// Consume one still-correlated complete Solve lowering into its checked FMI
/// component.
///
/// The prepared DAE and executable root cannot be supplied independently: the
/// only input is the phase-owned aggregate returned by [`lower_solve_model`].
pub fn finish_fmi_component(
    lowered: crate::LoweredSolveModel<'_>,
) -> Result<FmiComponent, FmiLoweringError> {
    let inputs = lowered.prepared_dae().inspect(lower_variables)?;
    let solve = lowered.into_model();
    FmiComponent::construct(solve, inputs).map_err(Into::into)
}

fn lower_variables(view: dae::DaeView<'_>) -> Result<Vec<FmiVariableInput>, FmiLoweringError> {
    let mut numeric = rumoca_eval_dae::NumericEvaluator::new(view);
    view.variables()
        .map(|(_, variable)| lower_variable(view, &mut numeric, variable))
        .collect()
}

fn lower_variable<'dae>(
    view: dae::DaeView<'dae>,
    numeric: &mut rumoca_eval_dae::NumericEvaluator<'dae>,
    variable: dae::VariableView<'dae>,
) -> Result<FmiVariableInput, FmiLoweringError> {
    let scalar_count = variable.scalar_count();
    let role = solve_role(variable.role());
    let causality = fmi_causality(variable.causality());
    let value_kind = solve_value_kind(variable)?;
    let minimum = published_minimum(numeric, variable)?;
    let maximum = numeric_attribute(numeric, variable, variable.maximum())?;
    // A host-written input publishes exactly the value it is seeded with
    // before the importer's first write. An input whose declaration equation
    // has a static value (`input Real g[3] = {0, 0, 9.81}`, or a bound field
    // of an input record) holds that value until the importer writes it, so
    // that value is its start. Any other declaration with neither a `start`
    // nor a binding holds its type's default start.
    let start = if let Some(binding) = static_input_binding(view, variable) {
        numeric_attribute(numeric, variable, Some(binding))?
    } else if is_host_written_input(variable) {
        input_start(
            view,
            numeric,
            variable,
            minimum.as_deref(),
            maximum.as_deref(),
        )?
    } else {
        match numeric_attribute(numeric, variable, variable.start())? {
            Some(values) => Some(values),
            None => match numeric_attribute(numeric, variable, variable.binding())? {
                Some(values) => Some(values),
                None => unmodified_start(variable, minimum.as_deref(), maximum.as_deref()),
            },
        }
    }
    .unwrap_or_else(|| vec![0.0; scalar_count]);
    Ok(FmiVariableInput {
        name: variable.name().to_string(),
        scalar_names: (0..scalar_count)
            .filter_map(|index| variable.scalar_name(index))
            .collect(),
        role,
        value_kind,
        dimensions: variable.value_type().dimensions().to_vec(),
        start,
        minimum,
        maximum,
        nominal: numeric_attribute(numeric, variable, variable.nominal())?,
        unit: variable.unit().map(str::to_owned),
        description: variable.description().map(str::to_owned),
        causality,
        declared_causality: declared_causality(variable.declared_causality())
            .filter(|declared| declared.exported() != causality),
        variability: fmi_variability(variable),
        tunable: variable.is_tunable(),
        evaluable: variable.is_evaluable(),
        declaration: variable.declaration().span(),
        text_start: text_start(view, variable),
    })
}

/// The per-scalar literal start of a `String` declaration: its `start`
/// attribute, else its binding, when that is a string literal or an array of
/// string literals (a scalar literal broadcasts over an array declaration).
/// Any other expression leaves the text start absent, and a consumer that
/// needs one refuses the declaration rather than inventing a value.
fn text_start<'dae>(
    view: dae::DaeView<'dae>,
    variable: dae::VariableView<'dae>,
) -> Option<Vec<String>> {
    if variable.value_type().scalar_type() != dae::ScalarType::String {
        return None;
    }
    let expression = variable.start().or_else(|| variable.binding())?;
    let mut values = Vec::new();
    collect_string_literals(view, expression, &mut values)?;
    let count = variable.scalar_count();
    dae::broadcast_scalar_values(&mut values, count);
    (values.len() == count).then_some(values)
}

fn collect_string_literals<'dae>(
    view: dae::DaeView<'dae>,
    expression: dae::ExprId<'dae>,
    values: &mut Vec<String>,
) -> Option<()> {
    match view.expression(expression)?.operation() {
        dae::ExpressionOperation::Literal(dae::DaeLiteral::String(text)) => {
            values.push(text.clone());
            Some(())
        }
        dae::ExpressionOperation::Array(elements) => {
            let mut index = 0;
            while let Some(element) = elements.get(index) {
                collect_string_literals(view, element, values)?;
                index += 1;
            }
            Some(())
        }
        // A literal filled over the declaration broadcasts like a scalar
        // binding: `fill(text, n)` or an iterator-free comprehension.
        dae::ExpressionOperation::Builtin {
            builtin: dae::PureBuiltin::Fill,
            arguments,
        } => collect_string_literals(view, arguments.get(0)?, values),
        dae::ExpressionOperation::Comprehension { body, .. } => {
            let start = values.len();
            collect_string_literals(view, body, values)?;
            (values.len() == start + 1).then_some(())
        }
        _ => None,
    }
}

fn numeric_attribute<'dae>(
    numeric: &mut rumoca_eval_dae::NumericEvaluator<'dae>,
    variable: dae::VariableView<'dae>,
    expression: Option<dae::ExprId<'dae>>,
) -> Result<Option<Vec<f64>>, FmiLoweringError> {
    let Some(expression) = expression else {
        return Ok(None);
    };
    if variable.scalar_count() == 0 {
        return Ok(Some(Vec::new()));
    }
    if !matches!(
        variable.value_type().scalar_type(),
        dae::ScalarType::Real
            | dae::ScalarType::Integer
            | dae::ScalarType::Boolean
            | dae::ScalarType::Enumeration
    ) {
        return Ok(None);
    }
    let mut values =
        numeric
            .expression(expression)
            .map_err(|error| FmiLoweringError::NumericMetadata {
                variable: variable.name().to_string(),
                message: error.to_string(),
                span: error.span(),
            })?;
    variable.broadcast_values(&mut values);
    if values.len() != variable.scalar_count() {
        return Err(FmiLoweringError::NumericMetadata {
            variable: variable.name().to_string(),
            message: format!(
                "attribute has {} scalars; expected {}",
                values.len(),
                variable.scalar_count()
            ),
            span: variable.declaration().span(),
        });
    }
    Ok(Some(values))
}

/// The lower bound a declaration publishes: its declared `min`, else, for an
/// enumeration, its first literal (ordinal 1). Every enumeration value is a
/// literal, and an enumeration exports as an ordinal-valued Int32/Integer
/// whose own default minimum would otherwise admit ordinals that name none.
fn published_minimum<'dae>(
    numeric: &mut rumoca_eval_dae::NumericEvaluator<'dae>,
    variable: dae::VariableView<'dae>,
) -> Result<Option<Vec<f64>>, FmiLoweringError> {
    let declared = numeric_attribute(numeric, variable, variable.minimum())?;
    Ok(declared.or_else(|| {
        (variable.value_type().scalar_type() == dae::ScalarType::Enumeration)
            .then(|| vec![1.0; variable.scalar_count()])
    }))
}

/// The declaration equation of an input when it has a static (constant or
/// parameter) value: the value the input holds before the importer's first
/// write.
fn static_input_binding<'dae>(
    view: dae::DaeView<'dae>,
    variable: dae::VariableView<'dae>,
) -> Option<dae::ExprId<'dae>> {
    if variable.role() != dae::VariableRole::Input {
        return None;
    }
    let binding = variable.binding()?;
    let variability = view.expression(binding)?.variability();
    matches!(
        variability,
        dae::ExpressionVariability::Constant | dae::ExpressionVariability::Parameter
    )
    .then_some(binding)
}

/// A top-level input with no binding: its value is whatever the host (an FMU
/// importer, a GPU driver) writes, and its `start` is only the value it holds
/// before the first write.
fn is_host_written_input(variable: dae::VariableView<'_>) -> bool {
    variable.role() == dae::VariableRole::Input && variable.binding().is_none()
}

/// The per-scalar start a declaration holds when nothing modifies it: its
/// predefined type's default start (MLS §4.9: `0`, `false`, or the first
/// enumeration literal), inside the declared range (see [`within_range`]).
/// `String` and record declarations have no numeric start, so they have none.
fn unmodified_start(
    variable: dae::VariableView<'_>,
    minimum: Option<&[f64]>,
    maximum: Option<&[f64]>,
) -> Option<Vec<f64>> {
    let default = variable.value_type().scalar_type().default_start_value()?;
    let mut values = vec![default; variable.scalar_count()];
    within_range(&mut values, minimum, maximum);
    Some(values)
}

/// One scalar's declared bound, when the declaration has one.
fn bound(bounds: Option<&[f64]>, scalar: usize) -> Option<f64> {
    bounds.and_then(|bounds| bounds.get(scalar)).copied()
}

/// Move every scalar that lies outside the declaration's own `min`/`max` to
/// the nearest bound. FMI requires `min <= start <= max` on every variable.
fn within_range(values: &mut [f64], minimum: Option<&[f64]>, maximum: Option<&[f64]>) {
    for (scalar, value) in values.iter_mut().enumerate() {
        if let Some(low) = bound(minimum, scalar).filter(|low| *value < *low) {
            *value = low;
        }
        if let Some(high) = bound(maximum, scalar).filter(|high| *value > *high) {
            *value = high;
        }
    }
}

/// The per-scalar value a host-written input holds before the host's first
/// write: its `start`, which FMI requires to lie in its declared range.
///
/// A start the model does not write (the language default start of MLS §4.9,
/// materialized with `DefaultStart` provenance, or none at all) is only the
/// pre-write value the exporter picks, so it is moved to the nearest bound
/// when the range excludes it: `input Real a(min = 1)` starts at 1. A start
/// the model writes outside the range is a model error and is refused rather
/// than silently replaced by a value the model never states.
fn input_start<'dae>(
    view: dae::DaeView<'dae>,
    numeric: &mut rumoca_eval_dae::NumericEvaluator<'dae>,
    variable: dae::VariableView<'dae>,
    minimum: Option<&[f64]>,
    maximum: Option<&[f64]>,
) -> Result<Option<Vec<f64>>, FmiLoweringError> {
    let Some(expression) = variable.start() else {
        return Ok(unmodified_start(variable, minimum, maximum));
    };
    let Some(mut values) = numeric_attribute(numeric, variable, Some(expression))? else {
        return Ok(None);
    };
    let provenance = view
        .expression(expression)
        .map(dae::ExpressionView::provenance);
    let defaulted = provenance.is_some_and(|provenance| {
        provenance.origin() == dae::DaeProvenanceOrigin::Generated(dae::DaeGeneration::DefaultStart)
    });
    let out_of_range = values.iter().enumerate().find(|&(scalar, value)| {
        *value < bound(minimum, scalar).unwrap_or(f64::NEG_INFINITY)
            || *value > bound(maximum, scalar).unwrap_or(f64::INFINITY)
    });
    if !defaulted && let Some((scalar, &start)) = out_of_range {
        return Err(FmiLoweringError::InputStartOutOfRange {
            variable: variable
                .scalar_name(scalar)
                .unwrap_or_else(|| variable.name().to_string()),
            start,
            minimum: bound(minimum, scalar).unwrap_or(f64::NEG_INFINITY),
            maximum: bound(maximum, scalar).unwrap_or(f64::INFINITY),
            span: provenance
                .map_or_else(|| variable.declaration().span(), dae::DaeProvenance::span),
        });
    }
    within_range(&mut values, minimum, maximum);
    Ok(Some(values))
}

/// The per-scalar value an unbound top-level input holds before its host's
/// first write (see [`input_start`]); the same value the FMI description
/// publishes as its start. `None` for an input without a numeric start (a
/// `String` input).
///
/// FMI export, GPU preparation and the bitcode simulator's `--free-inputs`
/// all read the pre-write value of an input here, so one input starts from
/// one value wherever it runs.
pub fn host_input_start_values<'dae>(
    view: dae::DaeView<'dae>,
    variable: dae::VariableView<'dae>,
) -> Result<Option<Vec<f64>>, FmiLoweringError> {
    let mut numeric = rumoca_eval_dae::NumericEvaluator::new(view);
    let minimum = published_minimum(&mut numeric, variable)?;
    let maximum = numeric_attribute(&mut numeric, variable, variable.maximum())?;
    input_start(
        view,
        &mut numeric,
        variable,
        minimum.as_deref(),
        maximum.as_deref(),
    )
}

/// Evaluate one numeric declaration attribute through the phase-owned DAE
/// evaluator and apply the declaration's scalar-broadcast rule.
pub fn numeric_attribute_values<'dae>(
    view: dae::DaeView<'dae>,
    variable: dae::VariableView<'dae>,
    expression: Option<dae::ExprId<'dae>>,
) -> Result<Option<Vec<f64>>, FmiLoweringError> {
    numeric_attribute(
        &mut rumoca_eval_dae::NumericEvaluator::new(view),
        variable,
        expression,
    )
}

const fn solve_role(role: dae::VariableRole) -> SolveVariableStorageRole {
    match role {
        dae::VariableRole::Parameter => SolveVariableStorageRole::Parameter,
        dae::VariableRole::Constant => SolveVariableStorageRole::Constant,
        dae::VariableRole::Input => SolveVariableStorageRole::ExternalInput,
        dae::VariableRole::State => SolveVariableStorageRole::State,
        dae::VariableRole::Algebraic => SolveVariableStorageRole::Algebraic,
        dae::VariableRole::Output => SolveVariableStorageRole::Output,
        dae::VariableRole::DiscreteReal => SolveVariableStorageRole::DiscreteReal,
        dae::VariableRole::DiscreteValue => SolveVariableStorageRole::DiscreteValue,
    }
}

fn solve_value_kind(
    variable: dae::VariableView<'_>,
) -> Result<SolveVariableValueKind, FmiLoweringError> {
    match variable.value_type().scalar_type() {
        dae::ScalarType::Real => Ok(SolveVariableValueKind::Real),
        dae::ScalarType::Integer => Ok(SolveVariableValueKind::Integer),
        dae::ScalarType::Boolean => Ok(SolveVariableValueKind::Boolean),
        dae::ScalarType::Enumeration => Ok(SolveVariableValueKind::Enumeration),
        dae::ScalarType::String => Ok(SolveVariableValueKind::String),
        kind @ dae::ScalarType::Record => Err(FmiLoweringError::UnsupportedScalarType {
            variable: variable.name().to_string(),
            kind,
            span: variable.declaration().span(),
        }),
    }
}

const fn fmi_causality(causality: dae::VariableCausality) -> FmiCausality {
    match causality {
        dae::VariableCausality::Input => FmiCausality::Input,
        dae::VariableCausality::Output => FmiCausality::Output,
        dae::VariableCausality::Parameter => FmiCausality::Parameter,
        dae::VariableCausality::CalculatedParameter => FmiCausality::CalculatedParameter,
        dae::VariableCausality::Independent => FmiCausality::Independent,
        dae::VariableCausality::Local => FmiCausality::Local,
    }
}

const fn declared_causality(causality: dae::DeclaredCausality) -> Option<FmiDeclaredCausality> {
    match causality {
        dae::DeclaredCausality::None => None,
        dae::DeclaredCausality::Input => Some(FmiDeclaredCausality::Input),
        dae::DeclaredCausality::Output => Some(FmiDeclaredCausality::Output),
    }
}

fn fmi_variability(variable: dae::VariableView<'_>) -> FmiVariability {
    match variable.variability() {
        dae::ExpressionVariability::Constant => FmiVariability::Constant,
        dae::ExpressionVariability::Parameter if variable.is_tunable() => FmiVariability::Tunable,
        dae::ExpressionVariability::Parameter => FmiVariability::Fixed,
        dae::ExpressionVariability::Discrete => FmiVariability::Discrete,
        dae::ExpressionVariability::Continuous => FmiVariability::Continuous,
    }
}

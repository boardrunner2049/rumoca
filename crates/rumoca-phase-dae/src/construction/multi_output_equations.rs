use super::*;

/// The discrete owners a model multi-output equation may define.
pub(super) struct MultiOutputDiscreteOwners<'scope, 'dae> {
    pub(super) discrete_values: &'scope mut DiscreteValueStaging<'dae>,
    pub(super) topology: &'scope DiscreteValueTopologyPlan,
    /// The clock of the partition the equation belongs to, when it is clocked.
    pub(super) owner_clock: Option<dae::ClockId<'dae>>,
}

/// The receiving tuple and the called function of one multi-result equation.
struct MultiOutputSource<'flat> {
    receivers: &'flat [Expression],
    name: &'flat rumoca_core::Reference,
    arguments: &'flat [Expression],
    provenance: dae::DaeProvenance,
}

/// The receiving tuple and call of `(a, b, ...) = f(...)`; any other residual
/// is not a multi-result equation.
fn multi_output_source(
    equation: &flat::Equation,
) -> Result<MultiOutputSource<'_>, dae::DaeConstructionError> {
    let invalid = || dae::DaeConstructionError::InvalidExpressionForm {
        span: equation.span,
    };
    let Expression::Binary {
        op: OpBinary::Sub,
        lhs,
        rhs,
        ..
    } = &equation.residual
    else {
        return Err(invalid());
    };
    let (
        Expression::Tuple { elements, .. },
        Expression::FunctionCall {
            name,
            args,
            is_constructor: false,
            span,
        },
    ) = (lhs.as_ref(), rhs.as_ref())
    else {
        return Err(invalid());
    };
    Ok(MultiOutputSource {
        receivers: elements,
        name,
        arguments: args,
        provenance: dae::DaeProvenance::source(*span)?,
    })
}

/// One retained receiver: its result ordinal, its target, and its call result.
type ReceiverResult<'flat, 'dae> = (usize, &'flat VarName, dae::ExprId<'dae>);

/// The owner that defines the discrete-valued receivers of one equation.
struct DiscreteValueDefinition<'scope, 'dae> {
    staging: &'scope mut DiscreteValueStaging<'dae>,
    owner: DiscreteValueOwnerHandle,
}

pub(super) fn lower_multi_output_equation<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    coordinates: &HashMap<VarName, Coordinate<'dae>>,
    functions: &FunctionRegistry<'_, 'dae>,
    equation: &flat::Equation,
    plan: &MultiOutputEquationPlan,
    owner: dae::DaeProvenance,
    discrete: Option<MultiOutputDiscreteOwners<'_, 'dae>>,
) -> Result<(), dae::DaeConstructionError> {
    let source = multi_output_source(equation)?;
    let symbols = LoweringSymbols {
        coordinates,
        functions,
        shapes: functions.shapes.model_values(),
        function_body: None,
        values: None,
        owner_clock: discrete.as_ref().and_then(|discrete| discrete.owner_clock),
    };
    let selected = plan
        .outputs
        .iter()
        .enumerate()
        .filter_map(|(ordinal, target)| target.as_ref().map(|target| (ordinal, target)))
        .collect::<Vec<_>>();
    let results = receiver_results(construction, symbols, &source, &selected)?;
    let initialization = discrete.is_none();
    let mut definition = discrete_value_definition(coordinates, &selected, discrete, owner)?;
    for (ordinal, target, value) in results {
        match (coordinates[target], definition.as_mut()) {
            (Coordinate::DiscreteValue(target), Some(definition)) => {
                definition.staging.always(
                    definition.owner,
                    target,
                    value,
                    owner,
                    source.provenance,
                )?;
            }
            (coordinate, _) => define_residual_receiver(
                construction,
                (coordinates, functions),
                (&source.receivers[ordinal], value),
                coordinate,
                (owner, initialization),
            )?,
        }
    }
    Ok(())
}

/// The call result of every retained receiver, in receiver order.
///
/// The continuous receivers read one shared call. Every discrete receiver
/// belongs to its own owner and reads its result from its own call of the same
/// pure function, which the analysis proved yields the same value.
fn receiver_results<'flat, 'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    symbols: LoweringSymbols<'_, 'dae>,
    source: &MultiOutputSource<'_>,
    selected: &[(usize, &'flat VarName)],
) -> Result<Vec<ReceiverResult<'flat, 'dae>>, dae::DaeConstructionError> {
    let call = |construction: &mut dae::DaeConstruction<'dae>| {
        lower_call_operands(
            construction,
            symbols,
            &HashMap::new(),
            source.name,
            source.arguments,
            source.provenance,
        )
    };
    let (discrete, continuous): (Vec<_>, Vec<_>) =
        selected.iter().copied().partition(|(_, target)| {
            matches!(
                symbols.coordinates[*target],
                Coordinate::DiscreteReal(_) | Coordinate::DiscreteValue(_)
            )
        });
    let shared = call(construction)?.results(
        construction,
        continuous.iter().map(|(ordinal, _)| *ordinal),
        source.provenance,
    )?;
    let mut results = continuous
        .into_iter()
        .zip(shared)
        .map(|((ordinal, target), value)| (ordinal, target, value))
        .collect::<Vec<_>>();
    for (ordinal, target) in discrete {
        let own = call(construction)?.results(construction, [ordinal], source.provenance)?;
        results.extend(own.into_iter().map(|value| (ordinal, target, value)));
    }
    results.sort_by_key(|(ordinal, _, _)| *ordinal);
    Ok(results)
}

/// The planned B.1c owner of the discrete-valued receivers, if any. Analysis
/// admits discrete receivers only in model equations, so an initialization
/// equation has none.
fn discrete_value_definition<'scope, 'dae>(
    coordinates: &HashMap<VarName, Coordinate<'dae>>,
    selected: &[(usize, &VarName)],
    discrete: Option<MultiOutputDiscreteOwners<'scope, 'dae>>,
    owner: dae::DaeProvenance,
) -> Result<Option<DiscreteValueDefinition<'scope, 'dae>>, dae::DaeConstructionError> {
    let targets = selected
        .iter()
        .filter(|(_, target)| matches!(coordinates[*target], Coordinate::DiscreteValue(_)))
        .map(|(_, target)| (*target).clone())
        .collect::<Vec<_>>();
    let Some(discrete) = discrete.filter(|_| !targets.is_empty()) else {
        return Ok(None);
    };
    let handle = discrete
        .discrete_values
        .owner(owner, targets, coordinates, discrete.topology)?;
    Ok(handle.map(|owner| DiscreteValueDefinition {
        staging: discrete.discrete_values,
        owner,
    }))
}

/// A continuous, initialization, or discrete Real receiver is defined by the
/// residual `receiver - result`.
fn define_residual_receiver<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    (coordinates, functions): (
        &HashMap<VarName, Coordinate<'dae>>,
        &FunctionRegistry<'_, 'dae>,
    ),
    (receiver, value): (&Expression, dae::ExprId<'dae>),
    coordinate: Coordinate<'dae>,
    (owner, initialization): (dae::DaeProvenance, bool),
) -> Result<(), dae::DaeConstructionError> {
    let lhs = lower_expression(construction, coordinates, functions, receiver, None)?;
    let residual = generated_residual(construction, owner, lhs, value)?;
    match coordinate {
        Coordinate::DiscreteReal(_) => {
            construction.discrete(|system| {
                system.real_equation(owner, |equation| equation.residual(residual))
            })?;
        }
        _ if initialization => {
            construction.initialization(|system| system.value_equation(owner, residual))?;
        }
        _ => {
            construction.continuous(|system| system.value_equation(owner, residual))?;
        }
    }
    Ok(())
}

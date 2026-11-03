use super::*;

pub(super) fn lower_clock_transfer<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    symbols: LoweringSymbols<'_, 'dae>,
    binders: &HashMap<VarName, dae::DomainBinderId<'dae>>,
    function: BuiltinFunction,
    arguments: &[Expression],
    provenance: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    let (source, input) =
        clock_transfer_input(function, arguments, symbols.functions, provenance.span())?;
    let source_plan = expression_clock_plan(source, symbols.functions)?.ok_or(
        dae::DaeConstructionError::MissingClockDomainOwner {
            span: provenance.span(),
        },
    )?;
    let source_clock = symbols
        .functions
        .clocks
        .id(&source_plan, provenance.span())?;
    let target_clock =
        symbols
            .owner_clock
            .ok_or(dae::DaeConstructionError::MissingClockDomainOwner {
                span: provenance.span(),
            })?;
    let kind = match input {
        TransferInput::Exact(kind) => kind,
        // MLS §16.5.2: the omitted factor is the one clock analysis proved
        // relates the source partition to this target partition.
        TransferInput::Inferred => symbols
            .functions
            .clocks
            .lattice(target_clock)
            .zip(source_plan.lattice())
            .and_then(|(target, source)| inferred_clock_transfer(function, source, target))
            .ok_or(dae::DaeConstructionError::InvalidClockedOperand {
                operator: function.name(),
                span: provenance.span(),
            })?,
    };
    let mut source_symbols = symbols;
    source_symbols.owner_clock = Some(source_clock);
    let source = lower_expression_scoped(construction, source_symbols, binders, source, None)?;
    construction.expressions(|expressions| {
        expressions
            .at(provenance)
            .clock_transfer(kind, source, source_clock, target_clock)
    })
}

/// The transfer one clock conversion states: exact when every argument is
/// given, inferred for `subSample(u)` / `superSample(u)` (MLS §16.5.2), whose
/// factor follows from the source and target partitions' clocks.
enum TransferInput {
    Exact(dae::ClockTransferKind),
    Inferred,
}

/// The source operand and MLS §16.5.2 transfer of one clock conversion.
fn clock_transfer_input<'expression>(
    function: BuiltinFunction,
    arguments: &'expression [Expression],
    functions: &FunctionRegistry<'_, '_>,
    span: Span,
) -> Result<(&'expression Expression, TransferInput), dae::DaeConstructionError> {
    let invalid = || dae::DaeConstructionError::InvalidClockedOperand {
        operator: function.name(),
        span,
    };
    let integer = |expression: &Expression| {
        eval_expr(expression, functions.constants)
            .ok()
            .and_then(|value| value.as_integer())
            .ok_or_else(invalid)
    };
    match (function, arguments) {
        (BuiltinFunction::SubSample | BuiltinFunction::SuperSample, [source]) => {
            Ok((source, TransferInput::Inferred))
        }
        (BuiltinFunction::SubSample, [source, factor]) => Ok((
            source,
            TransferInput::Exact(dae::ClockTransferKind::SubSample {
                factor: integer(factor)?,
            }),
        )),
        (BuiltinFunction::SuperSample, [source, factor]) => Ok((
            source,
            TransferInput::Exact(dae::ClockTransferKind::SuperSample {
                factor: integer(factor)?,
            }),
        )),
        (BuiltinFunction::ShiftSample, [source, counter]) => Ok((
            source,
            TransferInput::Exact(dae::ClockTransferKind::ShiftSample {
                counter: integer(counter)?,
                resolution: 1,
            }),
        )),
        (BuiltinFunction::ShiftSample, [source, counter, resolution]) => Ok((
            source,
            TransferInput::Exact(dae::ClockTransferKind::ShiftSample {
                counter: integer(counter)?,
                resolution: integer(resolution)?,
            }),
        )),
        (BuiltinFunction::BackSample, [source, counter]) => Ok((
            source,
            TransferInput::Exact(dae::ClockTransferKind::BackSample {
                counter: integer(counter)?,
                resolution: 1,
            }),
        )),
        (BuiltinFunction::BackSample, [source, counter, resolution]) => Ok((
            source,
            TransferInput::Exact(dae::ClockTransferKind::BackSample {
                counter: integer(counter)?,
                resolution: integer(resolution)?,
            }),
        )),
        _ => Err(invalid()),
    }
}

/// The clock partition an operand belongs to, `None` when the operand alone
/// does not name one; a malformed transfer operand or lattice is reported.
fn expression_clock_plan(
    expression: &Expression,
    functions: &FunctionRegistry<'_, '_>,
) -> Result<Option<ClockPlan>, dae::DaeConstructionError> {
    if let Expression::BuiltinCall {
        function,
        args,
        span,
        ..
    } = expression
        && matches!(
            function,
            BuiltinFunction::SubSample
                | BuiltinFunction::SuperSample
                | BuiltinFunction::ShiftSample
                | BuiltinFunction::BackSample
        )
    {
        let (source, input) = clock_transfer_input(*function, args, functions, *span)?;
        // An inferred factor is fixed only by the conversion's target partition,
        // which an operand alone does not name.
        let TransferInput::Exact(kind) = input else {
            return Ok(None);
        };
        let Some(source) = expression_clock_plan(source, functions)? else {
            return Ok(None);
        };
        let invalid = dae::DaeConstructionError::InvalidClockedOperand {
            operator: function.name(),
            span: *span,
        };
        // An event clock has no lattice: its only exact transfer is a
        // `shiftSample` by whole ticks (MLS §16.5.2).
        if source.event().is_some() {
            let dae::ClockTransferKind::ShiftSample {
                counter,
                resolution,
            } = kind
            else {
                return Err(invalid);
            };
            return source
                .shift_event(counter, resolution, *span)
                .and_then(Result::ok)
                .map(Some)
                .ok_or(invalid);
        }
        let source_lattice = source.lattice().ok_or(invalid)?;
        let lattice = match kind {
            dae::ClockTransferKind::SubSample { factor } => source_lattice.sub_sample(factor),
            dae::ClockTransferKind::SuperSample { factor } => source_lattice.super_sample(factor),
            dae::ClockTransferKind::ShiftSample {
                counter,
                resolution,
            } => source_lattice.shift_sample(counter, resolution),
            dae::ClockTransferKind::BackSample {
                counter,
                resolution,
            } => source_lattice.back_sample(counter, resolution),
        };
        let lattice = match lattice {
            Ok(lattice) => lattice,
            Err(source) => {
                return Err(dae::DaeConstructionError::InvalidClockLattice {
                    source,
                    span: *span,
                });
            }
        };
        return Ok(Some(ClockPlan::periodic(lattice, *span)));
    }
    let mut owner = None;
    collect_expression_clock_plan(expression, functions, &mut owner);
    Ok(owner)
}

fn collect_expression_clock_plan(
    expression: &Expression,
    functions: &FunctionRegistry<'_, '_>,
    owner: &mut Option<ClockPlan>,
) {
    if let Expression::VarRef { name, .. } = expression
        && let Some(variable) = functions.flat.variables.get(name.var_name())
        && let Some(plan) = functions
            .clocked_coordinate_owners
            .get(&variable.instance_id)
    {
        debug_assert!(owner.is_none_or(|existing| existing.schedule == plan.schedule));
        owner.get_or_insert(*plan);
    }
    for child in expression_children(expression) {
        collect_expression_clock_plan(child, functions, owner);
    }
}

/// MLS §16.10 Operator 16.15 `interval()` of an event clock: `startInterval`
/// at the first tick, and the time since the previous tick afterwards.
pub(super) fn lower_event_interval<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    symbols: LoweringSymbols<'_, 'dae>,
    binders: &HashMap<VarName, dae::DomainBinderId<'dae>>,
    clock: dae::ClockId<'dae>,
    provenance: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    let parts = symbols
        .functions
        .clocks
        .event_interval(clock, provenance.span())?;
    let start = match parts.start_interval {
        Some(start) => lower_expression_scoped(construction, symbols, binders, start, None)?,
        None => construction.expressions(|expressions| {
            expressions
                .at(provenance)
                .literal(dae::DaeLiteral::Real(0.0))
        })?,
    };
    construction.expressions(|expressions| {
        let indicator = expressions
            .at(provenance)
            .coordinate(dae::CoordinateInput::Previous(parts.first_tick))?;
        let half = expressions
            .at(provenance)
            .literal(dae::DaeLiteral::Real(0.5))?;
        let first =
            expressions
                .at(provenance)
                .binary(dae::BinaryOperator::Greater, indicator, half)?;
        let now = expressions
            .at(provenance)
            .coordinate(dae::CoordinateInput::Time)?;
        let last = expressions
            .at(provenance)
            .coordinate(dae::CoordinateInput::Previous(parts.last_tick))?;
        let elapsed =
            expressions
                .at(provenance)
                .binary(dae::BinaryOperator::Subtract, now, last)?;
        expressions
            .at(provenance)
            .conditional([(first, start)], elapsed)
    })
}

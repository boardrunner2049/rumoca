//! Rumoca Bitcode v2 → checked DAE.
//!
//! Import is deliberately *not* a deserializer that fills in structs. It runs
//! [`crate::validate()`] first, then rebuilds the model by issuing the DAE's own
//! checked construction operations. Every one of the DAE's construction
//! invariants therefore applies to imported bitcode exactly as it applies to a
//! freshly compiled model — an artifact that has been through an untrusted
//! external pass cannot produce an invalid DAE, only a rejection.
//!
//! # Source text
//!
//! DAE provenance is checked against the source map: a span must lie inside
//! its file and on character boundaries. An artifact exported with
//! `embed_sources` round-trips exactly. One exported without it is still
//! importable — filler of the required length is synthesised so spans stay
//! valid — but `source_text()` then returns filler rather than the original
//! program. Export embeds sources by default for this reason.

use rumoca_core::{SourceMap, Span};
use rumoca_ir_dae as dae;

mod clocks;
mod functions;
mod quotients;
mod strings;
mod temporal;
mod transactions;

use crate::schema::*;
use crate::validate::{ValidateOptions, ValidationError, validate};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("bitcode failed validation:\n{}", .0.iter().map(|error| format!("  - {error}")).collect::<Vec<_>>().join("\n"))]
    Invalid(Vec<ValidationError>),
    /// The DAE's checked constructors refused this artifact.
    ///
    /// The message is rendered rather than typed: `DaeConstructionError` is an
    /// internal type, and exposing it here would make the compiler's error
    /// taxonomy part of this crate's public API.
    #[error("cannot rebuild a checked DAE from this bitcode: {0}")]
    Construction(String),
    #[error("{0}")]
    Unsupported(String),
}

/// Validate an artifact and rebuild a checked DAE from it.
pub fn import(file: &RbcFile) -> Result<dae::Dae, ImportError> {
    file.check_header().map_err(ImportError::Unsupported)?;
    let model = &file.model;

    validate(
        model,
        &ValidateOptions {
            // A model containing a node the schema could not represent cannot be
            // faithfully rebuilt, so import refuses it rather than silently
            // dropping behaviour.
            reject_unsupported: true,
        },
    )
    .map_err(ImportError::Invalid)?;

    let (source_map, sources) = rebuild_source_map(model);

    let detail = std::cell::RefCell::new(None);
    let dae = dae::Dae::construct(source_map, |construction| {
        rebuild(construction, model, &sources, &detail)
    })
    .map_err(|error| match detail.into_inner() {
        Some(message) => ImportError::Unsupported(message),
        None => ImportError::Construction(error.to_string()),
    })?;
    Ok(dae)
}

/// Rebuild a source map whose ids match what the spans will reference.
///
/// `SourceId` is derived from the file name, so re-adding by name reproduces
/// the original identity. Returns the map plus the RBC-index → `SourceId`
/// translation.
fn rebuild_source_map(model: &RbcModel) -> (SourceMap, Vec<rumoca_core::SourceId>) {
    // A span must fit inside its file, so a source without embedded text needs
    // filler at least as long as the furthest offset that references it.
    let mut needed = vec![0usize; model.sources.len()];
    let mut note = |span: RbcSpan| {
        let slot = span.source.0 as usize;
        if let Some(entry) = needed.get_mut(slot) {
            *entry = (*entry).max(span.end as usize);
        }
    };
    for variable in &model.variables {
        note(variable.declaration.span);
    }
    for expression in &model.expressions {
        note(expression.provenance.span);
    }
    for equation in model.equations.iter().chain(&model.initial_equations) {
        note(equation.provenance.span);
    }
    for relation in &model.relations {
        note(relation.provenance.span);
    }
    for condition in &model.conditions {
        note(condition.provenance.span);
    }
    for clock in &model.clocks {
        note(clock.provenance.span);
    }
    for owner in &model.clock_ownerships {
        note(owner.provenance.span);
    }
    for root in &model.roots {
        note(root.provenance.span);
    }
    for event in &model.events {
        note(event.provenance.span);
    }
    for event in &model.time_events {
        note(event.provenance.span);
    }
    for connection in &model.connections {
        note(connection.provenance.span);
    }
    for value in model
        .initial_discrete_values
        .iter()
        .chain(&model.initial_parameter_values)
    {
        note(value.provenance.span);
    }
    // A carried function body anchors its values, statements and loops in
    // the source too, often further into a file than any equation does.
    for span in crate::validate::function_spans(model) {
        note(span);
    }
    for span in transactions::spans(model) {
        note(span);
    }
    for span in temporal::spans(model) {
        note(span);
    }

    let mut map = SourceMap::new();
    let mut ids = Vec::with_capacity(model.sources.len());
    for (index, source) in model.sources.iter().enumerate() {
        let text = match &source.text {
            Some(text) if text.len() >= needed[index] => text.clone(),
            // ASCII filler keeps every offset on a character boundary.
            _ => " ".repeat(needed[index]),
        };
        ids.push(map.add(&source.name, &text));
    }
    (map, ids)
}

fn span_of(span: RbcSpan, sources: &[rumoca_core::SourceId]) -> Span {
    let source = sources
        .get(span.source.0 as usize)
        .copied()
        .unwrap_or_default();
    Span {
        source,
        start: rumoca_core::BytePos(span.start as usize),
        end: rumoca_core::BytePos(span.end as usize),
    }
}

fn provenance_of(
    provenance: RbcProvenance,
    sources: &[rumoca_core::SourceId],
) -> Result<dae::DaeProvenance, ImportError> {
    let span = span_of(provenance.span, sources);
    let result = match provenance.origin {
        RbcOrigin::Source => dae::DaeProvenance::source(span),
        RbcOrigin::Generated { generation } => {
            dae::DaeProvenance::generated(generation_of(generation)?, span)
        }
    };
    result.map_err(|error| ImportError::Construction(error.to_string()))
}

fn generation_of(generation: RbcGeneration) -> Result<dae::DaeGeneration, ImportError> {
    use RbcGeneration as R;
    Ok(match generation {
        R::SyntheticResidual => dae::DaeGeneration::SyntheticResidual,
        R::BindingEquation => dae::DaeGeneration::BindingEquation,
        R::ConnectionEquation => dae::DaeGeneration::ConnectionEquation,
        R::FlowBalanceEquation => dae::DaeGeneration::FlowBalanceEquation,
        R::AlgorithmEquation => dae::DaeGeneration::AlgorithmEquation,
        R::DiscreteUpdate => dae::DaeGeneration::DiscreteUpdate,
        R::ConditionLowering => dae::DaeGeneration::ConditionLowering,
        R::PreValueLowering => dae::DaeGeneration::PreValueLowering,
        R::ClockLowering => dae::DaeGeneration::ClockLowering,
        R::DelayLowering => dae::DaeGeneration::DelayLowering,
        R::SemiLinearLowering => dae::DaeGeneration::SemiLinearLowering,
        R::TerminalLowering => dae::DaeGeneration::TerminalLowering,
        R::EventActionLowering => dae::DaeGeneration::EventActionLowering,
        R::InitializationEquation => dae::DaeGeneration::InitializationEquation,
        R::DefaultStart => dae::DaeGeneration::DefaultStart,
        R::ArrayEquationProjection => dae::DaeGeneration::ArrayEquationProjection,
        R::RecordEquationProjection => dae::DaeGeneration::RecordEquationProjection,
        R::FunctionLoopLowering => dae::DaeGeneration::FunctionLoopLowering,
        R::FunctionConditionLowering => dae::DaeGeneration::FunctionConditionLowering,
        R::FunctionAggregateLowering => dae::DaeGeneration::FunctionAggregateLowering,
        R::DerivedParameterLowering => dae::DaeGeneration::DerivedParameterLowering,
        R::IndexReduction => dae::DaeGeneration::IndexReduction,
        R::AliasElimination => dae::DaeGeneration::AliasElimination,
        R::RuntimeDiscontinuity => dae::DaeGeneration::RuntimeDiscontinuity,
        R::Other => {
            return Err(ImportError::Unsupported(
                "bitcode names a lowering kind this build does not know".to_string(),
            ));
        }
    })
}

/// Issue the construction operations that rebuild the model.
///
/// Order matters and is forced by the IR: variables are *reserved* before
/// expressions (so a coordinate can name one), then *defined* afterwards (so an
/// attribute can name an expression). This mirrors how the compiler's own
/// lowering resolves the same circularity.
/// Shared inputs every rebuild stage needs.
struct Rebuild<'a> {
    model: &'a RbcModel,
    sources: &'a [rumoca_core::SourceId],
    /// The construction closure may only return `DaeConstructionError`, whose
    /// `MalformedWire` carries a `&'static str`. Richer messages are recorded
    /// here so `import` can surface them instead of the bare variant.
    detail: &'a std::cell::RefCell<Option<String>>,
}

impl Rebuild<'_> {
    fn unsupported(&self, message: impl Into<String>) -> dae::DaeConstructionError {
        self.detail.borrow_mut().get_or_insert(message.into());
        dae::DaeConstructionError::MalformedWire {
            column: "rumoca bitcode",
        }
    }

    fn provenance(
        &self,
        provenance: RbcProvenance,
    ) -> Result<dae::DaeProvenance, dae::DaeConstructionError> {
        provenance_of(provenance, self.sources).map_err(|error| self.unsupported(error.to_string()))
    }
}

/// Resolve an id against a table built by an earlier stage.
fn resolve<T: Copy>(
    table: &[T],
    index: u32,
    what: &'static str,
    ctx: &Rebuild<'_>,
) -> Result<T, dae::DaeConstructionError> {
    table
        .get(index as usize)
        .copied()
        .ok_or_else(|| ctx.unsupported(format!("{what} {index} is not defined")))
}

/// Issue the construction operations that rebuild the model.
///
/// Order is forced by the IR: variables are *reserved* before expressions (so a
/// coordinate can name one), then *defined* afterwards (so an attribute can name
/// an expression). This mirrors how the compiler's own lowering resolves the
/// same circularity.
fn rebuild(
    construction: &mut dae::DaeConstruction<'_>,
    model: &RbcModel,
    sources: &[rumoca_core::SourceId],
    detail: &std::cell::RefCell<Option<String>>,
) -> Result<(), dae::DaeConstructionError> {
    let ctx = Rebuild {
        model,
        sources,
        detail,
    };
    let types = rebuild_types(construction, &ctx)?;
    let (variables, reservations) = reserve_variables(construction, &ctx, &types)?;
    // Domains come before expressions: a family body names its own iteration
    // binder (`x[i] = i`), and that coordinate cannot be interned until the
    // domain owning the binder exists.
    let (domains, binders) = rebuild_domains(construction, &ctx)?;
    // Conditions are reserved before expressions and defined after. An
    // expression can *read* a condition (`if initial() then ...`) and a
    // condition is *built from* expressions, so neither can be finished first;
    // reservation is what breaks the cycle, and it is the same order the
    // compiler's own lowering uses.
    let conditions = reserve_conditions(construction, &ctx)?;
    let rebuilt_clocks = clocks::rebuild(construction, &ctx, &variables, &conditions)?;
    let clocks = rebuilt_clocks.ids;
    let owners = temporal::rebuild_owners(construction, &ctx, &variables, &clocks)?;
    if model
        .expressions
        .iter()
        .any(|e| matches!(e.node, RbcExprNode::StringConversion { .. }))
    {
        construction.register_predefined_string(strings::DECLARATION)?;
    }
    let tables = Tables {
        variables: &variables,
        domains: &domains,
        binders: &binders,
        types: &types,
        conditions: &conditions,
        functions: &[],
        parameters: &[],
        clocks: &clocks,
        periodic: &rebuilt_clocks.periodic,
        previous: &owners.previous,
        terminals: &owners.terminals,
    };
    let arena = functions::rebuild_arena(construction, &ctx, &tables, reservations)?;
    let expressions = arena.expressions;
    define_variables(construction, &ctx, &expressions, arena.reservations)?;
    quotients::define_conditions(
        construction,
        &ctx,
        &expressions,
        &conditions,
        &clocks,
        arena.quotients,
    )?;
    temporal::rebuild_structured_roots(construction, &ctx, &expressions, &domains)?;
    rebuild_equations(construction, &ctx, &expressions)?;
    rebuild_families(construction, &ctx, &expressions, &domains)?;
    rebuild_discrete_real(construction, &ctx, &expressions, &variables, &conditions)?;
    construction.initialization(|owner| {
        for value in &model.initial_parameter_values {
            let VariableSlot::Parameter(target) =
                resolve(&variables, value.target.0, "initial parameter target", &ctx)?
            else {
                return Err(ctx.unsupported("initial parameter target is not a parameter"));
            };
            owner.parameter_initial_value(
                target,
                resolve(
                    &expressions,
                    value.value.0,
                    "initial parameter expression",
                    &ctx,
                )?,
                ctx.provenance(value.provenance)?,
            )?;
        }
        Ok(())
    })?;
    rebuild_events(construction, &ctx, &expressions, &variables, &conditions)?;
    rebuild_discrete_definitions(construction, &ctx, &expressions, &variables, &conditions)?;
    transactions::rebuild(
        construction,
        &ctx,
        &expressions,
        &variables,
        &conditions,
        &clocks,
    )
}

/// Replay the MLS Appendix B.1c topology: every discrete-valued variable and
/// what defines it.
///
/// `b1c` takes the complete plan of targets up front, so the whole topology is
/// one transaction and a missing definition is caught by the DAE rather than
/// producing a model with an undefined discrete variable.
fn rebuild_discrete_definitions<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    variables: &[VariableSlot<'dae>],
    conditions: &[dae::ConditionId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    let discrete_value = |id: VariableId| match variables.get(id.0 as usize) {
        Some(VariableSlot::DiscreteValue(target)) => Ok(*target),
        _ => Err(ctx.unsupported(format!(
            "B.1c target {} is not a discrete-valued variable",
            id.0
        ))),
    };
    let plan = ctx
        .model
        .discrete_definitions
        .iter()
        .flat_map(|definition| definition.targets.iter().copied())
        .map(discrete_value)
        .collect::<Result<Vec<_>, _>>()?;

    construction.b1c(plan, |topology| {
        for definition in &ctx.model.discrete_definitions {
            let at = ctx.provenance(definition.provenance)?;
            let targets = definition
                .targets
                .iter()
                .copied()
                .map(discrete_value)
                .collect::<Result<Vec<_>, _>>()?;
            let build = |owner: &mut dae::DiscreteValueOwner<'_, 'dae>| {
                rebuild_discrete_branches(owner, ctx, expressions, conditions, &definition.branches)
            };
            if definition.observed {
                topology.observed_owner(at, targets, build)?;
            } else {
                topology.owner(at, targets, build)?;
            }
        }
        Ok(())
    })
}

fn rebuild_discrete_branches<'dae>(
    owner: &mut dae::DiscreteValueOwner<'_, 'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    conditions: &[dae::ConditionId<'dae>],
    branches: &[RbcDiscreteBranch],
) -> Result<(), dae::DaeConstructionError> {
    for branch in branches {
        rebuild_discrete_branch(owner, ctx, expressions, conditions, branch)?;
    }
    Ok(())
}

fn rebuild_discrete_branch<'dae>(
    owner: &mut dae::DiscreteValueOwner<'_, 'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    conditions: &[dae::ConditionId<'dae>],
    branch: &RbcDiscreteBranch,
) -> Result<(), dae::DaeConstructionError> {
    let branch_at = ctx.provenance(branch.provenance)?;
    let values = branch
        .values
        .iter()
        .map(|value| {
            resolve(expressions, value.0, "expression", ctx)
                .map(|expression| (expression, branch_at))
        })
        .collect::<Result<Vec<_>, _>>()?;
    match branch.activation {
        RbcDiscreteActivation::Always => owner.always(branch_at, values),
        RbcDiscreteActivation::When { trigger, guard } => owner.when(
            resolve(conditions, trigger.0, "condition", ctx)?,
            resolve(conditions, guard.0, "condition", ctx)?,
            branch_at,
            values,
        ),
    }
}

fn rebuild_types<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
) -> Result<Vec<dae::ValueTypeId<'dae>>, dae::DaeConstructionError> {
    // Every checked object needs source-backed provenance, including a value
    // type. Any object in the artifact anchors it equally well.
    let anchor = ctx
        .model
        .variables
        .first()
        .map(|variable| variable.declaration)
        .or_else(|| ctx.model.expressions.first().map(|e| e.provenance))
        .or_else(|| ctx.model.functions.first().map(|f| f.declaration));
    // A model with no variable, expression or function -- `model Empty end
    // Empty;` -- has nothing that could name a type, so there are no types
    // to rebuild and nothing to anchor them to. Refusing it would turn an
    // empty model into an import error.
    let Some(anchor) = anchor else {
        return Ok(Vec::new());
    };
    let anchor = ctx.provenance(anchor)?;

    let mut types = Vec::with_capacity(ctx.model.types.len());
    construction.types(|owner| {
        for ty in &ctx.model.types {
            // A record is built through its own constructor: its fields are
            // `ValueTypeId`s the owner has to re-check, not data it can take
            // on trust from an artifact.
            if let Some(record) = &ty.record {
                let record = rebuild_record(owner, ctx, &types, record, ty, anchor)?;
                types.push(record);
                continue;
            }
            let scalar = scalar_of(ty.scalar);
            let value_type = if ty.dimensions.is_empty() {
                dae::ValueType::scalar(scalar)
            } else {
                dae::ValueType::array(scalar, ty.dimensions.clone())
            };
            types.push(owner.derived(value_type, anchor)?);
        }
        Ok(())
    })?;
    Ok(types)
}

fn rebuild_record<'dae>(
    owner: &mut dae::ValueTypes<'_, 'dae>,
    ctx: &Rebuild<'_>,
    types: &[dae::ValueTypeId<'dae>],
    record: &RbcRecord,
    ty: &RbcType,
    anchor: dae::DaeProvenance,
) -> Result<dae::ValueTypeId<'dae>, dae::DaeConstructionError> {
    let mut fields = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        fields.push((
            rumoca_core::VarName::intern(&field.name),
            resolve(types, field.value_type.0, "type", ctx)?,
        ));
    }
    let name = rumoca_core::VarName::intern(&record.name);
    owner.record_array(name, fields, ty.dimensions.clone(), anchor)
}

type Reservations<'dae> = Vec<dae::VariableReservation<'dae>>;

fn reserve_variables<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    types: &[dae::ValueTypeId<'dae>],
) -> Result<(Vec<VariableSlot<'dae>>, Reservations<'dae>), dae::DaeConstructionError> {
    let mut slots = Vec::with_capacity(ctx.model.variables.len());
    let mut reservations = Vec::with_capacity(ctx.model.variables.len());
    construction.variables(|owner| {
        for variable in &ctx.model.variables {
            let (slot, reservation) = reserve_variable(owner, ctx, types, variable)?;
            slots.push(slot);
            reservations.push(reservation);
        }
        Ok(())
    })?;
    Ok((slots, reservations))
}

fn reserve_variable<'dae>(
    owner: &mut dae::Variables<'_, 'dae>,
    ctx: &Rebuild<'_>,
    types: &[dae::ValueTypeId<'dae>],
    variable: &RbcVariable,
) -> Result<(VariableSlot<'dae>, dae::VariableReservation<'dae>), dae::DaeConstructionError> {
    let name = rumoca_core::VarName::intern(&variable.name);
    let ty = resolve(types, variable.value_type.0, "type", ctx)?;
    let at = ctx.provenance(variable.declaration)?;
    Ok(match variable.role {
        RbcRole::Parameter => {
            let (id, r) = owner.reserve_parameter(name, ty, at)?;
            (VariableSlot::Parameter(id), r)
        }
        RbcRole::Constant => {
            let (id, r) = owner.reserve_constant(name, ty, at)?;
            (VariableSlot::Parameter(id), r)
        }
        RbcRole::Input => {
            let variability = if variable.discrete_input {
                dae::InputVariability::Discrete
            } else {
                dae::InputVariability::Continuous
            };
            let (id, r) = owner.reserve_input(name, ty, variability, at)?;
            (VariableSlot::Input(id), r)
        }
        RbcRole::State => {
            let (id, r) = owner.reserve_state(name, ty, at)?;
            (VariableSlot::State(id), r)
        }
        RbcRole::Algebraic => {
            let (id, r) = owner.reserve_algebraic(name, ty, at)?;
            (VariableSlot::Algebraic(id), r)
        }
        RbcRole::Output => {
            let (id, r) = owner.reserve_output(name, ty, at)?;
            (VariableSlot::Algebraic(id), r)
        }
        RbcRole::DiscreteReal => {
            let (id, r) = owner.reserve_discrete_real(name, ty, at)?;
            (VariableSlot::DiscreteReal(id), r)
        }
        RbcRole::DiscreteValue => {
            let (id, r) = owner.reserve_discrete_value(name, ty, at)?;
            (VariableSlot::DiscreteValue(id), r)
        }
    })
}

/// Everything an expression node may name that was rebuilt before it.
#[derive(Clone, Copy)]
struct Tables<'a, 'dae> {
    variables: &'a [VariableSlot<'dae>],
    domains: &'a [dae::DomainId<'dae>],
    binders: &'a BinderTable<'dae>,
    types: &'a [dae::ValueTypeId<'dae>],
    conditions: &'a [dae::ConditionId<'dae>],
    /// Functions rebuilt so far; `None` where the artifact elides the body.
    functions: &'a [Option<dae::FunctionId<'dae>>],
    parameters: &'a [Vec<dae::FunctionParameterId<'dae>>],
    clocks: &'a [dae::ClockId<'dae>],
    periodic: &'a [Option<dae::PeriodicClockId<'dae>>],
    previous: &'a [dae::PreviousId<'dae>],
    terminals: &'a [dae::TerminalId<'dae>],
}

fn build_expression<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    expression: &RbcExpr,
    built: &[dae::ExprId<'dae>],
    tables: &Tables<'_, 'dae>,
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    Ok(match &expression.node {
        // Issued by the body replay in `functions`, never built directly: a
        // read has to see the definition the artifact names, and a loop
        // value is created by opening or closing its loop.
        RbcExprNode::FunctionValue { .. }
        | RbcExprNode::FunctionFoldParameter { .. }
        | RbcExprNode::FunctionFoldOutput { .. } => {
            return Err(ctx.unsupported(format!(
                "expression {} reads a function body outside that body's replay",
                expression.id.0
            )));
        }
        RbcExprNode::Call { .. }
        | RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::FunctionParameter { .. },
        } => functions::function_node(owner, ctx, expression, built, tables, at)?,
        RbcExprNode::ClockTransfer {
            transfer: kind,
            source,
            source_clock,
            target_clock,
        } => owner.at(at).clock_transfer(
            clocks::transfer_kind(*kind),
            resolve(built, source.0, "expression", ctx)?,
            resolve(tables.clocks, source_clock.0, "clock", ctx)?,
            resolve(tables.clocks, target_clock.0, "clock", ctx)?,
        )?,
        RbcExprNode::StringConversion { value, format } => {
            let value = resolve(built, value.0, "expression", ctx)?;
            owner.at(at).string_conversion(
                strings::DECLARATION,
                value,
                strings::format(format, built, ctx)?,
            )?
        }
        // An enumeration value has its own constructor: `literal` rejects
        // `DaeLiteral::Enumeration` outright, because the ordinal has to be
        // proved one-based (MLS §4.9.5) before the node is interned.
        RbcExprNode::Literal {
            value: RbcLiteral::Enumeration { ordinal },
        } => owner.at(at).enumeration_literal(*ordinal)?,
        RbcExprNode::Literal { value } => owner.at(at).literal(literal_of(value))?,
        RbcExprNode::Coordinate { coordinate } => {
            build_coordinate(owner, ctx, coordinate, tables, at)?
        }
        RbcExprNode::Unary { op, operand } => {
            let operand = resolve(built, operand.0, "expression", ctx)?;
            owner.at(at).unary(unary_of(*op), operand)?
        }
        RbcExprNode::Binary { op, lhs, rhs } => {
            let lhs = resolve(built, lhs.0, "expression", ctx)?;
            let rhs = resolve(built, rhs.0, "expression", ctx)?;
            owner.at(at).binary(binary_of(*op), lhs, rhs)?
        }
        RbcExprNode::Conditional { branches, fallback } => {
            build_conditional(owner, ctx, branches, *fallback, built, at)?
        }
        RbcExprNode::Builtin { name, arguments } => {
            let builtin = builtin_of(name)
                .ok_or_else(|| ctx.unsupported(format!("unknown built-in `{name}`")))?;
            owner
                .at(at)
                .builtin(builtin, resolve_operands(built, arguments, ctx)?)?
        }
        RbcExprNode::Array {
            elements,
            empty_type,
        } => build_array(owner, ctx, elements, *empty_type, built, tables, at)?,
        RbcExprNode::Record { ty, fields } => {
            let ty = resolve(tables.types, ty.0, "type", ctx)?;
            owner
                .at(at)
                .record(ty, resolve_operands(built, fields, ctx)?)?
        }
        RbcExprNode::Field { base, field } => {
            let base = resolve(built, base.0, "expression", ctx)?;
            owner.at(at).field(base, *field as usize)?
        }
        RbcExprNode::Range { start, step, stop } => {
            build_range(owner, ctx, *start, *step, *stop, built, at)?
        }
        RbcExprNode::Comprehension { domain, body } => {
            build_comprehension(owner, ctx, *domain, *body, built, tables, at)?
        }
        RbcExprNode::Index { base, subscripts } => {
            let base = resolve(built, base.0, "expression", ctx)?;
            let subscripts = subscripts_of(subscripts, built, ctx, at)?;
            owner.at(at).index(base, subscripts)?
        }
        RbcExprNode::ArrayUpdate {
            base,
            value,
            subscripts,
        } => {
            let base = resolve(built, base.0, "expression", ctx)?;
            let value = resolve(built, value.0, "expression", ctx)?;
            let subscripts = subscripts_of(subscripts, built, ctx, at)?;
            owner.at(at).array_update(base, value, subscripts)?
        }
        RbcExprNode::Unsupported { detail } => {
            return Err(ctx.unsupported(format!(
                "expression {} is unsupported: {detail}",
                expression.id
            )));
        }
    })
}

fn build_coordinate<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    coordinate: &RbcCoordinate,
    tables: &Tables<'_, 'dae>,
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    Ok(match coordinate {
        // A binder is not a `CoordinateInput`: it is owner-local to a domain
        // and has its own constructor, which re-checks the binder against the
        // domain it claims to come from.
        RbcCoordinate::Binder { domain, ordinal } => {
            let binder = *tables
                .binders
                .get(&(*domain, *ordinal))
                .ok_or_else(|| ctx.unsupported("binder names an unknown domain"))?;
            owner.at(at).binder(binder)?
        }
        RbcCoordinate::ClockInterval { clock } => {
            let clock = tables
                .periodic
                .get(clock.0 as usize)
                .copied()
                .flatten()
                .ok_or_else(|| ctx.unsupported("interval() names a clock that is not periodic"))?;
            owner
                .at(at)
                .coordinate(dae::CoordinateInput::ClockInterval(clock))?
        }
        RbcCoordinate::Previous { previous } => {
            owner
                .at(at)
                .coordinate(dae::CoordinateInput::Previous(resolve(
                    tables.previous,
                    previous.0,
                    "previous value",
                    ctx,
                )?))?
        }
        RbcCoordinate::Terminal { terminal } => {
            owner
                .at(at)
                .coordinate(dae::CoordinateInput::Terminal(resolve(
                    tables.terminals,
                    terminal.0,
                    "terminal",
                    ctx,
                )?))?
        }
        RbcCoordinate::Condition { condition } => {
            let condition = resolve(tables.conditions, condition.0, "condition", ctx)?;
            owner
                .at(at)
                .coordinate(dae::CoordinateInput::Condition(condition))?
        }
        coordinate => {
            let input = coordinate_of(*coordinate, tables.variables)
                .ok_or_else(|| ctx.unsupported("coordinate names an unknown variable"))?;
            owner.at(at).coordinate(input)?
        }
    })
}

/// Resolve a list of earlier expression operands, in order.
fn resolve_operands<'dae>(
    built: &[dae::ExprId<'dae>],
    operands: &[ExprId],
    ctx: &Rebuild<'_>,
) -> Result<Vec<dae::ExprId<'dae>>, dae::DaeConstructionError> {
    let mut resolved = Vec::with_capacity(operands.len());
    for operand in operands {
        resolved.push(resolve(built, operand.0, "expression", ctx)?);
    }
    Ok(resolved)
}

fn build_conditional<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    branches: &[RbcBranch],
    fallback: ExprId,
    built: &[dae::ExprId<'dae>],
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    let mut arms = Vec::with_capacity(branches.len());
    for branch in branches {
        let condition = resolve(built, branch.condition.0, "expression", ctx)?;
        let value = resolve(built, branch.value.0, "expression", ctx)?;
        arms.push((condition, value));
    }
    let fallback = resolve(built, fallback.0, "expression", ctx)?;
    owner.at(at).conditional(arms, fallback)
}

fn build_array<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    elements: &[ExprId],
    empty_type: Option<TypeId>,
    built: &[dae::ExprId<'dae>],
    tables: &Tables<'_, 'dae>,
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    if let Some(ty) = empty_type {
        // An empty literal carries no element from which to re-derive
        // its type, so the constructor takes the type directly.
        return owner
            .at(at)
            .empty_array(resolve(tables.types, ty.0, "type", ctx)?);
    }
    owner.at(at).array(resolve_operands(built, elements, ctx)?)
}

fn build_range<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    start: ExprId,
    step: Option<ExprId>,
    stop: ExprId,
    built: &[dae::ExprId<'dae>],
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    let start = resolve(built, start.0, "expression", ctx)?;
    let step = step
        .map(|step| resolve(built, step.0, "expression", ctx))
        .transpose()?;
    let stop = resolve(built, stop.0, "expression", ctx)?;
    owner.at(at).range(start, step, stop)
}

fn build_comprehension<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    domain: DomainId,
    body: ExprId,
    built: &[dae::ExprId<'dae>],
    tables: &Tables<'_, 'dae>,
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    let domain = *tables
        .domains
        .get(domain.0 as usize)
        .ok_or_else(|| ctx.unsupported("comprehension names an unknown domain"))?;
    let body = resolve(built, body.0, "expression", ctx)?;
    owner.at(at).comprehension(domain, body)
}

fn define_variables<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    mut reservations: Vec<Option<dae::VariableReservation<'dae>>>,
) -> Result<(), dae::DaeConstructionError> {
    construction.variables(|owner| {
        for (variable, slot) in ctx.model.variables.iter().zip(&mut reservations) {
            // Parameters whose attributes were ready early were defined
            // during the arena pass; see `functions::define_ready`.
            if let Some(reservation) = slot.take() {
                define_variable(owner, ctx, variable, reservation, expressions)?;
            }
        }
        Ok(())
    })
}

fn define_variable<'dae>(
    owner: &mut dae::Variables<'_, 'dae>,
    ctx: &Rebuild<'_>,
    variable: &RbcVariable,
    reservation: dae::VariableReservation<'dae>,
    expressions: &[dae::ExprId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    let at = ctx.provenance(variable.declaration)?;
    let attribute = |id: Option<ExprId>| match id {
        Some(id) => resolve(expressions, id.0, "expression", ctx).map(Some),
        None => Ok(None),
    };
    let attributes = dae::VariableAttributes {
        causality: causality_of(variable.causality),
        declared_causality: match variable.declared_causality {
            Some(RbcDeclaredCausality::Input) => dae::DeclaredCausality::Input,
            Some(RbcDeclaredCausality::Output) => dae::DeclaredCausality::Output,
            Some(RbcDeclaredCausality::None) => dae::DeclaredCausality::None,
            None => match variable.causality {
                RbcCausality::Input => dae::DeclaredCausality::Input,
                RbcCausality::Output => dae::DeclaredCausality::Output,
                _ => dae::DeclaredCausality::None,
            },
        },
        unit: variable.unit.clone(),
        description: variable.description.clone(),
        fixed: variable
            .fixed_elements
            .clone()
            .or_else(|| variable.fixed.map(|value| vec![value])),
        evaluable: variable.evaluable,
        is_held: variable.held,
        state_select: match variable.state_select {
            RbcStateSelect::Never => rumoca_core::StateSelect::Never,
            RbcStateSelect::Avoid => rumoca_core::StateSelect::Avoid,
            RbcStateSelect::Default => rumoca_core::StateSelect::Default,
            RbcStateSelect::Prefer => rumoca_core::StateSelect::Prefer,
            RbcStateSelect::Always => rumoca_core::StateSelect::Always,
        },
        is_tunable: variable.tunable,
        origin: if variable.from_source {
            dae::VariableOrigin::Source
        } else {
            dae::VariableOrigin::Generated
        },
        binding: attribute(variable.binding)?,
        start: attribute(variable.start)?,
        min: attribute(variable.min)?,
        max: attribute(variable.max)?,
        nominal: attribute(variable.nominal)?,
        ..Default::default()
    };
    owner.define(reservation, attributes, at)
}

/// Reserve every condition, so an expression can name one before it is defined.
///
/// The boolean algebra is also a DAG over earlier conditions, so reservation
/// serves that second purpose too.
fn reserve_conditions<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
) -> Result<Vec<dae::ConditionId<'dae>>, dae::DaeConstructionError> {
    let mut conditions = Vec::with_capacity(ctx.model.conditions.len());
    construction.conditions(|owner| {
        for condition in &ctx.model.conditions {
            let at = ctx.provenance(condition.provenance)?;
            conditions.push(owner.reserve(at)?);
        }
        Ok(())
    })?;
    Ok(conditions)
}

fn condition_input<'dae>(
    ctx: &Rebuild<'_>,
    condition: &RbcCondition,
    relations: &[dae::RelationId<'dae>],
    conditions: &[dae::ConditionId<'dae>],
    expressions: &[dae::ExprId<'dae>],
    clocks: &[dae::ClockId<'dae>],
) -> Result<dae::ConditionInput<'dae>, dae::DaeConstructionError> {
    let inner = |id: ConditionId| resolve(conditions, id.0, "condition", ctx);
    Ok(match &condition.node {
        RbcConditionNode::Initial => dae::ConditionInput::Initial,
        RbcConditionNode::Always => dae::ConditionInput::Always,
        RbcConditionNode::Relation { relation } => {
            dae::ConditionInput::Relation(resolve(relations, relation.0, "relation", ctx)?)
        }
        RbcConditionNode::Discrete { expression } => {
            dae::ConditionInput::Discrete(resolve(expressions, expression.0, "expression", ctx)?)
        }
        RbcConditionNode::Not { operand } => dae::ConditionInput::Not(inner(*operand)?),
        RbcConditionNode::And { lhs, rhs } => dae::ConditionInput::And(inner(*lhs)?, inner(*rhs)?),
        RbcConditionNode::Or { lhs, rhs } => dae::ConditionInput::Or(inner(*lhs)?, inner(*rhs)?),
        RbcConditionNode::AnyRise { lhs, rhs } => {
            dae::ConditionInput::AnyRise(inner(*lhs)?, inner(*rhs)?)
        }
        RbcConditionNode::ClockActivation { clock } => {
            dae::ConditionInput::Clock(resolve(clocks, clock.0, "clock", ctx)?)
        }
        RbcConditionNode::Unsupported { detail } => {
            return Err(ctx.unsupported(format!("unsupported condition: {detail}")));
        }
    })
}

/// Rebuild the iteration domains the families range over.
///
/// Returned in artifact order so a family's `DomainId` indexes straight into
/// the result. Parents are resolved first: the export writes domains sorted by
/// index and a parent always precedes its child, so one pass suffices.
type BinderTable<'dae> = std::collections::HashMap<(DomainId, u32), dae::DomainBinderId<'dae>>;

fn rebuild_domains<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
) -> Result<(Vec<dae::DomainId<'dae>>, BinderTable<'dae>), dae::DaeConstructionError> {
    let mut built: Vec<dae::DomainId<'dae>> = Vec::with_capacity(ctx.model.domains.len());
    let mut binders = BinderTable::default();
    for domain in &ctx.model.domains {
        let at = ctx.provenance(domain.provenance)?;
        let structured = rumoca_core::StructuredIndexDomain {
            binders: domain
                .binders
                .iter()
                .map(|binder| rumoca_core::StructuredIndexBinder {
                    id: binder.id as usize,
                    display_name: binder.display_name.clone(),
                    lower: binder.lower,
                    upper: binder.upper,
                    step: binder.step,
                })
                .collect(),
        };
        let id = match domain.parent {
            Some(parent) => {
                let parent = *built
                    .get(parent.0 as usize)
                    .ok_or_else(|| ctx.unsupported("domain names an unknown parent"))?;
                construction.domains(|domains| domains.nested(parent, structured, at))?
            }
            None => construction.domains(|domains| domains.structured(structured, at))?,
        };
        // A binder identity is owner-local to its domain and has no public
        // raw constructor, so it is minted here while the domain is in hand.
        for ordinal in 0..domain.binders.len() {
            let binder = construction.domains(|domains| domains.binder(id, ordinal, at))?;
            built_binder(&mut binders, DomainId(built.len() as u32), ordinal, binder);
        }
        built.push(id);
    }
    Ok((built, binders))
}

fn built_binder<'dae>(
    binders: &mut BinderTable<'dae>,
    domain: DomainId,
    ordinal: usize,
    binder: dae::DomainBinderId<'dae>,
) {
    binders.insert((domain, ordinal as u32), binder);
}

/// Rebuild the MLS Appendix B.1b partition and the initialization-instant
/// discrete values.
///
/// These are a separate DAE partition from the continuous residuals, and
/// nothing exported them, so an artifact was short one equation per
/// discrete-Real variable.
fn rebuild_discrete_real<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    variables: &[VariableSlot<'dae>],
    conditions: &[dae::ConditionId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    if !ctx.model.discrete_real_equations.is_empty() {
        construction.discrete(|owner| {
            for equation in &ctx.model.discrete_real_equations {
                rebuild_discrete_real_equation(owner, ctx, equation, expressions, conditions)?;
            }
            Ok(())
        })?;
    }
    if !ctx.model.initial_discrete_values.is_empty() {
        construction.initialization(|owner| {
            for entry in &ctx.model.initial_discrete_values {
                rebuild_initial_discrete_value(owner, ctx, entry, expressions, variables)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn rebuild_discrete_real_equation<'dae>(
    owner: &mut dae::DiscreteEquations<'_, 'dae>,
    ctx: &Rebuild<'_>,
    equation: &RbcDiscreteRealEquation,
    expressions: &[dae::ExprId<'dae>],
    conditions: &[dae::ConditionId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    let at = ctx.provenance(equation.provenance)?;
    let residual = resolve(expressions, equation.residual.0, "expression", ctx)?;
    let build = |target: &mut dae::ResidualEquation<'_, 'dae>| target.residual(residual);
    match equation.activation {
        RbcDiscreteRealActivation::Always => {
            owner.real_equation(at, build)?;
        }
        RbcDiscreteRealActivation::When { trigger, guard } => {
            let trigger = resolve(conditions, trigger.0, "condition", ctx)?;
            let guard = resolve(conditions, guard.0, "condition", ctx)?;
            owner.when_real_equation(trigger, guard, at, build)?;
        }
    }
    Ok(())
}

fn rebuild_initial_discrete_value<'dae>(
    owner: &mut dae::InitializationEquations<'_, 'dae>,
    ctx: &Rebuild<'_>,
    entry: &RbcInitialDiscreteValue,
    expressions: &[dae::ExprId<'dae>],
    variables: &[VariableSlot<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    let at = ctx.provenance(entry.provenance)?;
    let value = resolve(expressions, entry.value.0, "expression", ctx)?;
    match variables.get(entry.target.0 as usize) {
        Some(VariableSlot::DiscreteReal(target)) => {
            owner.discrete_real_initial_value(*target, value, at)?;
        }
        Some(VariableSlot::DiscreteValue(target)) => {
            owner.discrete_value_initial_value(*target, value, at)?;
        }
        _ => {
            return Err(ctx.unsupported(format!(
                "initial discrete value names variable {}, which is not \
                 a discrete coordinate",
                entry.target.0
            )));
        }
    }
    Ok(())
}

/// Rebuild the array/`for` equations.
///
/// Families were omitted from the artifact entirely until TOOLBUG-014, and
/// then exported but refused here because their domain was missing. With
/// domains carried, both halves close: an artifact containing a family
/// rebuilds into a DAE containing the same family.
fn rebuild_families<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    domains: &[dae::DomainId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    for (families, initial) in [
        (&ctx.model.equation_families, false),
        (&ctx.model.initial_equation_families, true),
    ] {
        for family in families {
            rebuild_family(construction, ctx, expressions, domains, family, initial)?;
        }
    }
    Ok(())
}

fn rebuild_family<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    domains: &[dae::DomainId<'dae>],
    family: &RbcEquationFamily,
    initial: bool,
) -> Result<(), dae::DaeConstructionError> {
    let at = ctx.provenance(family.provenance)?;
    // Validation guarantees the reference, but a corrupt artifact must
    // not panic here.
    let Some(&domain) = domains.get(family.domain.0 as usize) else {
        return Ok(());
    };
    let bodies: Vec<dae::ExprId<'dae>> = family
        .bodies
        .iter()
        .map(|body| resolve(expressions, body.0, "expression", ctx))
        .collect::<Result<_, _>>()?;
    let view = match family.scalar_view {
        RbcScalarView::BinderSubstitution => {
            rumoca_core::ComprehensionScalarView::BinderSubstitution
        }
        RbcScalarView::RowMajorProjection => {
            rumoca_core::ComprehensionScalarView::RowMajorProjection
        }
        RbcScalarView::BinderPrefixProjection { binder_count } => {
            rumoca_core::ComprehensionScalarView::BinderPrefixProjection { binder_count }
        }
    };
    // The two partitions are distinct owner types, so the call is
    // written twice rather than behind one closure.
    let outcome = if initial {
        construction.initialization(|owner| {
            owner.structured_family(at, domain, view, |family| family_bodies(family, &bodies))?;
            Ok(())
        })
    } else {
        construction.continuous(|owner| {
            owner.structured_family(at, domain, view, |family| family_bodies(family, &bodies))?;
            Ok(())
        })
    };
    outcome?;
    Ok(())
}

fn family_bodies<'dae>(
    family: &mut dae::StructuredResiduals<'_, 'dae>,
    bodies: &[dae::ExprId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    for body in bodies {
        family.body(*body)?;
    }
    Ok(())
}

fn rebuild_equations<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    construction.continuous(|owner| {
        for equation in &ctx.model.equations {
            let at = ctx.provenance(equation.provenance)?;
            let residual = resolve(expressions, equation.residual.0, "expression", ctx)?;
            owner.equation(at, |body| body.residual(residual))?;
        }
        Ok(())
    })?;
    construction.initialization(|owner| {
        for equation in &ctx.model.initial_equations {
            let at = ctx.provenance(equation.provenance)?;
            let residual = resolve(expressions, equation.residual.0, "expression", ctx)?;
            owner.equation(at, |body| body.residual(residual))?;
        }
        Ok(())
    })
}

fn rebuild_events<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    expressions: &[dae::ExprId<'dae>],
    variables: &[VariableSlot<'dae>],
    conditions: &[dae::ConditionId<'dae>],
) -> Result<(), dae::DaeConstructionError> {
    let legacy_warning = ctx.model.events.iter().find(|event| {
        matches!(&event.action, RbcAction::Assert { level, .. }
            if matches!(legacy_assertion_level(ctx, *level), Ok(dae::AssertionLevel::Warning)))
    });
    let legacy_condition = legacy_warning
        .map(|event| {
            let at = ctx.provenance(event.provenance)?;
            construction.expressions(|owner| owner.at(at).literal(dae::DaeLiteral::Boolean(false)))
        })
        .transpose()?;
    construction.events(|owner| {
        for event in &ctx.model.time_events {
            let at = ctx.provenance(event.provenance)?;
            match event.schedule {
                RbcSchedule::Static {
                    numerator,
                    denominator,
                } => {
                    let instant = rumoca_core::ClockRational::new(numerator, denominator).map_err(
                        |error| ctx.unsupported(format!("invalid time-event rational: {error}")),
                    )?;
                    owner.time_event(instant, at)?;
                }
                RbcSchedule::Dynamic { deadline } => {
                    owner.dynamic_time_event(
                        resolve(expressions, deadline.0, "expression", ctx)?,
                        at,
                    )?;
                }
            }
        }
        for event in &ctx.model.events {
            let at = ctx.provenance(event.provenance)?;
            let trigger = resolve(conditions, event.trigger.0, "condition", ctx)?;
            let guard = resolve(conditions, event.guard.0, "condition", ctx)?;
            rebuild_action(
                owner,
                ctx,
                event,
                expressions,
                variables,
                trigger,
                guard,
                at,
                legacy_condition,
            )?;
        }
        Ok(())
    })
}

// Bundling these into a context struct would hand this function the whole
// model and hide which four tables it actually reads.
// SPEC_0021: Exception - rebuilding one action needs every table it may reference at once.
#[allow(clippy::too_many_arguments)]
fn rebuild_action<'dae>(
    owner: &mut dae::Events<'_, 'dae>,
    ctx: &Rebuild<'_>,
    event: &RbcEventAction,
    expressions: &[dae::ExprId<'dae>],
    variables: &[VariableSlot<'dae>],
    trigger: dae::ConditionId<'dae>,
    guard: dae::ConditionId<'dae>,
    at: dae::DaeProvenance,
    legacy_condition: Option<dae::ExprId<'dae>>,
) -> Result<(), dae::DaeConstructionError> {
    let expression = |id: ExprId| resolve(expressions, id.0, "expression", ctx);
    match &event.action {
        RbcAction::Reinitialize { state, value } => {
            let state = reinit_state(ctx, variables, *state)?;
            owner.reinitialize(trigger, guard, state, expression(*value)?, at)?;
        }
        RbcAction::Assert { message, level } => {
            let message = expression(*message)?;
            match legacy_assertion_level(ctx, *level)? {
                dae::AssertionLevel::Error => {
                    owner.assert(trigger, guard, message, at)?;
                }
                dae::AssertionLevel::Warning => {
                    let condition = legacy_condition
                        .ok_or_else(|| ctx.unsupported("missing legacy warning condition"))?;
                    owner.warning(trigger, guard, condition, message, at)?;
                }
            }
        }
        RbcAction::Warning { condition, message } => {
            owner.warning(
                trigger,
                guard,
                expression(*condition)?,
                expression(*message)?,
                at,
            )?;
        }
        RbcAction::Terminate { message } => {
            owner.terminate(trigger, guard, expression(*message)?, at)?;
        }
    }
    Ok(())
}

/// Old v1 assertions encoded the predefined level as an expression ordinal.
fn legacy_assertion_level(
    ctx: &Rebuild<'_>,
    level: Option<ExprId>,
) -> Result<dae::AssertionLevel, dae::DaeConstructionError> {
    let Some(level) = level else {
        return Ok(dae::AssertionLevel::Error);
    };
    let ordinal = match ctx.model.expressions.get(level.0 as usize).map(|e| &e.node) {
        Some(RbcExprNode::Literal {
            value: RbcLiteral::Integer { value } | RbcLiteral::Enumeration { ordinal: value },
        }) => *value,
        Some(RbcExprNode::Literal {
            value: RbcLiteral::Real { value },
        }) if *value == 1.0 || *value == 2.0 => *value as i64,
        _ => {
            return Err(
                ctx.unsupported("legacy assertion level is not a literal AssertionLevel ordinal")
            );
        }
    };
    match ordinal {
        1 => Ok(dae::AssertionLevel::Error),
        2 => Ok(dae::AssertionLevel::Warning),
        _ => Err(ctx.unsupported("invalid legacy AssertionLevel ordinal")),
    }
}

/// `reinit` may only target a continuous state; anything else is a rejection,
/// not a coercion.
fn reinit_state<'dae>(
    ctx: &Rebuild<'_>,
    variables: &[VariableSlot<'dae>],
    state: VariableId,
) -> Result<dae::StateId<'dae>, dae::DaeConstructionError> {
    match resolve(variables, state.0, "variable", ctx)? {
        VariableSlot::State(state) => Ok(state),
        _ => Err(ctx.unsupported("reinit target is not a state")),
    }
}

/// Which typed id a variable slot holds. RBC uses one variable id space; the
/// DAE uses role-specific ids over the same index, so this keeps the mapping
/// explicit rather than casting between them.
#[derive(Clone, Copy)]
enum VariableSlot<'dae> {
    Parameter(dae::ParameterId<'dae>),
    Input(dae::InputId<'dae>),
    State(dae::StateId<'dae>),
    Algebraic(dae::AlgebraicId<'dae>),
    DiscreteReal(dae::DiscreteRealId<'dae>),
    DiscreteValue(dae::DiscreteValueId<'dae>),
}

fn subscripts_of<'dae>(
    subscripts: &[RbcSubscript],
    built: &[dae::ExprId<'dae>],
    ctx: &Rebuild<'_>,
    at: dae::DaeProvenance,
) -> Result<Vec<dae::Subscript<'dae>>, dae::DaeConstructionError> {
    subscripts
        .iter()
        .map(|subscript| {
            Ok(match subscript {
                RbcSubscript::Index { expression } => dae::Subscript::Index {
                    expression: resolve(built, expression.0, "expression", ctx)?,
                    provenance: at,
                },
                RbcSubscript::Whole => dae::Subscript::Whole { provenance: at },
                RbcSubscript::Slice { expression } => dae::Subscript::Slice {
                    expression: resolve(built, expression.0, "expression", ctx)?,
                    provenance: at,
                },
            })
        })
        .collect()
}

fn coordinate_of<'dae>(
    coordinate: RbcCoordinate,
    variables: &[VariableSlot<'dae>],
) -> Option<dae::CoordinateInput<'dae>> {
    let slot = |id: VariableId| variables.get(id.0 as usize).copied();
    Some(match coordinate {
        RbcCoordinate::Time => dae::CoordinateInput::Time,
        RbcCoordinate::Parameter { variable } => match slot(variable)? {
            VariableSlot::Parameter(id) => dae::CoordinateInput::Parameter(id),
            _ => return None,
        },
        RbcCoordinate::Input { variable } => match slot(variable)? {
            VariableSlot::Input(id) => dae::CoordinateInput::Input(id),
            _ => return None,
        },
        RbcCoordinate::State { variable } => match slot(variable)? {
            VariableSlot::State(id) => dae::CoordinateInput::State(id),
            _ => return None,
        },
        RbcCoordinate::Derivative { variable } => match slot(variable)? {
            VariableSlot::State(id) => dae::CoordinateInput::Derivative(id),
            _ => return None,
        },
        RbcCoordinate::PreState { variable } => match slot(variable)? {
            VariableSlot::State(id) => dae::CoordinateInput::PreState(id),
            _ => return None,
        },
        RbcCoordinate::Algebraic { variable } => match slot(variable)? {
            VariableSlot::Algebraic(id) => dae::CoordinateInput::Algebraic(id),
            _ => return None,
        },
        RbcCoordinate::PreAlgebraic { variable } => match slot(variable)? {
            VariableSlot::Algebraic(id) => dae::CoordinateInput::PreAlgebraic(id),
            _ => return None,
        },
        RbcCoordinate::DiscreteReal { variable } => match slot(variable)? {
            VariableSlot::DiscreteReal(id) => dae::CoordinateInput::DiscreteReal(id),
            _ => return None,
        },
        RbcCoordinate::PreDiscreteReal { variable } => match slot(variable)? {
            VariableSlot::DiscreteReal(id) => dae::CoordinateInput::PreDiscreteReal(id),
            _ => return None,
        },
        RbcCoordinate::DiscreteValue { variable } => match slot(variable)? {
            VariableSlot::DiscreteValue(id) => dae::CoordinateInput::DiscreteValue(id),
            _ => return None,
        },
        RbcCoordinate::PreDiscreteValue { variable } => match slot(variable)? {
            VariableSlot::DiscreteValue(id) => dae::CoordinateInput::PreDiscreteValue(id),
            _ => return None,
        },
        // Both are handled by `build_expression`, which holds the domain and
        // condition tables that `variables` alone cannot resolve.
        RbcCoordinate::Binder { .. } | RbcCoordinate::Condition { .. } => return None,
        // Handled by `build_expression` and the arena replay, which hold
        // the function, clock and temporal-owner tables these name.
        RbcCoordinate::FunctionParameter { .. }
        | RbcCoordinate::ClockInterval { .. }
        | RbcCoordinate::Delay { .. }
        | RbcCoordinate::Previous { .. }
        | RbcCoordinate::Terminal { .. } => return None,
    })
}

fn scalar_of(scalar: RbcScalar) -> dae::ScalarType {
    match scalar {
        RbcScalar::Real => dae::ScalarType::Real,
        RbcScalar::Integer => dae::ScalarType::Integer,
        RbcScalar::Boolean => dae::ScalarType::Boolean,
        RbcScalar::String => dae::ScalarType::String,
        RbcScalar::Enumeration => dae::ScalarType::Enumeration,
        RbcScalar::Record => dae::ScalarType::Record,
    }
}

/// Enumerations are routed to `enumeration_literal` before reaching this map,
/// which is the only constructor that proves the ordinal one-based. The arm
/// below is kept so a future caller fails closed on a typed
/// `InvalidEnumerationOrdinal` from `literal` rather than panicking here.
fn literal_of(literal: &RbcLiteral) -> dae::DaeLiteral {
    match literal {
        RbcLiteral::Real { value } => dae::DaeLiteral::Real(*value),
        RbcLiteral::Integer { value } => dae::DaeLiteral::Integer(*value),
        RbcLiteral::Boolean { value } => dae::DaeLiteral::Boolean(*value),
        RbcLiteral::String { value } => dae::DaeLiteral::String(value.clone()),
        RbcLiteral::Enumeration { ordinal } => dae::DaeLiteral::Enumeration(*ordinal),
    }
}

fn builtin_of(name: &str) -> Option<dae::PureBuiltin> {
    use dae::PureBuiltin as B;
    Some(match name {
        "LinearSolve" => B::LinearSolve,
        "abs" => B::Abs,
        "sign" => B::Sign,
        "sqrt" => B::Sqrt,
        "div" => B::Div,
        "mod" => B::Mod,
        "rem" => B::Rem,
        "floor" => B::Floor,
        "ceil" => B::Ceil,
        "integer" => B::Integer,
        "sin" => B::Sin,
        "cos" => B::Cos,
        "tan" => B::Tan,
        "asin" => B::Asin,
        "acos" => B::Acos,
        "atan" => B::Atan,
        "atan2" => B::Atan2,
        "sinh" => B::Sinh,
        "cosh" => B::Cosh,
        "tanh" => B::Tanh,
        "exp" => B::Exp,
        "log" => B::Log,
        "log10" => B::Log10,
        "smooth" => B::Smooth,
        "noEvent" => B::NoEvent,
        "homotopy" => B::Homotopy,
        "min" => B::Min,
        "max" => B::Max,
        "sum" => B::Sum,
        "product" => B::Product,
        "size" => B::Size,
        "zeros" => B::Zeros,
        "ones" => B::Ones,
        "fill" => B::Fill,
        "linspace" => B::Linspace,
        "cross" => B::Cross,
        "identity" => B::Identity,
        "vector" => B::Vector,
        "transpose" => B::Transpose,
        "diagonal" => B::Diagonal,
        "outerProduct" => B::OuterProduct,
        "skew" => B::Skew,
        "cat1" => B::PromotedCat1,
        "cat2" => B::PromotedCat2,
        _ => return None,
    })
}

fn unary_of(op: RbcUnaryOp) -> dae::UnaryOperator {
    match op {
        RbcUnaryOp::Negate => dae::UnaryOperator::Negate,
        RbcUnaryOp::Not => dae::UnaryOperator::Not,
        RbcUnaryOp::Plus => dae::UnaryOperator::Plus,
    }
}

fn binary_of(op: RbcBinaryOp) -> dae::BinaryOperator {
    match op {
        RbcBinaryOp::Add => dae::BinaryOperator::Add,
        RbcBinaryOp::Subtract => dae::BinaryOperator::Subtract,
        RbcBinaryOp::Multiply => dae::BinaryOperator::Multiply,
        RbcBinaryOp::Divide => dae::BinaryOperator::Divide,
        RbcBinaryOp::Power => dae::BinaryOperator::Power,
        RbcBinaryOp::Equal => dae::BinaryOperator::Equal,
        RbcBinaryOp::NotEqual => dae::BinaryOperator::NotEqual,
        RbcBinaryOp::Less => dae::BinaryOperator::Less,
        RbcBinaryOp::LessEqual => dae::BinaryOperator::LessEqual,
        RbcBinaryOp::Greater => dae::BinaryOperator::Greater,
        RbcBinaryOp::GreaterEqual => dae::BinaryOperator::GreaterEqual,
        RbcBinaryOp::And => dae::BinaryOperator::And,
        RbcBinaryOp::Or => dae::BinaryOperator::Or,
        RbcBinaryOp::ElementwiseAdd => dae::BinaryOperator::ElementwiseAdd,
        RbcBinaryOp::ElementwiseSubtract => dae::BinaryOperator::ElementwiseSubtract,
        RbcBinaryOp::ElementwiseMultiply => dae::BinaryOperator::ElementwiseMultiply,
        RbcBinaryOp::ElementwiseDivide => dae::BinaryOperator::ElementwiseDivide,
        RbcBinaryOp::ElementwisePower => dae::BinaryOperator::ElementwisePower,
    }
}

fn causality_of(causality: RbcCausality) -> dae::VariableCausality {
    match causality {
        RbcCausality::Input => dae::VariableCausality::Input,
        RbcCausality::Output => dae::VariableCausality::Output,
        RbcCausality::Parameter => dae::VariableCausality::Parameter,
        RbcCausality::CalculatedParameter => dae::VariableCausality::CalculatedParameter,
        RbcCausality::Independent => dae::VariableCausality::Independent,
        RbcCausality::Local => dae::VariableCausality::Local,
    }
}

//! Rebuilding carried function bodies alongside the expression arena.
//!
//! A carried body and the arena cannot be rebuilt one after the other. A
//! `FunctionValue` node in the arena names a definition that exists only
//! while its function is under construction, and that function's statements
//! name arena expressions. So the two interleave: each function is opened
//! in id order (callees first, which is the order the DAE assigned the ids
//! and the order `check_call_graph_acyclic` proves consistent), the arena is
//! rebuilt forward through the last node that function's body needs, and a
//! statement is issued as soon as every expression it names exists.
//!
//! Three kinds of node are not built directly but *issued* by the body:
//!
//! - `FunctionValue` is a read, which has to see exactly the definition the
//!   artifact names, so statements are replayed up to that definition first;
//! - `FunctionFoldParameter` and `FunctionFoldOutput` are created by opening
//!   and closing a loop, one per carried value, so reaching one means the
//!   next statement must be that loop's opening or closing.
//!
//! This is the same discipline the DAE's own wire replay follows, driven
//! from the artifact instead of from the DAE's private storage, and every
//! step goes through the checked constructor a compiler would call.

use super::*;

/// Functions rebuilt so far, as later expressions address them.
#[derive(Default)]
pub(super) struct FunctionTable<'dae> {
    /// `None` for a function whose body the artifact elides: it has no
    /// definition to rebuild, so a call to it is refused where it occurs.
    pub(super) ids: Vec<Option<dae::FunctionId<'dae>>>,
    pub(super) parameters: Vec<Vec<dae::FunctionParameterId<'dae>>>,
}

/// The arena as rebuilt so far, plus the function tables it may name.
struct Stream<'dae> {
    built: Vec<dae::ExprId<'dae>>,
    functions: FunctionTable<'dae>,
    reservations: Vec<Option<dae::VariableReservation<'dae>>>,
    /// Parameters and constants, as (arena length at which every attribute
    /// expression exists, variable index), in that order.
    early: Vec<(usize, usize)>,
    next_early: usize,
    /// Model quotient owners re-issued so far, awaiting their relation,
    /// activation and root.
    quotients: Vec<dae::QuotientReplayToken<'dae>>,
}

/// Rebuild the whole expression arena, and every carried function with it.
pub(super) fn rebuild_arena<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    base: &Tables<'_, 'dae>,
    reservations: Vec<dae::VariableReservation<'dae>>,
) -> Result<ArenaResult<'dae>, dae::DaeConstructionError> {
    let model = ctx.model;
    let ends = expression_ends(model);
    let mut stream = Stream {
        built: Vec::with_capacity(model.expressions.len()),
        functions: FunctionTable::default(),
        reservations: reservations.into_iter().map(Some).collect(),
        early: early_parameters(model),
        next_early: 0,
        quotients: Vec::new(),
    };
    for (index, function) in model.functions.iter().enumerate() {
        rebuild_function(
            construction,
            ctx,
            base,
            &mut stream,
            index,
            function,
            ends[index],
        )?;
    }
    rebuild_derivatives(construction, ctx, &stream.functions)?;
    while stream.built.len() < model.expressions.len() {
        let index = stream.built.len();
        if let Some(function) = body_scope(&model.expressions[index].node) {
            return Err(ctx.unsupported(format!(
                "expression {index} reads the body of function {function} outside \
                 the range that function's statements cover"
            )));
        }
        plain(construction, ctx, base, &mut stream)?;
    }
    Ok(ArenaResult {
        expressions: stream.built,
        reservations: stream.reservations,
        quotients: stream.quotients,
    })
}

/// What the arena pass hands to the stages after it.
pub(super) struct ArenaResult<'dae> {
    pub(super) expressions: Vec<dae::ExprId<'dae>>,
    /// Variables not yet defined; parameters defined early are `None`.
    pub(super) reservations: Vec<Option<dae::VariableReservation<'dae>>>,
    pub(super) quotients: Vec<dae::QuotientReplayToken<'dae>>,
}

/// Parameters and constants, keyed by when their attributes become buildable.
///
/// The DAE resolves a structural extent such as `fill(u, nout)` through the
/// parameter's binding, so `nout` has to be *defined*, not just reserved,
/// before that node is built. Defining every variable after the arena, as
/// import once did, rejected any model whose array sizes are parameters.
fn early_parameters(model: &RbcModel) -> Vec<(usize, usize)> {
    let mut early: Vec<(usize, usize)> = model
        .variables
        .iter()
        .enumerate()
        .filter(|(_, variable)| matches!(variable.role, RbcRole::Parameter | RbcRole::Constant))
        .map(|(index, variable)| {
            let ready = [
                variable.binding,
                variable.start,
                variable.min,
                variable.max,
                variable.nominal,
            ]
            .into_iter()
            .flatten()
            .map(|id| id.0 as usize + 1)
            .max()
            .unwrap_or(0);
            (ready, index)
        })
        .collect();
    early.sort_unstable();
    early
}

/// Define every parameter whose attribute expressions all exist by now.
fn define_ready<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    stream: &mut Stream<'dae>,
) -> Result<(), dae::DaeConstructionError> {
    while let Some(&(ready, index)) = stream.early.get(stream.next_early) {
        if ready > stream.built.len() {
            break;
        }
        stream.next_early += 1;
        let Some(reservation) = stream.reservations[index].take() else {
            continue;
        };
        let variable = &ctx.model.variables[index];
        let built = &stream.built;
        construction
            .variables(|owner| define_variable(owner, ctx, variable, reservation, built))?;
    }
    Ok(())
}

/// Build the next arena node through the ordinary expression constructors.
fn plain<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    base: &Tables<'_, 'dae>,
    stream: &mut Stream<'dae>,
) -> Result<(), dae::DaeConstructionError> {
    define_ready(construction, ctx, stream)?;
    let index = stream.built.len();
    let expression = &ctx.model.expressions[index];
    let at = ctx.provenance(expression.provenance)?;
    if let Some(builtin) = quotients::model_owner(ctx.model, index) {
        return replay_model_quotient(construction, ctx, stream, builtin, at);
    }
    if let RbcExprNode::Coordinate {
        coordinate: RbcCoordinate::Delay { delay },
    } = expression.node
    {
        let coordinate = temporal::replay_delay(construction, ctx, &stream.built, delay, at)?;
        return push_expected(ctx, stream, coordinate);
    }
    let tables = Tables {
        functions: &stream.functions.ids,
        parameters: &stream.functions.parameters,
        ..*base
    };
    let built = &stream.built;
    let id = construction
        .expressions(|owner| build_expression(owner, ctx, expression, built, &tables, at))?;
    stream.built.push(id);
    Ok(())
}

/// Re-issue one model quotient owner's seven-node batch at the current
/// position, keeping its token for the condition stage.
fn replay_model_quotient<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    stream: &mut Stream<'dae>,
    builtin: dae::PureBuiltin,
    at: dae::DaeProvenance,
) -> Result<(), dae::DaeConstructionError> {
    let node = &ctx.model.expressions[stream.built.len()].node;
    let Some((_, [lhs, rhs])) = quotients::operands(node) else {
        return Err(ctx.unsupported("quotient owner without two operands"));
    };
    let arguments = [
        resolve(&stream.built, lhs.0, "expression", ctx)?,
        resolve(&stream.built, rhs.0, "expression", ctx)?,
    ];
    let token = construction.begin_quotient_replay(builtin, arguments, at)?;
    push_expected(ctx, stream, token.quotient())?;
    for generated in token.generated() {
        push_expected(ctx, stream, generated)?;
    }
    stream.quotients.push(token);
    Ok(())
}

/// The function whose body a node reads directly, if any.
fn body_scope(node: &RbcExprNode) -> Option<u32> {
    match node {
        RbcExprNode::FunctionValue { function, .. }
        | RbcExprNode::FunctionFoldParameter { function, .. }
        | RbcExprNode::FunctionFoldOutput { function, .. } => Some(function.0),
        _ => None,
    }
}

/// The function a node belongs to: the one whose parameters or body it
/// reads, directly or through an operand.
fn scopes(model: &RbcModel) -> Vec<Option<u32>> {
    let mut scopes: Vec<Option<u32>> = Vec::with_capacity(model.expressions.len());
    for (index, expression) in model.expressions.iter().enumerate() {
        let mut scope = match &expression.node {
            RbcExprNode::Coordinate {
                coordinate: RbcCoordinate::FunctionParameter { function, .. },
            } => Some(function.0),
            node => body_scope(node),
        };
        for operand in crate::build::references(ExprId(index as u32), &expression.node) {
            if let Some(Some(function)) = scopes.get(operand.0 as usize) {
                scope = scope.or(Some(*function));
            }
        }
        scopes.push(scope);
    }
    scopes
}

/// Per function, one past the last arena node its reconstruction needs.
///
/// That is the furthest node its statements or external arguments reach,
/// and the furthest node scoped to it. Made monotone, because functions are
/// rebuilt in id order and a later function's range cannot end before an
/// earlier one's.
fn expression_ends(model: &RbcModel) -> Vec<usize> {
    let count = model.expressions.len();
    let mut ends = vec![0usize; model.functions.len()];
    for (index, scope) in scopes(model).into_iter().enumerate() {
        if let Some(end) = scope.and_then(|function| ends.get_mut(function as usize)) {
            *end = (*end).max(index + 1);
        }
    }
    // One stamp array for every walk, rather than a fresh visited set per
    // function, so a large arena with many functions is walked in linear
    // memory.
    let mut stamp = vec![usize::MAX; count];
    for (function_index, function) in model.functions.iter().enumerate() {
        let mut pending = roots(function);
        while let Some(id) = pending.pop() {
            let index = id.0 as usize;
            if index >= count || stamp[index] == function_index {
                continue;
            }
            stamp[index] = function_index;
            ends[function_index] = ends[function_index].max(index + 1);
            pending.extend(crate::build::references(id, &model.expressions[index].node));
        }
    }
    let mut previous = 0;
    for end in &mut ends {
        *end = (*end).max(previous);
        previous = *end;
    }
    ends
}

fn roots(function: &RbcFunction) -> Vec<ExprId> {
    let mut found = Vec::new();
    match &function.body {
        RbcFunctionBody::Modelica { statements } => statement_roots(statements, &mut found),
        RbcFunctionBody::External { arguments, .. } => {
            for argument in arguments {
                if let RbcExternalArgument::Input { expression } = argument {
                    found.push(*expression);
                }
            }
        }
        RbcFunctionBody::ElidedModelica => {}
    }
    found
}

fn statement_roots(statements: &[RbcFunctionStatement], found: &mut Vec<ExprId>) {
    for statement in statements {
        match statement {
            RbcFunctionStatement::Assignment { expression, .. } => found.push(*expression),
            RbcFunctionStatement::Assertion {
                condition, message, ..
            } => found.extend([*condition, *message]),
            RbcFunctionStatement::AssignmentGroup {
                expressions,
                conditional,
                ..
            } => {
                found.extend(expressions.iter().copied());
                if let Some(conditional) = conditional {
                    found.extend(conditional.conditions.iter().copied());
                    found.extend(conditional.branches.iter().flatten().copied());
                    found.extend(conditional.fallback.iter().copied());
                }
            }
            RbcFunctionStatement::For { statements, .. } => statement_roots(statements, found),
        }
    }
}

fn rebuild_function<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    base: &Tables<'_, 'dae>,
    stream: &mut Stream<'dae>,
    index: usize,
    function: &RbcFunction,
    end: usize,
) -> Result<(), dae::DaeConstructionError> {
    if matches!(function.body, RbcFunctionBody::ElidedModelica) {
        stream.functions.ids.push(None);
        stream.functions.parameters.push(Vec::new());
        return Ok(());
    }
    let declaration = ctx.provenance(function.declaration)?;
    let types = |ids: &mut dyn Iterator<Item = TypeId>| {
        ids.map(|ty| resolve(base.types, ty.0, "type", ctx))
            .collect::<Result<Vec<_>, _>>()
    };
    let parameters = types(&mut function.parameters.iter().map(|p| p.value_type))?;
    let results = types(&mut function.results.iter().copied())?;
    let signature = dae::FunctionSignature::new(
        rumoca_core::VarName::intern(&function.name),
        parameters,
        results,
        declaration,
    )
    .with_inline(match function.inline {
        RbcInline::Unstated => rumoca_core::InlineAnnotation::Unstated,
        RbcInline::Requested => rumoca_core::InlineAnnotation::Requested,
        RbcInline::Never => rumoca_core::InlineAnnotation::Never,
        RbcInline::AfterIndexReduction => rumoca_core::InlineAnnotation::AfterIndexReduction,
    });
    construction.function(signature, |c, reservation| {
        let mut parameters = Vec::with_capacity(function.parameters.len());
        for (ordinal, parameter) in function.parameters.iter().enumerate() {
            let at = match parameter.declaration {
                Some(provenance) => ctx.provenance(provenance)?,
                None => declaration,
            };
            let name = rumoca_core::VarName::intern(&parameter.name);
            parameters.push(c.functions(|f| f.parameter(&reservation, name, ordinal, at))?);
        }
        stream.functions.ids.push(Some(reservation.function()));
        stream.functions.parameters.push(parameters);

        let mut values = Vec::with_capacity(function.values.len());
        let mut outputs = 0;
        for value in &function.values {
            let at = ctx.provenance(value.declaration)?;
            let name = rumoca_core::VarName::intern(&value.name);
            values.push(match value.role {
                RbcFunctionValueRole::Output => {
                    let id = c.functions(|f| f.output(&reservation, name, outputs, at))?;
                    outputs += 1;
                    id
                }
                RbcFunctionValueRole::Local => {
                    let ty = resolve(base.types, value.value_type.0, "type", ctx)?;
                    c.functions(|f| f.local(&reservation, name, ty, at))?
                }
            });
        }

        match &function.body {
            RbcFunctionBody::External { .. } => {
                while stream.built.len() < end {
                    plain(c, ctx, base, stream)?;
                }
                let body = external_body(ctx, function, &values, &stream.built)?;
                c.functions(|f| f.define_external(reservation, body, declaration))
            }
            RbcFunctionBody::Modelica { statements } => {
                let body = c.functions(|f| f.begin(reservation, declaration))?;
                let mut replay = Replay {
                    index,
                    function,
                    base: *base,
                    values,
                    operations: flatten(statements),
                    next: 0,
                    capability: Some(Capability::Body(body)),
                    depth: 0,
                };
                replay.run(c, ctx, stream, end)?;
                replay.finish(c, ctx, stream, declaration)
            }
            RbcFunctionBody::ElidedModelica => unreachable!("elided bodies return above"),
        }
    })?;
    Ok(())
}

/// The export encoding: the DAE's variant name, lowercased.
fn external_language(language: &str) -> Option<dae::ExternalLanguage> {
    Some(match language {
        "c" => dae::ExternalLanguage::C,
        "fortran77" => dae::ExternalLanguage::Fortran77,
        "builtin" => dae::ExternalLanguage::Builtin,
        _ => return None,
    })
}

/// One step of a body, in the order the body issues them.
#[derive(Clone, Copy)]
enum Operation<'m> {
    Assign {
        value: u32,
        expression: ExprId,
        provenance: RbcProvenance,
    },
    Group {
        values: &'m [u32],
        expressions: &'m [ExprId],
        conditional: Option<&'m RbcFunctionConditional>,
        provenance: RbcProvenance,
    },
    Assert {
        condition: ExprId,
        message: ExprId,
        level: RbcAssertionLevel,
        provenance: RbcProvenance,
    },
    Open {
        fold: u32,
    },
    Close {
        fold: u32,
        provenance: RbcProvenance,
    },
}

fn flatten(statements: &[RbcFunctionStatement]) -> Vec<Operation<'_>> {
    let mut operations = Vec::with_capacity(statements.len());
    push_operations(statements, &mut operations);
    operations
}

fn push_operations<'m>(statements: &'m [RbcFunctionStatement], out: &mut Vec<Operation<'m>>) {
    for statement in statements {
        match statement {
            RbcFunctionStatement::Assignment {
                value,
                expression,
                provenance,
            } => out.push(Operation::Assign {
                value: *value,
                expression: *expression,
                provenance: *provenance,
            }),
            RbcFunctionStatement::AssignmentGroup {
                values,
                conditional,
                expressions,
                provenance,
            } => out.push(Operation::Group {
                values,
                expressions,
                conditional: conditional.as_ref(),
                provenance: *provenance,
            }),
            RbcFunctionStatement::Assertion {
                condition,
                message,
                level,
                provenance,
            } => out.push(Operation::Assert {
                condition: *condition,
                message: *message,
                level: *level,
                provenance: *provenance,
            }),
            RbcFunctionStatement::For {
                fold,
                statements,
                provenance,
            } => {
                out.push(Operation::Open { fold: *fold });
                push_operations(statements, out);
                out.push(Operation::Close {
                    fold: *fold,
                    provenance: *provenance,
                });
            }
        }
    }
}

/// Values, expressions, branch correlation and provenance of one group.
type GroupOperation<'m> = (
    &'m [u32],
    &'m [ExprId],
    Option<&'m RbcFunctionConditional>,
    RbcProvenance,
);

enum Capability<'dae> {
    Body(dae::FunctionBody<'dae>),
    Loop(dae::FunctionLoop<'dae>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transition {
    Open,
    Close,
}

struct Replay<'m, 'dae> {
    index: usize,
    function: &'m RbcFunction,
    base: Tables<'m, 'dae>,
    values: Vec<dae::FunctionValueId<'dae>>,
    operations: Vec<Operation<'m>>,
    next: usize,
    capability: Option<Capability<'dae>>,
    /// Open loops. Closing the outermost returns a body capability, closing
    /// an inner one returns the enclosing loop, and the constructors differ.
    depth: usize,
}

impl<'dae> Replay<'_, 'dae> {
    /// Rebuild the arena forward to `end`, issuing statements as the nodes
    /// that depend on them arrive.
    fn run(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        end: usize,
    ) -> Result<(), dae::DaeConstructionError> {
        while stream.built.len() < end {
            let position = stream.built.len();
            let expression = &ctx.model.expressions[position];
            if let Some(owner) = body_scope(&expression.node) {
                self.expect_owner(ctx, owner, position)?;
            }
            match &expression.node {
                RbcExprNode::FunctionValue {
                    value, definition, ..
                } => self.read(construction, ctx, stream, *value, *definition, expression)?,
                RbcExprNode::FunctionFoldParameter { .. } => {
                    self.transition(construction, ctx, stream, Transition::Open)?;
                }
                RbcExprNode::FunctionFoldOutput { .. } => {
                    self.transition(construction, ctx, stream, Transition::Close)?;
                }
                _ => self.plain_or_quotient(construction, ctx, stream)?,
            }
        }
        Ok(())
    }

    /// Build a plain node, or, for a quotient the DAE refuses as a bare
    /// builtin, re-issue it owned by this function (MLS §3.7.2: event-free
    /// inside a function body).
    fn plain_or_quotient(
        &self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
    ) -> Result<(), dae::DaeConstructionError> {
        let expression = &ctx.model.expressions[stream.built.len()];
        let refused = match plain(construction, ctx, &self.base, stream) {
            Err(dae::DaeConstructionError::NonStaticDiscontinuity { .. }) => {
                quotients::operands(&expression.node)
            }
            other => return other,
        };
        let Some((builtin, [lhs, rhs])) = refused else {
            return Err(ctx.unsupported("non-static discontinuity that is not a quotient"));
        };
        let at = ctx.provenance(expression.provenance)?;
        let arguments = [
            resolve(&stream.built, lhs.0, "expression", ctx)?,
            resolve(&stream.built, rhs.0, "expression", ctx)?,
        ];
        let body = self.body(ctx)?;
        let quotient = construction.function_runtime_quotient(body, builtin, arguments, at)?;
        push_expected(ctx, stream, quotient)
    }

    fn expect_owner(
        &self,
        ctx: &Rebuild<'_>,
        owner: u32,
        position: usize,
    ) -> Result<(), dae::DaeConstructionError> {
        if owner as usize == self.index {
            Ok(())
        } else {
            Err(ctx.unsupported(format!(
                "expression {position} reads the body of function {owner} while function {} \
                 is being rebuilt",
                self.index
            )))
        }
    }

    fn value(
        &self,
        ctx: &Rebuild<'_>,
        ordinal: u32,
    ) -> Result<dae::FunctionValueId<'dae>, dae::DaeConstructionError> {
        resolve(&self.values, ordinal, "function value", ctx)
    }

    fn body(
        &self,
        ctx: &Rebuild<'_>,
    ) -> Result<&dae::FunctionBody<'dae>, dae::DaeConstructionError> {
        match &self.capability {
            Some(Capability::Body(body)) => Ok(body),
            Some(Capability::Loop(body)) => Ok(body.body()),
            None => Err(ctx.unsupported("function body capability lost during replay")),
        }
    }

    /// The definition `value` currently holds, or `None` before its first.
    fn current(
        &self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        value: dae::FunctionValueId<'dae>,
        at: dae::DaeProvenance,
    ) -> Result<Option<u32>, dae::DaeConstructionError> {
        let body = self.body(ctx)?;
        match construction.functions(|f| f.current_definition_id(body, value, at)) {
            Ok(definition) => Ok(Some(definition.ordinal())),
            Err(dae::DaeConstructionError::IncompleteDefinition {
                kind: "function value",
                index,
                ..
            }) if index == value.ordinal() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Issue statements until `value` holds `definition`, then read it.
    fn read(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        value: u32,
        definition: u32,
        expression: &RbcExpr,
    ) -> Result<(), dae::DaeConstructionError> {
        let at = ctx.provenance(expression.provenance)?;
        let id = self.value(ctx, value)?;
        loop {
            let current = self.current(construction, ctx, id, at)?;
            if current == Some(definition) {
                break;
            }
            if current.is_some_and(|current| current > definition) {
                return Err(ctx.unsupported(format!(
                    "expression {} reads definition {definition} of value {value}, which \
                     a later statement has already replaced",
                    expression.id.0
                )));
            }
            if !self.apply_next(construction, ctx, stream)? {
                return Err(ctx.unsupported(format!(
                    "expression {} reads definition {definition} of value {value}, which \
                     no statement before it defines",
                    expression.id.0
                )));
            }
        }
        let body = self.body(ctx)?;
        let read = construction.functions(|f| f.read(body, id, at))?;
        push_expected(ctx, stream, read)
    }

    /// Issue the next statement if everything it names already exists.
    fn apply_next(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
    ) -> Result<bool, dae::DaeConstructionError> {
        let Some(operation) = self.operations.get(self.next).copied() else {
            return Ok(false);
        };
        let ready = |id: &ExprId| (id.0 as usize) < stream.built.len();
        let expression = |id: ExprId| resolve(&stream.built, id.0, "expression", ctx);
        match operation {
            Operation::Assign {
                value,
                expression: rhs,
                provenance,
            } => {
                if !ready(&rhs) {
                    return Ok(false);
                }
                let target = self.value(ctx, value)?;
                let rhs = expression(rhs)?;
                let at = ctx.provenance(provenance)?;
                let capability = self.capability_mut(ctx)?;
                construction.functions(|f| match capability {
                    Capability::Body(body) => f.assign(body, target, rhs, at),
                    Capability::Loop(body) => f.assign_loop(body, target, rhs, at),
                })?;
            }
            Operation::Group {
                values,
                expressions,
                conditional,
                provenance,
            } => {
                let group = (values, expressions, conditional, provenance);
                if !self.apply_group(construction, ctx, stream, group)? {
                    return Ok(false);
                }
            }
            Operation::Assert {
                condition,
                message,
                level,
                provenance,
            } => {
                if !ready(&condition) || !ready(&message) {
                    return Ok(false);
                }
                let condition = expression(condition)?;
                let message = expression(message)?;
                let at = ctx.provenance(provenance)?;
                let capability = self.capability_mut(ctx)?;
                let level = match level {
                    RbcAssertionLevel::Error => dae::AssertionLevel::Error,
                    RbcAssertionLevel::Warning => dae::AssertionLevel::Warning,
                };
                construction.functions(|f| match capability {
                    Capability::Body(body) => {
                        f.assertion_with_level(body, condition, message, level, at)
                    }
                    Capability::Loop(body) => {
                        f.assertion_loop_with_level(body, condition, message, level, at)
                    }
                })?;
            }
            // A loop with carried values issues nodes of its own, so it opens
            // or closes only when the arena reaches them; one without any can
            // be issued as soon as it is next.
            Operation::Open { fold } => {
                if !self.fold(ctx, fold)?.targets.is_empty() {
                    return Ok(false);
                }
                self.open(construction, ctx, stream, fold)?;
            }
            Operation::Close { fold, provenance } => {
                if !self.fold(ctx, fold)?.targets.is_empty() {
                    return Ok(false);
                }
                self.close(construction, ctx, stream, fold, provenance)?;
            }
        }
        self.next += 1;
        Ok(true)
    }

    /// Issue one grouped assignment, or report it not yet ready.
    fn apply_group(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &Stream<'dae>,
        group: GroupOperation<'_>,
    ) -> Result<bool, dae::DaeConstructionError> {
        let (values, expressions, conditional, provenance) = group;
        let ready = |id: &ExprId| (id.0 as usize) < stream.built.len();
        let conditional_ready = conditional.is_none_or(|c| {
            c.conditions
                .iter()
                .chain(c.branches.iter().flatten())
                .chain(&c.fallback)
                .all(ready)
        });
        if !expressions.iter().all(ready) || !conditional_ready {
            return Ok(false);
        }
        let expression = |id: ExprId| resolve(&stream.built, id.0, "expression", ctx);
        let at = ctx.provenance(provenance)?;
        let assignments = values
            .iter()
            .zip(expressions)
            .map(|(value, rhs)| Ok((self.value(ctx, *value)?, expression(*rhs)?)))
            .collect::<Result<Vec<_>, dae::DaeConstructionError>>()?;
        let list = |ids: &[ExprId]| {
            ids.iter()
                .map(|id| expression(*id))
                .collect::<Result<Vec<_>, _>>()
        };
        let correlation = match conditional {
            Some(c) => Some((
                list(&c.conditions)?,
                c.branches
                    .iter()
                    .map(|branch| list(branch))
                    .collect::<Result<Vec<_>, _>>()?,
                list(&c.fallback)?,
            )),
            None => None,
        };
        let capability = self.capability_mut(ctx)?;
        construction.functions(|f| match (capability, &correlation) {
            (Capability::Body(body), Some((conditions, branches, fallback))) => {
                f.replay_conditional_all(body, &assignments, conditions, branches, fallback, at)
            }
            (Capability::Loop(body), Some((conditions, branches, fallback))) => f
                .replay_conditional_all_loop(
                    body,
                    &assignments,
                    conditions,
                    branches,
                    fallback,
                    at,
                ),
            (Capability::Body(body), None) => f.assign_all(body, &assignments, at),
            (Capability::Loop(body), None) => f.assign_all_loop(body, &assignments, at),
        })?;
        Ok(true)
    }

    fn capability_mut(
        &mut self,
        ctx: &Rebuild<'_>,
    ) -> Result<&mut Capability<'dae>, dae::DaeConstructionError> {
        self.capability
            .as_mut()
            .ok_or_else(|| ctx.unsupported("function body capability lost during replay"))
    }

    fn fold<'s>(
        &'s self,
        ctx: &Rebuild<'_>,
        ordinal: u32,
    ) -> Result<&'s RbcFunctionFold, dae::DaeConstructionError> {
        self.function
            .folds
            .get(ordinal as usize)
            .filter(|fold| fold.ordinal == ordinal)
            .ok_or_else(|| ctx.unsupported(format!("fold {ordinal} is not declared")))
    }

    /// The arena reached a node a loop transition issues: issue everything
    /// ready before it, then the transition, which must be next.
    fn transition(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        expected: Transition,
    ) -> Result<(), dae::DaeConstructionError> {
        while self.apply_next(construction, ctx, stream)? {}
        let position = stream.built.len();
        match (self.operations.get(self.next).copied(), expected) {
            (Some(Operation::Open { fold }), Transition::Open) => {
                self.open(construction, ctx, stream, fold)?;
            }
            (Some(Operation::Close { fold, provenance }), Transition::Close) => {
                self.close(construction, ctx, stream, fold, provenance)?;
            }
            _ => {
                return Err(ctx.unsupported(format!(
                    "expression {position} is a loop {} value, but the body's next \
                     statement does not {} a loop",
                    if expected == Transition::Open {
                        "parameter"
                    } else {
                        "output"
                    },
                    if expected == Transition::Open {
                        "open"
                    } else {
                        "close"
                    },
                )));
            }
        }
        self.next += 1;
        Ok(())
    }

    fn open(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        ordinal: u32,
    ) -> Result<(), dae::DaeConstructionError> {
        let fold = self.fold(ctx, ordinal)?;
        let at = ctx.provenance(fold.provenance)?;
        let targets = fold
            .targets
            .iter()
            .map(|value| self.value(ctx, *value))
            .collect::<Result<Vec<_>, _>>()?;
        let locals = fold
            .iteration_locals
            .iter()
            .map(|value| self.value(ctx, *value))
            .collect::<Result<Vec<_>, _>>()?;
        let domain = resolve(self.base.domains, fold.domain.0, "domain", ctx)?;
        let capability = self
            .capability
            .take()
            .ok_or_else(|| ctx.unsupported("function body capability lost during replay"))?;
        let opened = construction.functions(|f| match capability {
            Capability::Body(parent) => f.begin_loop_with_iteration_locals(
                parent,
                domain,
                targets.iter().copied(),
                locals.iter().copied(),
                at,
            ),
            Capability::Loop(parent) => f.begin_nested_loop_with_iteration_locals(
                parent,
                domain,
                targets.iter().copied(),
                locals.iter().copied(),
                at,
            ),
        })?;
        self.capability = Some(Capability::Loop(opened));
        self.depth += 1;
        self.record_generated(construction, ctx, stream, &targets, at)
    }

    fn close(
        &mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        ordinal: u32,
        provenance: RbcProvenance,
    ) -> Result<(), dae::DaeConstructionError> {
        let targets = self
            .fold(ctx, ordinal)?
            .targets
            .iter()
            .map(|value| self.value(ctx, *value))
            .collect::<Result<Vec<_>, _>>()?;
        let at = ctx.provenance(provenance)?;
        let Some(Capability::Loop(body)) = self.capability.take() else {
            return Err(ctx.unsupported(format!("fold {ordinal} closes, but no loop is open")));
        };
        let nested = self.depth > 1;
        let closed = construction.functions(|f| {
            if nested {
                f.finish_nested_loop(body, at).map(Capability::Loop)
            } else {
                f.finish_loop(body, at).map(Capability::Body)
            }
        })?;
        self.capability = Some(closed);
        self.depth -= 1;
        self.record_generated(construction, ctx, stream, &targets, at)
    }

    /// Take up the nodes a loop transition just issued, one per carried
    /// value, as the arena entries at the current position.
    fn record_generated(
        &self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        targets: &[dae::FunctionValueId<'dae>],
        at: dae::DaeProvenance,
    ) -> Result<(), dae::DaeConstructionError> {
        for target in targets {
            let body = self.body(ctx)?;
            let issued = construction.functions(|f| f.current_definition_rhs(body, *target, at))?;
            push_expected(ctx, stream, issued)?;
        }
        Ok(())
    }

    fn finish(
        mut self,
        construction: &mut dae::DaeConstruction<'dae>,
        ctx: &Rebuild<'_>,
        stream: &mut Stream<'dae>,
        declaration: dae::DaeProvenance,
    ) -> Result<(), dae::DaeConstructionError> {
        while self.apply_next(construction, ctx, stream)? {}
        if self.next != self.operations.len() {
            return Err(ctx.unsupported(format!(
                "function {} ({}): statement {} of {} names an expression its \
                 range of the arena does not reach",
                self.index,
                self.function.name,
                self.next,
                self.operations.len()
            )));
        }
        match self.capability.take() {
            Some(Capability::Body(body)) => construction.functions(|f| f.define(body, declaration)),
            _ => Err(ctx.unsupported(format!(
                "function {} ({}) ends inside an open loop",
                self.index, self.function.name
            ))),
        }
    }
}

/// Record a node the body issued, which must take the next arena position.
///
/// The artifact's arena is the DAE's, projected id for id. A node landing
/// anywhere else means the replay has diverged from what was exported, and
/// every later reference would resolve to the wrong expression.
fn push_expected<'dae>(
    ctx: &Rebuild<'_>,
    stream: &mut Stream<'dae>,
    issued: dae::ExprId<'dae>,
) -> Result<(), dae::DaeConstructionError> {
    let position = stream.built.len();
    if issued.index() as usize != position {
        return Err(ctx.unsupported(format!(
            "replaying a function body issued expression {} where the artifact has \
             expression {position}",
            issued.index()
        )));
    }
    stream.built.push(issued);
    Ok(())
}

/// The MLS §12.9 interface of an external function, rebuilt from its ABI.
fn external_body<'dae>(
    ctx: &Rebuild<'_>,
    function: &RbcFunction,
    values: &[dae::FunctionValueId<'dae>],
    built: &[dae::ExprId<'dae>],
) -> Result<dae::ExternalFunctionBody<'dae>, dae::DaeConstructionError> {
    let RbcFunctionBody::External {
        language,
        symbol,
        purity,
        arguments,
        result,
        linkage,
    } = &function.body
    else {
        return Err(ctx.unsupported("external interface requested for a Modelica body"));
    };
    let value = |ordinal: u32| resolve(values, ordinal, "function value", ctx);
    let arguments = arguments
        .iter()
        .map(|argument| match argument {
            RbcExternalArgument::Input { expression } => {
                resolve(built, expression.0, "expression", ctx).map(dae::ExternalArgument::Input)
            }
            RbcExternalArgument::Output { value: ordinal } => {
                value(*ordinal).map(dae::ExternalArgument::Output)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let language = external_language(language)
        .ok_or_else(|| ctx.unsupported(format!("unknown external language `{language}`")))?;
    Ok(dae::ExternalFunctionBody::new(
        match purity {
            RbcPurity::Pure => dae::FunctionPurity::Pure,
            RbcPurity::Impure => dae::FunctionPurity::Impure,
        },
        language,
        rumoca_core::VarName::intern(symbol),
        arguments,
        result.map(value).transpose()?,
        dae::ExternalLinkage::new(
            linkage.libraries.iter().cloned(),
            linkage.include.clone(),
            linkage.include_directory.clone(),
            linkage.library_directory.clone(),
        ),
    ))
}

/// A node that names a function: a call to one, or a parameter of the one
/// under construction.
pub(super) fn function_node<'dae>(
    owner: &mut dae::Expressions<'_, 'dae>,
    ctx: &Rebuild<'_>,
    expression: &RbcExpr,
    built: &[dae::ExprId<'dae>],
    tables: &Tables<'_, 'dae>,
    at: dae::DaeProvenance,
) -> Result<dae::ExprId<'dae>, dae::DaeConstructionError> {
    Ok(match &expression.node {
        RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::FunctionParameter { function, ordinal },
        } => {
            let parameter = tables
                .parameters
                .get(function.0 as usize)
                .and_then(|parameters| parameters.get(*ordinal as usize))
                .copied()
                .ok_or_else(|| {
                    ctx.unsupported(format!(
                        "expression {} names parameter {ordinal} of function {}, \
                         which is not under construction",
                        expression.id.0, function.0
                    ))
                })?;
            owner.at(at).function_parameter(parameter)?
        }
        RbcExprNode::Call {
            owner: call_owner,
            function,
            output,
            arguments,
        } => {
            let callee = tables
                .functions
                .get(function.0 as usize)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    let name = ctx
                        .model
                        .functions
                        .get(function.0 as usize)
                        .map(|f| f.name.as_str())
                        .unwrap_or("<unknown>");
                    ctx.unsupported(format!(
                        "expression {} calls `{name}`, whose body the artifact elides",
                        expression.id.0
                    ))
                })?;
            // The head of a call names itself as owner and carries the
            // arguments; a further result of the same call is a projection
            // of that head, sharing its one evaluation.
            if call_owner.0 as usize == built.len() {
                let mut operands = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    operands.push(resolve(built, argument.0, "expression", ctx)?);
                }
                owner.at(at).call(callee, *output as usize, operands)?
            } else {
                let head = resolve(built, call_owner.0, "expression", ctx)?;
                owner
                    .at(at)
                    .replay_call_projection(head, callee, *output as usize, None)?
            }
        }
        _ => return Err(ctx.unsupported("function_node called on a node naming no function")),
    })
}

/// Replay derivative chains through the checked API once their functions exist.
fn rebuild_derivatives<'dae>(
    construction: &mut dae::DaeConstruction<'dae>,
    ctx: &Rebuild<'_>,
    functions: &FunctionTable<'dae>,
) -> Result<(), dae::DaeConstructionError> {
    let mut links = std::collections::BTreeMap::new();
    let mut pending: Vec<_> = ctx
        .model
        .functions
        .iter()
        .flat_map(|function| {
            function
                .derivatives
                .iter()
                .enumerate()
                .map(move |(ordinal, link)| (function.id, ordinal as u32, link))
        })
        .collect();
    while !pending.is_empty() {
        let before = pending.len();
        let mut deferred = Vec::new();
        for (function, ordinal, link) in pending {
            let key = (function.0, ordinal);
            if (ordinal > 0 && !links.contains_key(&(function.0, ordinal - 1)))
                || link
                    .previous
                    .is_some_and(|(id, n)| !links.contains_key(&(id.0, n)))
            {
                deferred.push((function, ordinal, link));
                continue;
            }
            let source = resolve(&functions.ids, function.0, "derivative source", ctx)?
                .ok_or_else(|| ctx.unsupported("derivative source has no carried body"))?;
            let target = resolve(&functions.ids, link.target.0, "derivative target", ctx)?
                .ok_or_else(|| ctx.unsupported("derivative target has no carried body"))?;
            let at = ctx.provenance(link.provenance)?;
            let id = construction.functions(|owner| match link.previous {
                Some((source_id, n)) => owner.next_derivative(
                    source,
                    links[&(source_id.0, n)],
                    target,
                    link.inputs.iter().copied(),
                    link.priority,
                    at,
                ),
                None => owner.first_derivative(
                    source,
                    target,
                    link.inputs.iter().copied(),
                    link.priority,
                    at,
                ),
            })?;
            links.insert(key, id);
        }
        if deferred.len() == before {
            return Err(ctx.unsupported("derivative predecessor links are cyclic or undefined"));
        }
        pending = deferred;
    }
    Ok(())
}

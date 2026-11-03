//! Structural validation of an untrusted artifact.
//!
//! An artifact that has been through an external pass may reference objects
//! that do not exist, declare duplicate identities, or contain an expression
//! DAG with a cycle. This module rejects those *before* reconstruction is
//! attempted, so failures name the offending record instead of surfacing as an
//! opaque construction error.
//!
//! This is the first of two gates. It checks references, identity, and
//! acyclicity. It deliberately does **not** re-check DAE semantics: type
//! agreement, role legality, event ownership and the rest are enforced by the
//! DAE's own checked constructors during [`crate::import()`], which is the same
//! machinery production compilation uses. Duplicating those rules here would
//! create a second, weaker answer.

use std::collections::BTreeSet;

mod functions;
pub(crate) use functions::function_spans;
use functions::{check_call_graph_acyclic, check_function_bodies};

use crate::schema::*;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("invalid clock metadata: {0}")]
    Clock(String),
    #[error("invalid semantic connector metadata: {0}")]
    Connector(String),
    /// A host program names something the model does not declare. Separate
    /// from `Connector` because the program, not the connector metadata, is
    /// what has to change.
    #[error("{0}")]
    ExecutionReference(String),
    #[error(
        "{collection} entry at position {position} declares id {declared}, expected {expected}"
    )]
    NonDenseId {
        collection: &'static str,
        position: u32,
        declared: u32,
        expected: u32,
    },
    #[error("{referrer} references {target} {id}, which does not exist ({count} defined)")]
    DanglingReference {
        referrer: String,
        target: &'static str,
        id: u32,
        count: u32,
    },
    #[error(
        "expression {expression} references operand {operand}, which is not strictly earlier; the arena must be topologically ordered"
    )]
    NonTopologicalOperand { expression: u32, operand: u32 },
    #[error(
        "discrete-valued variable {id} ({name:?}) has no B.1c definition; a discrete variable that nothing defines cannot be reconstructed"
    )]
    UndefinedDiscreteValue { id: u32, name: String },
    #[error(
        "discrete definition {definition} branch {branch} gives {values} value(s) for {targets} target(s); each branch must give exactly one value per target"
    )]
    DiscreteBranchArity {
        definition: u32,
        branch: u32,
        values: u32,
        targets: u32,
    },
    #[error("duplicate variable name {name:?} on ids {first} and {second}")]
    DuplicateVariableName {
        name: String,
        first: u32,
        second: u32,
    },
    #[error("summary.{field} is {declared}, but the artifact contains {actual}")]
    SummaryMismatch {
        field: &'static str,
        declared: u32,
        actual: u32,
    },
    #[error("expression {expression} is an unsupported node: {detail}")]
    UnsupportedNode { expression: u32, detail: String },
}

/// Options controlling how strict validation is.
///
/// Non-exhaustive: construct with `..ValidateOptions::default()` so a future
/// option does not break callers.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ValidateOptions {
    /// Reject `RbcExprNode::Unsupported`. Import sets this: a model containing
    /// a node the schema could not represent cannot be faithfully rebuilt.
    pub reject_unsupported: bool,
}

/// Require complete scalar-port contracts, in addition to ordinary validation.
/// Legacy compiler-exported connection provenance is descriptive and may lack
/// member declarations/equation pairing. It cannot authorize new wiring.
pub fn validate_connection_contracts(
    model: &RbcModel,
    options: &ValidateOptions,
) -> Result<(), Vec<ValidationError>> {
    validate(model, options)?;
    if model.connectors.is_empty()
        && (!model.connection_sets.is_empty()
            || !model.connections.is_empty()
            || model.variables.iter().any(|v| v.connector.is_some()))
    {
        return Err(vec![ValidationError::Connector(
            "complete port declarations required; legacy connection annotations are not certified for wiring".into(),
        )]);
    }
    Ok(())
}

/// Resolve a host program's references against the model it was derived from.
///
/// This is analysis 5 of the execution validator. It lives here rather than in
/// `rumoca-ir-solve` because it is the only one that needs the equation IR,
/// and an evaluation crate reaching up to the public model would invert the
/// tier. It reports through `rumoca_ir_solve::execution::code`, so a caller
/// matches one table of codes regardless of which crate raised it.
pub fn validate_execution_references(
    model: &RbcModel,
    execution: &rumoca_ir_solve::execution::ExecutionArtifact,
) -> Result<(), Vec<ValidationError>> {
    use rumoca_ir_solve::execution::{Instruction, code};
    let known: std::collections::BTreeSet<u32> =
        model.trace_points.iter().map(|point| point.id.0).collect();
    let mut errors = Vec::new();
    let mut check = |id: rumoca_ir_solve::execution::TracePointRef, where_: &str| {
        if !known.contains(&id.0) {
            errors.push(ValidationError::ExecutionReference(format!(
                "{}: trace point {id} referenced by {where_} is not declared by the model",
                code::UNKNOWN_TRACE_POINT
            )));
        }
    };
    for (name, function) in &execution.program.functions {
        let mut pending: Vec<&Instruction> = function.body.iter().collect();
        while let Some(instruction) = pending.pop() {
            match instruction {
                Instruction::Value { trace_point, .. } => {
                    check(*trace_point, &format!("`{name}`"));
                }
                Instruction::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    pending.extend(then_body.iter());
                    pending.extend(else_body.iter());
                }
                _ => {}
            }
        }
    }
    for sink in &execution.program.sinks {
        for member in &sink.metadata.members {
            check(member.trace_point, &format!("sink `{}`", sink.key));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate an artifact's internal consistency. Returns every problem found,
/// not just the first, so a pass author fixes one round of errors at a time.
pub fn validate(model: &RbcModel, options: &ValidateOptions) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    if let Err(message) = crate::connector_validation::validate(model) {
        errors.push(ValidationError::Connector(message));
    }

    check_discrete_real(&mut errors, model, &Counts::of(model));
    check_additional_table_ids(&mut errors, model);
    check_dense(&mut errors, "sources", model.sources.iter().map(|s| s.id.0));
    check_dense(&mut errors, "types", model.types.iter().map(|t| t.id.0));
    check_dense(
        &mut errors,
        "variables",
        model.variables.iter().map(|v| v.id.0),
    );
    check_dense(
        &mut errors,
        "expressions",
        model.expressions.iter().map(|e| e.id.0),
    );
    check_dense(
        &mut errors,
        "equations",
        model.equations.iter().map(|e| e.id.0),
    );
    check_dense(
        &mut errors,
        "relations",
        model.relations.iter().map(|r| r.id.0),
    );
    check_dense(
        &mut errors,
        "conditions",
        model.conditions.iter().map(|c| c.id.0),
    );
    check_dense(&mut errors, "roots", model.roots.iter().map(|r| r.id.0));
    check_dense(
        &mut errors,
        "components",
        model.components.iter().map(|c| c.id.0),
    );
    check_dense(
        &mut errors,
        "connections",
        model.connections.iter().map(|c| c.id.0),
    );

    let counts = Counts::of(model);

    check_variable_names(&mut errors, model);
    check_types(&mut errors, model, &counts);
    check_expressions(&mut errors, model, &counts, options);
    check_equations(&mut errors, model, &counts);
    check_events(&mut errors, model, &counts);
    check_clocks(&mut errors, model, &counts);
    check_connections(&mut errors, model, &counts);
    check_trace_points(&mut errors, model, &counts);
    check_function_bodies(&mut errors, model, &counts);
    check_call_graph_acyclic(&mut errors, model);
    check_discrete_definitions(&mut errors, model, &counts);
    check_summary(&mut errors, model);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// These tables also have independent dense ID spaces. In particular an initial
// equation's ID is not an index into continuous equations during relocation.
fn check_additional_table_ids(errors: &mut Vec<ValidationError>, model: &RbcModel) {
    macro_rules! table {
        ($($name:ident),+ $(,)?) => {$(
            check_dense(errors, stringify!($name), model.$name.iter().map(|v| v.id.0));
        )+};
    }
    table!(
        initial_equations,
        domains,
        functions,
        equation_families,
        initial_equation_families,
        events,
        time_events,
        trace_points,
        clocks
    );
}

struct Counts {
    sources: u32,
    types: u32,
    variables: u32,
    expressions: u32,
    equations: u32,
    relations: u32,
    conditions: u32,
    components: u32,
    connections: u32,
    connection_sets: u32,
    domains: u32,
    functions: u32,
    clocks: u32,
    delays: u32,
    previous_values: u32,
    terminals: u32,
}

impl Counts {
    fn of(model: &RbcModel) -> Self {
        Self {
            sources: model.sources.len() as u32,
            types: model.types.len() as u32,
            variables: model.variables.len() as u32,
            expressions: model.expressions.len() as u32,
            equations: model.equations.len() as u32,
            relations: model.relations.len() as u32,
            conditions: model.conditions.len() as u32,
            components: model.components.len() as u32,
            connections: model.connections.len() as u32,
            connection_sets: model.connection_sets.len() as u32,
            domains: model.domains.len() as u32,
            functions: model.functions.len() as u32,
            clocks: model.clocks.len() as u32,
            delays: model.delays.len() as u32,
            previous_values: model.previous_values.len() as u32,
            terminals: model.terminals.len() as u32,
        }
    }
}

/// The B.1b partition and the initialization-instant discrete values.
fn check_discrete_real(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    check_dense(
        errors,
        "discrete_real_equations",
        model.discrete_real_equations.iter().map(|e| e.id.0),
    );
    for equation in &model.discrete_real_equations {
        let owner = format!("discrete real equation {}", equation.id.0);
        reference(
            errors,
            owner.clone(),
            "expression",
            equation.residual.0,
            counts.expressions,
        );
        if let RbcDiscreteRealActivation::When { trigger, guard } = equation.activation {
            reference(
                errors,
                owner.clone(),
                "condition",
                trigger.0,
                counts.conditions,
            );
            reference(
                errors,
                owner.clone(),
                "condition",
                guard.0,
                counts.conditions,
            );
        }
        for variable in equation
            .reads
            .iter()
            .chain(&equation.reads_derivative)
            .chain(&equation.reads_previous)
        {
            reference(
                errors,
                owner.clone(),
                "variable",
                variable.0,
                counts.variables,
            );
        }
    }
    for (index, entry) in model
        .initial_discrete_values
        .iter()
        .chain(&model.initial_parameter_values)
        .enumerate()
    {
        let owner = format!("initial discrete value {index}");
        reference(
            errors,
            owner.clone(),
            "variable",
            entry.target.0,
            counts.variables,
        );
        reference(
            errors,
            owner,
            "expression",
            entry.value.0,
            counts.expressions,
        );
    }
}

fn check_dense(
    errors: &mut Vec<ValidationError>,
    collection: &'static str,
    ids: impl Iterator<Item = u32>,
) {
    for (position, declared) in ids.enumerate() {
        let expected = position as u32;
        if declared != expected {
            errors.push(ValidationError::NonDenseId {
                collection,
                position: expected,
                declared,
                expected,
            });
        }
    }
}

fn reference(
    errors: &mut Vec<ValidationError>,
    referrer: impl Into<String>,
    target: &'static str,
    id: u32,
    count: u32,
) {
    if id >= count {
        errors.push(ValidationError::DanglingReference {
            referrer: referrer.into(),
            target,
            id,
            count,
        });
    }
}

fn check_variable_names(errors: &mut Vec<ValidationError>, model: &RbcModel) {
    let mut seen: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
    for variable in &model.variables {
        if let Some(first) = seen.insert(variable.name.as_str(), variable.id.0) {
            errors.push(ValidationError::DuplicateVariableName {
                name: variable.name.clone(),
                first,
                second: variable.id.0,
            });
        }
    }
}

fn check_types(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for variable in &model.variables {
        reference(
            errors,
            format!("variable {}", variable.id),
            "type",
            variable.value_type.0,
            counts.types,
        );
        reference(
            errors,
            format!("variable {} declaration", variable.id),
            "source",
            variable.declaration.span.source.0,
            counts.sources,
        );
        if let Some(component) = variable.component {
            reference(
                errors,
                format!("variable {}", variable.id),
                "component",
                component.0,
                counts.components,
            );
        }
        for (label, attribute) in [
            ("binding", variable.binding),
            ("start", variable.start),
            ("min", variable.min),
            ("max", variable.max),
            ("nominal", variable.nominal),
        ] {
            if let Some(expression) = attribute {
                reference(
                    errors,
                    format!("variable {} {label}", variable.id),
                    "expression",
                    expression.0,
                    counts.expressions,
                );
            }
        }
    }
}

fn check_expressions(
    errors: &mut Vec<ValidationError>,
    model: &RbcModel,
    counts: &Counts,
    options: &ValidateOptions,
) {
    for expression in &model.expressions {
        let index = expression.id.0;
        expression_reference(errors, index, "type", expression.value_type.0, counts.types);
        check_expression_node(errors, index, &expression.node, counts, options);
    }
}

/// Report a dangling reference from expression `index` into another table.
fn expression_reference(
    errors: &mut Vec<ValidationError>,
    index: u32,
    target: &'static str,
    id: u32,
    count: u32,
) {
    reference(errors, format!("expression {index}"), target, id, count);
}

/// Operands must be strictly earlier. This is what makes the arena a DAG
/// and lets a consumer evaluate it in one forward pass; it also makes a
/// cycle unrepresentable rather than something to detect.
fn check_operand(errors: &mut Vec<ValidationError>, index: u32, operand: ExprId) {
    if operand.0 >= index {
        errors.push(ValidationError::NonTopologicalOperand {
            expression: index,
            operand: operand.0,
        });
    }
}

fn check_operands(
    errors: &mut Vec<ValidationError>,
    index: u32,
    operands: impl IntoIterator<Item = ExprId>,
) {
    for operand in operands {
        check_operand(errors, index, operand);
    }
}

fn check_expression_node(
    errors: &mut Vec<ValidationError>,
    index: u32,
    node: &RbcExprNode,
    counts: &Counts,
    options: &ValidateOptions,
) {
    let mut operand = |operand: ExprId| check_operand(errors, index, operand);
    match node {
        RbcExprNode::StringConversion { value, format } => {
            operand(*value);
            check_operands(errors, index, format.operands());
        }
        RbcExprNode::Literal { .. } => {}
        RbcExprNode::ClockTransfer {
            source,
            source_clock,
            target_clock,
            ..
        } => {
            operand(*source);
            for clock in [source_clock, target_clock] {
                expression_reference(errors, index, "clock", clock.0, counts.clocks);
            }
        }
        RbcExprNode::Coordinate { coordinate } => {
            check_coordinate(errors, index, *coordinate, counts)
        }
        RbcExprNode::Unary { operand: inner, .. } => operand(*inner),
        RbcExprNode::Binary { lhs, rhs, .. } => {
            operand(*lhs);
            operand(*rhs);
        }
        RbcExprNode::Builtin { arguments, .. } => {
            check_operands(errors, index, arguments.iter().copied())
        }
        RbcExprNode::Conditional { branches, fallback } => {
            for branch in branches {
                operand(branch.condition);
                operand(branch.value);
            }
            operand(*fallback);
        }
        RbcExprNode::Array {
            elements,
            empty_type,
        } => {
            check_operands(errors, index, elements.iter().copied());
            if let Some(ty) = empty_type {
                expression_reference(errors, index, "type", ty.0, counts.types);
            }
        }
        RbcExprNode::Record { ty, fields } => {
            check_operands(errors, index, fields.iter().copied());
            expression_reference(errors, index, "type", ty.0, counts.types);
        }
        RbcExprNode::Field { base, .. } => operand(*base),
        RbcExprNode::Range { start, step, stop } => {
            operand(*start);
            if let Some(step) = step {
                operand(*step);
            }
            operand(*stop);
        }
        RbcExprNode::Comprehension { domain, body } => {
            operand(*body);
            expression_reference(errors, index, "domain", domain.0, counts.domains);
        }
        RbcExprNode::Index { base, subscripts } => {
            operand(*base);
            let subscripts = subscripts.iter().filter_map(|s| s.expression());
            check_operands(errors, index, subscripts);
        }
        RbcExprNode::ArrayUpdate {
            base,
            value,
            subscripts,
        } => {
            operand(*base);
            operand(*value);
            let subscripts = subscripts.iter().filter_map(|s| s.expression());
            check_operands(errors, index, subscripts);
        }
        RbcExprNode::Call {
            owner,
            function,
            arguments,
            ..
        } => check_call(errors, index, *owner, *function, arguments, counts),
        RbcExprNode::FunctionValue { function, .. }
        | RbcExprNode::FunctionFoldParameter { function, .. }
        | RbcExprNode::FunctionFoldOutput { function, .. } => {
            // Only the function id is artifact-wide. The other fields
            // are owner-local ordinals, checked against that function's
            // own body and fold tables by `check_function_bodies`,
            // which is where the tables to check against are in scope.
            expression_reference(errors, index, "function", function.0, counts.functions);
        }
        RbcExprNode::Unsupported { detail } => {
            if options.reject_unsupported {
                errors.push(ValidationError::UnsupportedNode {
                    expression: index,
                    detail: detail.clone(),
                });
            }
        }
    }
}

fn check_coordinate(
    errors: &mut Vec<ValidationError>,
    index: u32,
    coordinate: RbcCoordinate,
    counts: &Counts,
) {
    match coordinate {
        RbcCoordinate::Binder { domain, .. } => {
            expression_reference(errors, index, "domain", domain.0, counts.domains);
        }
        RbcCoordinate::Condition { condition } => {
            expression_reference(errors, index, "condition", condition.0, counts.conditions);
        }
        RbcCoordinate::FunctionParameter { function, .. } => {
            expression_reference(errors, index, "function", function.0, counts.functions);
        }
        RbcCoordinate::ClockInterval { clock } => {
            expression_reference(errors, index, "clock", clock.0, counts.clocks);
        }
        RbcCoordinate::Delay { delay } => {
            expression_reference(errors, index, "delay", delay.0, counts.delays);
        }
        RbcCoordinate::Previous { previous } => {
            let count = counts.previous_values;
            expression_reference(errors, index, "previous value", previous.0, count);
        }
        RbcCoordinate::Terminal { terminal } => {
            expression_reference(errors, index, "terminal", terminal.0, counts.terminals);
        }
        _ => {}
    }
    if let Some(variable) = coordinate.variable() {
        expression_reference(errors, index, "variable", variable.0, counts.variables);
    }
}

fn check_call(
    errors: &mut Vec<ValidationError>,
    index: u32,
    owner: ExprId,
    function: FunctionId,
    arguments: &[ExprId],
    counts: &Counts,
) {
    // `owner` may be this node itself — a single-output call is
    // its own owner — so it is checked as a backward-or-self
    // reference rather than with the strict operand rule.
    let forward_owner = owner.0 > index;
    check_operands(errors, index, arguments.iter().copied());
    if forward_owner {
        errors.push(ValidationError::NonTopologicalOperand {
            expression: index,
            operand: owner.0,
        });
    }
    expression_reference(errors, index, "function", function.0, counts.functions);
}

fn check_equations(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for (label, equations) in [
        ("equation", &model.equations),
        ("initial equation", &model.initial_equations),
    ] {
        for equation in equations {
            reference(
                errors,
                format!("{label} {}", equation.id),
                "expression",
                equation.residual.0,
                counts.expressions,
            );
            for variable in equation.reads.iter().chain(&equation.reads_derivative) {
                reference(
                    errors,
                    format!("{label} {} reads", equation.id),
                    "variable",
                    variable.0,
                    counts.variables,
                );
            }
        }
    }
}

fn check_events(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    check_relations(errors, model, counts);
    check_conditions(errors, model, counts);
    check_roots(errors, model, counts);
    check_event_actions(errors, model, counts);
    check_time_events(errors, model, counts);
}

fn check_relations(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for relation in &model.relations {
        reference(
            errors,
            format!("relation {}", relation.id),
            "expression",
            relation.expression.0,
            counts.expressions,
        );
    }
}

fn check_conditions(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for condition in &model.conditions {
        let index = condition.id.0;
        let mut inner = |id: ConditionId| {
            // A condition may only reference strictly earlier conditions, for
            // the same reason expressions may only reference earlier nodes.
            if id.0 >= index {
                errors.push(ValidationError::NonTopologicalOperand {
                    expression: index,
                    operand: id.0,
                });
            }
        };
        match &condition.node {
            RbcConditionNode::Initial
            | RbcConditionNode::Always
            | RbcConditionNode::Unsupported { .. } => {}
            RbcConditionNode::ClockActivation { clock } => reference(
                errors,
                format!("condition {index}"),
                "clock",
                clock.0,
                model.clocks.len() as u32,
            ),
            RbcConditionNode::Relation { relation } => reference(
                errors,
                format!("condition {index}"),
                "relation",
                relation.0,
                counts.relations,
            ),
            RbcConditionNode::Discrete { expression } => reference(
                errors,
                format!("condition {index}"),
                "expression",
                expression.0,
                counts.expressions,
            ),
            RbcConditionNode::Not { operand } => inner(*operand),
            RbcConditionNode::And { lhs, rhs }
            | RbcConditionNode::Or { lhs, rhs }
            | RbcConditionNode::AnyRise { lhs, rhs } => {
                inner(*lhs);
                inner(*rhs);
            }
        }
    }
}

fn check_clocks(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for clock in &model.clocks {
        if let RbcClockNode::Shifted { base, counter, .. } = clock.node {
            reference(
                errors,
                format!("clock {} base", clock.id),
                "clock",
                base.0,
                counts.clocks,
            );
            if base.0 >= clock.id.0
                || counter == 0
                || !model
                    .clocks
                    .get(base.0 as usize)
                    .is_some_and(|clock| matches!(clock.node, RbcClockNode::Triggered { .. }))
            {
                errors.push(ValidationError::Clock(
                    "shifted clock requires an earlier triggered base and positive counter".into(),
                ));
            }
        }
        if let RbcClockNode::Triggered { condition } | RbcClockNode::Shifted { condition, .. } =
            clock.node
        {
            reference(
                errors,
                format!("clock {}", clock.id),
                "condition",
                condition.0,
                counts.conditions,
            );
        }
    }
    let mut owned = BTreeSet::new();
    for (index, owner) in model.clock_ownerships.iter().enumerate() {
        if !owned.insert(owner.variable.0) {
            errors.push(ValidationError::Clock(format!(
                "variable {} has multiple clock ownership records",
                owner.variable
            )));
        }
        reference(
            errors,
            format!("clock ownership {index}"),
            "clock",
            owner.clock.0,
            model.clocks.len() as u32,
        );
        reference(
            errors,
            format!("clock ownership {index}"),
            "variable",
            owner.variable.0,
            counts.variables,
        );
    }
}

fn check_roots(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for root in &model.roots {
        reference(
            errors,
            format!("root {}", root.id),
            "relation",
            root.relation.0,
            counts.relations,
        );
        reference(
            errors,
            format!("root {}", root.id),
            "condition",
            root.activation.0,
            counts.conditions,
        );
    }
}

fn check_event_actions(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for event in &model.events {
        reference(
            errors,
            format!("event {}", event.id),
            "condition",
            event.trigger.0,
            counts.conditions,
        );
        reference(
            errors,
            format!("event {}", event.id),
            "condition",
            event.guard.0,
            counts.conditions,
        );
        match &event.action {
            RbcAction::Reinitialize { state, value } => {
                reference(
                    errors,
                    format!("event {}", event.id),
                    "variable",
                    state.0,
                    counts.variables,
                );
                reference(
                    errors,
                    format!("event {}", event.id),
                    "expression",
                    value.0,
                    counts.expressions,
                );
            }
            RbcAction::Warning { message, condition } => {
                for expression in [message, condition] {
                    reference(
                        errors,
                        format!("event {} warning", event.id),
                        "expression",
                        expression.0,
                        counts.expressions,
                    );
                }
            }
            RbcAction::Assert { message, level } => {
                reference(
                    errors,
                    format!("event {}", event.id),
                    "expression",
                    message.0,
                    counts.expressions,
                );
                if let Some(level) = level {
                    reference(
                        errors,
                        format!("event {} level", event.id),
                        "expression",
                        level.0,
                        counts.expressions,
                    );
                }
            }
            RbcAction::Terminate { message } => reference(
                errors,
                format!("event {}", event.id),
                "expression",
                message.0,
                counts.expressions,
            ),
        }
    }
}

fn check_time_events(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for event in &model.time_events {
        if let RbcSchedule::Dynamic { deadline } = event.schedule {
            reference(
                errors,
                format!("time event {}", event.id),
                "expression",
                deadline.0,
                counts.expressions,
            );
        }
    }
}

fn check_connections(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for connection in &model.connections {
        for (label, variable) in [("left", connection.left), ("right", connection.right)] {
            reference(
                errors,
                format!("connection {} {label}", connection.id),
                "variable",
                variable.0,
                counts.variables,
            );
        }
        if let Some(equation) = connection.equation {
            reference(
                errors,
                format!("connection {}", connection.id),
                "equation",
                equation.0,
                counts.equations,
            );
        }
    }
    check_dense(
        errors,
        "connection_sets",
        model.connection_sets.iter().map(|set| set.id.0),
    );
    for set in &model.connection_sets {
        let owner = format!("connection set {}", set.id);
        for potential in &set.potentials {
            reference(
                errors,
                owner.clone(),
                "variable",
                potential.0,
                counts.variables,
            );
        }
        for balance in &set.balances {
            for term in &balance.terms {
                reference(
                    errors,
                    owner.clone(),
                    "variable",
                    term.variable.0,
                    counts.variables,
                );
            }
            if let Some(equation) = balance.equation {
                reference(
                    errors,
                    owner.clone(),
                    "equation",
                    equation.0,
                    counts.equations,
                );
            }
        }
        for equation in &set.potential_equations {
            reference(
                errors,
                owner.clone(),
                "equation",
                equation.0,
                counts.equations,
            );
        }
    }
}

fn check_trace_points(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    let mut seen = BTreeSet::new();
    for trace in &model.trace_points {
        if !seen.insert(trace.id.0) {
            errors.push(ValidationError::NonDenseId {
                collection: "trace_points",
                position: trace.id.0,
                declared: trace.id.0,
                expected: trace.id.0,
            });
        }
        reference(
            errors,
            format!("trace point {}", trace.id),
            "variable",
            trace.variable.0,
            counts.variables,
        );
        if let Some(connection) = trace.connection {
            reference(
                errors,
                format!("trace point {}", trace.id),
                "connection",
                connection.0,
                counts.connections,
            );
        }
        if let Some(set) = trace.connection_set {
            reference(
                errors,
                format!("trace point {}", trace.id),
                "connection_set",
                set.0,
                counts.connection_sets,
            );
        }
    }
}

/// The summary is denormalised, so a truncated or hand-edited artifact whose
/// counts disagree with its contents is rejected rather than silently trusted.
/// Every discrete-valued variable must be defined, and every branch must give
/// one value per target.
///
/// The first invariant is what bitcode v1 was missing: an artifact could
/// declare a `discrete_value` variable, carry no definition for it, and pass
/// every other check. Reconstruction then failed with "missing B.1c topology
/// definition", which is the right refusal in the wrong place — the producer
/// should not have been able to write the artifact.
fn check_discrete_definitions(
    errors: &mut Vec<ValidationError>,
    model: &RbcModel,
    counts: &Counts,
) {
    let mut defined = BTreeSet::new();
    for (index, definition) in model.discrete_definitions.iter().enumerate() {
        let index = index as u32;
        for target in &definition.targets {
            reference(
                errors,
                format!("discrete definition {index} target"),
                "variable",
                target.0,
                counts.variables,
            );
            defined.insert(target.0);
        }
        for (branch_index, branch) in definition.branches.iter().enumerate() {
            let targets = definition.targets.len();
            check_discrete_branch(errors, index, branch_index, branch, targets, counts);
        }
    }
    check_temporal_owners(errors, model, counts);
    // A transaction owns and defines its targets as surely as a B.1c owner.
    defined.extend(check_model_event_transactions(errors, model, counts));
    for variable in &model.variables {
        if variable.role == RbcRole::DiscreteValue && !defined.contains(&variable.id.0) {
            errors.push(ValidationError::UndefinedDiscreteValue {
                id: variable.id.0,
                name: variable.name.clone(),
            });
        }
    }
}

fn check_discrete_branch(
    errors: &mut Vec<ValidationError>,
    index: u32,
    branch_index: usize,
    branch: &RbcDiscreteBranch,
    targets: usize,
    counts: &Counts,
) {
    if branch.values.len() != targets {
        errors.push(ValidationError::DiscreteBranchArity {
            definition: index,
            branch: branch_index as u32,
            values: branch.values.len() as u32,
            targets: targets as u32,
        });
    }
    for value in &branch.values {
        reference(
            errors,
            format!("discrete definition {index} branch {branch_index}"),
            "expression",
            value.0,
            counts.expressions,
        );
    }
    if let RbcDiscreteActivation::When { trigger, guard } = branch.activation {
        for (label, condition) in [("trigger", trigger), ("guard", guard)] {
            reference(
                errors,
                format!("discrete definition {index} branch {branch_index} {label}"),
                "condition",
                condition.0,
                counts.conditions,
            );
        }
    }
}

/// References inside the temporal owner tables and structured roots.
fn check_temporal_owners(errors: &mut Vec<ValidationError>, model: &RbcModel, counts: &Counts) {
    for (index, previous) in model.previous_values.iter().enumerate() {
        let referrer = format!("previous value {index}");
        reference(
            errors,
            referrer.clone(),
            "variable",
            previous.variable.0,
            counts.variables,
        );
        reference(errors, referrer, "clock", previous.clock.0, counts.clocks);
    }
    for (index, root) in model.structured_roots.iter().enumerate() {
        let referrer = format!("structured root {index}");
        reference(
            errors,
            referrer.clone(),
            "domain",
            root.domain.0,
            counts.domains,
        );
        reference(
            errors,
            referrer,
            "expression",
            root.expression.0,
            counts.expressions,
        );
    }
    for (index, delay) in model.delays.iter().enumerate() {
        let referrer = format!("delay {index}");
        let expressions = match delay.delay {
            RbcDelayKind::Parameter { delay_time } => vec![delay.source, delay_time.expression],
            RbcDelayKind::Bounded {
                delay_time,
                maximum,
            } => vec![delay.source, delay_time, maximum.expression],
        };
        for expression in expressions {
            reference(
                errors,
                referrer.clone(),
                "expression",
                expression.0,
                counts.expressions,
            );
        }
    }
    if model.terminals.len() > 1 {
        errors.push(ValidationError::Connector(format!(
            "{} terminal() owners; a model has at most one",
            model.terminals.len()
        )));
    }
}

/// References inside model event transactions; returns the targets owned.
fn check_model_event_transactions(
    errors: &mut Vec<ValidationError>,
    model: &RbcModel,
    counts: &Counts,
) -> Vec<u32> {
    let mut owned = Vec::new();
    for (index, transaction) in model.model_event_transactions.iter().enumerate() {
        let referrer = format!("model event transaction {index}");
        for target in &transaction.targets {
            reference(
                errors,
                referrer.clone(),
                "variable",
                target.0,
                counts.variables,
            );
            owned.push(target.0);
        }
        for step in &transaction.steps {
            check_transaction_step(errors, &referrer, &transaction.targets, step, counts);
        }
    }
    owned
}

fn check_transaction_step(
    errors: &mut Vec<ValidationError>,
    referrer: &str,
    targets: &[VariableId],
    step: &RbcModelEventStep,
    counts: &Counts,
) {
    for condition in [step.trigger, step.guard] {
        reference(
            errors,
            referrer,
            "condition",
            condition.0,
            counts.conditions,
        );
    }
    if let Some(clock) = step.clock {
        reference(errors, referrer, "clock", clock.0, counts.clocks);
    }
    for definition in &step.definitions {
        reference(
            errors,
            referrer,
            "variable",
            definition.target.0,
            counts.variables,
        );
        reference(
            errors,
            referrer,
            "expression",
            definition.value.0,
            counts.expressions,
        );
        if !targets.contains(&definition.target) {
            errors.push(ValidationError::Connector(format!(
                "{referrer}: a step defines variable {}, which the transaction does not own",
                definition.target.0
            )));
        }
    }
}

fn check_summary(errors: &mut Vec<ValidationError>, model: &RbcModel) {
    let mut check = |field: &'static str, declared: u32, actual: usize| {
        if declared != actual as u32 {
            errors.push(ValidationError::SummaryMismatch {
                field,
                declared,
                actual: actual as u32,
            });
        }
    };
    let summary = &model.summary;
    check("variables", summary.variables, model.variables.len());
    check("equations", summary.equations, model.equations.len());
    check(
        "initial_equations",
        summary.initial_equations,
        model.initial_equations.len(),
    );
    check("expressions", summary.expressions, model.expressions.len());
    check("relations", summary.relations, model.relations.len());
    check("conditions", summary.conditions, model.conditions.len());
    check("clocks", summary.clocks, model.clocks.len());
    check(
        "clock_ownerships",
        summary.clock_ownerships,
        model.clock_ownerships.len(),
    );
    check("roots", summary.roots, model.roots.len());
    check("events", summary.events, model.events.len());
    check("time_events", summary.time_events, model.time_events.len());
    check("connections", summary.connections, model.connections.len());
    check(
        "connection_sets",
        summary.connection_sets,
        model.connection_sets.len(),
    );
    check("components", summary.components, model.components.len());
    check(
        "trace_points",
        summary.trace_points,
        model.trace_points.len(),
    );
}

/// Recompute a summary. A transformation pass calls this after editing so the
/// artifact it writes passes [`validate`].
pub fn recompute_summary(model: &mut RbcModel) {
    let count = |role: RbcRole| {
        model
            .variables
            .iter()
            .filter(|variable| variable.role == role)
            .count() as u32
    };
    let family_rows: u32 = model.equation_families.iter().map(|f| f.scalar_rows).sum();
    model.summary = RbcSummary {
        domains: model.domains.len() as u32,
        equation_families: model.equation_families.len() as u32,
        family_scalar_rows: family_rows,
        variables: model.variables.len() as u32,
        states: count(RbcRole::State),
        parameters: count(RbcRole::Parameter),
        constants: count(RbcRole::Constant),
        inputs: count(RbcRole::Input),
        outputs: count(RbcRole::Output),
        algebraics: count(RbcRole::Algebraic),
        discrete_reals: count(RbcRole::DiscreteReal),
        discrete_values: count(RbcRole::DiscreteValue),
        equations: model.equations.len() as u32,
        initial_equations: model.initial_equations.len() as u32,
        expressions: model.expressions.len() as u32,
        relations: model.relations.len() as u32,
        conditions: model.conditions.len() as u32,
        clocks: model.clocks.len() as u32,
        clock_ownerships: model.clock_ownerships.len() as u32,
        roots: model.roots.len() as u32,
        events: model.events.len() as u32,
        time_events: model.time_events.len() as u32,
        connections: model.connections.len() as u32,
        connection_sets: model.connection_sets.len() as u32,
        components: model.components.len() as u32,
        trace_points: model.trace_points.len() as u32,
        discrete_definitions: model.discrete_definitions.len() as u32,
        discrete_real_equations: model.discrete_real_equations.len() as u32,
        initial_discrete_values: model.initial_discrete_values.len() as u32,
    };
}

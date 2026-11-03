//! Validation of carried function bodies and the call graph above them.
//!
//! Added with the bodies themselves. Every other table's references are
//! checked by the parent module, and the one table that once was not --
//! domains -- is how the compiler came to emit artifacts that failed its own
//! validator with dangling ids (fixed in `8f86b8fe`). A new reference-bearing
//! table ships with its check or it repeats that.

use super::*;

/// No cycle in the call graph, once any function body is carried.
///
/// Until bodies were carried, termination rested on their absence: a call
/// was a leaf because the callee's body was not in the artifact, so
/// recursion could not be *executed* from it (SPEC_RUMOCA_BITCODE §9a).
/// Carrying a body removes that argument and this check replaces it. It is
/// the same guard Execution IR v2 uses for the same reason (`EX2-030`).
///
/// Only edges between carried bodies count. A call into an elided body is a
/// leaf by the original argument -- there is nothing there to execute -- so
/// a cycle that passes through one is a fact about the source, not a hazard
/// in the artifact. The exporter elides recursive bodies for this reason.
pub(super) fn check_call_graph_acyclic(errors: &mut Vec<ValidationError>, model: &RbcModel) {
    if !model
        .functions
        .iter()
        .any(|function| matches!(function.body, RbcFunctionBody::Modelica { .. }))
    {
        return;
    }
    let mut marks = vec![Mark::Unvisited; model.functions.len()];
    for root in 0..model.functions.len() {
        if marks[root] == Mark::Unvisited && carries_body(model, root) {
            walk_calls(errors, model, &mut marks, root);
        }
    }
}

fn carries_body(model: &RbcModel, function: usize) -> bool {
    model
        .functions
        .get(function)
        .is_some_and(|function| matches!(function.body, RbcFunctionBody::Modelica { .. }))
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Unvisited,
    InProgress,
    Done,
}

/// Iterative depth-first search with an explicit stack: a recursive walk
/// over a graph whose acyclicity is the thing in question is a way to
/// overflow the stack instead of reporting the cycle.
fn walk_calls(
    errors: &mut Vec<ValidationError>,
    model: &RbcModel,
    marks: &mut [Mark],
    root: usize,
) {
    let mut stack = vec![(root, 0usize)];
    marks[root] = Mark::InProgress;
    while let Some((current, next)) = stack.pop() {
        let Some(callee) = model
            .functions
            .get(current)
            .and_then(|function| function.calls.get(next))
        else {
            marks[current] = Mark::Done;
            continue;
        };
        stack.push((current, next + 1));
        let callee = callee.0 as usize;
        if !carries_body(model, callee) {
            continue;
        }
        match marks.get(callee).copied() {
            Some(Mark::InProgress) => {
                errors.push(ValidationError::Connector(format!(
                    "function '{}' is reachable from itself; an artifact that carries \
                     function bodies must have an acyclic call graph",
                    model.functions[callee].name
                )));
                marks[callee] = Mark::Done;
            }
            Some(Mark::Unvisited) => {
                marks[callee] = Mark::InProgress;
                stack.push((callee, 0));
            }
            _ => {}
        }
    }
}

/// Every reference a function's value table, external ABI, body and folds
/// make: types, expressions, domains, and owner-local value and fold
/// ordinals.
pub(super) fn check_function_bodies(
    errors: &mut Vec<ValidationError>,
    model: &RbcModel,
    counts: &Counts,
) {
    // Spans are references too: one naming a source the artifact does not
    // carry cannot be placed, and import rejects it only once it has begun
    // rebuilding.
    let sources = model.sources.len();
    if let Some(span) = function_spans(model)
        .into_iter()
        .find(|span| span.source.0 as usize >= sources)
    {
        errors.push(ValidationError::Connector(format!(
            "a function span names source {}, but the artifact carries {sources}",
            span.source.0
        )));
    }
    for function in &model.functions {
        let checker = Checker {
            function,
            counts,
            referrer: format!("function {}", function.id.0),
        };
        for derivative in &function.derivatives {
            reference(
                errors,
                checker.referrer.clone(),
                "function",
                derivative.target.0,
                counts.functions,
            );
            if derivative.inputs.len() != function.parameters.len() {
                checker.fail(
                    errors,
                    format_args!("derivative input classification does not match its function"),
                );
            }
            if let Some((previous, ordinal)) = derivative.previous
                && model
                    .functions
                    .get(previous.0 as usize)
                    .and_then(|owner| owner.derivatives.get(ordinal as usize))
                    .is_none()
            {
                checker.fail(
                    errors,
                    format_args!("derivative predecessor is not defined"),
                );
            }
        }
        for value in &function.values {
            checker.type_ref(errors, value.value_type);
        }
        match &function.body {
            RbcFunctionBody::External {
                arguments, result, ..
            } => checker.external(errors, arguments, *result),
            RbcFunctionBody::Modelica { statements } => {
                checker.statements(errors, statements);
                checker.folds(errors);
            }
            RbcFunctionBody::ElidedModelica => {}
        }
    }
}

/// One function's references, checked against its own tables.
struct Checker<'a> {
    function: &'a RbcFunction,
    counts: &'a Counts,
    referrer: String,
}

impl Checker<'_> {
    fn fail(&self, errors: &mut Vec<ValidationError>, message: std::fmt::Arguments<'_>) {
        errors.push(ValidationError::Connector(format!(
            "{}: {message}",
            self.referrer
        )));
    }

    fn type_ref(&self, errors: &mut Vec<ValidationError>, ty: TypeId) {
        reference(
            errors,
            self.referrer.clone(),
            "type",
            ty.0,
            self.counts.types,
        );
    }

    fn expression(&self, errors: &mut Vec<ValidationError>, id: ExprId) {
        reference(
            errors,
            self.referrer.clone(),
            "expression",
            id.0,
            self.counts.expressions,
        );
    }

    /// A body addresses values only through the value table, so an ordinal
    /// past its end names nothing a rebuild could define.
    fn value(&self, errors: &mut Vec<ValidationError>, ordinal: u32) {
        let count = self.function.values.len();
        if ordinal as usize >= count {
            self.fail(
                errors,
                format_args!("names value {ordinal}, but the function declares {count} values"),
            );
        }
    }

    fn external(
        &self,
        errors: &mut Vec<ValidationError>,
        arguments: &[RbcExternalArgument],
        result: Option<u32>,
    ) {
        for argument in arguments {
            match argument {
                RbcExternalArgument::Input { expression } => self.expression(errors, *expression),
                RbcExternalArgument::Output { value } => self.value(errors, *value),
            }
        }
        if let Some(result) = result {
            self.value(errors, result);
        }
    }

    fn folds(&self, errors: &mut Vec<ValidationError>) {
        let count = self.function.folds.len();
        for fold in &self.function.folds {
            reference(
                errors,
                self.referrer.clone(),
                "domain",
                fold.domain.0,
                self.counts.domains,
            );
            if fold.parent.is_some_and(|parent| parent as usize >= count) {
                self.fail(
                    errors,
                    format_args!(
                        "fold {} names a parent past its {count} folds",
                        fold.ordinal
                    ),
                );
            }
            for value in fold.targets.iter().chain(&fold.iteration_locals) {
                self.value(errors, *value);
            }
            let groups = [&fold.parameters, &fold.initial, &fold.update, &fold.output];
            for definition in groups.into_iter().flatten() {
                self.value(errors, definition.value);
                self.expression(errors, definition.expression);
            }
        }
    }

    /// One statement list, recursing into loop bodies.
    ///
    /// Recursive because a loop body is a statement list like any other, and
    /// a reference nested two loops deep is exactly as able to dangle as one
    /// at the top. Depth is bounded by the artifact's own nesting.
    fn statements(&self, errors: &mut Vec<ValidationError>, statements: &[RbcFunctionStatement]) {
        for statement in statements {
            match statement {
                RbcFunctionStatement::Assignment {
                    value, expression, ..
                } => {
                    self.value(errors, *value);
                    self.expression(errors, *expression);
                }
                RbcFunctionStatement::Assertion {
                    condition, message, ..
                } => {
                    self.expression(errors, *condition);
                    self.expression(errors, *message);
                }
                RbcFunctionStatement::AssignmentGroup {
                    values,
                    conditional,
                    expressions,
                    ..
                } => self.group(errors, values, expressions, conditional.as_ref()),
                RbcFunctionStatement::For {
                    fold, statements, ..
                } => self.loop_body(errors, *fold, statements),
            }
        }
    }

    fn loop_body(
        &self,
        errors: &mut Vec<ValidationError>,
        fold: u32,
        statements: &[RbcFunctionStatement],
    ) {
        if fold as usize >= self.function.folds.len() {
            self.fail(errors, format_args!("loop names undeclared fold {fold}"));
        }
        self.statements(errors, statements);
    }

    /// Arity is the property that makes a group a group: one expression per
    /// value, and each branch supplying exactly one per value. A short
    /// branch would silently leave a value undefined on the path selecting
    /// it.
    fn group(
        &self,
        errors: &mut Vec<ValidationError>,
        values: &[u32],
        expressions: &[ExprId],
        conditional: Option<&RbcFunctionConditional>,
    ) {
        for value in values {
            self.value(errors, *value);
        }
        if expressions.len() != values.len() {
            self.fail(
                errors,
                format_args!(
                    "{} expressions for {} grouped values",
                    expressions.len(),
                    values.len()
                ),
            );
        }
        for id in expressions {
            self.expression(errors, *id);
        }
        let Some(correlation) = conditional else {
            return;
        };
        if correlation.conditions.len() != correlation.branches.len() {
            self.fail(
                errors,
                format_args!(
                    "{} branch conditions for {} branches",
                    correlation.conditions.len(),
                    correlation.branches.len()
                ),
            );
        }
        let arms = correlation
            .branches
            .iter()
            .chain(std::iter::once(&correlation.fallback));
        for (ordinal, arm) in arms.enumerate() {
            if arm.len() != values.len() {
                self.fail(
                    errors,
                    format_args!(
                        "arm {ordinal} defines {} of {} grouped values",
                        arm.len(),
                        values.len()
                    ),
                );
            }
        }
        let referenced = correlation
            .conditions
            .iter()
            .chain(correlation.branches.iter().flatten())
            .chain(&correlation.fallback);
        for id in referenced {
            self.expression(errors, *id);
        }
    }
}

/// Every span a function declaration and its carried body reference.
pub(crate) fn function_spans(model: &RbcModel) -> Vec<RbcSpan> {
    let mut found = Vec::new();
    for function in &model.functions {
        found.push(function.declaration.span);
        found.extend(
            function
                .derivatives
                .iter()
                .map(|derivative| derivative.provenance.span),
        );
        found.extend(
            function
                .parameters
                .iter()
                .filter_map(|parameter| parameter.declaration.map(|p| p.span)),
        );
        found.extend(function.values.iter().map(|value| value.declaration.span));
        found.extend(function.folds.iter().map(|fold| fold.provenance.span));
        if let RbcFunctionBody::Modelica { statements } = &function.body {
            statement_spans(statements, &mut found);
        }
    }
    found
}

fn statement_spans(statements: &[RbcFunctionStatement], found: &mut Vec<RbcSpan>) {
    for statement in statements {
        match statement {
            RbcFunctionStatement::Assignment { provenance, .. }
            | RbcFunctionStatement::Assertion { provenance, .. }
            | RbcFunctionStatement::AssignmentGroup { provenance, .. } => {
                found.push(provenance.span);
            }
            RbcFunctionStatement::For {
                statements,
                provenance,
                ..
            } => {
                found.push(provenance.span);
                statement_spans(statements, found);
            }
        }
    }
}

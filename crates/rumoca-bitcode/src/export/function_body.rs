//! Projecting a lowered Modelica function body into the artifact.
//!
//! Separate from `export.rs` because it grows with each statement form the
//! schema learns to carry, and because the all-or-nothing rule below is a
//! decision worth stating once in a place that holds only it.
use super::Ctx;
use crate::schema::*;
use rumoca_ir_dae as dae;

/// Definitions of one transition group, projected in order.
fn definitions_of(values: dae::FunctionDefinitionValues<'_>) -> Vec<RbcFunctionDefinition> {
    values
        .iter()
        .map(|definition| RbcFunctionDefinition {
            value: definition.target().ordinal(),
            expression: ExprId(definition.rhs().index()),
        })
        .collect()
}

/// Every fold a body contains, in owner-local ordinal order.
///
/// Returns `None` when any fold's own statements contain a form this
/// version cannot hold, which elides the whole body for the same reason a
/// partial statement list would be worse than none.
pub(super) fn export_folds<'dae>(
    view: dae::DaeView<'dae>,
    function: dae::FunctionView<'dae>,
    ctx: &mut Ctx<'_>,
) -> Option<Vec<RbcFunctionFold>> {
    let mut folds = Vec::new();
    for index in 0..function.fold_count() {
        let id = function.fold_id(index)?;
        let fold = view.function_fold(id)?;
        folds.push(RbcFunctionFold {
            ordinal: id.ordinal(),
            domain: DomainId(fold.domain().index()),
            parent: fold.parent_ordinal(),
            targets: fold.targets().map(|value| value.ordinal()).collect(),
            iteration_locals: fold
                .iteration_locals()
                .map(|value| value.ordinal())
                .collect(),
            parameters: definitions_of(fold.parameter_values()),
            initial: definitions_of(fold.initial_values()),
            update: definitions_of(fold.update_values()),
            output: definitions_of(fold.output_values()),
            provenance: ctx.provenance(fold.provenance()),
        });
    }
    Some(folds)
}

/// Project one statement list, or `None` if it holds a form not carried.
fn statements_of<'dae>(
    source: dae::FunctionStatements<'dae>,
    ctx: &mut Ctx<'_>,
) -> Option<Vec<RbcFunctionStatement>> {
    let mut statements = Vec::new();
    for statement in source {
        match statement {
            dae::FunctionStatementView::Assignment { definition } => {
                statements.push(RbcFunctionStatement::Assignment {
                    value: definition.target().ordinal(),
                    expression: ExprId(definition.rhs().index()),
                    provenance: ctx.provenance(definition.provenance()),
                });
            }
            dae::FunctionStatementView::Assertion {
                condition,
                message,
                level,
                provenance,
            } => {
                statements.push(RbcFunctionStatement::Assertion {
                    condition: ExprId(condition.index()),
                    message: ExprId(message.index()),
                    level: match level {
                        dae::AssertionLevel::Error => RbcAssertionLevel::Error,
                        dae::AssertionLevel::Warning => RbcAssertionLevel::Warning,
                    },
                    provenance: ctx.provenance(provenance),
                });
            }
            dae::FunctionStatementView::AssignmentGroup {
                definitions,
                conditional,
            } => {
                let values = definitions
                    .iter()
                    .map(|definition| definition.target().ordinal())
                    .collect();
                let expressions = definitions
                    .iter()
                    .map(|definition| ExprId(definition.rhs().index()))
                    .collect();
                // The DAE gives every definition of a group one provenance,
                // so the first stands for all of them.
                let first = definitions.iter().next()?;
                let provenance = ctx.provenance(first.provenance());
                let conditional = conditional.map(|correlation| RbcFunctionConditional {
                    conditions: correlation
                        .conditions()
                        .map(|id| ExprId(id.index()))
                        .collect(),
                    branches: (0..correlation.branch_count())
                        .filter_map(|ordinal| {
                            correlation
                                .branch(ordinal)
                                .map(|values| values.map(|id| ExprId(id.index())).collect())
                        })
                        .collect(),
                    fallback: correlation
                        .fallback()
                        .map(|id| ExprId(id.index()))
                        .collect(),
                });
                statements.push(RbcFunctionStatement::AssignmentGroup {
                    values,
                    conditional,
                    expressions,
                    provenance,
                });
            }
            dae::FunctionStatementView::For {
                fold,
                statements: body,
                provenance,
            } => {
                statements.push(RbcFunctionStatement::For {
                    fold: fold.ordinal(),
                    statements: statements_of(body, ctx)?,
                    provenance: ctx.provenance(provenance),
                });
            }
        }
    }
    Some(statements)
}

/// A function's Modelica body and its folds, or `ElidedModelica` when any
/// statement is a form this schema version cannot hold.
///
/// All-or-nothing on purpose. Emitting the statements that fit and dropping
/// the rest would produce a body that reads as complete and computes
/// something else; a consumer has no way to detect the gap.
/// `ElidedModelica` already means "a body exists and is not here", which is
/// exactly true.
pub(super) fn export_function_body<'dae>(
    view: dae::DaeView<'dae>,
    function: dae::FunctionView<'dae>,
    ctx: &mut Ctx<'_>,
) -> (RbcFunctionBody, Vec<RbcFunctionFold>) {
    let Some(statements) = statements_of(function.statements(), ctx) else {
        return (RbcFunctionBody::ElidedModelica, Vec::new());
    };
    let Some(folds) = export_folds(view, function, ctx) else {
        return (RbcFunctionBody::ElidedModelica, Vec::new());
    };
    (RbcFunctionBody::Modelica { statements }, folds)
}

/// Project a reference to a value inside a function body.
///
/// Split from the main expression projection to keep `export.rs` inside the
/// SPEC_0021 file-size limit, and because these three belong with the body
/// projection they refer into rather than with equation expressions.
pub(super) fn function_node(
    operation: &dae::ExpressionOperation<'_>,
) -> Result<RbcExprNode, super::ExportError> {
    Ok(match operation {
        dae::ExpressionOperation::FunctionValue {
            value, definition, ..
        } => RbcExprNode::FunctionValue {
            function: FunctionId(value.function().index()),
            value: value.ordinal(),
            definition: definition.id().ordinal(),
        },
        dae::ExpressionOperation::FunctionFoldParameter {
            fold,
            carried,
            definition,
            ..
        } => RbcExprNode::FunctionFoldParameter {
            function: FunctionId(fold.function().index()),
            fold: fold.ordinal(),
            carried: *carried,
            definition: definition.id().ordinal(),
        },
        dae::ExpressionOperation::FunctionFoldOutput {
            fold,
            carried,
            definition,
            ..
        } => RbcExprNode::FunctionFoldOutput {
            function: FunctionId(fold.function().index()),
            fold: fold.ordinal(),
            carried: *carried,
            definition: definition.id().ordinal(),
        },
        _ => {
            return Err(super::ExportError::Projection(
                "function_node called with a non-function-body operation".into(),
            ));
        }
    })
}

/// The function's output and local values, in owner-local ordinal order.
///
/// The index into this list *is* the ordinal every body reference uses, so
/// it is emitted for every function that has values, external or Modelica.
pub(super) fn value_table(
    function: dae::FunctionView<'_>,
    ctx: &mut Ctx<'_>,
) -> Vec<RbcFunctionValue> {
    function
        .values()
        .map(|value| RbcFunctionValue {
            name: value.name().to_string(),
            value_type: TypeId(value.value_type().index()),
            role: match value.role() {
                dae::FunctionValueRole::Output => RbcFunctionValueRole::Output,
                dae::FunctionValueRole::Local => RbcFunctionValueRole::Local,
            },
            declaration: ctx.provenance(value.declaration()),
        })
        .collect()
}

/// An external body, with the ABI a rebuild needs.
///
/// Language and symbol alone say what to call and not with what. The
/// argument positions, the return binding, purity and linkage are the rest of
/// the call, and an artifact that drops them can be printed but not run.
pub(super) fn external_body(external: dae::ExternalFunctionView<'_>) -> RbcFunctionBody {
    let linkage = external.linkage();
    RbcFunctionBody::External {
        // Encoding kept as it was before this field set grew, so artifacts
        // written earlier still name their language the same way.
        language: format!("{:?}", external.language()).to_lowercase(),
        symbol: external.symbol().to_string(),
        purity: if external.purity().is_pure() {
            RbcPurity::Pure
        } else {
            RbcPurity::Impure
        },
        arguments: external
            .arguments()
            .map(|argument| match argument {
                dae::ExternalArgumentView::Input(expression) => RbcExternalArgument::Input {
                    expression: ExprId(expression.index()),
                },
                dae::ExternalArgumentView::Output(value) => RbcExternalArgument::Output {
                    value: value.ordinal(),
                },
            })
            .collect(),
        result: external.result().map(|value| value.ordinal()),
        linkage: RbcExternalLinkage {
            libraries: linkage.libraries().to_vec(),
            include: linkage.include().map(str::to_string),
            include_directory: linkage.include_directory().map(str::to_string),
            library_directory: linkage.library_directory().map(str::to_string),
        },
    }
}

/// Which functions sit on a call cycle, by function index.
///
/// The DAE admits a proven-recursive group of functions; the artifact does
/// not carry one. SPEC_RUMOCA_BITCODE §9a rests termination on the call
/// graph over carried bodies being acyclic, so a recursive function's body is
/// elided, exactly as a body the schema cannot hold is: the declaration and
/// its `calls` edges stay, and nothing claims a body that is not there.
pub(super) fn recursive_functions(edges: &[Vec<FunctionId>]) -> Vec<bool> {
    let dependencies: Vec<Vec<usize>> = edges
        .iter()
        .map(|callees| {
            callees
                .iter()
                .map(|callee| callee.0 as usize)
                .filter(|callee| *callee < edges.len())
                .collect()
        })
        .collect();
    let mut recursive = vec![false; edges.len()];
    if let Ok(components) = rumoca_core::dependency_first_sccs(&dependencies) {
        for component in components.iter().filter(|component| component.recursive) {
            for &member in component.members.iter() {
                recursive[member] = true;
            }
        }
    }
    recursive
}

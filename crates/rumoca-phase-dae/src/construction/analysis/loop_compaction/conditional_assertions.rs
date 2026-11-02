//! Hoist function assertions out of runtime conditionals.
//!
//! A function body lowers to a pure result DAG, and a conditional lowers to a
//! selection between the values its branches assign. An assertion has no
//! value to select, so one inside a branch has no owner there. It does have
//! one at the enclosing statement level, where it is checked once per
//! execution: `if c then assert(a, m); end if` is `assert(not c or a, m)`,
//! because the assertion fails exactly when the branch is taken and `a` does
//! not hold (MLS §8.3.7, §11.5).
//!
//! The rewrite splits each branch around its assertions and guards every
//! piece by the condition that selects that branch, so a branch's statements
//! still run in their written order and only in the execution that selects
//! it. That is exact only while the selecting conditions read nothing the
//! branches assign; a conditional that does is left as written, and analysis
//! still refuses it. A conditional with a `break` or `return` is left alone
//! for the same reason: splitting it would move statements across the jump.

use super::*;

pub(super) fn hoist_conditional_assertions(
    statements: &[rumoca_core::Statement],
) -> Vec<rumoca_core::Statement> {
    let mut hoisted = Vec::with_capacity(statements.len());
    for statement in statements {
        match statement {
            rumoca_core::Statement::For {
                indices,
                equations,
                span,
            } => hoisted.push(rumoca_core::Statement::For {
                indices: indices.clone(),
                equations: hoist_conditional_assertions(equations),
                span: *span,
            }),
            rumoca_core::Statement::If {
                cond_blocks,
                else_block,
                span,
            } => hoisted.extend(hoist_conditional(cond_blocks, else_block.as_deref(), *span)),
            _ => hoisted.push(statement.clone()),
        }
    }
    hoisted
}

fn hoist_conditional(
    cond_blocks: &[rumoca_core::StatementBlock],
    else_block: Option<&[rumoca_core::Statement]>,
    span: Span,
) -> Vec<rumoca_core::Statement> {
    let cond_blocks = cond_blocks
        .iter()
        .map(|block| rumoca_core::StatementBlock {
            cond: block.cond.clone(),
            stmts: hoist_conditional_assertions(&block.stmts),
        })
        .collect::<Vec<_>>();
    let else_block = else_block.map(hoist_conditional_assertions);
    let unchanged = || {
        vec![rumoca_core::Statement::If {
            cond_blocks: cond_blocks.clone(),
            else_block: else_block.clone(),
            span,
        }]
    };
    let branches = cond_blocks
        .iter()
        .map(|block| block.stmts.as_slice())
        .chain(else_block.as_deref())
        .collect::<Vec<_>>();
    if !branches
        .iter()
        .any(|branch| branch.iter().any(holds_assertion))
        || branches.iter().any(|branch| branch.iter().any(jumps))
    {
        return unchanged();
    }
    let assigned = branches
        .iter()
        .flat_map(|branch| assigned_function_targets(branch))
        .map(|name| VarName::new(&name))
        .collect::<Vec<_>>();
    if cond_blocks.iter().any(|block| {
        assigned
            .iter()
            .any(|name| expression_reads_name(&block.cond, name))
    }) {
        return unchanged();
    }

    // Branch k runs when its condition holds and none before it does.
    let mut earlier_fail = Vec::new();
    let mut guards = Vec::with_capacity(branches.len());
    for block in &cond_blocks {
        guards.push(conjunction(
            earlier_fail.iter().cloned().chain([block.cond.clone()]),
            span,
        ));
        earlier_fail.push(negation(block.cond.clone(), span));
    }
    if else_block.is_some() {
        guards.push(conjunction(earlier_fail, span));
    }

    let mut hoisted = Vec::new();
    for (branch, guard) in branches.into_iter().zip(guards) {
        let mut run = Vec::new();
        for statement in branch {
            let Some(guarded) = guarded_statement(statement, &guard, span) else {
                run.push(statement.clone());
                continue;
            };
            flush_guarded_run(&mut hoisted, &mut run, &guard, span);
            hoisted.push(guarded);
        }
        flush_guarded_run(&mut hoisted, &mut run, &guard, span);
    }
    hoisted
}

/// `statement` checked only when `guard` holds, if it carries an assertion
/// that can leave the branch.
fn guarded_statement(
    statement: &rumoca_core::Statement,
    guard: &Expression,
    span: Span,
) -> Option<rumoca_core::Statement> {
    match statement {
        rumoca_core::Statement::Assert {
            condition,
            message,
            level,
            span,
        } => Some(rumoca_core::Statement::Assert {
            condition: implication(guard.clone(), condition.clone(), *span),
            message: message.clone(),
            level: level.clone(),
            span: *span,
        }),
        rumoca_core::Statement::FunctionCall {
            comp,
            args,
            outputs,
            span,
        } if is_assert_call(statement) => {
            let mut args = args.clone();
            args[0] = implication(guard.clone(), args[0].clone(), *span);
            Some(rumoca_core::Statement::FunctionCall {
                comp: comp.clone(),
                args,
                outputs: outputs.clone(),
                span: *span,
            })
        }
        // The guard is fixed for the whole loop: it reads no binder of it
        // and nothing the branches (so the loop) assign.
        rumoca_core::Statement::For {
            indices,
            equations,
            span: loop_span,
        } if equations.iter().any(holds_assertion)
            && !indices
                .iter()
                .any(|index| expression_reads_name(guard, &VarName::new(&index.ident))) =>
        {
            Some(rumoca_core::Statement::For {
                indices: indices.clone(),
                equations: hoist_conditional(
                    &[rumoca_core::StatementBlock {
                        cond: guard.clone(),
                        stmts: equations.clone(),
                    }],
                    None,
                    span,
                ),
                span: *loop_span,
            })
        }
        _ => None,
    }
}

fn flush_guarded_run(
    hoisted: &mut Vec<rumoca_core::Statement>,
    run: &mut Vec<rumoca_core::Statement>,
    guard: &Expression,
    span: Span,
) {
    if run.is_empty() {
        return;
    }
    hoisted.push(rumoca_core::Statement::If {
        cond_blocks: vec![rumoca_core::StatementBlock {
            cond: guard.clone(),
            stmts: std::mem::take(run),
        }],
        else_block: None,
        span,
    });
}

fn holds_assertion(statement: &rumoca_core::Statement) -> bool {
    match statement {
        rumoca_core::Statement::Assert { .. } => true,
        rumoca_core::Statement::FunctionCall { .. } => is_assert_call(statement),
        rumoca_core::Statement::For { equations, .. } => equations.iter().any(holds_assertion),
        rumoca_core::Statement::If {
            cond_blocks,
            else_block,
            ..
        } => cond_blocks
            .iter()
            .flat_map(|block| &block.stmts)
            .chain(else_block.iter().flatten())
            .any(holds_assertion),
        _ => false,
    }
}

fn jumps(statement: &rumoca_core::Statement) -> bool {
    match statement {
        rumoca_core::Statement::Break { .. } | rumoca_core::Statement::Return { .. } => true,
        rumoca_core::Statement::For { equations, .. } => equations.iter().any(jumps),
        rumoca_core::Statement::While { block, .. } => block.stmts.iter().any(jumps),
        rumoca_core::Statement::If {
            cond_blocks,
            else_block,
            ..
        } => cond_blocks
            .iter()
            .flat_map(|block| &block.stmts)
            .chain(else_block.iter().flatten())
            .any(jumps),
        _ => false,
    }
}

/// The predefined `assert` spelled as a call statement with no results.
fn is_assert_call(statement: &rumoca_core::Statement) -> bool {
    let rumoca_core::Statement::FunctionCall {
        comp,
        args,
        outputs,
        ..
    } = statement
    else {
        return false;
    };
    comp.resolved_function().is_none()
        && outputs.iter().all(Option::is_none)
        && (2..=3).contains(&args.len())
        && rumoca_core::runtime_flow_action_function_short_name(comp.as_str()) == Some("assert")
}

fn negation(operand: Expression, span: Span) -> Expression {
    Expression::Unary {
        op: rumoca_core::OpUnary::Not,
        rhs: Box::new(operand),
        span,
    }
}

fn conjunction(operands: impl IntoIterator<Item = Expression>, span: Span) -> Expression {
    operands
        .into_iter()
        .reduce(|lhs, rhs| Expression::Binary {
            op: rumoca_core::OpBinary::And,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span,
        })
        .unwrap_or(Expression::Literal {
            value: Literal::Boolean(true),
            span,
        })
}

/// `guard implies condition`.
fn implication(guard: Expression, condition: Expression, span: Span) -> Expression {
    Expression::Binary {
        op: rumoca_core::OpBinary::Or,
        lhs: Box::new(negation(guard, span)),
        rhs: Box::new(condition),
        span,
    }
}

//! One walk over every expression a Flat model owns, for postprocess
//! rewrites that replace expressions in place (record-constructor marking,
//! elementary external builtins).

use crate::flat;
use rumoca_core::{ExpressionRewriter, StatementRewriter};

/// Rewrite every expression of `flat`: variable attributes, equations and
/// their structured templates, assertions, algorithms, `when` chains, and
/// function parameter expressions and bodies.
pub(crate) fn rewrite_model_expressions<R>(flat: &mut flat::Model, rewriter: R)
where
    R: ExpressionRewriter + StatementRewriter + Copy,
{
    let walk = ModelWalk(rewriter);
    for var in flat.variables.values_mut() {
        walk.opt_expr(&mut var.binding);
        walk.opt_expr(&mut var.start);
        walk.opt_expr(&mut var.min);
        walk.opt_expr(&mut var.max);
        walk.opt_expr(&mut var.nominal);
    }
    for eq in flat
        .equations
        .iter_mut()
        .chain(flat.initial_equations.iter_mut())
    {
        walk.expr(&mut eq.residual);
    }
    // Structured templates are authoritative peer copies of their scalar
    // residuals, so every rewrite must cover them too.
    for family in flat
        .structured_equations
        .iter_mut()
        .chain(flat.initial_structured_equations.iter_mut())
    {
        if let Some(template) = &mut family.template {
            for expression in &mut template.body {
                walk.expr(expression);
            }
        }
    }
    for assert_eq in flat
        .assert_equations
        .iter_mut()
        .chain(flat.initial_assert_equations.iter_mut())
    {
        walk.expr(&mut assert_eq.condition);
        walk.expr(&mut assert_eq.message);
        walk.opt_expr(&mut assert_eq.level);
    }
    for algorithm in flat
        .algorithms
        .iter_mut()
        .chain(flat.initial_algorithms.iter_mut())
    {
        walk.statements(&mut algorithm.statements);
    }
    for chain in &mut flat.when_chains {
        for branch in chain.branches_mut() {
            walk.expr(&mut branch.condition);
            walk.when_equations(&mut branch.equations);
        }
    }
    for function in flat.functions.values_mut() {
        for param in function
            .inputs
            .iter_mut()
            .chain(function.outputs.iter_mut())
            .chain(function.locals.iter_mut())
        {
            walk.function_param(param);
        }
        walk.statements(&mut function.body);
    }
}

#[derive(Clone, Copy)]
struct ModelWalk<R>(R);

impl<R: ExpressionRewriter + StatementRewriter + Copy> ModelWalk<R> {
    fn opt_expr(self, expr: &mut Option<rumoca_core::Expression>) {
        if let Some(expr) = expr {
            self.expr(expr);
        }
    }

    fn expr(self, expr: &mut rumoca_core::Expression) {
        let mut rewriter = self.0;
        *expr = rewriter.rewrite_expression(expr);
    }

    fn statements(self, statements: &mut [rumoca_core::Statement]) {
        let mut rewriter = self.0;
        for statement in statements {
            *statement = rewriter.rewrite_statement(statement);
        }
    }

    fn function_param(self, param: &mut rumoca_core::FunctionParam) {
        for expression in [&mut param.default, &mut param.min, &mut param.max]
            .into_iter()
            .flatten()
        {
            self.expr(expression);
        }
        for subscript in &mut param.shape_expr {
            if let rumoca_core::Subscript::Expr { expr, .. } = subscript {
                self.expr(expr);
            }
        }
    }

    fn when_equations(self, equations: &mut [flat::WhenEquation]) {
        for equation in equations {
            self.when_equation(equation);
        }
    }

    fn when_equation(self, equation: &mut flat::WhenEquation) {
        match equation {
            flat::WhenEquation::Assign { value, .. } | flat::WhenEquation::Reinit { value, .. } => {
                self.expr(value);
            }
            flat::WhenEquation::Assert {
                condition,
                message,
                level,
                ..
            } => {
                self.expr(condition);
                self.expr(message);
                self.boxed_expr(level);
            }
            flat::WhenEquation::Conditional {
                branches,
                else_branch,
                ..
            } => self.conditional(branches, else_branch),
            flat::WhenEquation::FunctionCallOutputs { function, .. } => self.expr(function),
            flat::WhenEquation::Terminate { message, .. } => self.expr(message),
        }
    }

    fn boxed_expr(self, expr: &mut Option<Box<rumoca_core::Expression>>) {
        if let Some(expr) = expr {
            self.expr(expr);
        }
    }

    fn conditional(
        self,
        branches: &mut [(rumoca_core::Expression, Vec<flat::WhenEquation>)],
        else_branch: &mut Option<Vec<flat::WhenEquation>>,
    ) {
        for (condition, branch_equations) in branches {
            self.expr(condition);
            self.when_equations(branch_equations);
        }
        if let Some(else_branch) = else_branch {
            self.when_equations(else_branch);
        }
    }
}

//! Check a function's loop assertions once, after the loop.
//!
//! `for i loop offset := ...; assert(offset[1] >= 0, msg); end for` asserts
//! over a value the iteration itself computed. A function body lowers to a
//! pure result DAG in which a loop is a fold, and an assertion inside it is
//! checked by a separate map over the loop domain -- which can read the
//! inputs and binders but not values the fold computes per iteration.
//!
//! The fold can carry one more value, though: `ok := ok and (offset[1] >= 0)`
//! accumulates the condition over every iteration, and `assert(ok, msg)`
//! after the loops fails exactly when some iteration violated it. A function
//! has no side effects, so checking at the end instead of mid-loop changes
//! only how far the (discarded) computation ran before the failure is
//! reported (MLS §12.2, §8.3.7).
//!
//! Only assertions whose condition reads a value the body assigns are moved;
//! the rest keep their per-iteration map. A body with `return` or `break` is
//! left alone, since the deferred check could then be skipped, and so is an
//! assertion whose message or level reads a loop index or an assigned value,
//! whose meaning would change after the loop.

use super::*;

const FLAG_PREFIX: &str = "__rumoca_assertion_holds_";

pub(crate) fn accumulate_loop_assertions(flat: &mut flat::Model) {
    let boolean = flat.predefined_types.boolean;
    let Ok(effective) = rumoca_core::EffectiveType::new(boolean, boolean, Vec::<i64>::new()) else {
        return;
    };
    for function in flat.functions.values_mut() {
        accumulate_in_function(function, &effective);
    }
}

fn accumulate_in_function(
    function: &mut rumoca_core::Function,
    effective: &rumoca_core::EffectiveType,
) {
    if function.body.iter().any(jumps) {
        return;
    }
    let Some(def_id) = function.def_id else {
        return;
    };
    let mut written = HashSet::new();
    collect_written(&function.body, &mut written);
    if written.is_empty() {
        return;
    }
    let mut rewriter = Accumulator {
        written: &written,
        binders: Vec::new(),
        def_id,
        flags: Vec::new(),
        taken: function
            .inputs
            .iter()
            .chain(&function.outputs)
            .chain(&function.locals)
            .map(|value| value.name.clone())
            .collect(),
    };
    let body = rewriter.statements(&function.body, false);
    if rewriter.flags.is_empty() {
        return;
    }
    let mut checks = Vec::with_capacity(rewriter.flags.len());
    for flag in rewriter.flags {
        let mut local = rumoca_core::FunctionParam::new(
            flag.name.clone(),
            "Boolean",
            effective.clone(),
            flag.span,
        );
        local.default = Some(rumoca_core::Expression::Literal {
            value: rumoca_core::Literal::Boolean(true),
            span: flag.span,
        });
        function.add_local(local);
        checks.push(rumoca_core::Statement::Assert {
            condition: flag_read(&flag.name, flag.span),
            message: Box::new(flag.message),
            level: flag.level.map(Box::new),
            span: flag.span,
        });
    }
    function.body = body;
    function.body.extend(checks);
}

struct Flag {
    name: String,
    message: rumoca_core::Expression,
    level: Option<rumoca_core::Expression>,
    span: rumoca_core::Span,
}

struct Accumulator<'a> {
    written: &'a HashSet<String>,
    binders: Vec<String>,
    def_id: rumoca_core::DefId,
    flags: Vec<Flag>,
    taken: HashSet<String>,
}

impl Accumulator<'_> {
    fn statements(
        &mut self,
        statements: &[rumoca_core::Statement],
        in_loop: bool,
    ) -> Vec<rumoca_core::Statement> {
        statements
            .iter()
            .map(|statement| self.statement(statement, in_loop))
            .collect()
    }

    fn statement(
        &mut self,
        statement: &rumoca_core::Statement,
        in_loop: bool,
    ) -> rumoca_core::Statement {
        match statement {
            rumoca_core::Statement::For {
                indices,
                equations,
                span,
            } => {
                let depth = self.binders.len();
                self.binders
                    .extend(indices.iter().map(|index| index.ident.clone()));
                let equations = self.statements(equations, true);
                self.binders.truncate(depth);
                rumoca_core::Statement::For {
                    indices: indices.clone(),
                    equations,
                    span: *span,
                }
            }
            rumoca_core::Statement::Assert {
                condition,
                message,
                level,
                span,
            } if in_loop
                && self.reads_written(condition)
                && !self.reads_scope(message)
                && level
                    .as_deref()
                    .is_none_or(|level| !self.reads_scope(level)) =>
            {
                let name = self.fresh_name();
                let flag = flag_read(&name, *span);
                self.flags.push(Flag {
                    name: name.clone(),
                    message: message.as_ref().clone(),
                    level: level.as_deref().cloned(),
                    span: *span,
                });
                rumoca_core::Statement::Assignment {
                    comp: flag_target(&name, self.def_id, *span),
                    value: rumoca_core::Expression::Binary {
                        op: rumoca_core::OpBinary::And,
                        lhs: Box::new(flag),
                        rhs: Box::new(condition.clone()),
                        span: *span,
                    },
                    span: *span,
                }
            }
            other => other.clone(),
        }
    }

    fn fresh_name(&mut self) -> String {
        let mut ordinal = self.flags.len();
        loop {
            let name = format!("{FLAG_PREFIX}{ordinal}");
            if self.taken.insert(name.clone()) {
                return name;
            }
            ordinal += 1;
        }
    }

    fn reads_written(&self, expression: &rumoca_core::Expression) -> bool {
        reads_any(expression, |name| self.written.contains(name))
    }

    /// Whether `expression` reads a loop index or a value the body assigns.
    fn reads_scope(&self, expression: &rumoca_core::Expression) -> bool {
        reads_any(expression, |name| {
            self.written.contains(name) || self.binders.iter().any(|binder| binder == name)
        })
    }
}

fn reads_any(expression: &rumoca_core::Expression, test: impl Fn(&str) -> bool) -> bool {
    let mut references = Vec::new();
    expression.collect_var_refs(&mut references);
    references.iter().any(|name| {
        let root = name.as_str().split(['.', '[']).next().unwrap_or_default();
        test(root)
    })
}

fn flag_read(name: &str, span: rumoca_core::Span) -> rumoca_core::Expression {
    rumoca_core::Expression::VarRef {
        name: rumoca_core::Reference::generated(name),
        subscripts: Vec::new(),
        span,
    }
}

fn flag_target(
    name: &str,
    def_id: rumoca_core::DefId,
    span: rumoca_core::Span,
) -> rumoca_core::ComponentReference {
    rumoca_core::ComponentReference::construct(
        false,
        span,
        vec![rumoca_core::ComponentRefPart {
            ident: name.to_string(),
            span,
            subs: Vec::new(),
            def_id,
        }],
    )
    .expect("a one-part reference is nonempty")
}

fn collect_written(statements: &[rumoca_core::Statement], written: &mut HashSet<String>) {
    for statement in statements {
        match statement {
            rumoca_core::Statement::Assignment { comp, .. } => {
                if let Some(root) = comp.parts().first() {
                    written.insert(root.ident.clone());
                }
            }
            rumoca_core::Statement::FunctionCall { outputs, .. } => {
                written.extend(
                    outputs
                        .iter()
                        .flatten()
                        .filter_map(|output| output.parts().first())
                        .map(|root| root.ident.clone()),
                );
            }
            rumoca_core::Statement::For { equations, .. } => collect_written(equations, written),
            rumoca_core::Statement::While { block, .. } => collect_written(&block.stmts, written),
            rumoca_core::Statement::If {
                cond_blocks,
                else_block,
                ..
            } => {
                for block in cond_blocks {
                    collect_written(&block.stmts, written);
                }
                if let Some(block) = else_block {
                    collect_written(block, written);
                }
            }
            _ => {}
        }
    }
}

fn jumps(statement: &rumoca_core::Statement) -> bool {
    match statement {
        rumoca_core::Statement::Return { .. } | rumoca_core::Statement::Break { .. } => true,
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

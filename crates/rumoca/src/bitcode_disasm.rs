//! Render a bitcode artifact as a readable listing.
//!
//! `bitcode dump` prints the serialization, which is the right thing for a
//! machine and close to useless for a person: an equation reads `"residual":
//! 83`, and answering "what equation is that" means walking an index graph by
//! hand through a quarter-megabyte of JSON. Expressions are stored flat and
//! address their operands by index, exactly like a constant pool.
//!
//! So this resolves the graph and prints what the model says:
//!
//! ```text
//! equations
//!   [ 0]  0 = (L.v - (L.p.v - L.n.v))              ChuaCircuit.mo:7
//!   [ 1]  0 = (der(L.i) - (L.v / L.L))             ChuaCircuit.mo:8
//! ```
//!
//! Deliberately not valid Modelica. It is a *listing* of the canonical DAE —
//! residual form, coordinates spelled as `der(x)`/`pre(x)`, expression ids
//! available on request — and the point is to show what the artifact contains,
//! including the parts a `.mo` rendering would smooth over.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;
use rumoca_bitcode::schema::{
    ExprId, FunctionId, RbcAction, RbcBinaryOp, RbcBranch, RbcClockTransferKind, RbcCoordinate,
    RbcExprNode, RbcLiteral, RbcModel, RbcStringConversionFormat, RbcUnaryOp, VariableId,
};

/// What to include beyond the default variable and equation listing.
#[derive(Debug, Default, Clone, Copy)]
pub struct DisasmOptions {
    /// Print the flat expression table, the artifact's constant pool.
    pub expressions: bool,
    /// Annotate each line with the source span it came from.
    pub provenance: bool,
    /// Print expression ids alongside the rendered form.
    pub ids: bool,
}

struct Listing<'a> {
    model: &'a RbcModel,
    names: HashMap<u32, &'a str>,
    sources: HashMap<u32, &'a str>,
}

impl<'a> Listing<'a> {
    fn new(model: &'a RbcModel) -> Self {
        Self {
            names: model
                .variables
                .iter()
                .map(|variable| (variable.id.0, variable.name.as_str()))
                .collect(),
            sources: model
                .sources
                .iter()
                .map(|source| (source.id.0, source.name.as_str()))
                .collect(),
            model,
        }
    }

    fn variable(&self, id: VariableId) -> &str {
        self.names.get(&id.0).copied().unwrap_or("<unknown>")
    }

    /// Render an expression by resolving its operands, parenthesised
    /// throughout: a listing a human reads should never depend on the reader
    /// remembering this language's precedence table.
    fn expression(&self, id: ExprId) -> String {
        let Some(expression) = self.model.expressions.get(id.0 as usize) else {
            return format!("<expr {} missing>", id.0);
        };
        match &expression.node {
            RbcExprNode::Literal { value } => literal(value),
            RbcExprNode::Coordinate { coordinate } => self.coordinate(*coordinate),
            RbcExprNode::Unary { op, operand } => {
                let inner = self.expression(*operand);
                match op {
                    RbcUnaryOp::Negate => format!("-{inner}"),
                    RbcUnaryOp::Plus => format!("+{inner}"),
                    RbcUnaryOp::Not => format!("not {inner}"),
                }
            }
            RbcExprNode::Binary { op, lhs, rhs } => format!(
                "({} {} {})",
                self.expression(*lhs),
                binary(*op),
                self.expression(*rhs)
            ),
            RbcExprNode::Conditional { branches, fallback } => {
                self.conditional(branches, *fallback)
            }
            RbcExprNode::Builtin { name, arguments } => {
                format!("{name}({})", self.expression_list(arguments))
            }
            RbcExprNode::StringConversion { value, format } => {
                self.string_conversion(*value, format)
            }
            RbcExprNode::Array { elements, .. } => {
                format!("{{{}}}", self.expression_list(elements))
            }
            RbcExprNode::Record { fields, .. } => {
                format!("record({})", self.expression_list(fields))
            }
            RbcExprNode::Field { base, field } => {
                format!("{}.[{field}]", self.expression(*base))
            }
            RbcExprNode::Range { start, step, stop } => self.range(*start, *step, *stop),
            RbcExprNode::Comprehension { domain, body } => format!(
                "{{{} for {}}}",
                self.expression(*body),
                self.domain_binders(*domain).join(", ")
            ),
            RbcExprNode::Index { base, subscripts } => {
                format!(
                    "{}[{}]",
                    self.expression(*base),
                    self.subscripts(subscripts)
                )
            }
            RbcExprNode::ArrayUpdate {
                base,
                value,
                subscripts,
            } => format!(
                "({} with [{}] := {})",
                self.expression(*base),
                self.subscripts(subscripts),
                self.expression(*value)
            ),
            RbcExprNode::Call {
                function,
                output,
                arguments,
                ..
            } => self.call(*function, *output, arguments),
            // Printed rather than skipped: a gap in the artifact is exactly the
            // thing a reader is looking for, and silently rendering around it
            // would make a partial export look complete.
            // A value inside a function body, named by the owner-local
            // ordinals the DAE uses. Printed as the function and ordinal
            // rather than resolved, because resolving would mean rendering
            // the body here and the listing shows it under its function.
            RbcExprNode::FunctionValue {
                function, value, ..
            } => format!("fnvalue ${}#{}", function.0, value),
            RbcExprNode::FunctionFoldParameter {
                function,
                fold,
                carried,
                ..
            } => format!("foldparam ${}#{}[{}]", function.0, fold, carried),
            RbcExprNode::FunctionFoldOutput {
                function,
                fold,
                carried,
                ..
            } => format!("foldout ${}#{}[{}]", function.0, fold, carried),
            RbcExprNode::ClockTransfer {
                transfer: kind,
                source,
                source_clock,
                target_clock,
            } => format!(
                "{}[clock {} -> {}]({})",
                clock_transfer_name(kind),
                source_clock.0,
                target_clock.0,
                self.expression(*source)
            ),
            RbcExprNode::Unsupported { detail } => format!("<unsupported: {detail}>"),
        }
    }

    /// Operands rendered and comma-separated, as they appear in an argument
    /// or element list.
    fn expression_list(&self, ids: &[ExprId]) -> String {
        let rendered: Vec<String> = ids.iter().map(|id| self.expression(*id)).collect();
        rendered.join(", ")
    }

    fn range(&self, start: ExprId, step: Option<ExprId>, stop: ExprId) -> String {
        match step {
            Some(step) => format!(
                "{}:{}:{}",
                self.expression(start),
                self.expression(step),
                self.expression(stop)
            ),
            None => format!("{}:{}", self.expression(start), self.expression(stop)),
        }
    }

    fn conditional(&self, branches: &[RbcBranch], fallback: ExprId) -> String {
        let mut out = String::new();
        for (index, branch) in branches.iter().enumerate() {
            let keyword = if index == 0 { "if" } else { "elseif" };
            let _ = write!(
                out,
                "{keyword} {} then {} ",
                self.expression(branch.condition),
                self.expression(branch.value)
            );
        }
        let _ = write!(out, "else {}", self.expression(fallback));
        format!("({})", out.trim())
    }

    fn string_conversion(&self, value: ExprId, format: &RbcStringConversionFormat) -> String {
        let mut arguments = vec![self.expression(value)];
        let options = match format {
            RbcStringConversionFormat::Options {
                minimum_length,
                left_justified,
                significant_digits,
            } => vec![
                ("minimumLength", *minimum_length),
                ("leftJustified", *left_justified),
                ("significantDigits", *significant_digits),
            ],
            RbcStringConversionFormat::Format { value } => vec![("format", Some(*value))],
        };
        arguments.extend(
            options.into_iter().filter_map(|(name, value)| {
                value.map(|id| format!("{name}={}", self.expression(id)))
            }),
        );
        format!("String({})", arguments.join(", "))
    }

    fn call(&self, function: FunctionId, output: u32, arguments: &[ExprId]) -> String {
        let callee = self.model.functions.get(function.0 as usize);
        let name = callee.map(|f| f.name.as_str()).unwrap_or("<function>");
        // The output ordinal is shown only when the callee has more
        // than one, so the common case reads like the source.
        let results = callee.map_or(1, |f| f.results.len());
        let projection = if results > 1 {
            format!(".[{output}]")
        } else {
            String::new()
        };
        format!("{name}({}){projection}", self.expression_list(arguments))
    }

    fn subscripts(&self, subscripts: &[rumoca_bitcode::schema::RbcSubscript]) -> String {
        use rumoca_bitcode::schema::RbcSubscript;
        subscripts
            .iter()
            .map(|subscript| match subscript {
                RbcSubscript::Index { expression } | RbcSubscript::Slice { expression } => {
                    self.expression(*expression)
                }
                RbcSubscript::Whole => ":".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The `i in 1:3` clauses of a domain, so a comprehension reads the way it
    /// was written rather than as an opaque domain number.
    fn domain_binders(&self, domain: rumoca_bitcode::schema::DomainId) -> Vec<String> {
        let Some(domain) = self.model.domains.get(domain.0 as usize) else {
            return vec![format!("<domain {} missing>", domain.0)];
        };
        domain
            .binders
            .iter()
            .map(|binder| {
                format!(
                    "{} in {}:{}",
                    binder.display_name, binder.lower, binder.upper
                )
            })
            .collect()
    }

    fn binder_name(&self, domain: rumoca_bitcode::schema::DomainId, ordinal: u32) -> String {
        self.model
            .domains
            .get(domain.0 as usize)
            .and_then(|domain| domain.binders.get(ordinal as usize))
            .map(|binder| binder.display_name.clone())
            .unwrap_or_else(|| format!("<binder {}.{ordinal}>", domain.0))
    }

    fn coordinate(&self, coordinate: RbcCoordinate) -> String {
        match coordinate {
            RbcCoordinate::Time => "time".to_string(),
            RbcCoordinate::Binder { domain, ordinal } => self.binder_name(domain, ordinal),
            RbcCoordinate::Condition { condition } => format!("<condition {}>", condition.0),
            RbcCoordinate::FunctionParameter { function, ordinal } => self
                .model
                .functions
                .get(function.0 as usize)
                .and_then(|f| f.parameters.get(ordinal as usize))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("<parameter {}.{ordinal}>", function.0)),
            RbcCoordinate::Derivative { variable } => format!("der({})", self.variable(variable)),
            RbcCoordinate::PreState { variable }
            | RbcCoordinate::PreAlgebraic { variable }
            | RbcCoordinate::PreDiscreteReal { variable }
            | RbcCoordinate::PreDiscreteValue { variable } => {
                format!("pre({})", self.variable(variable))
            }
            other => other
                .variable()
                .map(|variable| self.variable(variable).to_string())
                .unwrap_or_else(|| "<coordinate>".to_string()),
        }
    }

    fn span(&self, provenance: &rumoca_bitcode::schema::RbcProvenance) -> String {
        let span = &provenance.span;
        let file = self
            .sources
            .get(&span.source.0)
            .map_or_else(|| "?".to_string(), |name| file_base_name(name));
        format!("{file}:{}", span.line)
    }

    /// The ` ; file:line` suffix for a listed row, or nothing when
    /// provenance was not requested.
    fn provenance_suffix(
        &self,
        provenance: &rumoca_bitcode::schema::RbcProvenance,
        options: DisasmOptions,
    ) -> String {
        if !options.provenance {
            return String::new();
        }
        let span = self.span(provenance);
        if span.is_empty() {
            return String::new();
        }
        format!("   ; {span}")
    }
}

/// The final path component of a source name, falling back to the whole name.
fn file_base_name(name: &str) -> String {
    Path::new(name)
        .file_name()
        .map(|base| base.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

fn clock_transfer_name(kind: &RbcClockTransferKind) -> String {
    match kind {
        RbcClockTransferKind::SubSample { factor } => format!("subSample({factor})"),
        RbcClockTransferKind::SuperSample { factor } => format!("superSample({factor})"),
        RbcClockTransferKind::ShiftSample {
            counter,
            resolution,
        } => format!("shiftSample({counter}/{resolution})"),
        RbcClockTransferKind::BackSample {
            counter,
            resolution,
        } => format!("backSample({counter}/{resolution})"),
    }
}

fn literal(value: &RbcLiteral) -> String {
    match value {
        RbcLiteral::Real { value } => format!("{value}"),
        RbcLiteral::Integer { value } => format!("{value}"),
        RbcLiteral::Boolean { value } => format!("{value}"),
        RbcLiteral::String { value } => format!("{value:?}"),
        RbcLiteral::Enumeration { ordinal } => format!("enum({ordinal})"),
    }
}

fn binary(op: RbcBinaryOp) -> &'static str {
    match op {
        RbcBinaryOp::Add => "+",
        RbcBinaryOp::Subtract => "-",
        RbcBinaryOp::Multiply => "*",
        RbcBinaryOp::Divide => "/",
        RbcBinaryOp::Power => "^",
        RbcBinaryOp::Equal => "==",
        RbcBinaryOp::NotEqual => "<>",
        RbcBinaryOp::Less => "<",
        RbcBinaryOp::LessEqual => "<=",
        RbcBinaryOp::Greater => ">",
        RbcBinaryOp::GreaterEqual => ">=",
        RbcBinaryOp::And => "and",
        RbcBinaryOp::Or => "or",
        RbcBinaryOp::ElementwiseAdd => ".+",
        RbcBinaryOp::ElementwiseSubtract => ".-",
        RbcBinaryOp::ElementwiseMultiply => ".*",
        RbcBinaryOp::ElementwiseDivide => "./",
        RbcBinaryOp::ElementwisePower => ".^",
    }
}

mod execution;

pub fn run_disasm(path: &Path, options: DisasmOptions) -> Result<()> {
    let (file, encoding) = rumoca_bitcode::read_file(path).map_err(anyhow::Error::from)?;
    let model = &file.model;
    let listing = Listing::new(model);

    println!("; {} ({})", path.display(), encoding.as_str());
    println!(
        "; bitcode v{}, produced by {}",
        file.bitcode_version, file.producer
    );
    println!("; model {}", model.name);
    println!();

    print_variables(&listing);
    print_equations(&listing, options);
    print_equation_families(&listing, options);
    print_events(&listing);
    print_connections(model);
    print_connection_sets(model);
    if options.expressions {
        print_expression_table(&listing);
    }

    if let Some(artifact) = &file.execution {
        execution::print(artifact);
    }

    Ok(())
}

fn print_variables(listing: &Listing<'_>) {
    let model = listing.model;
    println!("variables ({})", model.variables.len());
    for variable in &model.variables {
        let mut annotations: Vec<String> = Vec::new();
        if let Some(unit) = &variable.unit {
            annotations.push(format!("unit={unit}"));
        }
        if let Some(quantity) = &variable.physical_quantity {
            annotations.push(format!("quantity={quantity}"));
        }
        if let Some(class) = &variable.declaring_class {
            annotations.push(format!("from={class}"));
        }
        for (label, slot) in [
            ("start", variable.start),
            ("min", variable.min),
            ("max", variable.max),
            ("binding", variable.binding),
        ] {
            if let Some(expression) = slot {
                annotations.push(format!("{label}={}", listing.expression(expression)));
            }
        }
        println!(
            "  [{:>3}] {:<10} {:<28} {}",
            variable.id.0,
            format!("{:?}", variable.role).to_lowercase(),
            variable.name,
            annotations.join(" ")
        );
    }
}

fn print_equations(listing: &Listing<'_>, options: DisasmOptions) {
    let model = listing.model;
    for (label, equations) in [
        ("equations", &model.equations),
        ("initial equations", &model.initial_equations),
    ] {
        if equations.is_empty() {
            continue;
        }
        println!();
        println!("{label} ({})", equations.len());
        for equation in equations {
            let rendered = listing.expression(equation.residual);
            let id = if options.ids {
                format!(" ; expr {}", equation.residual.0)
            } else {
                String::new()
            };
            let where_from = listing.provenance_suffix(&equation.provenance, options);
            println!("  [{:>3}]  0 = {rendered}{where_from}{id}", equation.id.0);
        }
    }
}

// An array equation is one family, not N scalar rows: printing the `for`
// header keeps the listing the same shape as the source it came from.
fn print_equation_families(listing: &Listing<'_>, options: DisasmOptions) {
    let model = listing.model;
    for (label, families) in [
        ("equation families", &model.equation_families),
        (
            "initial equation families",
            &model.initial_equation_families,
        ),
    ] {
        if families.is_empty() {
            continue;
        }
        println!();
        println!("{label} ({})", families.len());
        for family in families {
            let where_from = listing.provenance_suffix(&family.provenance, options);
            println!(
                "  [{:>3}]  for {} loop   ; {} scalar row{}{where_from}",
                family.id.0,
                listing.domain_binders(family.domain).join(", "),
                family.scalar_rows,
                if family.scalar_rows == 1 { "" } else { "s" }
            );
            for body in &family.bodies {
                println!("           0 = {}", listing.expression(*body));
            }
            println!("         end for;");
        }
    }
}

fn print_events(listing: &Listing<'_>) {
    let model = listing.model;
    if model.events.is_empty() {
        return;
    }
    println!();
    println!("events ({})", model.events.len());
    for event in &model.events {
        // The action is what a reader is looking for; the trigger and guard
        // are condition ids that mean nothing without the condition table.
        let action = match &event.action {
            RbcAction::Reinitialize { state, value } => format!(
                "reinit({}, {})",
                listing.variable(*state),
                listing.expression(*value)
            ),
            RbcAction::Assert { message, level } => format!(
                "assert(.., {}{})",
                listing.expression(*message),
                level
                    .map(|l| format!(", {}", listing.expression(l)))
                    .unwrap_or_default()
            ),
            RbcAction::Warning { condition, message } => {
                format!("warning ^{} ^{}", condition.0, message.0)
            }
            RbcAction::Terminate { message } => {
                format!("terminate({})", listing.expression(*message))
            }
        };
        println!("  [{:>3}] {action}", event.id.0);
    }
}

fn print_connections(model: &RbcModel) {
    if model.connections.is_empty() {
        return;
    }
    println!();
    println!("connections ({})", model.connections.len());
    for connection in &model.connections {
        println!(
            "  [{:>3}] {} <-> {}  ({:?})",
            connection.id.0,
            connection.left_connector,
            connection.right_connector,
            connection.quantity
        );
    }
}

fn print_connection_sets(model: &RbcModel) {
    if model.connection_sets.is_empty() {
        return;
    }
    println!();
    println!("connection sets ({})", model.connection_sets.len());
    for set in &model.connection_sets {
        let kind = if set.unconnected {
            "unconnected"
        } else if set.balances.is_empty() {
            "signal"
        } else {
            "acausal"
        };
        println!(
            "  [{:>3}] {:<11} {}",
            set.id.0,
            kind,
            set.connectors.join(" -- ")
        );
        for balance in &set.balances {
            let terms: Vec<String> = balance
                .terms
                .iter()
                .map(|term| balance_term(model, term.variable, term.negated))
                .collect();
            println!("        conserves  {} = 0", terms.join(" "));
        }
    }
}

/// One signed term of a flow balance, e.g. `+R.p.i`.
fn balance_term(model: &RbcModel, variable: VariableId, negated: bool) -> String {
    let name = model
        .variables
        .get(variable.0 as usize)
        .map(|variable| variable.name.as_str())
        .unwrap_or("?");
    format!("{}{}", if negated { "-" } else { "+" }, name)
}

fn print_expression_table(listing: &Listing<'_>) {
    let model = listing.model;
    println!();
    println!("expression table ({})", model.expressions.len());
    for expression in &model.expressions {
        println!(
            "  [{:>4}] {}",
            expression.id.0,
            listing.expression(expression.id)
        );
    }
}

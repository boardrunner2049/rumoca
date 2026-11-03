//! Textual emission, including lossless typed semantic records.
use super::*;
use std::fmt::Write as _;

// ── printing ─────────────────────────────────────────────────────────────────

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A real that reads back as the same bits.
///
/// `{}` renders `18.0` as `18`, which then parses as an integer literal and
/// changes the node's type. `{:?}` keeps the decimal point and round-trips.
fn real(value: f64) -> String {
    if value.is_finite() {
        format!("{value:?}")
    } else if value.is_nan() {
        "nan".to_string()
    } else if value > 0.0 {
        "inf".to_string()
    } else {
        "-inf".to_string()
    }
}

fn generation(value: RbcGeneration) -> &'static str {
    use RbcGeneration::*;
    match value {
        SyntheticResidual => "synthetic_residual",
        BindingEquation => "binding_equation",
        ConnectionEquation => "connection_equation",
        FlowBalanceEquation => "flow_balance_equation",
        AlgorithmEquation => "algorithm_equation",
        DiscreteUpdate => "discrete_update",
        ConditionLowering => "condition_lowering",
        PreValueLowering => "pre_value_lowering",
        ClockLowering => "clock_lowering",
        DelayLowering => "delay_lowering",
        SemiLinearLowering => "semi_linear_lowering",
        TerminalLowering => "terminal_lowering",
        EventActionLowering => "event_action_lowering",
        InitializationEquation => "initialization_equation",
        DefaultStart => "default_start",
        ArrayEquationProjection => "array_equation_projection",
        RecordEquationProjection => "record_equation_projection",
        FunctionLoopLowering => "function_loop_lowering",
        FunctionConditionLowering => "function_condition_lowering",
        FunctionAggregateLowering => "function_aggregate_lowering",
        DerivedParameterLowering => "derived_parameter_lowering",
        IndexReduction => "index_reduction",
        AliasElimination => "alias_elimination",
        RuntimeDiscontinuity => "runtime_discontinuity",
        Other => "other",
    }
}

fn provenance(value: &RbcProvenance) -> String {
    let span = &value.span;
    match value.origin {
        RbcOrigin::Source => format!(
            "@src {} {} {} {} {}",
            span.source.0, span.start, span.end, span.line, span.column
        ),
        RbcOrigin::Generated { generation: kind } => format!(
            "@gen {} {} {} {} {} {}",
            generation(kind),
            span.source.0,
            span.start,
            span.end,
            span.line,
            span.column
        ),
    }
}

fn scalar(value: RbcScalar) -> &'static str {
    match value {
        RbcScalar::Real => "real",
        RbcScalar::Integer => "integer",
        RbcScalar::Boolean => "boolean",
        RbcScalar::String => "string",
        RbcScalar::Enumeration => "enumeration",
        RbcScalar::Record => "record",
    }
}

fn role(value: RbcRole) -> &'static str {
    match value {
        RbcRole::Parameter => "parameter",
        RbcRole::Constant => "constant",
        RbcRole::Input => "input",
        RbcRole::State => "state",
        RbcRole::Algebraic => "algebraic",
        RbcRole::Output => "output",
        RbcRole::DiscreteReal => "discrete_real",
        RbcRole::DiscreteValue => "discrete_value",
    }
}

fn causality(value: RbcCausality) -> &'static str {
    match value {
        RbcCausality::Input => "input",
        RbcCausality::Output => "output",
        RbcCausality::Parameter => "parameter",
        RbcCausality::CalculatedParameter => "calculated_parameter",
        RbcCausality::Independent => "independent",
        RbcCausality::Local => "local",
    }
}

fn quantity_kind(value: RbcQuantityKind) -> &'static str {
    match value {
        RbcQuantityKind::Potential => "potential",
        RbcQuantityKind::Flow => "flow",
        RbcQuantityKind::Stream => "stream",
    }
}

fn unary(value: RbcUnaryOp) -> &'static str {
    match value {
        RbcUnaryOp::Negate => "negate",
        RbcUnaryOp::Not => "not",
        RbcUnaryOp::Plus => "plus",
    }
}

fn binary(value: RbcBinaryOp) -> &'static str {
    use RbcBinaryOp::*;
    match value {
        Add => "add",
        Subtract => "sub",
        Multiply => "mul",
        Divide => "div",
        Power => "pow",
        Equal => "eq",
        NotEqual => "ne",
        Less => "lt",
        LessEqual => "le",
        Greater => "gt",
        GreaterEqual => "ge",
        And => "and",
        Or => "or",
        ElementwiseAdd => "eadd",
        ElementwiseSubtract => "esub",
        ElementwiseMultiply => "emul",
        ElementwiseDivide => "ediv",
        ElementwisePower => "epow",
    }
}

fn coordinate(value: RbcCoordinate) -> String {
    use RbcCoordinate::*;
    match value {
        Time => "time".to_string(),
        Parameter { variable } => format!("param %{}", variable.0),
        Input { variable } => format!("input %{}", variable.0),
        State { variable } => format!("state %{}", variable.0),
        Derivative { variable } => format!("der %{}", variable.0),
        Algebraic { variable } => format!("alg %{}", variable.0),
        DiscreteReal { variable } => format!("dreal %{}", variable.0),
        DiscreteValue { variable } => format!("dval %{}", variable.0),
        PreState { variable } => format!("pre_state %{}", variable.0),
        PreAlgebraic { variable } => format!("pre_alg %{}", variable.0),
        PreDiscreteReal { variable } => format!("pre_dreal %{}", variable.0),
        PreDiscreteValue { variable } => format!("pre_dval %{}", variable.0),
        Binder { domain, ordinal } => format!("binder &{} {ordinal}", domain.0),
        Condition { condition } => format!("cond ?{}", condition.0),
        FunctionParameter { function, ordinal } => format!("fnparam ~{} {ordinal}", function.0),
        ClockInterval { clock } => format!("clockint {}", clock.0),
        Delay { delay } => format!("delay {}", delay.0),
        Previous { previous } => format!("previous {}", previous.0),
        Terminal { terminal } => format!("terminal {}", terminal.0),
    }
}

fn subscript(value: &RbcSubscript) -> String {
    match value {
        RbcSubscript::Index { expression } => format!("at ^{}", expression.0),
        RbcSubscript::Whole => "all".to_string(),
        RbcSubscript::Slice { expression } => format!("slice ^{}", expression.0),
    }
}

fn literal(value: &RbcLiteral) -> String {
    match value {
        RbcLiteral::Real { value } => format!("lit real {}", real(*value)),
        RbcLiteral::Integer { value } => format!("lit integer {value}"),
        RbcLiteral::Enumeration { ordinal } => format!("lit enum {ordinal}"),
        RbcLiteral::Boolean { value } => format!("lit boolean {value}"),
        RbcLiteral::String { value } => format!("lit string {}", quote(value)),
    }
}

/// Render `file` as the textual IR.
/// A table entry as a quoted JSON record, for constructs whose structure is
/// nested deeper than one line reads well.
fn json(value: &impl serde::Serialize) -> Result<String, TextError> {
    Ok(quote(
        &serde_json::to_string(value).map_err(|e| TextError::at(0, e.to_string()))?,
    ))
}

/// Whether a function carries anything the one-line `fn` form omits.
fn needs_record(function: &RbcFunction) -> bool {
    let external_abi = matches!(
        &function.body,
        RbcFunctionBody::External {
            purity,
            arguments,
            result,
            linkage,
            ..
        } if *purity == RbcPurity::Pure
            || !arguments.is_empty()
            || result.is_some()
            || !linkage.is_empty()
    );
    matches!(function.body, RbcFunctionBody::Modelica { .. })
        || external_abi
        || !function.values.is_empty()
        || !function.folds.is_empty()
        || !function.derivatives.is_empty()
        || !function.calls.is_empty()
        || function.parameters.iter().any(|p| p.declaration.is_some())
}

/// `%id` references, space separated.
fn variable_refs(ids: &[VariableId]) -> String {
    ids.iter()
        .map(|v| format!("%{}", v.0))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `^id` references, space separated.
fn expression_refs(ids: &[ExprId]) -> String {
    ids.iter()
        .map(|e| format!("^{}", e.0))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extents as `a,b,c`, or `-` when there are none.
fn extents(values: &[u32]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Append the `reads`, `dreads` and `preads` clauses, each only when
/// non-empty.
fn write_reads(
    line: &mut String,
    reads: &[VariableId],
    reads_derivative: &[VariableId],
    reads_previous: &[VariableId],
) {
    for (key, ids) in [
        ("reads", reads),
        ("dreads", reads_derivative),
        ("preads", reads_previous),
    ] {
        if !ids.is_empty() {
            let _ = write!(line, " {key} {}", variable_refs(ids));
        }
    }
}

pub fn print_text(file: &RbcFile) -> Result<String, TextError> {
    print_text_with(file, TextOptions::default())
}

pub fn print_text_with(file: &RbcFile, options: TextOptions) -> Result<String, TextError> {
    if file.execution.is_some()
        || !file.model.connectors.is_empty()
        || !file.model.connector_types.is_empty()
    {
        return Err(TextError::at(
            0,
            "text profile cannot represent executable/connector declarations; use JSON or CBOR",
        ));
    }
    let model = &file.model;
    let mut out = String::new();
    let _ = writeln!(out, "; rumoca bitcode, textual form");
    let _ = writeln!(out, "rbc {}", file.bitcode_version);
    let _ = writeln!(out, "producer {}", quote(&file.producer));
    let _ = writeln!(out, "model {}", quote(&model.name));

    write_sources(&mut out, model, options);
    write_types(&mut out, model);
    write_functions(&mut out, model)?;
    write_components(&mut out, model);
    write_variables(&mut out, model);
    write_expressions(&mut out, model)?;
    write_equations(&mut out, model);
    write_domains(&mut out, model);
    write_families(&mut out, model);
    write_discrete_real_equations(&mut out, model);
    write_initial_discrete_values(&mut out, model);
    for entry in &model.initial_parameter_values {
        let _ = writeln!(
            out,
            "ipval %{} ^{} {}",
            entry.target.0,
            entry.value.0,
            provenance(&entry.provenance)
        );
    }
    write_relations(&mut out, model);
    write_records(&mut out, model)?;
    write_conditions(&mut out, model);
    write_roots(&mut out, model);
    write_events(&mut out, model);
    write_time_events(&mut out, model);
    write_connections(&mut out, model);
    write_connection_sets(&mut out, model);
    write_discrete_definitions(&mut out, model);
    write_trace_points(&mut out, model);

    Ok(out)
}

fn write_sources(out: &mut String, model: &RbcModel, options: TextOptions) {
    if model.sources.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; sources");
    for source in &model.sources {
        match (&source.text, options.sources) {
            (Some(text), true) => {
                let _ = writeln!(
                    out,
                    "!{} source {} text {}",
                    source.id.0,
                    quote(&source.name),
                    quote(text)
                );
            }
            _ => {
                let _ = writeln!(out, "!{} source {}", source.id.0, quote(&source.name));
            }
        }
    }
}

fn write_types(out: &mut String, model: &RbcModel) {
    if model.types.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; types");
    for entry in &model.types {
        let dims = if entry.dimensions.is_empty() {
            String::new()
        } else {
            format!(
                " dims {}",
                entry
                    .dimensions
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let record = match &entry.record {
            Some(record) => format!(
                " record {} {} {}",
                quote(&record.name),
                record.fields.len(),
                record
                    .fields
                    .iter()
                    .map(|f| format!("{} ${}", quote(&f.name), f.value_type.0))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "${} type {}{dims}{record}",
            entry.id.0,
            scalar(entry.scalar)
        );
    }
}

fn write_functions(out: &mut String, model: &RbcModel) -> Result<(), TextError> {
    if model.functions.is_empty() {
        return Ok(());
    }
    let _ = writeln!(out, "\n; functions");
    for function in &model.functions {
        // A function the one-line form cannot hold -- a carried body,
        // its value table and folds, call edges, an external ABI,
        // parameter spans -- is written as a record, the way clocks are,
        // rather than printed with those parts missing.
        if needs_record(function) {
            let _ = writeln!(out, "function_record {}", json(function)?);
            continue;
        }
        let _ = writeln!(out, "{}", function_line(function));
    }
    Ok(())
}

/// The one-line `fn` form of a function `needs_record` lets through.
fn function_line(function: &RbcFunction) -> String {
    let parameters = function
        .parameters
        .iter()
        .map(|p| format!("{} ${}", quote(&p.name), p.value_type.0))
        .collect::<Vec<_>>()
        .join(" ");
    let results = function
        .results
        .iter()
        .map(|t| format!("${}", t.0))
        .collect::<Vec<_>>()
        .join(" ");
    let body = match &function.body {
        RbcFunctionBody::ElidedModelica => "body elided".to_string(),
        RbcFunctionBody::External {
            language, symbol, ..
        } => {
            format!("body external {} {}", quote(language), quote(symbol))
        }
        RbcFunctionBody::Modelica { .. } => {
            unreachable!("a carried body is written as a function record")
        }
    };
    let inline = match function.inline {
        RbcInline::Unstated => "",
        RbcInline::Requested => " inline",
        RbcInline::Never => " noinline",
        RbcInline::AfterIndexReduction => " lateinline",
    };
    format!(
        "~{} fn {} params {} {parameters} results {} {results} \
{body}{inline} {}",
        function.id.0,
        quote(&function.name),
        function.parameters.len(),
        function.results.len(),
        provenance(&function.declaration)
    )
}

fn write_components(out: &mut String, model: &RbcModel) {
    if model.components.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; components");
    for component in &model.components {
        let class = component
            .class_name
            .as_deref()
            .map(|name| format!(" of {}", quote(name)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "#{} comp {}{class}",
            component.id.0,
            quote(&component.path)
        );
    }
}

fn write_variables(out: &mut String, model: &RbcModel) {
    if model.variables.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; variables");
    for variable in &model.variables {
        let _ = writeln!(
            out,
            "{} {}",
            variable_line(variable),
            provenance(&variable.declaration)
        );
    }
}

/// A variable's line, up to but not including its provenance.
fn variable_line(variable: &RbcVariable) -> String {
    let mut line = format!(
        "%{} var {} ${} {} {} scalars {}",
        variable.id.0,
        quote(&variable.name),
        variable.value_type.0,
        role(variable.role),
        causality(variable.causality),
        variable.scalar_count
    );
    if variable.discrete_input {
        let _ = write!(line, " discrete");
    }
    if let Some(contract) = &variable.contract {
        write_contract(&mut line, contract);
    }
    if let Some(component) = variable.component {
        let _ = write!(line, " comp #{}", component.0);
    }
    for (key, slot) in [
        ("unit", &variable.unit),
        ("quantity", &variable.physical_quantity),
        ("class", &variable.declaring_class),
        ("desc", &variable.description),
    ] {
        if let Some(value) = slot {
            let _ = write!(line, " {key} {}", quote(value));
        }
    }
    if let Some(fixed) = variable.fixed {
        let _ = write!(line, " fixed {fixed}");
    }
    if let Some(values) = &variable.fixed_elements {
        let value = serde_json::to_string(values).expect("boolean list serializes");
        let _ = write!(line, " fixed_elements {}", quote(&value));
    }
    if let Some(causality) = variable.declared_causality {
        let name = match causality {
            RbcDeclaredCausality::Input => "input",
            RbcDeclaredCausality::Output => "output",
            RbcDeclaredCausality::None => "none",
        };
        let _ = write!(line, " declared {name}");
    }
    if variable.held {
        let _ = write!(line, " held");
    }
    if variable.state_select != RbcStateSelect::Default {
        let value = serde_json::to_string(&variable.state_select).expect("enum serializes");
        let _ = write!(line, " state_select {}", quote(&value));
    }
    if variable.evaluable {
        let _ = write!(line, " evaluable");
    }
    if variable.tunable {
        let _ = write!(line, " tunable");
    }
    if variable.from_source {
        let _ = write!(line, " from_source");
    }
    for (key, slot) in [
        ("start", variable.start),
        ("min", variable.min),
        ("max", variable.max),
        ("nominal", variable.nominal),
        ("binding", variable.binding),
    ] {
        if let Some(expression) = slot {
            let _ = write!(line, " {key} ^{}", expression.0);
        }
    }
    if let Some(connector) = &variable.connector {
        let connected = if connector.connected {
            " connected"
        } else {
            ""
        };
        let _ = write!(
            line,
            " connector {}{connected}",
            quantity_kind(connector.quantity)
        );
    }
    line
}

/// Append a variable's `contract` clause.
fn write_contract(line: &mut String, contract: &RbcSymbolContract) {
    let _ = write!(
        line,
        " contract {}",
        match contract.variability {
            RbcVariability::Constant => "constant",
            RbcVariability::Parameter => "parameter",
            RbcVariability::Discrete => "discrete",
            RbcVariability::Continuous => "continuous",
        }
    );
    for (flag, set) in [
        ("final", contract.is_final),
        ("protected", contract.is_protected),
        ("evaluate", contract.evaluate),
        ("structural", contract.structural),
        ("frommod", contract.binding_from_modification),
    ] {
        if set {
            let _ = write!(line, " {flag}");
        }
    }
    if let Some(value) = contract.effective_value {
        let _ = write!(line, " value {}", real(value));
    }
    if !contract.binding_depends_on.is_empty() {
        let _ = write!(
            line,
            " uses {}",
            variable_refs(&contract.binding_depends_on)
        );
    }
    if let Some(declared) = &contract.declared_in {
        let _ = write!(line, " declaredin {}", quote(declared));
    }
}

fn write_expressions(out: &mut String, model: &RbcModel) -> Result<(), TextError> {
    if model.expressions.is_empty() {
        return Ok(());
    }
    let _ = writeln!(out, "\n; expressions");
    for expression in &model.expressions {
        let body = expression_body(&expression.node)?;
        let _ = writeln!(
            out,
            "^{} expr ${} {body} {}",
            expression.id.0,
            expression.value_type.0,
            provenance(&expression.provenance)
        );
    }
    Ok(())
}

fn expression_body(node: &RbcExprNode) -> Result<String, TextError> {
    Ok(match node {
        RbcExprNode::Literal { value } => literal(value),
        RbcExprNode::StringConversion { value, format } => format!(
            "string_conversion ^{} {}",
            value.0,
            quote(&serde_json::to_string(format).map_err(|e| TextError::at(0, e.to_string()))?)
        ),
        RbcExprNode::Coordinate { coordinate: c } => format!("coord {}", coordinate(*c)),
        RbcExprNode::Unary { op, operand } => format!("un {} ^{}", unary(*op), operand.0),
        RbcExprNode::Binary { op, lhs, rhs } => {
            format!("bin {} ^{} ^{}", binary(*op), lhs.0, rhs.0)
        }
        RbcExprNode::Conditional { branches, fallback } => conditional_body(branches, *fallback),
        RbcExprNode::Builtin { name, arguments } => format!(
            "call {} {} {}",
            quote(name),
            arguments.len(),
            expression_refs(arguments)
        ),
        RbcExprNode::Array {
            elements,
            empty_type,
        } => format!(
            "array {}{} {}",
            elements.len(),
            match empty_type {
                Some(ty) => format!(" of ${}", ty.0),
                None => String::new(),
            },
            expression_refs(elements)
        ),
        RbcExprNode::Record { ty, fields } => format!(
            "record ${} {} {}",
            ty.0,
            fields.len(),
            expression_refs(fields)
        ),
        RbcExprNode::Field { base, field } => format!("field ^{} {field}", base.0),
        RbcExprNode::Range { start, step, stop } => format!(
            "range ^{} {} ^{}",
            start.0,
            match step {
                Some(step) => format!("step ^{}", step.0),
                None => "nostep".to_string(),
            },
            stop.0
        ),
        RbcExprNode::Comprehension { domain, body } => {
            format!("comp &{} ^{}", domain.0, body.0)
        }
        RbcExprNode::Index { base, subscripts } => format!(
            "index ^{} {} {}",
            base.0,
            subscripts.len(),
            subscript_list(subscripts)
        ),
        RbcExprNode::ArrayUpdate {
            base,
            value,
            subscripts,
        } => format!(
            "update ^{} ^{} {} {}",
            base.0,
            value.0,
            subscripts.len(),
            subscript_list(subscripts)
        ),
        RbcExprNode::Call {
            owner,
            function,
            output,
            arguments,
        } => format!(
            "invoke ~{} out {output} owner ^{} {} {}",
            function.0,
            owner.0,
            arguments.len(),
            expression_refs(arguments)
        ),
        // The text profile carries no function bodies, so it cannot
        // render a reference into one. `emit-text` refuses such an
        // artifact outright; this keeps the listing honest if it is
        // ever reached another way.
        inner @ (RbcExprNode::FunctionValue { .. }
        | RbcExprNode::FunctionFoldParameter { .. }
        | RbcExprNode::FunctionFoldOutput { .. }) => function_reference_body(inner),
        RbcExprNode::ClockTransfer {
            transfer: kind,
            source,
            source_clock,
            target_clock,
        } => format!(
            "ctransfer {} ^{} clocks {} {}",
            clock_transfer(kind),
            source.0,
            source_clock.0,
            target_clock.0
        ),
        RbcExprNode::Unsupported { detail } => format!("unsupported {}", quote(detail)),
    })
}

fn conditional_body(branches: &[RbcBranch], fallback: ExprId) -> String {
    let arms: Vec<String> = branches
        .iter()
        .map(|b| format!("^{} ^{}", b.condition.0, b.value.0))
        .collect();
    format!(
        "cond {} {} else ^{}",
        arms.len(),
        arms.join(" "),
        fallback.0
    )
}

/// A reference to a value inside a function body; see `expression_body`.
fn function_reference_body(node: &RbcExprNode) -> String {
    match node {
        RbcExprNode::FunctionValue {
            function,
            value,
            definition,
        } => format!("fnvalue ~{} {value} def {definition}", function.0),
        RbcExprNode::FunctionFoldParameter {
            function,
            fold,
            carried,
            definition,
        } => format!(
            "foldparam ~{} {fold} {carried} def {definition}",
            function.0
        ),
        RbcExprNode::FunctionFoldOutput {
            function,
            fold,
            carried,
            definition,
        } => format!("foldout ~{} {fold} {carried} def {definition}", function.0),
        _ => unreachable!("only function-body references are passed here"),
    }
}

fn subscript_list(subscripts: &[RbcSubscript]) -> String {
    subscripts
        .iter()
        .map(subscript)
        .collect::<Vec<_>>()
        .join(" ")
}

fn clock_transfer(kind: &RbcClockTransferKind) -> String {
    match kind {
        RbcClockTransferKind::SubSample { factor } => format!("sub {factor}"),
        RbcClockTransferKind::SuperSample { factor } => format!("super {factor}"),
        RbcClockTransferKind::ShiftSample {
            counter,
            resolution,
        } => format!("shift {counter} {resolution}"),
        RbcClockTransferKind::BackSample {
            counter,
            resolution,
        } => format!("back {counter} {resolution}"),
    }
}

fn write_equations(out: &mut String, model: &RbcModel) {
    for (keyword, equations) in [("eq", &model.equations), ("ieq", &model.initial_equations)] {
        if equations.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n; {keyword}");
        for equation in equations {
            let mut line = format!("{keyword} {} ^{}", equation.id.0, equation.residual.0);
            write_reads(
                &mut line,
                &equation.reads,
                &equation.reads_derivative,
                &equation.reads_previous,
            );
            let _ = writeln!(out, "{line} {}", provenance(&equation.provenance));
        }
    }
}

fn write_domains(out: &mut String, model: &RbcModel) {
    if model.domains.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; domains");
    for domain in &model.domains {
        let binders = domain
            .binders
            .iter()
            .map(|b| {
                format!(
                    "{} {} {} {} {}",
                    b.id,
                    quote(&b.display_name),
                    b.lower,
                    b.upper,
                    b.step
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let parent = domain
            .parent
            .map(|p| format!(" parent &{}", p.0))
            .unwrap_or_default();
        let extents = extents(&domain.extents);
        let _ = writeln!(
            out,
            "&{} domain scalars {} extents {extents}{parent} \
binders {} {binders} {}",
            domain.id.0,
            domain.scalar_count,
            domain.binders.len(),
            provenance(&domain.provenance)
        );
    }
}

fn write_families(out: &mut String, model: &RbcModel) {
    for (keyword, families) in [
        ("family", &model.equation_families),
        ("ifamily", &model.initial_equation_families),
    ] {
        if families.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n; {keyword}");
        for family in families {
            let _ = writeln!(
                out,
                "{} {}",
                family_line(keyword, family),
                provenance(&family.provenance)
            );
        }
    }
}

/// An equation family's line, up to but not including its provenance.
fn family_line(keyword: &str, family: &RbcEquationFamily) -> String {
    let view = match family.scalar_view {
        RbcScalarView::BinderSubstitution => "binder".to_string(),
        RbcScalarView::RowMajorProjection => "rowmajor".to_string(),
        RbcScalarView::BinderPrefixProjection { binder_count } => {
            format!("prefix {binder_count}")
        }
    };
    let extents = extents(&family.extents);
    let bodies = expression_refs(&family.bodies);
    let mut line = format!(
        "{keyword} {} domain &{} rows {} extents {extents} \
view {view} bodies {} {bodies}",
        family.id.0,
        family.domain.0,
        family.scalar_rows,
        family.bodies.len()
    );
    write_reads(
        &mut line,
        &family.reads,
        &family.reads_derivative,
        &family.reads_previous,
    );
    line
}

fn write_discrete_real_equations(out: &mut String, model: &RbcModel) {
    if model.discrete_real_equations.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; discrete real equations (MLS B.1b)");
    for equation in &model.discrete_real_equations {
        let activation = match equation.activation {
            RbcDiscreteRealActivation::Always => "always".to_string(),
            RbcDiscreteRealActivation::When { trigger, guard } => {
                format!("when ?{} guard ?{}", trigger.0, guard.0)
            }
        };
        let mut line = format!(
            "dreq {} ^{} {activation}",
            equation.id.0, equation.residual.0
        );
        write_reads(
            &mut line,
            &equation.reads,
            &equation.reads_derivative,
            &equation.reads_previous,
        );
        let _ = writeln!(out, "{line} {}", provenance(&equation.provenance));
    }
}

fn write_initial_discrete_values(out: &mut String, model: &RbcModel) {
    if model.initial_discrete_values.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; initial discrete values");
    for entry in &model.initial_discrete_values {
        let _ = writeln!(
            out,
            "idval %{} ^{} {}",
            entry.target.0,
            entry.value.0,
            provenance(&entry.provenance)
        );
    }
}

fn write_relations(out: &mut String, model: &RbcModel) {
    if model.relations.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; relations");
    for relation in &model.relations {
        let _ = writeln!(
            out,
            "rel {} ^{} {}",
            relation.id.0,
            relation.expression.0,
            provenance(&relation.provenance)
        );
    }
}

/// The tables written as quoted JSON records rather than line forms.
fn write_records(out: &mut String, model: &RbcModel) -> Result<(), TextError> {
    for clock in &model.clocks {
        let _ = writeln!(
            out,
            "clock_record {}",
            quote(&serde_json::to_string(clock).map_err(|e| TextError::at(0, e.to_string()))?)
        );
    }
    for owner in &model.clock_ownerships {
        let _ = writeln!(
            out,
            "clock_ownership {}",
            quote(&serde_json::to_string(owner).map_err(|e| TextError::at(0, e.to_string()))?)
        );
    }
    for transaction in &model.model_event_transactions {
        let _ = writeln!(out, "model_event_transaction {}", json(transaction)?);
    }
    for previous in &model.previous_values {
        let _ = writeln!(out, "previous_value {}", json(previous)?);
    }
    for terminal in &model.terminals {
        let _ = writeln!(out, "terminal_record {}", json(terminal)?);
    }
    for root in &model.structured_roots {
        let _ = writeln!(out, "structured_root {}", json(root)?);
    }
    for delay in &model.delays {
        let _ = writeln!(out, "delay_record {}", json(delay)?);
    }
    Ok(())
}

fn write_conditions(out: &mut String, model: &RbcModel) {
    if model.conditions.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; conditions");
    for condition in &model.conditions {
        use RbcConditionNode::*;
        let body = match &condition.node {
            Initial => "initial".to_string(),
            Always => "always".to_string(),
            ClockActivation { clock } => format!("clock_activation {}", clock.0),
            Relation { relation } => format!("rel {}", relation.0),
            Discrete { expression } => format!("discrete ^{}", expression.0),
            Not { operand } => format!("not {}", operand.0),
            And { lhs, rhs } => format!("and {} {}", lhs.0, rhs.0),
            Or { lhs, rhs } => format!("or {} {}", lhs.0, rhs.0),
            AnyRise { lhs, rhs } => format!("any_rise {} {}", lhs.0, rhs.0),
            Unsupported { detail } => format!("unsupported {}", quote(detail)),
        };
        let _ = writeln!(
            out,
            "cond {} {body} {}",
            condition.id.0,
            provenance(&condition.provenance)
        );
    }
}

fn write_roots(out: &mut String, model: &RbcModel) {
    if model.roots.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; roots");
    for root in &model.roots {
        let _ = writeln!(
            out,
            "root {} rel {} act {} {}",
            root.id.0,
            root.relation.0,
            root.activation.0,
            provenance(&root.provenance)
        );
    }
}

fn write_events(out: &mut String, model: &RbcModel) {
    if model.events.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; events");
    for event in &model.events {
        let action = match &event.action {
            RbcAction::Reinitialize { state, value } => {
                format!("reinit %{} ^{}", state.0, value.0)
            }
            RbcAction::Assert { message, level } => match level {
                Some(level) => format!("assert ^{} level ^{}", message.0, level.0),
                None => format!("assert ^{}", message.0),
            },
            RbcAction::Warning { condition, message } => {
                format!("warning ^{} ^{}", condition.0, message.0)
            }
            RbcAction::Terminate { message } => format!("terminate ^{}", message.0),
        };
        let _ = writeln!(
            out,
            "event {} trig {} guard {} {action} {}",
            event.id.0,
            event.trigger.0,
            event.guard.0,
            provenance(&event.provenance)
        );
    }
}

fn write_time_events(out: &mut String, model: &RbcModel) {
    if model.time_events.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; time events");
    for event in &model.time_events {
        let schedule = match event.schedule {
            RbcSchedule::Static {
                numerator,
                denominator,
            } => format!("static {numerator} {denominator}"),
            RbcSchedule::Dynamic { deadline } => format!("dynamic ^{}", deadline.0),
        };
        let _ = writeln!(
            out,
            "tevent {} {schedule} {}",
            event.id.0,
            provenance(&event.provenance)
        );
    }
}

fn write_connections(out: &mut String, model: &RbcModel) {
    if model.connections.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; connections");
    for connection in &model.connections {
        let equation = connection
            .equation
            .map(|e| format!(" eq {}", e.0))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "conn {} %{} %{} {} {} {}{equation} {}",
            connection.id.0,
            connection.left.0,
            connection.right.0,
            quantity_kind(connection.quantity),
            quote(&connection.left_connector),
            quote(&connection.right_connector),
            provenance(&connection.provenance)
        );
    }
}

fn write_connection_sets(out: &mut String, model: &RbcModel) {
    if model.connection_sets.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; connection sets");
    for set in &model.connection_sets {
        let mut line = format!("connset {}", set.id.0);
        for connector in &set.connectors {
            let _ = write!(line, " at {}", quote(connector));
        }
        for potential in &set.potentials {
            let _ = write!(line, " pot %{}", potential.0);
        }
        for balance in &set.balances {
            write_balance(&mut line, balance);
        }
        for equation in &set.potential_equations {
            let _ = write!(line, " poteq {}", equation.0);
        }
        if set.unconnected {
            let _ = write!(line, " unconnected");
        }
        let _ = writeln!(out, "{line} {}", provenance(&set.provenance));
    }
}

/// Append one flow balance of a connection set, `balance ... end`.
fn write_balance(line: &mut String, balance: &RbcFlowBalance) {
    let _ = write!(line, " balance");
    if let Some(equation) = balance.equation {
        let _ = write!(line, " eq {}", equation.0);
    }
    for term in &balance.terms {
        let sign = if term.negated { "-" } else { "+" };
        let _ = write!(line, " {sign}%{}", term.variable.0);
    }
    let _ = write!(line, " end");
}

fn write_discrete_definitions(out: &mut String, model: &RbcModel) {
    if model.discrete_definitions.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; discrete definitions");
    for definition in &model.discrete_definitions {
        let targets = variable_refs(&definition.targets);
        let _ = writeln!(
            out,
            "disc {} targets {}{}",
            definition.targets.len(),
            targets,
            if definition.observed { " observed" } else { "" }
        );
        for branch in &definition.branches {
            write_discrete_branch(out, branch);
        }
        let _ = writeln!(out, "  end {}", provenance(&definition.provenance));
    }
}

fn write_discrete_branch(out: &mut String, branch: &RbcDiscreteBranch) {
    let activation = match branch.activation {
        RbcDiscreteActivation::Always => "always".to_string(),
        RbcDiscreteActivation::When { trigger, guard } => {
            format!("when {} {}", trigger.0, guard.0)
        }
    };
    let values = expression_refs(&branch.values);
    let _ = writeln!(
        out,
        "  branch {activation} values {} {values} {}",
        branch.values.len(),
        provenance(&branch.provenance)
    );
}

fn write_trace_points(out: &mut String, model: &RbcModel) {
    if model.trace_points.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n; trace points");
    for point in &model.trace_points {
        let mut line = format!(
            "trace {} %{} {}",
            point.id.0,
            point.variable.0,
            quote(&point.label)
        );
        if let Some(connection) = point.connection {
            let _ = write!(line, " conn {}", connection.0);
        }
        if let Some(set) = point.connection_set {
            let _ = write!(line, " connset {}", set.0);
        }
        if let Some(kind) = point.quantity {
            let _ = write!(line, " kind {}", quantity_kind(kind));
        }
        if let Some(unit) = &point.unit {
            let _ = write!(line, " unit {}", quote(unit));
        }
        if let Some(added_by) = &point.added_by {
            let _ = write!(line, " added_by {}", quote(added_by));
        }
        let _ = writeln!(out, "{line}");
    }
}

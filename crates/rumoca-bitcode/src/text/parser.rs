//! Parse explicit identities and reconstruct the current public document.
use super::*;

fn parse_generation(word: &str) -> Option<RbcGeneration> {
    use RbcGeneration::*;
    Some(match word {
        "synthetic_residual" => SyntheticResidual,
        "binding_equation" => BindingEquation,
        "connection_equation" => ConnectionEquation,
        "flow_balance_equation" => FlowBalanceEquation,
        "algorithm_equation" => AlgorithmEquation,
        "discrete_update" => DiscreteUpdate,
        "condition_lowering" => ConditionLowering,
        "pre_value_lowering" => PreValueLowering,
        "clock_lowering" => ClockLowering,
        "delay_lowering" => DelayLowering,
        "semi_linear_lowering" => SemiLinearLowering,
        "terminal_lowering" => TerminalLowering,
        "event_action_lowering" => EventActionLowering,
        "initialization_equation" => InitializationEquation,
        "default_start" => DefaultStart,
        "array_equation_projection" => ArrayEquationProjection,
        "record_equation_projection" => RecordEquationProjection,
        "function_loop_lowering" => FunctionLoopLowering,
        "function_condition_lowering" => FunctionConditionLowering,
        "function_aggregate_lowering" => FunctionAggregateLowering,
        "derived_parameter_lowering" => DerivedParameterLowering,
        "index_reduction" => IndexReduction,
        "alias_elimination" => AliasElimination,
        "runtime_discontinuity" => RuntimeDiscontinuity,
        "other" => Other,
        _ => return None,
    })
}

// ── parsing ──────────────────────────────────────────────────────────────────

/// A line split into tokens, with quoted strings kept whole.
///
/// Hand-rolled rather than a lexer crate: the grammar is one statement per
/// line over a fixed keyword set, and the only subtlety is that a quoted
/// string may contain spaces and escapes.
fn tokenize(line: &str, number: usize) -> Result<Vec<String>, TextError> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&character) = chars.peek() {
        if character.is_whitespace() {
            chars.next();
            continue;
        }
        if character == ';' {
            break; // comment to end of line
        }
        if character == '"' {
            chars.next();
            let mut value = String::new();
            loop {
                match chars.next() {
                    None => return Err(TextError::at(number, "unterminated string")),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some('n') => value.push('\n'),
                        Some('r') => value.push('\r'),
                        Some('t') => value.push('\t'),
                        Some('\\') => value.push('\\'),
                        Some('"') => value.push('"'),
                        Some(other) => value.push(other),
                        None => return Err(TextError::at(number, "trailing escape")),
                    },
                    Some(other) => value.push(other),
                }
            }
            tokens.push(format!("\u{0}{value}")); // marked as a string literal
            continue;
        }
        let mut word = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_whitespace() || next == ';' {
                break;
            }
            word.push(next);
            chars.next();
        }
        tokens.push(word);
    }
    Ok(tokens)
}

/// Cursor over one line's tokens. Every accessor reports the line on failure.
struct Cursor<'a> {
    tokens: &'a [String],
    at: usize,
    line: usize,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [String], line: usize) -> Self {
        Self {
            tokens,
            at: 0,
            line,
        }
    }

    fn done(&self) -> bool {
        self.at >= self.tokens.len()
    }

    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.at).map(String::as_str)
    }

    fn next_raw(&mut self) -> Result<&'a str, TextError> {
        let token = self
            .tokens
            .get(self.at)
            .ok_or_else(|| TextError::at(self.line, "unexpected end of line"))?;
        self.at += 1;
        Ok(token.as_str())
    }

    fn word(&mut self) -> Result<&'a str, TextError> {
        let token = self.next_raw()?;
        if let Some(text) = token.strip_prefix('\u{0}') {
            return Err(TextError::at(
                self.line,
                format!("expected a word, got string {text:?}"),
            ));
        }
        Ok(token)
    }

    fn string(&mut self) -> Result<String, TextError> {
        let token = self.next_raw()?;
        token
            .strip_prefix('\u{0}')
            .map(str::to_string)
            .ok_or_else(|| {
                TextError::at(
                    self.line,
                    format!("expected a quoted string, got `{token}`"),
                )
            })
    }

    /// A sigil-prefixed id: `%3`, `^12`, `$0`, `#1`, `!2`.
    fn id(&mut self, sigil: char) -> Result<u32, TextError> {
        let token = self.word()?;
        let rest = token.strip_prefix(sigil).ok_or_else(|| {
            TextError::at(self.line, format!("expected `{sigil}<id>`, got `{token}`"))
        })?;
        rest.parse()
            .map_err(|_| TextError::at(self.line, format!("`{rest}` is not an id")))
    }

    fn number<T: std::str::FromStr>(&mut self) -> Result<T, TextError> {
        let token = self.word()?;
        token
            .parse()
            .map_err(|_| TextError::at(self.line, format!("`{token}` is not a number")))
    }

    fn expect(&mut self, keyword: &str) -> Result<(), TextError> {
        let token = self.word()?;
        if token != keyword {
            return Err(TextError::at(
                self.line,
                format!("expected `{keyword}`, got `{token}`"),
            ));
        }
        Ok(())
    }

    /// Consume `keyword` if it is next. Used for optional clauses.
    fn eat(&mut self, keyword: &str) -> bool {
        if self.peek() == Some(keyword) {
            self.at += 1;
            return true;
        }
        false
    }

    fn provenance(&mut self) -> Result<RbcProvenance, TextError> {
        let marker = self.word()?;
        let origin = match marker {
            "@src" => RbcOrigin::Source,
            "@gen" => {
                let word = self.word()?;
                let generation = parse_generation(word).ok_or_else(|| {
                    TextError::at(self.line, format!("unknown generation `{word}`"))
                })?;
                RbcOrigin::Generated { generation }
            }
            other => {
                return Err(TextError::at(
                    self.line,
                    format!("expected `@src` or `@gen`, got `{other}`"),
                ));
            }
        };
        Ok(RbcProvenance {
            origin,
            span: RbcSpan {
                source: SourceId(self.number()?),
                start: self.number()?,
                end: self.number()?,
                line: self.number()?,
                column: self.number()?,
            },
        })
    }
}

fn parse_scalar(word: &str, line: usize) -> Result<RbcScalar, TextError> {
    Ok(match word {
        "real" => RbcScalar::Real,
        "integer" => RbcScalar::Integer,
        "boolean" => RbcScalar::Boolean,
        "string" => RbcScalar::String,
        "enumeration" => RbcScalar::Enumeration,
        "record" => RbcScalar::Record,
        other => return Err(TextError::at(line, format!("unknown scalar `{other}`"))),
    })
}

fn parse_role(word: &str, line: usize) -> Result<RbcRole, TextError> {
    Ok(match word {
        "parameter" => RbcRole::Parameter,
        "constant" => RbcRole::Constant,
        "input" => RbcRole::Input,
        "state" => RbcRole::State,
        "algebraic" => RbcRole::Algebraic,
        "output" => RbcRole::Output,
        "discrete_real" => RbcRole::DiscreteReal,
        "discrete_value" => RbcRole::DiscreteValue,
        other => return Err(TextError::at(line, format!("unknown role `{other}`"))),
    })
}

fn parse_causality(word: &str, line: usize) -> Result<RbcCausality, TextError> {
    Ok(match word {
        "input" => RbcCausality::Input,
        "output" => RbcCausality::Output,
        "parameter" => RbcCausality::Parameter,
        "calculated_parameter" => RbcCausality::CalculatedParameter,
        "independent" => RbcCausality::Independent,
        "local" => RbcCausality::Local,
        other => return Err(TextError::at(line, format!("unknown causality `{other}`"))),
    })
}

fn parse_quantity(word: &str, line: usize) -> Result<RbcQuantityKind, TextError> {
    Ok(match word {
        "potential" => RbcQuantityKind::Potential,
        "flow" => RbcQuantityKind::Flow,
        "stream" => RbcQuantityKind::Stream,
        other => {
            return Err(TextError::at(
                line,
                format!("unknown quantity kind `{other}`"),
            ));
        }
    })
}

fn parse_unary(word: &str, line: usize) -> Result<RbcUnaryOp, TextError> {
    Ok(match word {
        "negate" => RbcUnaryOp::Negate,
        "not" => RbcUnaryOp::Not,
        "plus" => RbcUnaryOp::Plus,
        other => return Err(TextError::at(line, format!("unknown unary op `{other}`"))),
    })
}

fn parse_binary(word: &str, line: usize) -> Result<RbcBinaryOp, TextError> {
    use RbcBinaryOp::*;
    Ok(match word {
        "add" => Add,
        "sub" => Subtract,
        "mul" => Multiply,
        "div" => Divide,
        "pow" => Power,
        "eq" => Equal,
        "ne" => NotEqual,
        "lt" => Less,
        "le" => LessEqual,
        "gt" => Greater,
        "ge" => GreaterEqual,
        "and" => And,
        "or" => Or,
        "eadd" => ElementwiseAdd,
        "esub" => ElementwiseSubtract,
        "emul" => ElementwiseMultiply,
        "ediv" => ElementwiseDivide,
        "epow" => ElementwisePower,
        other => return Err(TextError::at(line, format!("unknown binary op `{other}`"))),
    })
}

fn parse_real(word: &str, line: usize) -> Result<f64, TextError> {
    match word {
        "nan" => Ok(f64::NAN),
        "inf" => Ok(f64::INFINITY),
        "-inf" => Ok(f64::NEG_INFINITY),
        other => other
            .parse()
            .map_err(|_| TextError::at(line, format!("`{other}` is not a real"))),
    }
}

fn parse_subscripts(cursor: &mut Cursor<'_>) -> Result<Vec<RbcSubscript>, TextError> {
    let count: usize = cursor.number()?;
    let mut subscripts = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = cursor.word()?;
        subscripts.push(match kind {
            "at" => RbcSubscript::Index {
                expression: ExprId(cursor.id('^')?),
            },
            "all" => RbcSubscript::Whole,
            "slice" => RbcSubscript::Slice {
                expression: ExprId(cursor.id('^')?),
            },
            other => {
                return Err(TextError::at(
                    cursor.line,
                    format!("unknown subscript kind `{other}`"),
                ));
            }
        });
    }
    Ok(subscripts)
}

fn parse_coordinate(cursor: &mut Cursor<'_>) -> Result<RbcCoordinate, TextError> {
    use RbcCoordinate::*;
    let kind = cursor.word()?;
    if kind == "time" {
        return Ok(Time);
    }
    if kind == "binder" {
        let domain = DomainId(cursor.id('&')?);
        return Ok(Binder {
            domain,
            ordinal: cursor.number()?,
        });
    }
    if kind == "cond" {
        return Ok(Condition {
            condition: ConditionId(cursor.id('?')?),
        });
    }
    if kind == "clockint" {
        return Ok(ClockInterval {
            clock: ClockId(cursor.number()?),
        });
    }
    if kind == "delay" {
        return Ok(Delay {
            delay: DelayId(cursor.number()?),
        });
    }
    if kind == "previous" {
        return Ok(Previous {
            previous: PreviousId(cursor.number()?),
        });
    }
    if kind == "terminal" {
        return Ok(Terminal {
            terminal: TerminalId(cursor.number()?),
        });
    }
    if kind == "fnparam" {
        let function = FunctionId(cursor.id('~')?);
        return Ok(FunctionParameter {
            function,
            ordinal: cursor.number()?,
        });
    }
    let variable = VariableId(cursor.id('%')?);
    Ok(match kind {
        "param" => Parameter { variable },
        "input" => Input { variable },
        "state" => State { variable },
        "der" => Derivative { variable },
        "alg" => Algebraic { variable },
        "dreal" => DiscreteReal { variable },
        "dval" => DiscreteValue { variable },
        "pre_state" => PreState { variable },
        "pre_alg" => PreAlgebraic { variable },
        "pre_dreal" => PreDiscreteReal { variable },
        "pre_dval" => PreDiscreteValue { variable },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown coordinate `{other}`"),
            ));
        }
    })
}

fn parse_equation(cursor: &mut Cursor<'_>) -> Result<RbcEquation, TextError> {
    let id = EquationId(cursor.number()?);
    let residual = ExprId(cursor.id('^')?);
    let mut reads = Vec::new();
    let mut reads_derivative = Vec::new();
    let mut reads_previous = Vec::new();
    for (keyword, sink) in [
        ("reads", &mut reads),
        ("dreads", &mut reads_derivative),
        ("preads", &mut reads_previous),
    ] {
        if cursor.eat(keyword) {
            while cursor.peek().is_some_and(|t| t.starts_with('%')) {
                sink.push(VariableId(cursor.id('%')?));
            }
        }
    }
    Ok(RbcEquation {
        id,
        residual,
        provenance: cursor.provenance()?,
        reads,
        reads_derivative,
        reads_previous,
    })
}

/// A provenance for an item whose own line has not been read yet.
///
/// Every item that carries one overwrites this before it is stored; it exists
/// because a partially built record has to hold *something*, not because an
/// unknown origin is meaningful.
fn placeholder_provenance() -> RbcProvenance {
    RbcProvenance {
        origin: RbcOrigin::Source,
        span: RbcSpan {
            source: SourceId::PLACEHOLDER,
            start: 0,
            end: 0,
            line: 0,
            column: 0,
        },
    }
}

fn empty_model() -> RbcModel {
    RbcModel {
        connector_types: Vec::new(),
        connectors: Vec::new(),
        connection_sets: Vec::new(),
        functions: Vec::new(),
        discrete_real_equations: Vec::new(),
        initial_discrete_values: Vec::new(),
        initial_parameter_values: Vec::new(),
        name: String::new(),
        domains: Vec::new(),
        equation_families: Vec::new(),
        initial_equation_families: Vec::new(),
        sources: Vec::new(),
        types: Vec::new(),
        variables: Vec::new(),
        expressions: Vec::new(),
        equations: Vec::new(),
        initial_equations: Vec::new(),
        relations: Vec::new(),
        conditions: Vec::new(),
        roots: Vec::new(),
        events: Vec::new(),
        clocks: Vec::new(),
        clock_ownerships: Vec::new(),
        time_events: Vec::new(),
        connections: Vec::new(),
        components: Vec::new(),
        trace_points: Vec::new(),
        discrete_definitions: Vec::new(),
        model_event_transactions: Vec::new(),
        previous_values: Vec::new(),
        terminals: Vec::new(),
        structured_roots: Vec::new(),
        delays: Vec::new(),
        summary: RbcSummary::default(),
    }
}

/// The number after a declaration's sigil, e.g. the `3` of `%3`.
fn item_id(rest: &str, line: usize, message: &'static str) -> Result<u32, TextError> {
    rest.parse().map_err(|_| TextError::at(line, message))
}

/// `count` expression ids in a row.
fn parse_expr_ids(cursor: &mut Cursor<'_>, count: usize) -> Result<Vec<ExprId>, TextError> {
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(ExprId(cursor.id('^')?));
    }
    Ok(ids)
}

/// A run of variable ids, ending at the first token that is not one.
fn parse_variable_run(cursor: &mut Cursor<'_>) -> Result<Vec<VariableId>, TextError> {
    let mut variables = Vec::new();
    while cursor.peek().is_some_and(|t| t.starts_with('%')) {
        variables.push(VariableId(cursor.id('%')?));
    }
    Ok(variables)
}

/// An optional `keyword %a %b ...` clause; an absent clause reads as empty.
fn parse_variable_clause(
    cursor: &mut Cursor<'_>,
    keyword: &str,
) -> Result<Vec<VariableId>, TextError> {
    if cursor.eat(keyword) {
        parse_variable_run(cursor)
    } else {
        Ok(Vec::new())
    }
}

/// A comma-separated extent list, or `-` for none.
fn parse_extents(raw: &str, line: usize) -> Result<Vec<u32>, TextError> {
    if raw == "-" {
        return Ok(Vec::new());
    }
    raw.split(',')
        .map(|p| {
            p.parse()
                .map_err(|_| TextError::at(line, format!("bad extent `{p}`")))
        })
        .collect()
}

/// A record carried whole as one quoted JSON string.
fn parse_json<T: serde::de::DeserializeOwned>(cursor: &mut Cursor<'_>) -> Result<T, TextError> {
    serde_json::from_str(&cursor.string()?).map_err(|e| TextError::at(cursor.line, e.to_string()))
}

fn parse_source(id: SourceId, cursor: &mut Cursor<'_>) -> Result<RbcSource, TextError> {
    cursor.expect("source")?;
    let name = cursor.string()?;
    let text = if cursor.eat("text") {
        Some(cursor.string()?)
    } else {
        None
    };
    Ok(RbcSource { id, name, text })
}

fn parse_dimensions(cursor: &mut Cursor<'_>) -> Result<Vec<u32>, TextError> {
    let mut dimensions = Vec::new();
    if !cursor.eat("dims") {
        return Ok(dimensions);
    }
    let line = cursor.line;
    for part in cursor.word()?.split(',') {
        dimensions.push(
            part.parse()
                .map_err(|_| TextError::at(line, format!("bad dimension `{part}`")))?,
        );
    }
    Ok(dimensions)
}

fn parse_record(cursor: &mut Cursor<'_>) -> Result<Option<RbcRecord>, TextError> {
    if !cursor.eat("record") {
        return Ok(None);
    }
    let name = cursor.string()?;
    let count: usize = cursor.number()?;
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        fields.push(RbcRecordField {
            name: cursor.string()?,
            value_type: TypeId(cursor.id('$')?),
        });
    }
    Ok(Some(RbcRecord { name, fields }))
}

fn parse_type(id: TypeId, cursor: &mut Cursor<'_>) -> Result<RbcType, TextError> {
    cursor.expect("type")?;
    let scalar = parse_scalar(cursor.word()?, cursor.line)?;
    let dimensions = parse_dimensions(cursor)?;
    let record = parse_record(cursor)?;
    Ok(RbcType {
        id,
        scalar,
        dimensions,
        record,
    })
}

fn parse_function_body(cursor: &mut Cursor<'_>) -> Result<RbcFunctionBody, TextError> {
    cursor.expect("body")?;
    Ok(match cursor.word()? {
        "elided" => RbcFunctionBody::ElidedModelica,
        "external" => RbcFunctionBody::External {
            language: cursor.string()?,
            symbol: cursor.string()?,
            purity: RbcPurity::Impure,
            arguments: Vec::new(),
            result: None,
            linkage: RbcExternalLinkage::default(),
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown function body `{other}`"),
            ));
        }
    })
}

fn parse_function(id: FunctionId, cursor: &mut Cursor<'_>) -> Result<RbcFunction, TextError> {
    cursor.expect("fn")?;
    let name = cursor.string()?;
    cursor.expect("params")?;
    let count: usize = cursor.number()?;
    let mut parameters = Vec::with_capacity(count);
    for _ in 0..count {
        parameters.push(RbcFunctionParameter {
            name: cursor.string()?,
            value_type: TypeId(cursor.id('$')?),
            declaration: None,
        });
    }
    cursor.expect("results")?;
    let count: usize = cursor.number()?;
    let mut results = Vec::with_capacity(count);
    for _ in 0..count {
        results.push(TypeId(cursor.id('$')?));
    }
    let body = parse_function_body(cursor)?;
    let inline = if cursor.eat("inline") {
        RbcInline::Requested
    } else if cursor.eat("lateinline") {
        RbcInline::AfterIndexReduction
    } else if cursor.eat("noinline") {
        RbcInline::Never
    } else {
        RbcInline::Unstated
    };
    Ok(RbcFunction {
        // The text profile does not carry bodies, so it carries no
        // folds either; `emit-text` refuses an artifact whose body
        // it cannot represent rather than writing an empty one.
        folds: Vec::new(),
        values: Vec::new(),
        calls: Vec::new(),
        id,
        name,
        parameters,
        results,
        inline,
        derivatives: Vec::new(),
        body,
        declaration: cursor.provenance()?,
    })
}

fn parse_component(id: ComponentId, cursor: &mut Cursor<'_>) -> Result<RbcComponent, TextError> {
    cursor.expect("comp")?;
    let path = cursor.string()?;
    let class_name = if cursor.eat("of") {
        Some(cursor.string()?)
    } else {
        None
    };
    Ok(RbcComponent {
        id,
        path,
        class_name,
    })
}

/// Read one optional clause of a symbol contract, if the next token starts
/// one. Returns whether a clause was read.
fn parse_contract_clause(
    contract: &mut RbcSymbolContract,
    cursor: &mut Cursor<'_>,
) -> Result<bool, TextError> {
    match cursor.peek().map(|t| t.to_string()) {
        Some(t) if t == "final" => {
            cursor.word()?;
            contract.is_final = true
        }
        Some(t) if t == "protected" => {
            cursor.word()?;
            contract.is_protected = true
        }
        Some(t) if t == "evaluate" => {
            cursor.word()?;
            contract.evaluate = true
        }
        Some(t) if t == "structural" => {
            cursor.word()?;
            contract.structural = true
        }
        Some(t) if t == "frommod" => {
            cursor.word()?;
            contract.binding_from_modification = true
        }
        Some(t) if t == "value" => {
            cursor.word()?;
            let raw = cursor.word()?;
            contract.effective_value = Some(parse_real(raw, cursor.line)?)
        }
        Some(t) if t == "uses" => {
            cursor.word()?;
            contract
                .binding_depends_on
                .extend(parse_variable_run(cursor)?);
        }
        Some(t) if t == "declaredin" => {
            cursor.word()?;
            contract.declared_in = Some(cursor.string()?)
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn parse_contract(cursor: &mut Cursor<'_>) -> Result<RbcSymbolContract, TextError> {
    let variability = match cursor.word()? {
        "constant" => RbcVariability::Constant,
        "parameter" => RbcVariability::Parameter,
        "discrete" => RbcVariability::Discrete,
        "continuous" => RbcVariability::Continuous,
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown variability `{other}`"),
            ));
        }
    };
    let mut contract = RbcSymbolContract {
        variability,
        is_final: false,
        is_protected: false,
        evaluate: false,
        structural: false,
        effective_value: None,
        binding_depends_on: Vec::new(),
        binding_from_modification: false,
        declared_in: None,
    };
    while parse_contract_clause(&mut contract, cursor)? {}
    Ok(contract)
}

fn parse_variable_attribute(
    variable: &mut RbcVariable,
    keyword: &str,
    cursor: &mut Cursor<'_>,
) -> Result<(), TextError> {
    match keyword {
        "comp" => variable.component = Some(ComponentId(cursor.id('#')?)),
        "unit" => variable.unit = Some(cursor.string()?),
        "quantity" => variable.physical_quantity = Some(cursor.string()?),
        "class" => variable.declaring_class = Some(cursor.string()?),
        "desc" => variable.description = Some(cursor.string()?),
        "fixed" => variable.fixed = Some(cursor.word()? == "true"),
        "fixed_elements" => {
            variable.fixed_elements = Some(
                serde_json::from_str(&cursor.string()?)
                    .map_err(|error| TextError::at(cursor.line, error.to_string()))?,
            );
        }
        "evaluable" => variable.evaluable = true,
        "held" => variable.held = true,
        "state_select" => {
            variable.state_select = serde_json::from_str(&cursor.string()?)
                .map_err(|error| TextError::at(cursor.line, error.to_string()))?
        }
        "declared" => {
            variable.declared_causality = Some(match cursor.word()? {
                "input" => RbcDeclaredCausality::Input,
                "output" => RbcDeclaredCausality::Output,
                "none" => RbcDeclaredCausality::None,
                value => {
                    return Err(TextError::at(
                        cursor.line,
                        format!("invalid declared causality {value}"),
                    ));
                }
            })
        }
        "tunable" => variable.tunable = true,
        "discrete" => variable.discrete_input = true,
        "contract" => variable.contract = Some(parse_contract(cursor)?),
        "from_source" => variable.from_source = true,
        "start" => variable.start = Some(ExprId(cursor.id('^')?)),
        "min" => variable.min = Some(ExprId(cursor.id('^')?)),
        "max" => variable.max = Some(ExprId(cursor.id('^')?)),
        "nominal" => variable.nominal = Some(ExprId(cursor.id('^')?)),
        "binding" => variable.binding = Some(ExprId(cursor.id('^')?)),
        "connector" => {
            let quantity = parse_quantity(cursor.word()?, cursor.line)?;
            variable.connector = Some(RbcConnectorMember {
                quantity,
                connected: cursor.eat("connected"),
            });
        }
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown variable attribute `{other}`"),
            ));
        }
    }
    Ok(())
}

fn parse_variable(id: VariableId, cursor: &mut Cursor<'_>) -> Result<RbcVariable, TextError> {
    cursor.expect("var")?;
    let name = cursor.string()?;
    let value_type = TypeId(cursor.id('$')?);
    let role = parse_role(cursor.word()?, cursor.line)?;
    let causality = parse_causality(cursor.word()?, cursor.line)?;
    cursor.expect("scalars")?;
    let scalar_count = cursor.number()?;
    let mut variable = RbcVariable {
        id,
        name,
        role,
        causality,
        declared_causality: None,
        value_type,
        scalar_count,
        declaration: placeholder_provenance(),
        component: None,
        unit: None,
        description: None,
        fixed: None,
        fixed_elements: None,
        evaluable: false,
        held: false,
        state_select: RbcStateSelect::Default,
        start: None,
        min: None,
        max: None,
        nominal: None,
        binding: None,
        connector: None,
        tunable: false,
        from_source: false,
        physical_quantity: None,
        declaring_class: None,
        discrete_input: false,
        contract: None,
    };
    while cursor
        .peek()
        .is_some_and(|keyword| !keyword.starts_with('@'))
    {
        let keyword = cursor.word()?;
        parse_variable_attribute(&mut variable, keyword, cursor)?;
    }
    variable.declaration = cursor.provenance()?;
    Ok(variable)
}

fn parse_domain(id: DomainId, cursor: &mut Cursor<'_>) -> Result<RbcDomain, TextError> {
    cursor.expect("domain")?;
    cursor.expect("scalars")?;
    let scalar_count = cursor.number()?;
    cursor.expect("extents")?;
    let extents = parse_extents(cursor.word()?, cursor.line)?;
    let parent = cursor
        .eat("parent")
        .then(|| cursor.id('&'))
        .transpose()?
        .map(DomainId);
    cursor.expect("binders")?;
    let count: usize = cursor.number()?;
    let mut binders = Vec::with_capacity(count);
    for _ in 0..count {
        binders.push(RbcBinder {
            id: cursor.number()?,
            display_name: cursor.string()?,
            lower: cursor.number()?,
            upper: cursor.number()?,
            step: cursor.number()?,
        });
    }
    Ok(RbcDomain {
        id,
        binders,
        parent,
        extents,
        scalar_count,
        provenance: cursor.provenance()?,
    })
}

fn parse_literal(cursor: &mut Cursor<'_>) -> Result<RbcLiteral, TextError> {
    Ok(match cursor.word()? {
        "real" => RbcLiteral::Real {
            value: parse_real(cursor.word()?, cursor.line)?,
        },
        "integer" => RbcLiteral::Integer {
            value: cursor.number()?,
        },
        "enum" => RbcLiteral::Enumeration {
            ordinal: cursor.number()?,
        },
        "boolean" => RbcLiteral::Boolean {
            value: cursor.word()? == "true",
        },
        "string" => RbcLiteral::String {
            value: cursor.string()?,
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown literal kind `{other}`"),
            ));
        }
    })
}

fn parse_conditional(cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    let count: usize = cursor.number()?;
    let mut branches = Vec::with_capacity(count);
    for _ in 0..count {
        branches.push(RbcBranch {
            condition: ExprId(cursor.id('^')?),
            value: ExprId(cursor.id('^')?),
        });
    }
    cursor.expect("else")?;
    Ok(RbcExprNode::Conditional {
        branches,
        fallback: ExprId(cursor.id('^')?),
    })
}

fn parse_array(cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    let count: usize = cursor.number()?;
    let empty_type = cursor
        .eat("of")
        .then(|| cursor.id('$'))
        .transpose()?
        .map(TypeId);
    Ok(RbcExprNode::Array {
        elements: parse_expr_ids(cursor, count)?,
        empty_type,
    })
}

fn parse_range(cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    let start = ExprId(cursor.id('^')?);
    let step = if cursor.eat("step") {
        Some(ExprId(cursor.id('^')?))
    } else {
        cursor.expect("nostep")?;
        None
    };
    Ok(RbcExprNode::Range {
        start,
        step,
        stop: ExprId(cursor.id('^')?),
    })
}

fn parse_invoke(cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    let function = FunctionId(cursor.id('~')?);
    cursor.expect("out")?;
    let output = cursor.number()?;
    cursor.expect("owner")?;
    let owner = ExprId(cursor.id('^')?);
    let count: usize = cursor.number()?;
    Ok(RbcExprNode::Call {
        owner,
        function,
        output,
        arguments: parse_expr_ids(cursor, count)?,
    })
}

fn parse_function_value(cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    let function = FunctionId(cursor.id('~')?);
    let value = cursor.number()?;
    cursor.expect("def")?;
    Ok(RbcExprNode::FunctionValue {
        function,
        value,
        definition: cursor.number()?,
    })
}

fn parse_fold_value(cursor: &mut Cursor<'_>, output: bool) -> Result<RbcExprNode, TextError> {
    let function = FunctionId(cursor.id('~')?);
    let fold = cursor.number()?;
    let carried = cursor.number()?;
    cursor.expect("def")?;
    let definition = cursor.number()?;
    Ok(if output {
        RbcExprNode::FunctionFoldOutput {
            function,
            fold,
            carried,
            definition,
        }
    } else {
        RbcExprNode::FunctionFoldParameter {
            function,
            fold,
            carried,
            definition,
        }
    })
}

fn parse_clock_transfer(cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    let kind = match cursor.word()? {
        "sub" => RbcClockTransferKind::SubSample {
            factor: cursor.number()?,
        },
        "super" => RbcClockTransferKind::SuperSample {
            factor: cursor.number()?,
        },
        "shift" => RbcClockTransferKind::ShiftSample {
            counter: cursor.number()?,
            resolution: cursor.number()?,
        },
        "back" => RbcClockTransferKind::BackSample {
            counter: cursor.number()?,
            resolution: cursor.number()?,
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown clock transfer `{other}`"),
            ));
        }
    };
    let source = ExprId(cursor.id('^')?);
    cursor.expect("clocks")?;
    Ok(RbcExprNode::ClockTransfer {
        transfer: kind,
        source,
        source_clock: ClockId(cursor.number()?),
        target_clock: ClockId(cursor.number()?),
    })
}

fn parse_expr_node(kind: &str, cursor: &mut Cursor<'_>) -> Result<RbcExprNode, TextError> {
    Ok(match kind {
        "lit" => RbcExprNode::Literal {
            value: parse_literal(cursor)?,
        },
        "string_conversion" => RbcExprNode::StringConversion {
            value: ExprId(cursor.id('^')?),
            format: parse_json(cursor)?,
        },
        "coord" => RbcExprNode::Coordinate {
            coordinate: parse_coordinate(cursor)?,
        },
        "un" => RbcExprNode::Unary {
            op: parse_unary(cursor.word()?, cursor.line)?,
            operand: ExprId(cursor.id('^')?),
        },
        "bin" => RbcExprNode::Binary {
            op: parse_binary(cursor.word()?, cursor.line)?,
            lhs: ExprId(cursor.id('^')?),
            rhs: ExprId(cursor.id('^')?),
        },
        "cond" => parse_conditional(cursor)?,
        "call" => {
            let name = cursor.string()?;
            let count: usize = cursor.number()?;
            RbcExprNode::Builtin {
                name,
                arguments: parse_expr_ids(cursor, count)?,
            }
        }
        "array" => parse_array(cursor)?,
        "record" => {
            let ty = TypeId(cursor.id('$')?);
            let count: usize = cursor.number()?;
            RbcExprNode::Record {
                ty,
                fields: parse_expr_ids(cursor, count)?,
            }
        }
        "field" => RbcExprNode::Field {
            base: ExprId(cursor.id('^')?),
            field: cursor.number()?,
        },
        "range" => parse_range(cursor)?,
        "comp" => RbcExprNode::Comprehension {
            domain: DomainId(cursor.id('&')?),
            body: ExprId(cursor.id('^')?),
        },
        "index" => {
            let base = ExprId(cursor.id('^')?);
            RbcExprNode::Index {
                base,
                subscripts: parse_subscripts(cursor)?,
            }
        }
        "update" => {
            let base = ExprId(cursor.id('^')?);
            let value = ExprId(cursor.id('^')?);
            RbcExprNode::ArrayUpdate {
                base,
                value,
                subscripts: parse_subscripts(cursor)?,
            }
        }
        "invoke" => parse_invoke(cursor)?,
        "fnvalue" => parse_function_value(cursor)?,
        "foldparam" | "foldout" => parse_fold_value(cursor, kind == "foldout")?,
        "ctransfer" => parse_clock_transfer(cursor)?,
        "unsupported" => RbcExprNode::Unsupported {
            detail: cursor.string()?,
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown expression kind `{other}`"),
            ));
        }
    })
}

fn parse_expr(id: ExprId, cursor: &mut Cursor<'_>) -> Result<RbcExpr, TextError> {
    cursor.expect("expr")?;
    let value_type = TypeId(cursor.id('$')?);
    let kind = cursor.word()?;
    let node = parse_expr_node(kind, cursor)?;
    Ok(RbcExpr {
        id,
        value_type,
        node,
        provenance: cursor.provenance()?,
    })
}

/// Read a sigil-led line, which declares a numbered item, into `model`.
/// Returns `false`, having read nothing, when `head` carries no sigil.
fn parse_numbered_item(
    model: &mut RbcModel,
    head: &str,
    cursor: &mut Cursor<'_>,
) -> Result<bool, TextError> {
    let line = cursor.line;
    let Some(sigil) = head.chars().next() else {
        return Ok(false);
    };
    let rest = &head[sigil.len_utf8()..];
    match sigil {
        '!' => {
            let id = SourceId(item_id(rest, line, "bad source id")?);
            model.sources.push(parse_source(id, cursor)?);
        }
        '$' => {
            let id = TypeId(item_id(rest, line, "bad type id")?);
            model.types.push(parse_type(id, cursor)?);
        }
        '~' => {
            let id = FunctionId(item_id(rest, line, "bad function id")?);
            model.functions.push(parse_function(id, cursor)?);
        }
        '#' => {
            let id = ComponentId(item_id(rest, line, "bad component id")?);
            model.components.push(parse_component(id, cursor)?);
        }
        '%' => {
            let id = VariableId(item_id(rest, line, "bad variable id")?);
            model.variables.push(parse_variable(id, cursor)?);
        }
        '&' => {
            let id = DomainId(item_id(rest, line, "bad domain id")?);
            model.domains.push(parse_domain(id, cursor)?);
        }
        '^' => {
            let id = ExprId(item_id(rest, line, "bad expression id")?);
            model.expressions.push(parse_expr(id, cursor)?);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn parse_scalar_view(cursor: &mut Cursor<'_>) -> Result<RbcScalarView, TextError> {
    cursor.expect("view")?;
    Ok(match cursor.word()? {
        "binder" => RbcScalarView::BinderSubstitution,
        "rowmajor" => RbcScalarView::RowMajorProjection,
        "prefix" => RbcScalarView::BinderPrefixProjection {
            binder_count: cursor.number()?,
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown scalar view `{other}`"),
            ));
        }
    })
}

fn parse_family(cursor: &mut Cursor<'_>) -> Result<RbcEquationFamily, TextError> {
    let id = FamilyId(cursor.number()?);
    cursor.expect("domain")?;
    let domain = DomainId(cursor.id('&')?);
    cursor.expect("rows")?;
    let scalar_rows = cursor.number()?;
    cursor.expect("extents")?;
    let extents = parse_extents(cursor.word()?, cursor.line)?;
    let scalar_view = parse_scalar_view(cursor)?;
    cursor.expect("bodies")?;
    let count: usize = cursor.number()?;
    let bodies = parse_expr_ids(cursor, count)?;
    let reads = parse_variable_clause(cursor, "reads")?;
    let reads_derivative = parse_variable_clause(cursor, "dreads")?;
    let reads_previous = parse_variable_clause(cursor, "preads")?;
    Ok(RbcEquationFamily {
        id,
        domain,
        bodies,
        scalar_rows,
        extents,
        scalar_view,
        reads,
        reads_derivative,
        reads_previous,
        provenance: cursor.provenance()?,
    })
}

fn parse_discrete_real_equation(
    cursor: &mut Cursor<'_>,
) -> Result<RbcDiscreteRealEquation, TextError> {
    let id = EquationId(cursor.number()?);
    let residual = ExprId(cursor.id('^')?);
    let activation = if cursor.eat("always") {
        RbcDiscreteRealActivation::Always
    } else {
        cursor.expect("when")?;
        let trigger = ConditionId(cursor.id('?')?);
        cursor.expect("guard")?;
        RbcDiscreteRealActivation::When {
            trigger,
            guard: ConditionId(cursor.id('?')?),
        }
    };
    let reads = parse_variable_clause(cursor, "reads")?;
    let reads_derivative = parse_variable_clause(cursor, "dreads")?;
    let reads_previous = parse_variable_clause(cursor, "preads")?;
    Ok(RbcDiscreteRealEquation {
        id,
        residual,
        activation,
        reads,
        reads_derivative,
        reads_previous,
        provenance: cursor.provenance()?,
    })
}

fn parse_condition(cursor: &mut Cursor<'_>) -> Result<RbcCondition, TextError> {
    let id = ConditionId(cursor.number()?);
    let kind = cursor.word()?;
    use RbcConditionNode::*;
    let node = match kind {
        "initial" => Initial,
        "always" => Always,
        "clock_activation" => ClockActivation {
            clock: ClockId(cursor.number()?),
        },
        "rel" => Relation {
            relation: RelationId(cursor.number()?),
        },
        "discrete" => Discrete {
            expression: ExprId(cursor.id('^')?),
        },
        "not" => Not {
            operand: ConditionId(cursor.number()?),
        },
        "and" => And {
            lhs: ConditionId(cursor.number()?),
            rhs: ConditionId(cursor.number()?),
        },
        "or" => Or {
            lhs: ConditionId(cursor.number()?),
            rhs: ConditionId(cursor.number()?),
        },
        "any_rise" => AnyRise {
            lhs: ConditionId(cursor.number()?),
            rhs: ConditionId(cursor.number()?),
        },
        "unsupported" => Unsupported {
            detail: cursor.string()?,
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown condition `{other}`"),
            ));
        }
    };
    Ok(RbcCondition {
        id,
        node,
        provenance: cursor.provenance()?,
    })
}

fn parse_root(cursor: &mut Cursor<'_>) -> Result<RbcRoot, TextError> {
    let id = RootId(cursor.number()?);
    cursor.expect("rel")?;
    let relation = RelationId(cursor.number()?);
    cursor.expect("act")?;
    let activation = ConditionId(cursor.number()?);
    Ok(RbcRoot {
        id,
        relation,
        activation,
        provenance: cursor.provenance()?,
    })
}

fn parse_action(cursor: &mut Cursor<'_>) -> Result<RbcAction, TextError> {
    Ok(match cursor.word()? {
        "reinit" => RbcAction::Reinitialize {
            state: VariableId(cursor.id('%')?),
            value: ExprId(cursor.id('^')?),
        },
        "assert" => {
            let message = ExprId(cursor.id('^')?);
            let level = if cursor.eat("level") {
                Some(ExprId(cursor.id('^')?))
            } else {
                None
            };
            RbcAction::Assert { message, level }
        }
        "warning" => RbcAction::Warning {
            condition: ExprId(cursor.id('^')?),
            message: ExprId(cursor.id('^')?),
        },
        "terminate" => RbcAction::Terminate {
            message: ExprId(cursor.id('^')?),
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown action `{other}`"),
            ));
        }
    })
}

fn parse_event(cursor: &mut Cursor<'_>) -> Result<RbcEventAction, TextError> {
    let id = EventId(cursor.number()?);
    cursor.expect("trig")?;
    let trigger = ConditionId(cursor.number()?);
    cursor.expect("guard")?;
    let guard = ConditionId(cursor.number()?);
    let action = parse_action(cursor)?;
    Ok(RbcEventAction {
        id,
        trigger,
        guard,
        action,
        provenance: cursor.provenance()?,
    })
}

fn parse_time_event(cursor: &mut Cursor<'_>) -> Result<RbcTimeEvent, TextError> {
    let id = EventId(cursor.number()?);
    let schedule = match cursor.word()? {
        "static" => RbcSchedule::Static {
            numerator: cursor.number()?,
            denominator: cursor.number()?,
        },
        "dynamic" => RbcSchedule::Dynamic {
            deadline: ExprId(cursor.id('^')?),
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown schedule `{other}`"),
            ));
        }
    };
    Ok(RbcTimeEvent {
        id,
        schedule,
        provenance: cursor.provenance()?,
    })
}

/// An optional `eq <id>` clause naming the equation an item produced.
fn parse_optional_equation(cursor: &mut Cursor<'_>) -> Result<Option<EquationId>, TextError> {
    if cursor.eat("eq") {
        Ok(Some(EquationId(cursor.number()?)))
    } else {
        Ok(None)
    }
}

fn parse_connection(cursor: &mut Cursor<'_>) -> Result<RbcConnection, TextError> {
    let id = ConnectionId(cursor.number()?);
    let left = VariableId(cursor.id('%')?);
    let right = VariableId(cursor.id('%')?);
    let quantity = parse_quantity(cursor.word()?, cursor.line)?;
    let left_connector = cursor.string()?;
    let right_connector = cursor.string()?;
    let equation = parse_optional_equation(cursor)?;
    Ok(RbcConnection {
        id,
        left,
        right,
        quantity,
        left_connector,
        right_connector,
        equation,
        provenance: cursor.provenance()?,
    })
}

fn parse_flow_term(token: &str, line: usize) -> Result<RbcFlowTerm, TextError> {
    let negated = token.starts_with('-');
    let digits = token.trim_start_matches(['+', '-']).trim_start_matches('%');
    Ok(RbcFlowTerm {
        variable: VariableId(
            digits
                .parse()
                .map_err(|_| TextError::at(line, "bad flow member"))?,
        ),
        negated,
    })
}

fn parse_flow_balance(cursor: &mut Cursor<'_>) -> Result<RbcFlowBalance, TextError> {
    let equation = parse_optional_equation(cursor)?;
    let mut terms = Vec::new();
    while !cursor.eat("end") {
        let token = cursor.word()?;
        terms.push(parse_flow_term(token, cursor.line)?);
    }
    Ok(RbcFlowBalance { equation, terms })
}

fn parse_connection_set(cursor: &mut Cursor<'_>) -> Result<RbcConnectionSet, TextError> {
    let id = ConnectionSetId(cursor.number()?);
    let mut connectors = Vec::new();
    let mut potentials = Vec::new();
    let mut balances: Vec<RbcFlowBalance> = Vec::new();
    let mut potential_equations = Vec::new();
    let mut unconnected = false;
    // Clauses first, provenance last: it is the one token that
    // starts with `@`, so the loop stops there rather than
    // needing to put anything back.
    while !cursor.peek().is_some_and(|word| word.starts_with('@')) {
        match cursor.word()? {
            "at" => connectors.push(cursor.string()?),
            "pot" => potentials.push(VariableId(cursor.id('%')?)),
            "balance" => balances.push(parse_flow_balance(cursor)?),
            "poteq" => potential_equations.push(EquationId(cursor.number()?)),
            "unconnected" => unconnected = true,
            other => {
                return Err(TextError::at(
                    cursor.line,
                    format!("unknown connset clause `{other}`"),
                ));
            }
        }
    }
    Ok(RbcConnectionSet {
        id,
        connectors,
        potentials,
        balances,
        potential_equations,
        unconnected,
        provenance: cursor.provenance()?,
    })
}

/// The `disc` line that opens a discrete definition; its branches and its
/// provenance arrive on later lines.
fn parse_discrete_header(cursor: &mut Cursor<'_>) -> Result<RbcDiscreteDefinition, TextError> {
    let count: usize = cursor.number()?;
    cursor.expect("targets")?;
    let mut targets = Vec::with_capacity(count);
    for _ in 0..count {
        targets.push(VariableId(cursor.id('%')?));
    }
    Ok(RbcDiscreteDefinition {
        observed: cursor.eat("observed"),
        targets,
        branches: Vec::new(),
        provenance: placeholder_provenance(),
    })
}

fn parse_discrete_branch(cursor: &mut Cursor<'_>) -> Result<RbcDiscreteBranch, TextError> {
    let activation = match cursor.word()? {
        "always" => RbcDiscreteActivation::Always,
        "when" => RbcDiscreteActivation::When {
            trigger: ConditionId(cursor.number()?),
            guard: ConditionId(cursor.number()?),
        },
        other => {
            return Err(TextError::at(
                cursor.line,
                format!("unknown activation `{other}`"),
            ));
        }
    };
    cursor.expect("values")?;
    let count: usize = cursor.number()?;
    Ok(RbcDiscreteBranch {
        activation,
        values: parse_expr_ids(cursor, count)?,
        provenance: cursor.provenance()?,
    })
}

fn parse_trace_point(cursor: &mut Cursor<'_>) -> Result<RbcTracePoint, TextError> {
    let id = TracePointId(cursor.number()?);
    let variable = VariableId(cursor.id('%')?);
    let label = cursor.string()?;
    let mut point = RbcTracePoint {
        id,
        variable,
        label,
        connection: None,
        connection_set: None,
        quantity: None,
        unit: None,
        added_by: None,
    };
    while !cursor.done() {
        match cursor.word()? {
            "conn" => point.connection = Some(ConnectionId(cursor.number()?)),
            "connset" => point.connection_set = Some(ConnectionSetId(cursor.number()?)),
            "kind" => point.quantity = Some(parse_quantity(cursor.word()?, cursor.line)?),
            "unit" => point.unit = Some(cursor.string()?),
            "added_by" => point.added_by = Some(cursor.string()?),
            other => {
                return Err(TextError::at(
                    cursor.line,
                    format!("unknown trace attribute `{other}`"),
                ));
            }
        }
    }
    Ok(point)
}

/// Read a keyword-led line into `file`. `open_discrete` carries a discrete
/// definition between its `disc`, `branch` and `end` lines.
fn parse_statement(
    file: &mut RbcFile,
    open_discrete: &mut Option<RbcDiscreteDefinition>,
    head: &str,
    cursor: &mut Cursor<'_>,
) -> Result<(), TextError> {
    let line = cursor.line;
    let model = &mut file.model;
    match head {
        "rbc" => file.bitcode_version = cursor.number()?,
        "producer" => file.producer = cursor.string()?,
        "model" => model.name = cursor.string()?,
        "eq" => model.equations.push(parse_equation(cursor)?),
        "ieq" => model.initial_equations.push(parse_equation(cursor)?),
        "family" => model.equation_families.push(parse_family(cursor)?),
        "ifamily" => model.initial_equation_families.push(parse_family(cursor)?),
        "dreq" => model
            .discrete_real_equations
            .push(parse_discrete_real_equation(cursor)?),
        "ipval" => model
            .initial_parameter_values
            .push(RbcInitialDiscreteValue {
                target: VariableId(cursor.id('%')?),
                value: ExprId(cursor.id('^')?),
                provenance: cursor.provenance()?,
            }),
        "idval" => model.initial_discrete_values.push(RbcInitialDiscreteValue {
            target: VariableId(cursor.id('%')?),
            value: ExprId(cursor.id('^')?),
            provenance: cursor.provenance()?,
        }),
        "function_record" => model.functions.push(parse_json(cursor)?),
        "model_event_transaction" => model.model_event_transactions.push(parse_json(cursor)?),
        "previous_value" => model.previous_values.push(parse_json(cursor)?),
        "terminal_record" => model.terminals.push(parse_json(cursor)?),
        "structured_root" => model.structured_roots.push(parse_json(cursor)?),
        "delay_record" => model.delays.push(parse_json(cursor)?),
        "clock_record" => model.clocks.push(parse_json(cursor)?),
        "clock_ownership" => model.clock_ownerships.push(parse_json(cursor)?),
        "rel" => model.relations.push(RbcRelation {
            id: RelationId(cursor.number()?),
            expression: ExprId(cursor.id('^')?),
            provenance: cursor.provenance()?,
        }),
        "cond" => model.conditions.push(parse_condition(cursor)?),
        "root" => model.roots.push(parse_root(cursor)?),
        "event" => model.events.push(parse_event(cursor)?),
        "tevent" => model.time_events.push(parse_time_event(cursor)?),
        "conn" => model.connections.push(parse_connection(cursor)?),
        "connset" => model.connection_sets.push(parse_connection_set(cursor)?),
        "disc" => *open_discrete = Some(parse_discrete_header(cursor)?),
        "branch" => {
            let definition = open_discrete
                .as_mut()
                .ok_or_else(|| TextError::at(line, "`branch` outside a `disc` block"))?;
            definition.branches.push(parse_discrete_branch(cursor)?);
        }
        "end" => {
            let mut definition = open_discrete
                .take()
                .ok_or_else(|| TextError::at(line, "`end` outside a `disc` block"))?;
            definition.provenance = cursor.provenance()?;
            model.discrete_definitions.push(definition);
        }
        "trace" => model.trace_points.push(parse_trace_point(cursor)?),
        other => {
            return Err(TextError::at(line, format!("unknown statement `{other}`")));
        }
    }
    Ok(())
}

/// Read the textual IR back into an artifact.
pub fn parse_text(text: &str) -> Result<RbcFile, TextError> {
    let mut file = RbcFile {
        execution: None,
        magic: RBC_MAGIC.to_string(),
        bitcode_version: RBC_VERSION,
        producer: String::new(),
        model: empty_model(),
    };
    // A discrete definition spans several lines, so it is assembled across
    // iterations and closed by its `end` line.
    let mut open_discrete: Option<RbcDiscreteDefinition> = None;

    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let tokens = tokenize(raw, number)?;
        if tokens.is_empty() {
            continue;
        }
        let mut cursor = Cursor::new(&tokens, number);
        let head = cursor.next_raw()?;

        // Sigil-led lines declare a numbered item; keyword-led lines do not.
        if !parse_numbered_item(&mut file.model, head, &mut cursor)? {
            parse_statement(&mut file, &mut open_discrete, head, &mut cursor)?;
        }
    }

    if open_discrete.is_some() {
        return Err(TextError::at(
            text.lines().count(),
            "a `disc` block was never closed by `end`",
        ));
    }

    // The summary is derived, never written: two sources of truth for a count
    // is one source of truth and one way to be wrong.
    crate::validate::recompute_summary(&mut file.model);
    Ok(file)
}

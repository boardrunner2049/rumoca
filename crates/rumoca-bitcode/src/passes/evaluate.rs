//! A bounded evaluator for calls whose arguments are literals.
//!
//! Total by construction, for the same reasons the artifact is (SPEC §9a):
//! the arena is a DAG, a loop is a fold over a domain whose trip count is
//! fixed, and the call graph over carried bodies is acyclic. It needs no
//! step budget, and it has none.
//!
//! It fails closed. Anything it does not model -- records, strings, an
//! external body, a model variable, a built-in outside the list below, an
//! integer overflow, a non-finite result -- makes the whole evaluation
//! `None`, and the caller leaves the call alone. A fold that is skipped costs
//! a little runtime; a fold that is wrong changes the model.

use std::collections::HashMap;

use crate::schema::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Real(f64),
    Integer(i64),
    Boolean(bool),
    /// Row-major elements and extents.
    Array(Vec<usize>, Vec<Value>),
}

impl Value {
    fn real(&self) -> Option<f64> {
        match self {
            Self::Real(v) => Some(*v),
            Self::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    fn integer(&self) -> Option<i64> {
        match self {
            Self::Integer(v) => Some(*v),
            _ => None,
        }
    }

    fn boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean(v) => Some(*v),
            _ => None,
        }
    }

    fn finite(self) -> Option<Self> {
        match &self {
            Self::Real(v) if !v.is_finite() => None,
            _ => Some(self),
        }
    }

    fn elements(&self) -> Option<&[Value]> {
        match self {
            Self::Array(_, elements) => Some(elements),
            _ => None,
        }
    }
}

/// Evaluate every output of `function` applied to `arguments`.
pub(crate) fn call(
    model: &RbcModel,
    function: FunctionId,
    arguments: &[Value],
) -> Option<Vec<Value>> {
    let function = model.functions.get(function.0 as usize)?;
    let RbcFunctionBody::Modelica { statements } = &function.body else {
        return None;
    };
    if arguments.len() != function.parameters.len() {
        return None;
    }
    let mut frame = Frame {
        model,
        function,
        arguments,
        definitions: HashMap::new(),
        current: vec![None; function.values.len()],
        next_definition: 0,
        binders: HashMap::new(),
    };
    frame.run(statements)?;
    function
        .values
        .iter()
        .enumerate()
        .filter(|(_, value)| value.role == RbcFunctionValueRole::Output)
        .map(|(ordinal, _)| {
            let definition = frame.current[ordinal]?;
            frame.definitions.get(&definition).cloned()
        })
        .collect()
}

/// Evaluate a model-scope expression that reads no model variable.
pub(crate) fn expression(model: &RbcModel, id: ExprId) -> Option<Value> {
    let mut scope = Scope {
        model,
        frame: None,
        parameters: None,
        cache: HashMap::new(),
    };
    scope.eval(id)
}

/// Evaluate a model-scope expression whose only model reads are parameters
/// and constants with a value in `parameters`.
pub(crate) fn expression_with_parameters(
    model: &RbcModel,
    id: ExprId,
    parameters: &HashMap<VariableId, Value>,
) -> Option<Value> {
    let mut scope = Scope {
        model,
        frame: None,
        parameters: Some(parameters),
        cache: HashMap::new(),
    };
    scope.eval(id)
}

/// One activation of a function body.
struct Frame<'m> {
    model: &'m RbcModel,
    function: &'m RbcFunction,
    arguments: &'m [Value],
    /// Values by definition ordinal. A definition inside a loop body is
    /// overwritten each iteration, which is exactly its meaning.
    definitions: HashMap<u32, Value>,
    /// Current definition of each value, as construction tracked it.
    current: Vec<Option<u32>>,
    /// Definitions are numbered in the order the body issues them; the
    /// counter replays that order so a read names the same ordinal here as
    /// in the DAE.
    next_definition: u32,
    binders: HashMap<(u32, u32), i64>,
}

impl Frame<'_> {
    fn define(&mut self, value: u32, result: Value) -> Option<()> {
        let definition = self.next_definition;
        self.next_definition += 1;
        self.definitions.insert(definition, result);
        *self.current.get_mut(value as usize)? = Some(definition);
        Some(())
    }

    fn eval(&mut self, id: ExprId) -> Option<Value> {
        // A fresh cache per evaluation: inside a loop the same node takes a
        // different value each iteration.
        let mut scope = Scope {
            model: self.model,
            frame: Some(self),
            parameters: None,
            cache: HashMap::new(),
        };
        scope.eval(id)
    }

    fn run(&mut self, statements: &[RbcFunctionStatement]) -> Option<()> {
        for statement in statements {
            match statement {
                RbcFunctionStatement::Assignment {
                    value, expression, ..
                } => {
                    let result = self.eval(*expression)?;
                    self.define(*value, result)?;
                }
                // A failing assertion is the runtime's to report.
                RbcFunctionStatement::Assertion { condition, .. } => {
                    self.eval(*condition)?.boolean()?.then_some(())?;
                }
                RbcFunctionStatement::AssignmentGroup {
                    values,
                    expressions,
                    ..
                } => self.group(values, expressions)?,
                RbcFunctionStatement::For {
                    fold, statements, ..
                } => self.fold(*fold, statements)?,
            }
        }
        Some(())
    }

    /// Each expression is already the join for its value, so evaluating it
    /// selects the branch; the correlation only records that the joins share
    /// one selection.
    fn group(&mut self, values: &[u32], expressions: &[ExprId]) -> Option<()> {
        let results = expressions
            .iter()
            .map(|expression| self.eval(*expression))
            .collect::<Option<Vec<_>>>()?;
        for (value, result) in values.iter().zip(results) {
            self.define(*value, result)?;
        }
        Some(())
    }

    fn fold(&mut self, ordinal: u32, body: &[RbcFunctionStatement]) -> Option<()> {
        let fold = self.function.folds.get(ordinal as usize)?;
        let domain = self.model.domains.get(fold.domain.0 as usize)?;
        let initial = fold
            .targets
            .iter()
            .map(|target| {
                let definition = self.current.get(*target as usize).copied().flatten()?;
                self.definitions.get(&definition).cloned()
            })
            .collect::<Option<Vec<_>>>()?;
        for local in &fold.iteration_locals {
            *self.current.get_mut(*local as usize)? = None;
        }
        // Parameter definitions are issued once, at loop entry; each
        // iteration rebinds them to the carried tuple.
        let parameters = (0..fold.targets.len() as u32)
            .map(|k| self.next_definition + k)
            .collect::<Vec<_>>();
        self.next_definition += fold.targets.len() as u32;
        let body_start = self.next_definition;
        let mut carried = initial;
        let mut body_end = body_start;
        for point in iterate(domain)? {
            for (index, value) in point.iter().enumerate() {
                self.binders.insert((fold.domain.0, index as u32), *value);
            }
            for ((definition, target), value) in parameters.iter().zip(&fold.targets).zip(&carried)
            {
                self.definitions.insert(*definition, value.clone());
                *self.current.get_mut(*target as usize)? = Some(*definition);
            }
            self.next_definition = body_start;
            self.run(body)?;
            body_end = self.next_definition;
            carried = fold
                .targets
                .iter()
                .map(|target| {
                    let definition = self.current.get(*target as usize).copied().flatten()?;
                    self.definitions.get(&definition).cloned()
                })
                .collect::<Option<Vec<_>>>()?;
        }
        // An empty domain runs no iteration, but the body's definitions
        // were still issued once at construction.
        if body_end == body_start {
            body_end = body_start + count_definitions(self.function, body)?;
        }
        self.next_definition = body_end;
        for (target, value) in fold.targets.iter().zip(carried) {
            self.define(*target, value)?;
        }
        Some(())
    }
}

/// Definitions a statement list issues, as construction numbers them.
fn count_definitions(function: &RbcFunction, statements: &[RbcFunctionStatement]) -> Option<u32> {
    let mut count = 0u32;
    for statement in statements {
        count += match statement {
            RbcFunctionStatement::Assignment { .. } => 1,
            RbcFunctionStatement::Assertion { .. } => 0,
            RbcFunctionStatement::AssignmentGroup { values, .. } => values.len() as u32,
            RbcFunctionStatement::For {
                fold, statements, ..
            } => {
                let targets = function.folds.get(*fold as usize)?.targets.len() as u32;
                2 * targets + count_definitions(function, statements)?
            }
        };
    }
    Some(count)
}

/// Every binder tuple of a domain, first binder outermost.
fn iterate(domain: &RbcDomain) -> Option<Vec<Vec<i64>>> {
    let mut points = vec![Vec::new()];
    for binder in &domain.binders {
        if binder.step == 0 {
            return None;
        }
        let mut values = Vec::new();
        let mut value = binder.lower;
        while (binder.step > 0 && value <= binder.upper)
            || (binder.step < 0 && value >= binder.upper)
        {
            values.push(value);
            value = value.checked_add(binder.step)?;
        }
        points = points
            .into_iter()
            .flat_map(|point| {
                values.iter().map(move |value| {
                    let mut next = point.clone();
                    next.push(*value);
                    next
                })
            })
            .collect();
    }
    Some(points)
}

struct Scope<'a, 'm> {
    model: &'m RbcModel,
    frame: Option<&'a mut Frame<'m>>,
    /// Values of the model's parameters and constants, for a model-scope
    /// evaluation that may read them.
    parameters: Option<&'a HashMap<VariableId, Value>>,
    cache: HashMap<u32, Value>,
}

impl Scope<'_, '_> {
    fn eval(&mut self, id: ExprId) -> Option<Value> {
        if let Some(value) = self.cache.get(&id.0) {
            return Some(value.clone());
        }
        let node = &self.model.expressions.get(id.0 as usize)?.node;
        let value = self.node(id, node)?.finite()?;
        self.cache.insert(id.0, value.clone());
        Some(value)
    }

    fn node(&mut self, id: ExprId, node: &RbcExprNode) -> Option<Value> {
        Some(match node {
            RbcExprNode::Literal { value } => match value {
                RbcLiteral::Real { value } => Value::Real(*value),
                RbcLiteral::Integer { value } => Value::Integer(*value),
                RbcLiteral::Boolean { value } => Value::Boolean(*value),
                _ => return None,
            },
            RbcExprNode::Coordinate { coordinate } => self.coordinate(*coordinate)?,
            RbcExprNode::FunctionValue { definition, .. }
            | RbcExprNode::FunctionFoldParameter { definition, .. }
            | RbcExprNode::FunctionFoldOutput { definition, .. } => {
                self.frame.as_ref()?.definitions.get(definition)?.clone()
            }
            RbcExprNode::Unary { op, operand } => unary(*op, self.eval(*operand)?)?,
            RbcExprNode::Binary { op, lhs, rhs } => {
                let lhs = self.eval(*lhs)?;
                let rhs = self.eval(*rhs)?;
                binary(*op, &lhs, &rhs)?
            }
            RbcExprNode::Conditional { branches, fallback } => {
                let taken = self.select(branches, *fallback)?;
                self.eval(taken)?
            }
            RbcExprNode::Array { elements, .. } => {
                let values = elements
                    .iter()
                    .map(|element| self.eval(*element))
                    .collect::<Option<Vec<_>>>()?;
                array_of(values)?
            }
            RbcExprNode::Index { base, subscripts } => {
                let base = self.eval(*base)?;
                let subscripts = subscripts
                    .iter()
                    .map(|subscript| match subscript {
                        RbcSubscript::Index { expression } => {
                            Some(Some(self.eval(*expression)?.integer()?))
                        }
                        RbcSubscript::Whole => Some(None),
                        RbcSubscript::Slice { .. } => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                index(&base, &subscripts)?
            }
            RbcExprNode::Builtin { name, arguments } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.eval(*argument))
                    .collect::<Option<Vec<_>>>()?;
                builtin(name, &arguments)?
            }
            RbcExprNode::Call {
                owner,
                function,
                output,
                arguments,
            } => {
                // A further result of a call shares its head's arguments.
                let arguments = if *owner == id {
                    arguments.clone()
                } else {
                    match &self.model.expressions.get(owner.0 as usize)?.node {
                        RbcExprNode::Call { arguments, .. } => arguments.clone(),
                        _ => return None,
                    }
                };
                let arguments = arguments
                    .iter()
                    .map(|argument| self.eval(*argument))
                    .collect::<Option<Vec<_>>>()?;
                call(self.model, *function, &arguments)?
                    .into_iter()
                    .nth(*output as usize)?
            }
            _ => return None,
        })
    }

    /// The branch an `if` expression takes: the first whose condition holds.
    fn select(&mut self, branches: &[RbcBranch], fallback: ExprId) -> Option<ExprId> {
        for branch in branches {
            if self.eval(branch.condition)?.boolean()? {
                return Some(branch.value);
            }
        }
        Some(fallback)
    }

    fn coordinate(&self, coordinate: RbcCoordinate) -> Option<Value> {
        if let (RbcCoordinate::Parameter { variable }, Some(parameters)) =
            (coordinate, self.parameters)
        {
            return parameters.get(&variable).cloned();
        }
        let frame = self.frame.as_ref()?;
        match coordinate {
            RbcCoordinate::FunctionParameter { ordinal, .. } => {
                frame.arguments.get(ordinal as usize).cloned()
            }
            RbcCoordinate::Binder { domain, ordinal } => frame
                .binders
                .get(&(domain.0, ordinal))
                .copied()
                .map(Value::Integer),
            _ => None,
        }
    }
}

fn array_of(values: Vec<Value>) -> Option<Value> {
    let Some(first) = values.first() else {
        return Some(Value::Array(vec![0], Vec::new()));
    };
    Some(match first {
        Value::Array(dims, _) => {
            let inner = dims.clone();
            let mut data = Vec::new();
            for value in values.iter() {
                match value {
                    Value::Array(d, elements) if *d == inner => {
                        data.extend(elements.iter().cloned())
                    }
                    _ => return None,
                }
            }
            let mut dims = vec![values.len()];
            dims.extend(inner);
            Value::Array(dims, data)
        }
        _ => {
            if values.iter().any(|value| matches!(value, Value::Array(..))) {
                return None;
            }
            Value::Array(vec![values.len()], values)
        }
    })
}

fn index(base: &Value, subscripts: &[Option<i64>]) -> Option<Value> {
    let Value::Array(dims, data) = base else {
        return None;
    };
    if subscripts.len() > dims.len() || subscripts.iter().any(Option::is_none) {
        return None;
    }
    let mut offset = 0usize;
    let mut stride = data.len();
    for (position, subscript) in subscripts.iter().enumerate() {
        let extent = dims[position];
        stride /= extent.max(1);
        let one_based = usize::try_from(subscript.unwrap_or(0)).ok()?;
        if one_based == 0 || one_based > extent {
            return None;
        }
        offset += (one_based - 1) * stride;
    }
    let rest = &dims[subscripts.len()..];
    if rest.is_empty() {
        data.get(offset).cloned()
    } else {
        Some(Value::Array(
            rest.to_vec(),
            data.get(offset..offset + stride)?.to_vec(),
        ))
    }
}

fn unary(op: RbcUnaryOp, value: Value) -> Option<Value> {
    Some(match (op, value) {
        (RbcUnaryOp::Negate, Value::Real(v)) => Value::Real(-v),
        (RbcUnaryOp::Negate, Value::Integer(v)) => Value::Integer(v.checked_neg()?),
        (RbcUnaryOp::Not, Value::Boolean(v)) => Value::Boolean(!v),
        (RbcUnaryOp::Plus, value @ (Value::Real(_) | Value::Integer(_))) => value,
        (op, Value::Array(dims, data)) => Value::Array(
            dims,
            data.into_iter()
                .map(|element| unary(op, element))
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

fn binary(op: RbcBinaryOp, lhs: &Value, rhs: &Value) -> Option<Value> {
    use RbcBinaryOp as Op;
    let elementwise = matches!(
        op,
        Op::ElementwiseAdd
            | Op::ElementwiseSubtract
            | Op::ElementwiseMultiply
            | Op::ElementwiseDivide
            | Op::ElementwisePower
    );
    let scalar_op = match op {
        Op::ElementwiseAdd => Op::Add,
        Op::ElementwiseSubtract => Op::Subtract,
        Op::ElementwiseMultiply => Op::Multiply,
        Op::ElementwiseDivide => Op::Divide,
        Op::ElementwisePower => Op::Power,
        other => other,
    };
    match (lhs, rhs) {
        (Value::Array(a, x), Value::Array(b, y)) => {
            // Array-array: element-wise for `+ -` and the dotted operators;
            // `*` between arrays is a matrix product, which this evaluator
            // does not model.
            if a != b || !(elementwise || matches!(op, Op::Add | Op::Subtract)) {
                return None;
            }
            Some(Value::Array(
                a.clone(),
                x.iter()
                    .zip(y)
                    .map(|(x, y)| binary(scalar_op, x, y))
                    .collect::<Option<_>>()?,
            ))
        }
        (Value::Array(a, x), scalar) => {
            if !(elementwise || matches!(op, Op::Multiply | Op::Divide)) {
                return None;
            }
            Some(Value::Array(
                a.clone(),
                x.iter()
                    .map(|x| binary(scalar_op, x, scalar))
                    .collect::<Option<_>>()?,
            ))
        }
        (scalar, Value::Array(b, y)) => {
            if !(elementwise || matches!(op, Op::Multiply)) {
                return None;
            }
            Some(Value::Array(
                b.clone(),
                y.iter()
                    .map(|y| binary(scalar_op, scalar, y))
                    .collect::<Option<_>>()?,
            ))
        }
        (Value::Integer(a), Value::Integer(b)) => Some(match scalar_op {
            Op::Add => Value::Integer(a.checked_add(*b)?),
            Op::Subtract => Value::Integer(a.checked_sub(*b)?),
            Op::Multiply => Value::Integer(a.checked_mul(*b)?),
            Op::Divide if *b != 0 => Value::Real(*a as f64 / *b as f64),
            Op::Power => Value::Real((*a as f64).powf(*b as f64)),
            _ => compare(scalar_op, a.cmp(b))?,
        }),
        (Value::Boolean(a), Value::Boolean(b)) => Some(Value::Boolean(match scalar_op {
            Op::And => *a && *b,
            Op::Or => *a || *b,
            Op::Equal => a == b,
            Op::NotEqual => a != b,
            _ => return None,
        })),
        (a, b) => {
            let (a, b) = (a.real()?, b.real()?);
            Some(match scalar_op {
                Op::Add => Value::Real(a + b),
                Op::Subtract => Value::Real(a - b),
                Op::Multiply => Value::Real(a * b),
                Op::Divide if b != 0.0 => Value::Real(a / b),
                Op::Power => Value::Real(a.powf(b)),
                _ => compare(scalar_op, a.partial_cmp(&b)?)?,
            })
        }
    }
}

fn compare(op: RbcBinaryOp, ordering: std::cmp::Ordering) -> Option<Value> {
    use RbcBinaryOp as Op;
    use std::cmp::Ordering::{Equal, Greater, Less};
    Some(Value::Boolean(match op {
        Op::Equal => ordering == Equal,
        Op::NotEqual => ordering != Equal,
        Op::Less => ordering == Less,
        Op::LessEqual => ordering != Greater,
        Op::Greater => ordering == Greater,
        Op::GreaterEqual => ordering != Less,
        _ => return None,
    }))
}

fn builtin(name: &str, arguments: &[Value]) -> Option<Value> {
    let real1 = |f: fn(f64) -> f64| -> Option<Value> {
        match arguments {
            [value] => Some(Value::Real(f(value.real()?))),
            _ => None,
        }
    };
    match name {
        "sin" => real1(f64::sin),
        "cos" => real1(f64::cos),
        "tan" => real1(f64::tan),
        "asin" => real1(f64::asin),
        "acos" => real1(f64::acos),
        "atan" => real1(f64::atan),
        "sinh" => real1(f64::sinh),
        "cosh" => real1(f64::cosh),
        "tanh" => real1(f64::tanh),
        "exp" => real1(f64::exp),
        "log" => real1(f64::ln),
        "log10" => real1(f64::log10),
        "sqrt" => real1(f64::sqrt),
        "atan2" => match arguments {
            [y, x] => Some(Value::Real(y.real()?.atan2(x.real()?))),
            _ => None,
        },
        "abs" => match arguments {
            [Value::Integer(v)] => Some(Value::Integer(v.checked_abs()?)),
            [value] => Some(Value::Real(value.real()?.abs())),
            _ => None,
        },
        "sign" => match arguments {
            [value] => {
                let v = value.real()?;
                Some(Value::Integer(if v > 0.0 {
                    1
                } else if v < 0.0 {
                    -1
                } else {
                    0
                }))
            }
            _ => None,
        },
        "floor" => real1(f64::floor),
        "ceil" => real1(f64::ceil),
        "integer" => match arguments {
            [value] => {
                let v = value.real()?.floor();
                (v.is_finite() && v.abs() < 9.0e15).then_some(Value::Integer(v as i64))
            }
            _ => None,
        },
        // Pass-through wrappers: at translation time none of them changes a
        // value (`homotopy` takes its actual argument).
        "noEvent" | "smooth" => arguments.last().cloned(),
        "homotopy" => arguments.first().cloned(),
        "min" | "max" => {
            let values: Vec<Value> = match arguments {
                [Value::Array(_, elements)] => elements.clone(),
                [a, b] => vec![a.clone(), b.clone()],
                _ => return None,
            };
            let mut best = values.first()?.clone();
            for value in &values[1..] {
                let take = match name {
                    "min" => value.real()? < best.real()?,
                    _ => value.real()? > best.real()?,
                };
                if take {
                    best = value.clone();
                }
            }
            Some(best)
        }
        "sum" | "product" => {
            let elements = arguments.first()?.elements()?;
            let mut total = if name == "sum" {
                Value::Integer(0)
            } else {
                Value::Integer(1)
            };
            let op = if name == "sum" {
                RbcBinaryOp::Add
            } else {
                RbcBinaryOp::Multiply
            };
            for element in elements {
                total = binary(op, &total, element)?;
            }
            Some(total)
        }
        "size" => match arguments {
            [Value::Array(dims, _), dimension] => {
                let d = usize::try_from(dimension.integer()?).ok()?;
                Some(Value::Integer(
                    i64::try_from(*dims.get(d.checked_sub(1)?)?).ok()?,
                ))
            }
            _ => None,
        },
        _ => None,
    }
}

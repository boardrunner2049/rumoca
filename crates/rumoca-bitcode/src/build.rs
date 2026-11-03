//! Building an artifact in Rust.
//!
//! The format calls itself a public interchange format, and a Rust consumer
//! had no way to produce one except by writing `RbcModel { .. }` out in full.
//! That has a cost the test suite has been paying: every field added to the
//! schema breaks every struct literal that names all of them, and this crate's
//! fixtures were repaired by hand three times in one week for exactly that
//! reason. A builder fills new fields with their defaults, so adding one is a
//! schema change and not a fixture change.
//!
//! Three invariants it holds, all of them cheap to break and expensive to
//! find later:
//!
//! - **The expression arena is topologically ordered.** An operand id must be
//!   strictly less than its node's, which is what makes a cycle
//!   unrepresentable and evaluation a single forward pass. [`Builder::expr`]
//!   checks it at the call rather than leaving it to validation.
//! - **Ids are positions.** Every table is dense, so the builder assigns ids
//!   and the caller never does.
//! - **Provenance is not optional.** Everything added carries `Generated`
//!   provenance, because an entity a tool produced must be distinguishable
//!   from one a modeller wrote.
//!
//! ```
//! use rumoca_bitcode::build::Builder;
//! use rumoca_bitcode::schema::RbcBinaryOp;
//!
//! let mut builder = Builder::new("Decay");
//! let k = builder.parameter("k", 2.0);
//! let x = builder.state("x", 1.0);
//!
//! // Operands are appended before the node that uses them, which is the
//! // topological rule the arena is built on.
//! let left = builder.parameter_ref(k);
//! let right = builder.state_ref(x);
//! let scaled = builder.binary(RbcBinaryOp::Multiply, left, right);
//! let rhs = builder.negate_of(scaled);
//! builder.derivative_equation(x, rhs);
//!
//! let model = builder.finish();
//! assert_eq!(model.variables.len(), 2);
//! assert_eq!(model.equations.len(), 1);
//! ```

use crate::schema::{
    ComponentId, ConditionId, EquationId, ExprId, RbcAction, RbcBinaryOp, RbcCausality,
    RbcCondition, RbcConditionNode, RbcCoordinate, RbcEquation, RbcEventAction, RbcExpr,
    RbcExprNode, RbcGeneration, RbcLiteral, RbcModel, RbcOrigin, RbcProvenance, RbcRelation,
    RbcRole, RbcRoot, RbcScalar, RbcSource, RbcSpan, RbcSummary, RbcSymbolContract, RbcTracePoint,
    RbcType, RbcUnaryOp, RbcVariability, RbcVariable, RelationId, RootId, SourceId, TracePointId,
    TypeId, VariableId,
};

/// Appends to a model, keeping the arena ordered and the tables dense.
pub struct Builder {
    model: RbcModel,
    generation: RbcGeneration,
}

impl Builder {
    /// A new, empty artifact with one source and one scalar `Real` type.
    ///
    /// The source exists so that generated provenance has somewhere to point:
    /// a span names a source by id, and an empty source table makes every
    /// entity dangle, which validation reports as "source 0 does not exist".
    pub fn new(name: impl Into<String>) -> Self {
        let mut model = RbcModel {
            connector_types: Vec::new(),
            connectors: Vec::new(),
            name: name.into(),
            sources: vec![RbcSource {
                id: SourceId(0),
                name: "<rumoca-bitcode::build>".into(),
                text: None,
            }],
            types: Vec::new(),
            variables: Vec::new(),
            expressions: Vec::new(),
            equations: Vec::new(),
            initial_equations: Vec::new(),
            domains: Vec::new(),
            discrete_real_equations: Vec::new(),
            initial_discrete_values: Vec::new(),
            initial_parameter_values: Vec::new(),
            functions: Vec::new(),
            equation_families: Vec::new(),
            initial_equation_families: Vec::new(),
            relations: Vec::new(),
            conditions: Vec::new(),
            clocks: Vec::new(),
            clock_ownerships: Vec::new(),
            roots: Vec::new(),
            events: Vec::new(),
            time_events: Vec::new(),
            connections: Vec::new(),
            connection_sets: Vec::new(),
            components: Vec::new(),
            discrete_definitions: Vec::new(),
            model_event_transactions: Vec::new(),
            previous_values: Vec::new(),
            terminals: Vec::new(),
            structured_roots: Vec::new(),
            delays: Vec::new(),
            trace_points: Vec::new(),
            summary: RbcSummary::default(),
        };
        model.types.push(RbcType {
            id: TypeId(0),
            scalar: RbcScalar::Real,
            dimensions: Vec::new(),
            record: None,
        });
        Self {
            model,
            generation: RbcGeneration::SyntheticResidual,
        }
    }

    /// Continue from an existing artifact rather than a fresh one.
    pub fn from_model(model: RbcModel) -> Self {
        Self {
            model,
            generation: RbcGeneration::SyntheticResidual,
        }
    }

    /// What to record as the origin of everything added from here on.
    pub fn generation(&mut self, generation: RbcGeneration) -> &mut Self {
        self.generation = generation;
        self
    }

    fn provenance(&self) -> RbcProvenance {
        RbcProvenance {
            origin: RbcOrigin::Generated {
                generation: self.generation,
            },
            span: RbcSpan {
                source: SourceId::PLACEHOLDER,
                start: 0,
                end: 0,
                line: 0,
                column: 0,
            },
        }
    }

    // ── types ───────────────────────────────────────────────────────────────

    /// A scalar type of this kind, reusing one the model already declares.
    pub fn scalar_type(&mut self, scalar: RbcScalar) -> TypeId {
        if let Some(found) = self
            .model
            .types
            .iter()
            .find(|entry| entry.scalar == scalar && entry.dimensions.is_empty())
        {
            return found.id;
        }
        let id = TypeId(self.model.types.len() as u32);
        self.model.types.push(RbcType {
            id,
            scalar,
            dimensions: Vec::new(),
            record: None,
        });
        id
    }

    // ── variables ───────────────────────────────────────────────────────────

    /// A variable in any role, with everything else defaulted.
    pub fn variable(&mut self, name: impl Into<String>, role: RbcRole) -> VariableId {
        let id = VariableId(self.model.variables.len() as u32);
        let value_type = self.scalar_type(RbcScalar::Real);
        let variability = match role {
            RbcRole::Parameter => RbcVariability::Parameter,
            RbcRole::Constant => RbcVariability::Constant,
            RbcRole::DiscreteReal | RbcRole::DiscreteValue => RbcVariability::Discrete,
            _ => RbcVariability::Continuous,
        };
        let causality = match role {
            RbcRole::Parameter | RbcRole::Constant => RbcCausality::Parameter,
            RbcRole::Input => RbcCausality::Input,
            RbcRole::Output => RbcCausality::Output,
            _ => RbcCausality::Local,
        };
        self.model.variables.push(RbcVariable {
            id,
            name: name.into(),
            role,
            causality,
            declared_causality: None,
            value_type,
            scalar_count: 1,
            contract: Some(RbcSymbolContract {
                variability,
                is_final: false,
                is_protected: false,
                evaluate: false,
                structural: false,
                effective_value: None,
                binding_depends_on: Vec::new(),
                binding_from_modification: false,
                declared_in: None,
            }),
            discrete_input: false,
            declaration: self.provenance(),
            component: None,
            unit: None,
            physical_quantity: None,
            description: None,
            binding: None,
            start: None,
            min: None,
            max: None,
            nominal: None,
            fixed: None,
            fixed_elements: None,
            evaluable: false,
            held: false,
            state_select: crate::schema::RbcStateSelect::Default,
            tunable: false,
            from_source: false,
            connector: None,
            declaring_class: None,
        });
        id
    }

    /// A continuous state fixed at `start`.
    pub fn state(&mut self, name: impl Into<String>, start: f64) -> VariableId {
        let literal = self.real(start);
        let id = self.variable(name, RbcRole::State);
        let entry = &mut self.model.variables[id.0 as usize];
        entry.start = Some(literal);
        entry.fixed = Some(true);
        id
    }

    /// A parameter bound to `value`.
    pub fn parameter(&mut self, name: impl Into<String>, value: f64) -> VariableId {
        let literal = self.real(value);
        let id = self.variable(name, RbcRole::Parameter);
        self.model.variables[id.0 as usize].binding = Some(literal);
        id
    }

    // ── expressions ─────────────────────────────────────────────────────────

    /// Append a node, rejecting an operand that is not strictly earlier.
    ///
    /// The check is here rather than left to validation because validation
    /// runs much later and names a node id, where this names the call.
    pub fn expr(&mut self, node: RbcExprNode) -> ExprId {
        let index = self.model.expressions.len() as u32;
        for operand in references(ExprId(index), &node) {
            assert!(
                operand.0 < index,
                "node {index} would reference operand {}, which is not strictly \
                 earlier; the arena must stay topologically ordered",
                operand.0
            );
        }
        let value_type = self.scalar_type(RbcScalar::Real);
        let id = ExprId(index);
        self.model.expressions.push(RbcExpr {
            id,
            value_type,
            node,
            provenance: self.provenance(),
        });
        id
    }

    pub fn real(&mut self, value: f64) -> ExprId {
        self.expr(RbcExprNode::Literal {
            value: RbcLiteral::Real { value },
        })
    }

    pub fn state_ref(&mut self, variable: VariableId) -> ExprId {
        self.expr(RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::State { variable },
        })
    }

    pub fn parameter_ref(&mut self, variable: VariableId) -> ExprId {
        self.expr(RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::Parameter { variable },
        })
    }

    pub fn derivative_ref(&mut self, variable: VariableId) -> ExprId {
        self.expr(RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::Derivative { variable },
        })
    }

    pub fn binary(&mut self, op: RbcBinaryOp, lhs: ExprId, rhs: ExprId) -> ExprId {
        self.expr(RbcExprNode::Binary { op, lhs, rhs })
    }

    pub fn negate_of(&mut self, operand: ExprId) -> ExprId {
        self.expr(RbcExprNode::Unary {
            op: RbcUnaryOp::Negate,
            operand,
        })
    }

    // ── equations ───────────────────────────────────────────────────────────

    /// `residual = 0`, with `reads` derived from the expression graph.
    pub fn equation(&mut self, residual: ExprId) -> EquationId {
        let (reads, reads_derivative, reads_previous) = self.reads_of(residual);
        let id = EquationId(self.model.equations.len() as u32);
        self.model.equations.push(RbcEquation {
            id,
            residual,
            provenance: self.provenance(),
            reads,
            reads_derivative,
            reads_previous,
        });
        id
    }

    /// `der(state) - rhs = 0`.
    ///
    /// The subtractive form is required, not preferred: the runtime rejects a
    /// state equation that is not a subtractive derivative residual, and the
    /// algebraically identical `der(x) + k*x` fails at simulation with a
    /// message about a constraint nothing else states.
    pub fn derivative_equation(&mut self, state: VariableId, rhs: ExprId) -> EquationId {
        let derivative = self.derivative_ref(state);
        let residual = self.binary(RbcBinaryOp::Subtract, derivative, rhs);
        self.equation(residual)
    }

    // ── events ──────────────────────────────────────────────────────────────

    pub fn relation(&mut self, expression: ExprId) -> RelationId {
        let id = RelationId(self.model.relations.len() as u32);
        self.model.relations.push(RbcRelation {
            id,
            expression,
            provenance: self.provenance(),
        });
        id
    }

    pub fn condition(&mut self, node: RbcConditionNode) -> ConditionId {
        let id = ConditionId(self.model.conditions.len() as u32);
        self.model.conditions.push(RbcCondition {
            id,
            node,
            provenance: self.provenance(),
        });
        id
    }

    pub fn always(&mut self) -> ConditionId {
        self.condition(RbcConditionNode::Always)
    }

    pub fn root(&mut self, relation: RelationId, activation: ConditionId) -> RootId {
        let id = RootId(self.model.roots.len() as u32);
        self.model.roots.push(RbcRoot {
            id,
            relation,
            activation,
            provenance: self.provenance(),
        });
        id
    }

    pub fn event(&mut self, trigger: ConditionId, guard: ConditionId, action: RbcAction) {
        let id = crate::schema::EventId(self.model.events.len() as u32);
        self.model.events.push(RbcEventAction {
            id,
            trigger,
            guard,
            action,
            provenance: self.provenance(),
        });
    }

    // ── observation ─────────────────────────────────────────────────────────

    pub fn trace_point(
        &mut self,
        variable: VariableId,
        label: impl Into<String>,
        added_by: impl Into<String>,
    ) -> TracePointId {
        let id = TracePointId(self.model.trace_points.len() as u32);
        self.model.trace_points.push(RbcTracePoint {
            id,
            variable,
            label: label.into(),
            connection: None,
            connection_set: None,
            quantity: None,
            unit: None,
            added_by: Some(added_by.into()),
        });
        id
    }

    pub fn component(
        &mut self,
        path: impl Into<String>,
        class_name: Option<String>,
    ) -> ComponentId {
        let id = ComponentId(self.model.components.len() as u32);
        self.model.components.push(crate::schema::RbcComponent {
            id,
            path: path.into(),
            class_name,
        });
        id
    }

    // ── inspection ──────────────────────────────────────────────────────────

    fn reads_of(&self, root: ExprId) -> (Vec<VariableId>, Vec<VariableId>, Vec<VariableId>) {
        let mut reads = Vec::new();
        let mut derivatives = Vec::new();
        let mut previous = Vec::new();
        let mut stack = vec![root];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.0) {
                continue;
            }
            let Some(entry) = self.model.expressions.get(id.0 as usize) else {
                continue;
            };
            stack.extend(references(id, &entry.node));
            let RbcExprNode::Coordinate { coordinate } = &entry.node else {
                continue;
            };
            match coordinate {
                RbcCoordinate::Derivative { variable } => derivatives.push(*variable),
                RbcCoordinate::PreState { variable }
                | RbcCoordinate::PreAlgebraic { variable }
                | RbcCoordinate::PreDiscreteReal { variable }
                | RbcCoordinate::PreDiscreteValue { variable } => previous.push(*variable),
                RbcCoordinate::Parameter { variable }
                | RbcCoordinate::Input { variable }
                | RbcCoordinate::State { variable }
                | RbcCoordinate::Algebraic { variable }
                | RbcCoordinate::DiscreteReal { variable }
                | RbcCoordinate::DiscreteValue { variable } => reads.push(*variable),
                _ => {}
            }
        }
        for list in [&mut reads, &mut derivatives, &mut previous] {
            list.sort_by_key(|variable| variable.0);
            list.dedup();
        }
        (reads, derivatives, previous)
    }

    /// The finished model, with the summary the validator checks recomputed.
    pub fn finish(mut self) -> RbcModel {
        crate::validate::recompute_summary(&mut self.model);
        self.model
    }
}

/// Every expression a node references.
///
/// One place, so a caller never needs to know the shape of every node to walk
/// or rewrite one. Exhaustive by construction: a new node kind that carries
/// operands and is not listed here would be walked as a leaf, so this is
/// checked against the arena in the crate's tests.
pub fn operands(node: &RbcExprNode) -> Vec<ExprId> {
    match node {
        RbcExprNode::StringConversion { value, format } => {
            let mut found = vec![*value];
            found.extend(format.operands());
            found
        }
        RbcExprNode::Literal { .. } | RbcExprNode::Coordinate { .. } => Vec::new(),
        RbcExprNode::ClockTransfer { source, .. } => vec![*source],
        RbcExprNode::Unary { operand, .. } => vec![*operand],
        RbcExprNode::Binary { lhs, rhs, .. } => vec![*lhs, *rhs],
        RbcExprNode::Conditional { branches, fallback } => {
            let mut found: Vec<ExprId> = branches
                .iter()
                .flat_map(|branch| [branch.condition, branch.value])
                .collect();
            found.push(*fallback);
            found
        }
        RbcExprNode::Builtin { arguments, .. } => arguments.clone(),
        RbcExprNode::Array { elements, .. } => elements.clone(),
        RbcExprNode::Record { fields, .. } => fields.clone(),
        RbcExprNode::Field { base, .. } => vec![*base],
        RbcExprNode::Range { start, step, stop } => {
            let mut found = vec![*start, *stop];
            found.extend(step.iter().copied());
            found
        }
        RbcExprNode::Comprehension { body, .. } => vec![*body],
        // A subscript is an expression like any other operand: `x[i]` reads
        // `i`. Leaving subscripts out made a variable used only as an index
        // invisible to every walk built on this function.
        RbcExprNode::Index { base, subscripts } => std::iter::once(*base)
            .chain(subscripts.iter().filter_map(|s| s.expression()))
            .collect(),
        RbcExprNode::ArrayUpdate {
            base,
            value,
            subscripts,
        } => [*base, *value]
            .into_iter()
            .chain(subscripts.iter().filter_map(|s| s.expression()))
            .collect(),
        RbcExprNode::Call { arguments, .. } => arguments.clone(),
        // Leaves: each names a value inside a function body by owner-local
        // ordinal, not by expression id, so there is no operand to walk.
        RbcExprNode::FunctionValue { .. }
        | RbcExprNode::FunctionFoldParameter { .. }
        | RbcExprNode::FunctionFoldOutput { .. } => Vec::new(),
        RbcExprNode::Unsupported { .. } => Vec::new(),
    }
}

/// Every expression the node at `id` depends on: its operands, plus the head
/// of the call when the node is a further result of one.
///
/// A call projection carries no arguments of its own; it shares the head's
/// one evaluation, so what the head reads the projection reads too. That
/// edge needs the node's own id to tell a head (which names itself as
/// owner) from a projection, which is why it is not in [`operands`].
pub fn references(id: ExprId, node: &RbcExprNode) -> Vec<ExprId> {
    let mut found = operands(node);
    if let RbcExprNode::Call { owner, .. } = node
        && *owner != id
    {
        found.push(*owner);
    }
    found
}

//! Rumoca Bitcode v2 — the public wire schema.
//!
//! This module is the contract external tools depend on. It is deliberately
//! **not** a mirror of any internal Rumoca type: no field here is required to
//! correspond to a Rust struct field in `rumoca-ir-dae`, and the internal
//! `DAE_SCHEMA_VERSION` may change without changing [`RBC_VERSION`].
//!
//! Rules for changing this file:
//!
//! * Adding an optional field, a new enum variant with an explicit tag, or a
//!   new collection is a **compatible** change and does not bump
//!   [`RBC_VERSION`].
//! * Removing a field, renaming a tag, or changing a field's meaning is an
//!   **incompatible** change and bumps [`RBC_VERSION`].
//! * Every enum is externally tagged by an explicit `kind` string, never by
//!   declaration order, so appending or reordering variants cannot silently
//!   change how an older file decodes.

use serde::{Deserialize, Serialize};

/// Container magic. Present so a reader can reject a non-RBC file before
/// attempting to decode it as one.
pub const RBC_MAGIC: &str = "RUMOCA-RBC";

/// Public bitcode contract version. Independent of `DAE_SCHEMA_VERSION`.
pub const RBC_VERSION: u32 = 2;

// ── Identities ───────────────────────────────────────────────────────────────
//
// Every ID is unique within one artifact. IDs are carried as explicit `id`
// fields on each record rather than implied by array position, so a consumer
// never has to assume an index is an identity. They are NOT stable across two
// separate compilations; cross-artifact matching uses names, roles, and types.

macro_rules! rbc_id {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    )+};
}

rbc_id! {
    /// Identifies a source file within this artifact.
    SourceId,
    /// Identifies a value type within this artifact.
    TypeId,
    /// Identifies a variable within this artifact.
    VariableId,
    /// Identifies an expression node within this artifact.
    ExprId,
    /// Identifies an equation within this artifact.
    EquationId,
    /// Identifies a relation (a primitive comparison) within this artifact.
    RelationId,
    /// Identifies a boolean condition within this artifact.
    ConditionId,
    /// Identifies a scheduled or Boolean-triggered clock.
    ClockId,
    /// Identifies a zero-crossing root within this artifact.
    RootId,
    /// Identifies an event action within this artifact.
    EventId,
    /// Identifies a connection within this artifact.
    ConnectionId,
    /// Identifies a connection set --- one node of the connection graph.
    ConnectionSetId,
    /// Identifies a component instance within this artifact.
    ComponentId,
    /// Identifies a trace point within this artifact.
    TracePointId,
}

// ── Container ────────────────────────────────────────────────────────────────

/// One `.rbc` artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcFile {
    /// Optional public, independently versioned executable projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<rumoca_ir_solve::execution::ExecutionArtifact>,
    /// Always [`RBC_MAGIC`].
    pub magic: String,
    /// Public contract version. See [`RBC_VERSION`].
    pub bitcode_version: u32,
    /// Free-form producer identification, e.g. `"rumoca 0.10.0"`. Informational
    /// only: a consumer must not change behaviour based on it.
    pub producer: String,
    /// The compiled model.
    pub model: RbcModel,
}

/// A compiled Modelica model in public form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcModel {
    /// Semantic connector type declarations for public model construction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_types: Vec<RbcConnectorType>,
    /// Connector instances own member identities independently of connection edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connectors: Vec<RbcConnectorInstance>,
    /// Top-level model name as compiled, e.g. `"Circuit.Test"`.
    pub name: String,
    /// Source files referenced by provenance in this artifact.
    pub sources: Vec<RbcSource>,
    /// Value types referenced by variables and expressions.
    pub types: Vec<RbcType>,
    /// Every variable in the model, in every role.
    pub variables: Vec<RbcVariable>,
    /// Flat expression arena. Operands reference earlier entries by [`ExprId`].
    pub expressions: Vec<RbcExpr>,
    /// Continuous residual equations: each asserts `residual == 0`.
    pub equations: Vec<RbcEquation>,
    /// Initialization residual equations.
    pub initial_equations: Vec<RbcEquation>,
    /// Iteration domains referenced by the equation families.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<RbcDomain>,
    /// MLS Appendix B.1b coupled discrete-Real equations, which `equations`
    /// does not contain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discrete_real_equations: Vec<RbcDiscreteRealEquation>,
    /// Values discrete-valued variables take at the initialization instant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_discrete_values: Vec<RbcInitialDiscreteValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_parameter_values: Vec<RbcInitialDiscreteValue>,
    /// Function declarations named by `RbcExprNode::Call`, with their
    /// bodies when this artifact carries them (`RbcFunctionBody`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<RbcFunction>,
    /// Array/`for` equations, which `equations` does not contain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equation_families: Vec<RbcEquationFamily>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_equation_families: Vec<RbcEquationFamily>,
    /// Primitive comparisons that can generate events.
    pub relations: Vec<RbcRelation>,
    /// Boolean activation conditions over relations, clocks and discretes.
    pub conditions: Vec<RbcCondition>,
    /// Exact schedules referenced by clock activations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clocks: Vec<RbcClock>,
    /// Clock ownership of discrete variables, including sampled left-limit reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clock_ownerships: Vec<RbcClockOwnership>,
    /// Zero-crossing surfaces the solver must monitor.
    pub roots: Vec<RbcRoot>,
    /// Actions performed when an event fires.
    pub events: Vec<RbcEventAction>,
    /// Scheduled (time-triggered) events.
    pub time_events: Vec<RbcTimeEvent>,
    /// Connector-level provenance recovered from flattening.
    pub connections: Vec<RbcConnection>,
    /// The connection graph's nodes: what each `connect` set equates and
    /// conserves. Empty where flattening did not record connections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_sets: Vec<RbcConnectionSet>,
    /// Component instances referenced by connections and variables.
    pub components: Vec<RbcComponent>,
    /// Observation requests. Empty on export; a transformation pass adds these.
    #[serde(default)]
    pub trace_points: Vec<RbcTracePoint>,
    /// MLS Appendix B.1c definitions: what each discrete-valued variable
    /// (Boolean, Integer, enumeration) is equal to, and under what activation.
    ///
    /// Separate from `equations`, which carries only continuous residuals, and
    /// from `events`, which carries reinit/assert/terminate actions. Without
    /// this a `discrete_value` variable is declared and never defined, and
    /// reconstruction rejects the artifact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discrete_definitions: Vec<RbcDiscreteDefinition>,
    /// Discrete variables defined together by one event-guarded algorithm or
    /// `when` clause (MLS §8.3.5, §11.1.2): each transaction owns its targets,
    /// and every target is defined by at least one of its steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_event_transactions: Vec<RbcModelEventTransaction>,
    /// MLS §16.5 `previous(v)` coordinates, each owned by one clock.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_values: Vec<RbcPreviousValue>,
    /// The MLS §8.3.6 `terminal()` observation, when the model reads it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<RbcTerminal>,
    /// Tensor-native families of root surfaces over a compact domain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structured_roots: Vec<RbcStructuredRoot>,
    /// MLS §3.7.4.1 `delay` owners, in the order their coordinates occur in
    /// the expression arena; each is read by exactly one `Delay` coordinate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delays: Vec<RbcDelay>,
    /// Counts a consumer can check against the collections above. Present so a
    /// truncated or partially-written artifact fails loudly.
    pub summary: RbcSummary,
}

/// Identifies one `previous` coordinate owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PreviousId(pub u32);

/// Identifies the `terminal()` observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TerminalId(pub u32);

/// Identifies one delay owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DelayId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RbcPreviousValue {
    /// A discrete-Real or discrete-valued variable owned by `clock`.
    pub variable: VariableId,
    pub clock: ClockId,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RbcTerminal {
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RbcStructuredRoot {
    pub domain: DomainId,
    pub expression: ExprId,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RbcDelay {
    /// The delayed expression.
    pub source: ExprId,
    pub delay: RbcDelayKind,
    pub provenance: RbcProvenance,
}

/// A fixed delay time known at translation, or a varying one with a fixed
/// bound; either way the runtime buffer has a size known before it starts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcDelayKind {
    Parameter {
        delay_time: RbcPositiveParameter,
    },
    Bounded {
        delay_time: ExprId,
        maximum: RbcPositiveParameter,
    },
}

/// A positive parameter-variability Real, with the value the compiler proved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RbcPositiveParameter {
    pub expression: ExprId,
    pub value: f64,
    pub provenance: RbcProvenance,
}

/// One model event transaction: the discrete variables it owns and the
/// guarded steps that assign them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcModelEventTransaction {
    /// Discrete-Real or discrete-valued variables this transaction owns.
    pub targets: Vec<VariableId>,
    pub steps: Vec<RbcModelEventStep>,
    pub provenance: RbcProvenance,
}

/// One activation of a transaction: when `trigger` fires and `guard` holds
/// (on `clock`, for a clocked step), each definition takes its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcModelEventStep {
    pub trigger: ConditionId,
    pub guard: ConditionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<ClockId>,
    pub definitions: Vec<RbcModelEventDefinition>,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RbcModelEventDefinition {
    pub target: VariableId,
    pub value: ExprId,
    pub provenance: RbcProvenance,
}

/// Denormalised counts, for cheap validation and for `bitcode inspect`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcSummary {
    pub variables: u32,
    pub states: u32,
    pub parameters: u32,
    pub constants: u32,
    pub inputs: u32,
    pub outputs: u32,
    pub algebraics: u32,
    pub discrete_reals: u32,
    pub discrete_values: u32,
    pub equations: u32,
    pub initial_equations: u32,
    pub expressions: u32,
    pub relations: u32,
    pub conditions: u32,
    #[serde(default)]
    pub clocks: u32,
    #[serde(default)]
    pub clock_ownerships: u32,
    pub roots: u32,
    pub events: u32,
    pub time_events: u32,
    pub connections: u32,
    /// Nodes of the connection graph. Defaulted so artifacts written before
    /// the exporter emitted sets still load.
    #[serde(default)]
    pub connection_sets: u32,
    pub components: u32,
    pub trace_points: u32,
    #[serde(default)]
    pub discrete_definitions: u32,
    /// MLS Appendix B.1b equations. Counted separately from `equations`
    /// because they determine a different partition of unknowns: adding them
    /// to the continuous count would make every model with a `sample` look
    /// over-constrained.
    #[serde(default)]
    pub discrete_real_equations: u32,
    #[serde(default)]
    pub initial_discrete_values: u32,
    /// Families, and the scalar rows they stand for. Both are needed: a
    /// consumer counting equations wants the rows, one walking the artifact
    /// wants the families.
    #[serde(default)]
    pub domains: u32,
    #[serde(default)]
    pub equation_families: u32,
    #[serde(default)]
    pub family_scalar_rows: u32,
}

// ── Provenance ───────────────────────────────────────────────────────────────

impl SourceId {
    /// The first entry of this artifact's own source table.
    ///
    /// A parser has to put *something* in a span before it reads the item that
    /// carries one, and every such span is overwritten before it is stored.
    /// Naming it says so, where a bare `SourceId(0)` reads as a source nobody
    /// bothered to resolve. This indexes `RbcModel::sources`; it is not a
    /// `rumoca_core::SourceId` and names no file on its own.
    pub const PLACEHOLDER: SourceId = SourceId(0);
}

/// One source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcSource {
    pub id: SourceId,
    /// Path or logical name as Rumoca saw it.
    pub name: String,
    /// Full source text, when the producer was asked to embed it. Absent keeps
    /// artifacts small; present makes them self-contained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A resolved source location. Byte offsets are authoritative; line and column
/// are precomputed at export time so consumers never need the source text to
/// report `Model.mo:52`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcSpan {
    pub source: SourceId,
    pub start: u32,
    pub end: u32,
    /// 1-based line of `start`.
    pub line: u32,
    /// 1-based column of `start`.
    pub column: u32,
}

/// Why a model object exists and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcProvenance {
    pub origin: RbcOrigin,
    pub span: RbcSpan,
}

/// Whether an object was written by the author or produced by lowering.
///
/// `Generated` carries the lowering kind so a consumer can distinguish a
/// connection equation from a flow balance without guessing from shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcOrigin {
    /// Written in the source at `span`.
    Source,
    /// Produced by the compiler; `span` names the nearest responsible source.
    Generated { generation: RbcGeneration },
}

/// Lowering kinds a consumer may care about. Mirrors the compiler's own
/// classification; `Other` keeps a current reader working against a producer that
/// learns a new kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcGeneration {
    SyntheticResidual,
    BindingEquation,
    ConnectionEquation,
    FlowBalanceEquation,
    AlgorithmEquation,
    DiscreteUpdate,
    ConditionLowering,
    PreValueLowering,
    ClockLowering,
    DelayLowering,
    SemiLinearLowering,
    TerminalLowering,
    EventActionLowering,
    InitializationEquation,
    DefaultStart,
    ArrayEquationProjection,
    RecordEquationProjection,
    FunctionLoopLowering,
    FunctionConditionLowering,
    FunctionAggregateLowering,
    DerivedParameterLowering,
    IndexReduction,
    AliasElimination,
    RuntimeDiscontinuity,
    /// A lowering kind this schema version does not name.
    Other,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// A value type: a scalar, or a rectangular array of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcType {
    pub id: TypeId,
    pub scalar: RbcScalar,
    /// Empty for a scalar; otherwise the array extents in row-major order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<u32>,
    /// Set when `scalar` is `record`. Without it a record type flattened to a
    /// bare scalar tag, and a `Record` or `Field` node rebuilt against it was
    /// rejected for having the wrong field count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<RbcRecord>,
}

/// A record type's name and fields, in declaration order.
///
/// A field's type is a `TypeId` into the same table, and always precedes the
/// record that names it, so one forward pass rebuilds the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcRecord {
    pub name: String,
    pub fields: Vec<RbcRecordField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcRecordField {
    pub name: String,
    pub value_type: TypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcScalar {
    Real,
    Integer,
    Boolean,
    String,
    Enumeration,
    Record,
}

// ── Variables ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcDeclaredCausality {
    None,
    Input,
    Output,
}

/// What a declaration promises about a symbol.
///
/// Carried per symbol and preserved through flattening, so a consumer need not
/// re-derive it from names or guess it from the DAE partition. Each field
/// answers a question that the others do not:
///
/// * `variability` — may the value change, and when.
/// * `is_final` — may a *modifier* override this declaration. Says nothing
///   about whether the binding depends on something adjustable.
/// * `is_protected` — who may see it. Visibility is not immutability, and a
///   `protected parameter` is still settable before translation.
/// * `evaluate` — the MLS §18.3 hint that a value may be substituted at
///   translation time. A hint, not a guarantee, and not equivalent to
///   `constant`.
/// * `effective_value` — the binding's value when it is statically evaluable.
/// * `binding_depends_on` — the symbols the binding reads, so a consumer can
///   walk the chain rather than trusting a leaf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcSymbolContract {
    pub variability: RbcVariability,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_final: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_protected: bool,
    /// `annotation(Evaluate=true)`, or a `final` declaration the compiler
    /// treats the same way for structural evaluation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub evaluate: bool,
    /// MLS §18.3: changing this requires retranslation, not just a restart.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub structural: bool,
    /// The binding's value where the compiler could evaluate it statically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_value: Option<f64>,
    /// Symbols the binding reads. Empty for a literal binding or none at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binding_depends_on: Vec<VariableId>,
    /// Whether the binding came from a modifier rather than the declaration.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binding_from_modification: bool,
    /// The class that declared this symbol, and the declaration's own span.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_in: Option<String>,
}

/// MLS §3.8 variability, as the declaration wrote it.
///
/// Distinct from [`RbcRole`], which is the Appendix-B partition the DAE placed
/// the coordinate in. A `constant` and a `parameter` are both role
/// `parameter`; only variability tells them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcVariability {
    Constant,
    Parameter,
    Discrete,
    Continuous,
}

/// Appendix-B partition a variable belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcRole {
    Parameter,
    Constant,
    Input,
    State,
    Algebraic,
    Output,
    DiscreteReal,
    DiscreteValue,
}

/// Interface causality, orthogonal to [`RbcRole`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcCausality {
    Input,
    Output,
    Parameter,
    CalculatedParameter,
    Independent,
    Local,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcStateSelect {
    Never,
    Avoid,
    #[default]
    Default,
    Prefer,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcVariable {
    pub id: VariableId,
    /// Fully-qualified flattened name, e.g. `"motor.flange.tau"`.
    pub name: String,
    pub role: RbcRole,
    pub causality: RbcCausality,
    /// The written prefix, independently of the exported storage role.
    /// Absent in older artifacts, where input/output causality implied it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_causality: Option<RbcDeclaredCausality>,
    pub value_type: TypeId,
    /// Number of scalars this variable expands to (1 for a scalar).
    pub scalar_count: u32,
    /// What the source says this symbol *is*: whether it may change, who may
    /// change it, and what its value depends on.
    ///
    /// Without this an analysis has only `role`, which answers a different
    /// question. `role` is the Appendix-B partition a coordinate belongs to;
    /// the contract is the declaration's promise. A divide-by-zero analysis
    /// that reads `role` alone proposes setting `Modelica.Constants.pi` to
    /// zero, and did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<RbcSymbolContract>,
    /// Set on an `input` whose variability is discrete rather than continuous.
    ///
    /// The role alone does not determine it: a `Boolean` input is discrete and
    /// a `Real` input is usually, but not always, continuous. Import assumed
    /// continuous for every input, so a model with a `Boolean` input exported
    /// cleanly and was then rejected by its own importer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub discrete_input: bool,
    pub declaration: RbcProvenance,
    /// Component instance this variable belongs to, when it is inside one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The MLS §4.8 `quantity` attribute the declaration carries — `"Mass"`,
    /// `"Resistance"`, `"ThermodynamicTemperature"`.
    ///
    /// This is the *semantic* identity of what the variable measures, which a
    /// unit alone does not give: `"Ohm"` says nothing about whether a value must
    /// be positive, while `quantity="Resistance"` on a passive component does.
    /// An external physical-invariant checker needs this to match rules to
    /// variables without inferring meaning from their names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_quantity: Option<String>,
    /// Fully qualified Modelica class that declared this variable, e.g.
    /// `"Modelica.Electrical.Analog.Basic.Resistor"`.
    ///
    /// `physical_quantity` says *what a variable measures*; this says *what
    /// declared it*, and the two answer different questions. `quantity =
    /// "Resistance"` holds equally for a passive resistor and for a
    /// negative-impedance converter, so a rule that must apply to only one of
    /// them has to match on the declaring class. Flattening reduces a component
    /// to a path prefix on a name, so no consumer can recover this afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaring_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Declaration binding expression, when the source gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ExprId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<ExprId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<ExprId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<ExprId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal: Option<ExprId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed: Option<bool>,
    /// Per-element fixed attributes when they are not uniform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_elements: Option<Vec<bool>>,
    #[serde(default)]
    pub evaluable: bool,
    #[serde(default)]
    pub held: bool,
    #[serde(default)]
    pub state_select: RbcStateSelect,
    #[serde(default)]
    pub tunable: bool,
    /// True when the variable was written in the source rather than generated.
    #[serde(default)]
    pub from_source: bool,
    /// Connector semantics recovered from flattening, when this variable is a
    /// connector member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<RbcConnectorMember>,
}

/// Modelica connector semantics for one variable.
///
/// A `flow` member obeys a sum-to-zero conservation law; a `potential` member
/// is equated across a connection. Preserving this distinction is why
/// connections are not modelled as directional messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcConnectorMember {
    pub quantity: RbcQuantityKind,
    /// True when this member participates in at least one `connect(...)`.
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcConnectorType {
    pub id: u32,
    pub name: String,
    pub members: Vec<RbcConnectorField>,
    pub flow_convention: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcConnectorField {
    pub name: String,
    pub scalar_type: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub quantity: String,
    pub kind: RbcQuantityKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcConnectorInstance {
    pub id: u32,
    pub path: String,
    pub owner: ComponentId,
    pub type_id: u32,
    pub orientation: String,
    pub members: Vec<RbcConnectorFieldBinding>,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcConnectorFieldBinding {
    pub name: String,
    pub variable: VariableId,
    pub kind: RbcQuantityKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcQuantityKind {
    /// Equated across a connection (voltage, angle, temperature).
    Potential,
    /// Conserved across a connection; signed sum is zero (current, torque).
    Flow,
    /// Stream variable (MLS §15).
    Stream,
}

// ── Components and connections ───────────────────────────────────────────────

/// One node of the connection graph: everything `connect(...)` joined together.
///
/// A `connect` is written pairwise, but the object it creates is n-ary. Three
/// pins wired to one node share **one** potential and **one** conservation law,
/// and the flow balance over them is not expressible as three pairs. MLS §9.2
/// calls this the connection set, and it is the unit every question worth
/// asking about a network is asked about: what is equated here, what is
/// conserved here, and which components meet here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcConnectionSet {
    pub id: ConnectionSetId,
    /// Connector instances joined at this node, e.g. `["L.n", "Ro.p"]`.
    pub connectors: Vec<String>,
    /// Potential members, all equal at this node.
    pub potentials: Vec<VariableId>,
    /// The conservation laws this node asserts, one per flow member kind.
    ///
    /// Plural because a connector may declare several flow members: a
    /// MultiBody frame conserves a force *and* a torque at the same node.
    /// They are separate sums and must stay separate --- a check that treated
    /// them as one reported a force and a torque as a unit mismatch, which is
    /// the node's structure being wrong rather than the model's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub balances: Vec<RbcFlowBalance>,
    /// The DAE equations carrying the potential equalities of this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub potential_equations: Vec<EquationId>,
    /// A connector nothing was connected to, whose flow MLS §9.2 sets to zero.
    ///
    /// Not a defect on its own --- a model compiled standalone has unconnected
    /// ports by construction --- but the distinction between "conserved among
    /// several" and "forced to zero alone" is one a reader needs, and it is
    /// invisible once both are just equations.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconnected: bool,
    pub provenance: RbcProvenance,
}

/// One conservation law at a node: a signed sum of flow members, equal to zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcFlowBalance {
    /// The DAE equation carrying it, when it could be paired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equation: Option<EquationId>,
    pub terms: Vec<RbcFlowTerm>,
}

/// One flow member of a connection set, with the sign the balance gives it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RbcFlowTerm {
    pub variable: VariableId,
    /// Whether the balance subtracts this term. Outflow from one component is
    /// inflow to another, and the sign is what makes the sum mean anything.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub negated: bool,
}

/// One component instance, identified by its flattened path prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcComponent {
    pub id: ComponentId,
    /// Dotted instance path, e.g. `"motor"` or `"drive.motor"`. Empty for the
    /// top-level model itself.
    pub path: String,
    /// Fully qualified class this instance is of, when flattening recorded it.
    ///
    /// Without this a consumer can see that a node joins `L.n` to `Ro.p` and
    /// cannot see that it joins an inductor to a resistor, which is most of
    /// what a connection graph is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
}

/// One `connect(...)` relationship recovered from flattening.
///
/// A connection is an equality between two connector endpoints, plus the
/// conservation law over the flow members of the connection set. It is **not**
/// a directional message: `left` and `right` are symmetric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcConnection {
    pub id: ConnectionId,
    /// Endpoint variable on one side.
    pub left: VariableId,
    /// Endpoint variable on the other side.
    pub right: VariableId,
    /// Physical role of the quantity being connected.
    pub quantity: RbcQuantityKind,
    /// Connector instance paths, e.g. `("battery.pin", "motor.pin")`.
    pub left_connector: String,
    pub right_connector: String,
    /// Equation this connection produced, when one was generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equation: Option<EquationId>,
    pub provenance: RbcProvenance,
}

// ── Expressions ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcExpr {
    pub id: ExprId,
    pub value_type: TypeId,
    pub node: RbcExprNode,
    pub provenance: RbcProvenance,
}

/// How a clock conversion derives its target clock (MLS §16.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcClockTransferKind {
    SubSample { factor: i64 },
    SuperSample { factor: i64 },
    ShiftSample { counter: i64, resolution: i64 },
    BackSample { counter: i64, resolution: i64 },
}

/// Expression node. Operands always reference nodes with a **lower** [`ExprId`],
/// so the arena is a DAG in topological order and can be evaluated in one pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcExprNode {
    /// MLS §16.5.2 clock conversion: the last value of `source` on
    /// `source_clock`, carried onto `target_clock`, which must be exactly the
    /// clock `transfer` derives from the source clock (the DAE re-checks it).
    ClockTransfer {
        transfer: RbcClockTransferKind,
        source: ExprId,
        source_clock: ClockId,
        target_clock: ClockId,
    },
    /// The predefined MLS scalar-to-String operation, never an arbitrary call
    /// whose display name happens to be String.
    StringConversion {
        value: ExprId,
        format: RbcStringConversionFormat,
    },
    Literal {
        value: RbcLiteral,
    },
    Coordinate {
        coordinate: RbcCoordinate,
    },
    Unary {
        op: RbcUnaryOp,
        operand: ExprId,
    },
    Binary {
        op: RbcBinaryOp,
        lhs: ExprId,
        rhs: ExprId,
    },
    /// `if c1 then v1 elseif c2 then v2 else fallback`. Branches are evaluated
    /// in order; `fallback` is required, so the node is total.
    Conditional {
        branches: Vec<RbcBranch>,
        fallback: ExprId,
    },
    /// A pure built-in call: `sqrt(x)`, `log(x)`, `abs(x)`, `min(a, b)`, ...
    ///
    /// `name` is the Modelica spelling in lower case, so a consumer matches on
    /// `"sqrt"` rather than on an enum ordinal that could shift.
    Builtin {
        name: String,
        arguments: Vec<ExprId>,
    },
    /// An array construction `{a, b, c}`. An empty array carries its element
    /// type, which cannot be inferred from zero elements.
    Array {
        elements: Vec<ExprId>,
        /// Set only when `elements` is empty.
        empty_type: Option<TypeId>,
    },
    /// A record construction, one operand per field in declaration order.
    Record {
        ty: TypeId,
        fields: Vec<ExprId>,
    },
    /// `base.field`, by field ordinal in the record's declaration order.
    Field {
        base: ExprId,
        field: u32,
    },
    /// `start:step:stop`. The bounds are Integer literals by construction.
    Range {
        start: ExprId,
        step: Option<ExprId>,
        stop: ExprId,
    },
    /// `{body for i in ...}` over an iteration domain.
    Comprehension {
        domain: DomainId,
        body: ExprId,
    },
    /// `base[s1, s2, ...]`.
    Index {
        base: ExprId,
        subscripts: Vec<RbcSubscript>,
    },
    /// A functional array update: `base` with `subscripts` replaced by `value`.
    ArrayUpdate {
        base: ExprId,
        value: ExprId,
        subscripts: Vec<RbcSubscript>,
    },
    /// One result projection of a function call.
    ///
    /// `owner` is the first projection of this call occurrence, and equals the
    /// node's own id for a single-output call. Two projections of one call
    /// share it, so a consumer knows `(a, b) = f(x)` is one evaluation of `f`
    /// and not two.
    Call {
        owner: ExprId,
        function: FunctionId,
        output: u32,
        arguments: Vec<ExprId>,
    },
    /// Read a value defined inside a function body.
    ///
    /// Representable only because bodies are now carried: the value this
    /// names is one of `RbcFunction::body`'s definitions, addressed by the
    /// owner-local ordinal the DAE uses. While bodies were elided there was
    /// nothing for it to refer to, which is why it was `Unsupported` -- and
    /// why 97% of unrepresentable nodes across eleven libraries were this
    /// one form.
    FunctionValue {
        function: FunctionId,
        /// Owner-local ordinal of the value.
        value: u32,
        /// Owner-local ordinal of the SSA definition that produced it.
        definition: u32,
    },
    /// Read a fold's carried parameter at the current iteration.
    FunctionFoldParameter {
        function: FunctionId,
        /// Owner-local ordinal of the fold.
        fold: u32,
        /// Position within the carried tuple.
        carried: u32,
        definition: u32,
    },
    /// Read a fold's carried output after the last iteration.
    FunctionFoldOutput {
        function: FunctionId,
        fold: u32,
        carried: u32,
        definition: u32,
    },
    /// A node this schema version cannot represent. A consumer must treat the
    /// containing model as not fully understood rather than assume a default.
    /// Producers only emit this when explicitly asked to tolerate gaps.
    Unsupported {
        detail: String,
    },
}

/// One subscript position of an index or array-update node.
///
/// `Index` selects a single element and drops the dimension; `Slice` selects
/// several and keeps it; `Whole` is a bare `:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcSubscript {
    Index { expression: ExprId },
    Whole,
    Slice { expression: ExprId },
}

impl RbcSubscript {
    /// The expression this subscript evaluates, if any. `Whole` evaluates none.
    pub fn expression(self) -> Option<ExprId> {
        match self {
            Self::Index { expression } | Self::Slice { expression } => Some(expression),
            Self::Whole => None,
        }
    }
}

/// One `condition -> value` arm of a conditional expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcBranch {
    pub condition: ExprId,
    pub value: ExprId,
}

/// One MLS Appendix B.1c definition owner: a set of discrete-valued targets
/// defined together, and the branches that give them values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcDiscreteDefinition {
    #[serde(default)]
    pub observed: bool,
    /// The discrete-valued variables this owner defines, in the order the
    /// branches' `values` follow.
    pub targets: Vec<VariableId>,
    pub branches: Vec<RbcDiscreteBranch>,
    pub provenance: RbcProvenance,
}

/// One activation branch of a B.1c definition. `values` has exactly one entry
/// per target of the owning definition, in the same order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcDiscreteBranch {
    pub activation: RbcDiscreteActivation,
    pub values: Vec<ExprId>,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcDiscreteActivation {
    /// A plain equation, active whenever the model is: `b = x < 0.5`.
    Always,
    /// A `when` branch, active on its trigger under its guard.
    When {
        trigger: ConditionId,
        guard: ConditionId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcLiteral {
    Real {
        value: f64,
    },
    Integer {
        value: i64,
    },
    /// An enumeration value, carried as its 1-based ordinal (MLS §4.9.5).
    ///
    /// This is distinct from `Integer` even though both hold an integer: the
    /// DAE's type checker demands an `Enumeration` where the declaration says
    /// enumeration, and rebuilding one from an `Integer` literal is rejected
    /// with `expected Enumeration, found Integer`.
    Enumeration {
        ordinal: i64,
    },
    Boolean {
        value: bool,
    },
    String {
        value: String,
    },
}

/// A leaf referencing a model quantity. This is how expressions name
/// variables — never by string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcCoordinate {
    Parameter {
        variable: VariableId,
    },
    Input {
        variable: VariableId,
    },
    State {
        variable: VariableId,
    },
    /// `der(x)` for state `variable`.
    Derivative {
        variable: VariableId,
    },
    Algebraic {
        variable: VariableId,
    },
    DiscreteReal {
        variable: VariableId,
    },
    DiscreteValue {
        variable: VariableId,
    },
    PreState {
        variable: VariableId,
    },
    PreAlgebraic {
        variable: VariableId,
    },
    PreDiscreteReal {
        variable: VariableId,
    },
    PreDiscreteValue {
        variable: VariableId,
    },
    /// Simulation time.
    Time,
    /// A family's iteration binder — the `i` in `x[i] = i`.
    ///
    /// Without this a family body cannot be expressed, so exporting families
    /// without it produced artifacts that failed their own import.
    Binder {
        domain: DomainId,
        ordinal: u32,
    },
    /// A condition's value used inside an expression, such as `initial()`.
    Condition {
        condition: ConditionId,
    },
    /// A function's formal parameter, read from inside that function's body.
    /// `interval(c)` of a periodic clock.
    ClockInterval {
        clock: ClockId,
    },
    /// The value a `delay` owner produces.
    Delay {
        delay: DelayId,
    },
    /// The value a `previous` owner reads.
    Previous {
        previous: PreviousId,
    },
    /// `terminal()`.
    Terminal {
        terminal: TerminalId,
    },
    FunctionParameter {
        function: FunctionId,
        ordinal: u32,
    },
}

impl RbcCoordinate {
    /// The variable this coordinate reads, if any. `Time` reads none.
    pub fn variable(self) -> Option<VariableId> {
        match self {
            Self::Parameter { variable }
            | Self::Input { variable }
            | Self::State { variable }
            | Self::Derivative { variable }
            | Self::Algebraic { variable }
            | Self::DiscreteReal { variable }
            | Self::DiscreteValue { variable }
            | Self::PreState { variable }
            | Self::PreAlgebraic { variable }
            | Self::PreDiscreteReal { variable }
            | Self::PreDiscreteValue { variable } => Some(variable),
            Self::Time
            | Self::Binder { .. }
            | Self::Condition { .. }
            | Self::FunctionParameter { .. }
            | Self::ClockInterval { .. }
            | Self::Delay { .. }
            | Self::Previous { .. }
            | Self::Terminal { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcUnaryOp {
    Negate,
    Not,
    /// MLS §3.4 unary plus, the identity.
    ///
    /// Carried rather than dropped because it reaches the DAE intact whenever
    /// constant folding does not consume it — `parameter SI.Voltage Vps=+15`
    /// under `--no-fold-parameter-bindings`. Encoding it as its operand is not
    /// possible here: expressions are addressed by index and each node must
    /// occupy its own, so the alternative to a variant is failing the export.
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    /// MLS §10.6 element-wise operators (`.+ .- .* ./ .^`). Distinct from
    /// their scalar spellings because on arrays they mean something else:
    /// `A * B` is a matrix product, `A .* B` is not.
    ElementwiseAdd,
    ElementwiseSubtract,
    ElementwiseMultiply,
    ElementwiseDivide,
    ElementwisePower,
}

// ── Equations ────────────────────────────────────────────────────────────────

/// Identifies one iteration domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DomainId(pub u32);

/// Identifies one function declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FunctionId(pub u32);

/// One iteration binder: `for i in lower:step:upper`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcBinder {
    pub id: u32,
    pub display_name: String,
    pub lower: i64,
    pub upper: i64,
    pub step: i64,
}

/// The iteration domain an equation family ranges over.
///
/// Carried so a family can be *rebuilt*, not merely counted. Without it,
/// import had to refuse any artifact containing a family, because a DAE
/// reconstructed without the domain would be missing those equations and
/// would still validate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcDomain {
    pub id: DomainId,
    pub binders: Vec<RbcBinder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<DomainId>,
    pub extents: Vec<u32>,
    pub scalar_count: u32,
    pub provenance: RbcProvenance,
}

/// One function declaration, and its body when the artifact carries it.
///
/// The signature is what a *call site* needs: which function, how many
/// arguments, what they mean. The body -- SSA definitions, loop transitions,
/// conditionals, or an external interface -- is `body`, with the value
/// table and folds its statements address; `ElidedModelica` records a body
/// that exists and is not here (a recursive function), so an absent body is
/// never mistaken for an empty one.
/// Checked derivative links attached to a function in priority order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcFunctionDerivative {
    pub target: FunctionId,
    pub inputs: Vec<rumoca_core::FunctionDerivativeInput>,
    pub priority: u32,
    pub previous: Option<(FunctionId, u32)>,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RbcFunction {
    pub id: FunctionId,
    /// Fully qualified Modelica name, e.g. `"Modelica.Math.asin"`.
    pub name: String,
    pub parameters: Vec<RbcFunctionParameter>,
    /// Result types in declaration order; a call names one by ordinal.
    pub results: Vec<TypeId>,
    /// The MLS §18.3 `Inline`/`LateInline` request the declaration wrote.
    pub inline: RbcInline,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derivatives: Vec<RbcFunctionDerivative>,
    pub body: RbcFunctionBody,
    /// Output and local values, in owner-local ordinal order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<RbcFunctionValue>,
    /// Bounded loops this body contains, addressed by owner-local ordinal.
    ///
    /// Beside the statements rather than inside them so a nested fold can
    /// name its parent by ordinal, which is how the DAE addresses them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<RbcFunctionFold>,
    /// Functions this one calls, sorted by id.
    ///
    /// The call graph, carried whether or not the bodies are. When a body is
    /// elided -- a recursive function, or an exporter that does not carry
    /// bodies -- the calls inside it are not visible in the arena, and
    /// without this field the graph would stop at the first function.
    /// Consumers that need reachability over callables (dead-code
    /// elimination, coverage, "is this function used") read it here, and
    /// validation checks it acyclic over carried bodies (SPEC §9a).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<FunctionId>,
    pub declaration: RbcProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcFunctionParameter {
    pub name: String,
    pub value_type: TypeId,
    /// Where the parameter was declared. Absent in artifacts written before
    /// bodies were carried; a rebuild then anchors it at the function.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<RbcProvenance>,
}

/// One output or local value of a function, in owner-local ordinal order.
///
/// The table `FunctionValue`, `AssignmentGroup` and the fold transitions
/// address by ordinal. Without it an artifact names value 3 of a function
/// and says nothing about what value 3 *is*, so a reader can print the body
/// but not rebuild it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcFunctionValue {
    pub name: String,
    pub value_type: TypeId,
    pub role: RbcFunctionValueRole,
    pub declaration: RbcProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcFunctionValueRole {
    /// Returned to the caller; its position among outputs is its result index.
    Output,
    /// Internal to the body.
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcPurity {
    Pure,
    #[default]
    Impure,
}

/// One ordered ABI position of an external call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcExternalArgument {
    /// An expression the external body reads, closed over the function's
    /// own parameters.
    Input { expression: ExprId },
    /// An output the external body writes through this position.
    Output { value: u32 },
}

/// Where an external symbol is found. Carried so a rebuilt model links the
/// same code the source named.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcExternalLinkage {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_directory: Option<String>,
}

impl RbcExternalLinkage {
    pub fn is_empty(&self) -> bool {
        self.libraries.is_empty()
            && self.include.is_none()
            && self.include_directory.is_none()
            && self.library_directory.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcInline {
    #[default]
    Unstated,
    Requested,
    Never,
    AfterIndexReduction,
}

/// What kind of body the declaration has, and whether this artifact carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcFunctionBody {
    /// A Modelica body exists but is not in this artifact.
    ///
    /// Still valid, and still the right answer for a producer that cannot
    /// or need not carry bodies. An artifact that elides every body is
    /// total by the argument in SPEC_RUMOCA_BITCODE §9a as originally
    /// written; one that carries them is total by the acyclic call-graph
    /// check over [`RbcFunction::calls`]. Which applies is readable here.
    ElidedModelica,
    /// A Modelica body, in the bounded form the DAE holds.
    ///
    /// Not Modelica statements: by the time a body reaches the DAE it has
    /// been lowered to assignment, grouped assignment, assertion and a fold
    /// over a compact domain. There is no `while`, no `break` and no
    /// `return`, so the body itself cannot express unbounded iteration —
    /// the only route to non-termination is a call cycle, which
    /// `RbcFunction::calls` makes checkable.
    Modelica {
        statements: Vec<RbcFunctionStatement>,
    },
    /// An MLS §12.9 external body, named by language and symbol. Carried
    /// because it is the whole of what the function does: there is no
    /// Modelica body that could be elided.
    External {
        language: String,
        symbol: String,
        /// MLS 3.7 §12.3. A bare external declaration is impure, so that is
        /// the default an artifact written before this field existed reads.
        #[serde(default)]
        purity: RbcPurity,
        /// Ordered ABI positions. Without them the call cannot be rebuilt:
        /// language and symbol say *what* to call, not *with what*.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        arguments: Vec<RbcExternalArgument>,
        /// Output bound by the §12.9 `output = symbol(...)` return form, by
        /// owner-local value ordinal.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<u32>,
        #[serde(default, skip_serializing_if = "RbcExternalLinkage::is_empty")]
        linkage: RbcExternalLinkage,
    },
}

/// MLS assertion severity, independent of compiler-private enum ordinals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcAssertionLevel {
    #[default]
    Error,
    Warning,
}

/// One statement of a lowered function body.
///
/// The four forms `rumoca_ir_dae::FunctionStatementView` distinguishes. A
/// consumer that only needs to *read* a body — reachability, coverage, a
/// constant-folding pass — works from these without reconstructing a DAE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcFunctionStatement {
    /// One SSA definition: `value := expression`.
    Assignment {
        /// Owner-local ordinal of the value this defines.
        value: u32,
        expression: ExprId,
        provenance: RbcProvenance,
    },
    /// An MLS §8.3.7 assertion inside the body.
    Assertion {
        condition: ExprId,
        message: ExprId,
        #[serde(default)]
        level: RbcAssertionLevel,
        provenance: RbcProvenance,
    },
    /// Several values defined together, optionally under a shared branch.
    ///
    /// One statement rather than a run of assignments because the branch
    /// correlation is shared: every value takes its result from the *same*
    /// selected branch. Splitting them would let a consumer pick different
    /// branches for different values, which is a different program.
    AssignmentGroup {
        /// Owner-local ordinals of the values defined, in order.
        values: Vec<u32>,
        /// Absent when the group is unconditional.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        conditional: Option<RbcFunctionConditional>,
        /// One expression per value: the value it takes.
        ///
        /// Carried for conditional groups too. There each is the join of the
        /// branches for that value, which already exists in the arena and
        /// which the expressions after it may read; `conditional` records
        /// that the joins share one branch selection, not a second way of
        /// computing them.
        expressions: Vec<ExprId>,
        provenance: RbcProvenance,
    },
    /// A bounded loop. `fold` is an owner-local ordinal into the function's
    /// fold table, which carries the transition; the body is here, because
    /// that is where the DAE keeps it and a second home would let the two
    /// disagree about what the loop runs.
    For {
        fold: u32,
        statements: Vec<RbcFunctionStatement>,
        /// Where the loop closes; the fold carries where it opens.
        provenance: RbcProvenance,
    },
}

/// A fold over a compact domain: the bounded loop a function body can hold.
///
/// Not a general loop. The domain fixes the trip count before evaluation,
/// exactly as `RbcExprNode::Comprehension` does, which is why carrying a
/// body does not reintroduce unbounded iteration. The four definition
/// groups are the transition: `parameters` bind the binder values,
/// `initial` seeds the carried tuple, `update` produces the next one, and
/// `output` reads the result out after the last iteration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcFunctionFold {
    /// Owner-local ordinal, so a nested fold can name its parent.
    pub ordinal: u32,
    /// The compact domain iterated; fixes the trip count.
    pub domain: DomainId,
    /// Owner-local ordinal of the lexically enclosing fold, when nested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// Values carried from one iteration to the next.
    pub targets: Vec<u32>,
    /// Values defined afresh each iteration: in the transition region, but
    /// neither seeded from nor returned in the carried tuple.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iteration_locals: Vec<u32>,
    pub parameters: Vec<RbcFunctionDefinition>,
    pub initial: Vec<RbcFunctionDefinition>,
    pub update: Vec<RbcFunctionDefinition>,
    pub output: Vec<RbcFunctionDefinition>,
    /// Where the loop opens.
    pub provenance: RbcProvenance,
}

/// One SSA definition inside a fold's transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcFunctionDefinition {
    /// Owner-local ordinal of the value defined.
    pub value: u32,
    pub expression: ExprId,
}

/// Shared branch correlation for one grouped assignment.
///
/// `conditions[i]` selects `branches[i]`; `fallback` is the `else`. Each
/// branch supplies one expression per value in the group, so the arity is
/// checked rather than assumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RbcFunctionConditional {
    pub conditions: Vec<ExprId>,
    pub branches: Vec<Vec<ExprId>>,
    pub fallback: Vec<ExprId>,
}

/// Identifies one structured equation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FamilyId(pub u32);

/// How a family's symbolic body becomes one scalar row (MLS §8.3.2, §11.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcScalarView {
    /// Substitute the binder values into the symbolic body.
    BinderSubstitution,
    /// Select the row-major element from an aggregate body.
    RowMajorProjection,
    /// Substitute leading binders, project the remaining axes.
    BinderPrefixProjection { binder_count: u32 },
}

/// An array or `for` equation, kept in the compact form the DAE holds.
///
/// Expanding it to scalar rows at export would multiply the artifact by the
/// array extent and lose the fact that the rows share one source equation. The
/// alternative that was in place — omitting families entirely — is worse: the
/// artifact then describes fewer equations than the model has, validates
/// cleanly, and any consumer reasoning about solvability reads an incomplete
/// system with no way to detect it. See TOOLBUG-014.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcEquationFamily {
    pub id: FamilyId,
    /// The domain this family ranges over.
    pub domain: DomainId,
    /// Residual bodies, one per equation in the family's body list.
    pub bodies: Vec<ExprId>,
    /// How many scalar equations this stands for. This is the number a
    /// balance or matching analysis must count.
    pub scalar_rows: u32,
    /// Extent of each iteration axis, outermost first.
    pub extents: Vec<u32>,
    pub scalar_view: RbcScalarView,
    /// Variables the family's rows read, from the compiler's own scalar
    /// coordinate projection — the same authority `RbcEquation.reads` uses.
    /// Walking the symbolic body instead misses array and record selections,
    /// which is a missing incidence edge, which is a matching failure a
    /// consumer would report as a defect in the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<VariableId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_derivative: Vec<VariableId>,
    /// Variables whose *left limit* `pre(v)` this equation reads (MLS §3.7.5).
    ///
    /// Kept apart from `reads`, because the two answer different questions. A
    /// dependency analysis wants them together: the equation does depend on
    /// `v`. A *matching* analysis must not see them at all — `pre(v)` is the
    /// value `v` held at event entry, a known, so an equation reading it is
    /// not a candidate to determine `v`. Merged into `reads`, the B.1b
    /// equation `y_last = f(pre(x))` looked able to determine the state `x`,
    /// the matching spent it there, and `y_last` came out unmatched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_previous: Vec<VariableId>,
    pub provenance: RbcProvenance,
}

/// An MLS Appendix B.1b coupled discrete-Real equation.
///
/// These determine the discrete-Real variables — a `sample`d hold, a mean held
/// between events — and live in their own DAE partition, which nothing
/// exported. An artifact was therefore short by exactly one equation per
/// discrete-Real variable, and a consumer counting equations saw a model with
/// more constraints than unknowns: 19 phantom `UNMATCHED_EQUATION`s on
/// `Polyphase.Examples.Utilities.AnalysatorAC` alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcDiscreteRealEquation {
    pub id: EquationId,
    pub residual: ExprId,
    pub activation: RbcDiscreteRealActivation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<VariableId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_derivative: Vec<VariableId>,
    /// Variables whose *left limit* `pre(v)` this equation reads (MLS §3.7.5).
    ///
    /// Kept apart from `reads`, because the two answer different questions. A
    /// dependency analysis wants them together: the equation does depend on
    /// `v`. A *matching* analysis must not see them at all — `pre(v)` is the
    /// value `v` held at event entry, a known, so an equation reading it is
    /// not a candidate to determine `v`. Merged into `reads`, the B.1b
    /// equation `y_last = f(pre(x))` looked able to determine the state `x`,
    /// the matching spent it there, and `y_last` came out unmatched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_previous: Vec<VariableId>,
    pub provenance: RbcProvenance,
}

/// When a B.1b equation holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcDiscreteRealActivation {
    /// Holds continuously, like a B.1a residual.
    Always,
    /// Holds while `guard` is active, re-evaluated when `trigger` rises.
    When {
        trigger: ConditionId,
        guard: ConditionId,
    },
}

/// One discrete-valued variable's value at the initialization instant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcInitialDiscreteValue {
    pub target: VariableId,
    pub value: ExprId,
    pub provenance: RbcProvenance,
}

/// A residual equation: the model asserts `residual == 0`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcEquation {
    pub id: EquationId,
    pub residual: ExprId,
    pub provenance: RbcProvenance,
    /// Variables this equation reads, precomputed by the compiler's own
    /// dependency projection. A consumer must not have to re-derive these by
    /// walking expressions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<VariableId>,
    /// States whose derivative this equation reads, i.e. `der(x)` occurrences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_derivative: Vec<VariableId>,
    /// Variables whose *left limit* `pre(v)` this equation reads (MLS §3.7.5).
    ///
    /// Kept apart from `reads`, because the two answer different questions. A
    /// dependency analysis wants them together: the equation does depend on
    /// `v`. A *matching* analysis must not see them at all — `pre(v)` is the
    /// value `v` held at event entry, a known, so an equation reading it is
    /// not a candidate to determine `v`. Merged into `reads`, the B.1b
    /// equation `y_last = f(pre(x))` looked able to determine the state `x`,
    /// the matching spent it there, and `y_last` came out unmatched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads_previous: Vec<VariableId>,
}

// ── Events ───────────────────────────────────────────────────────────────────

/// A primitive comparison that can generate an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcRelation {
    pub id: RelationId,
    pub expression: ExprId,
    pub provenance: RbcProvenance,
}

/// Boolean activation condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcCondition {
    pub id: ConditionId,
    pub node: RbcConditionNode,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcConditionNode {
    /// True during initialization only.
    Initial,
    /// Always active; the section is not a `when` at all.
    Always,
    Relation {
        relation: RelationId,
    },
    Discrete {
        expression: ExprId,
    },
    Not {
        operand: ConditionId,
    },
    And {
        lhs: ConditionId,
        rhs: ConditionId,
    },
    Or {
        lhs: ConditionId,
        rhs: ConditionId,
    },
    /// MLS §8.3.5 vector activation: fires when any element rises.
    AnyRise {
        lhs: ConditionId,
        rhs: ConditionId,
    },
    /// Activation of one clock in the exact schedule table.
    ClockActivation {
        clock: ClockId,
    },
    Unsupported {
        detail: String,
    },
}

/// Optional operands of the predefined String conversion (MLS §3.7.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcStringConversionFormat {
    Options {
        minimum_length: Option<ExprId>,
        left_justified: Option<ExprId>,
        significant_digits: Option<ExprId>,
    },
    Format {
        value: ExprId,
    },
}

impl RbcStringConversionFormat {
    pub fn operands(&self) -> Vec<ExprId> {
        match self {
            Self::Options {
                minimum_length,
                left_justified,
                significant_digits,
            } => [*minimum_length, *left_justified, *significant_digits]
                .into_iter()
                .flatten()
                .collect(),
            Self::Format { value } => vec![*value],
        }
    }
}

/// Exact clock rational. Decimal strings preserve 128-bit integers in both
/// CBOR and JSON, including clients whose JSON numbers use floating point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcClockRational {
    pub numerator: String,
    pub denominator: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RbcClockAnchor {
    Absolute,
    SimulationStart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcClockNode {
    Shifted {
        base: ClockId,
        counter: u32,
        condition: ConditionId,
    },
    Periodic {
        period: RbcClockRational,
        phase: RbcClockRational,
        anchor: RbcClockAnchor,
    },
    Triggered {
        condition: ConditionId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcClock {
    pub id: ClockId,
    pub node: RbcClockNode,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcClockOwnership {
    pub variable: VariableId,
    pub clock: ClockId,
    pub sampled: bool,
    pub provenance: RbcProvenance,
}

/// A zero-crossing surface the solver monitors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcRoot {
    pub id: RootId,
    pub relation: RelationId,
    pub activation: ConditionId,
    pub provenance: RbcProvenance,
}

/// Something the model does when an event fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcEventAction {
    pub id: EventId,
    pub trigger: ConditionId,
    pub guard: ConditionId,
    pub action: RbcAction,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcAction {
    /// Report when the condition is false, without introducing a state event.
    Warning {
        condition: ExprId,
        message: ExprId,
    },
    /// `reinit(state, value)`.
    Reinitialize {
        state: VariableId,
        value: ExprId,
    },
    Assert {
        message: ExprId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<ExprId>,
    },
    Terminate {
        message: ExprId,
    },
}

/// A scheduled event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcTimeEvent {
    pub id: EventId,
    pub schedule: RbcSchedule,
    pub provenance: RbcProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RbcSchedule {
    /// Fires at a fixed instant, as an exact rational `numerator/denominator`.
    Static { numerator: i64, denominator: i64 },
    /// Fires at a time computed by an expression.
    Dynamic { deadline: ExprId },
}

// ── Instrumentation ──────────────────────────────────────────────────────────

/// A request to observe a value at runtime.
///
/// Trace points are **observation metadata**, deliberately separate from the
/// physical equations. Adding one does not change model semantics, which is why
/// an instrumentation pass does not need to rewrite equations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbcTracePoint {
    pub id: TracePointId,
    /// The variable to observe.
    pub variable: VariableId,
    /// Human-readable label for the trace output.
    pub label: String,
    /// Connection this observation belongs to, when it came from connector
    /// instrumentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionId>,
    /// Connection *set* --- the node --- this observation belongs to.
    ///
    /// Separate from `connection` because they answer different questions: a
    /// pairwise connect is an edge, and the node is where a conservation law
    /// lives. An observation of a flow member belongs to the node, and
    /// recording it against one of the edges that happen to form that node
    /// would name an arbitrary one of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_set: Option<ConnectionSetId>,
    /// Physical role of the observed quantity, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<RbcQuantityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Tool that added this trace point, for provenance of the instrumentation
    /// itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<String>,
}

impl RbcFile {
    /// Reject a file that is not RBC, or is a version this build cannot read.
    pub fn check_header(&self) -> Result<(), String> {
        if self.magic != RBC_MAGIC {
            return Err(format!(
                "not a Rumoca Bitcode file: magic is {:?}, expected {RBC_MAGIC:?}",
                self.magic
            ));
        }
        if self.bitcode_version != RBC_VERSION {
            return Err(format!(
                "unsupported bitcode version {}: this build reads version {RBC_VERSION}",
                self.bitcode_version
            ));
        }
        Ok(())
    }
}

# Rumoca Notes (Milestone 0)

Orientation notes for the Rumoca analysis/pass and world-model project.

**Scope.** What the four IRs contain, how code generation works, how provenance
is carried, and which existing Rumoca facilities the pass framework should
build on rather than reimplement. Every claim below was checked against
`rumoca 0.10.0` at commit `97eb3ab7` and, where marked *(verified)*, reproduced
by running the compiler.

**Status of the tree.** `cargo check --workspace` passes. `cargo build -p rumoca
--bin rumoca` produces a working CLI. The clone is a fork of
`cognipilot/rumoca` sitting exactly on upstream `main` with no fork-specific
commits.

---

## 0. Reproducing everything here

```bash
cargo build -p rumoca --bin rumoca

# Source-root discovery loads every .mo beside the target file, so compile
# from a directory containing only files that resolve without MSL.
mkdir -p /tmp/m0 && cp examples/models/{SympyDecay,Ball,Circuit,SwitchedRLC}.mo /tmp/m0/

./target/debug/rumoca compile /tmp/m0/SympyDecay.mo --model SympyDecay --emit ast-json
./target/debug/rumoca compile /tmp/m0/SympyDecay.mo --model SympyDecay --emit flat-mo
./target/debug/rumoca compile /tmp/m0/SympyDecay.mo --model SympyDecay --emit dae-mo
./target/debug/rumoca compile /tmp/m0/SympyDecay.mo --model SympyDecay --emit dae-json
./target/debug/rumoca compile /tmp/m0/SympyDecay.mo --model SympyDecay --emit solve-json
./target/debug/rumoca compile /tmp/m0/Circuit.mo   --model Circuit.Test --inspect structure
./target/debug/rumoca targets
```

`--emit` accepts `ast-json`, `flat-mo`, `flat-json`, `dae-mo`, `dae-json`,
`solve-json`. There is deliberately no `solve-mo`: Solve IR has no Modelica form.

---

## 1. The pipeline at a glance

```text
Modelica source (.mo)
   │  rumoca-phase-parse
   ▼
  AST ─────────────── rumoca-ir-ast
   │  rumoca-phase-resolve → typecheck → instantiate → flatten
   ▼
  Flat ────────────── rumoca-ir-flat
   │  rumoca-phase-dae
   ▼
  DAE ─────────────── rumoca-ir-dae      ◄── canonical semantic contract
   │  rumoca-phase-structural, rumoca-phase-solve
   ▼
  Solve ───────────── rumoca-ir-solve    ◄── executable contract
   │
   ├── rumoca-sim / rumoca-solver-*      (simulation)
   └── rumoca-phase-codegen + targets    (code generation)
```

The normative statements are `spec/SPEC_0007_IR_PIPELINE.md` (stage contracts),
`spec/SPEC_0031_COMPILER_PHILOSOPHY.md` (layering), and
`spec/SPEC_0029_CRATE_BOUNDARIES.md` (crate tiers).

The governing rule is **Modelica semantics end at DAE generation.** Structural
analysis and Solve lowering add no language meaning; they only make the same
system executable. That is precisely why DAE is the right default level for our
analyses.

---

## 2. AST — `rumoca-ir-ast`

**What it is.** Concrete syntax: classes, components, equations, statements,
comments, and a `Span` on every node.

**What `--emit ast-json` actually gives you.** Not a bare parse tree — the
*resolved* tree (`CompilationResult::resolved`, a `ResolvedTree`). Top-level
keys *(verified)*:

```text
definitions   classes, each with def_id, scope_id, class_type, tokens
type_table
scope_tree
def_map
name_map
scope_to_class
source_map
```

Each token carries both line/column **and** byte offsets:

```json
{"start_line": 1, "start_column": 7, "end_line": 1, "end_column": 11,
 "start": 6, "end": 10, "source": 8941929140670131274}
```

**Use it for.** Source-level linting, formatting, documentation — anything that
depends on how the model was *written*. `rumoca-tool-lint` works here.

**Do not use it for.** Anything mathematical. No types, no instances, no
equation semantics.

---

## 3. Flat — `rumoca-ir-flat`

**What it is.** The instantiated hierarchy collapsed to a single flat model:
one variable list with fully-qualified dotted names, one equation list,
`extends` inlined, modifications applied, `connect` expanded.

**What is still present.** `der(...)`, `pre(...)`, `initial()` as expression
nodes; structured function bodies; arrays still symbolic (not scalarized).
Flat is the last level where the model still looks like Modelica.

**Shape.** `rumoca_ir_flat::Model` is a plain public `serde` struct with
`IndexMap`-backed fields — no lifetime branding, no opaque constructor. Easy to
consume. Key fields: `variables: VarNameIndexMap<Variable>`, `effective_types`,
`type_ids_by_def_id`, `record_types`, plus a `visitor.rs` traversal helper.

**Verified — connection expansion.** `Circuit.Test` flattens to:

```modelica
class Circuit_Test
  Real src.p.v(start = 0.0) "voltage";
  ...
  parameter Real res.R(start = 2.5) = 2.5 "Resistance";
equation
  src.v = (src.p.v - src.n.v);      // from TwoPin, inlined per instance
  res.v = (res.R * res.i);
  cap.i = (cap.C * der(cap.v));
  0 = (src.p.i + res.p.i);          // flow sum from connect(...)
  0 = ((cap.n.i + src.n.i) + gnd.p.i);
  src.p.v = res.p.v;                // potential equality from connect(...)
end Circuit_Test;
```

**Use it for.** Analyses that need Modelica-level intent: which component a
variable came from, whether something was a `connect`, what the author wrote
before operator lowering.

---

## 4. DAE — `rumoca-ir-dae` (the important one)

**What it is.** The computable MLS Appendix B canonical system: pure functions
over `v = [p; t; ẋ; x; y; z; m; pre(z); pre(m)]`, with the four B.1 functions
`fx` (continuous residual), `fz` (coupled discrete Real residual), `fm`
(solved discrete assignment), `fc` (event conditions).

**Verified — the residual transformation.** Source `der(x) = -k*x;` becomes:

| Stage | Form |
|---|---|
| Source | `der(x) = -k*x;` |
| Flat | `der(x) = (-(k * x));` |
| DAE | `0.0 = (der(x) - (-(k * x)));` |

Equality equations become residuals. `der(x)` is a first-class *derivative
coordinate*, not a function call.

### 4.1 How you read a DAE

`Dae` is opaque and **valid by construction**. There are no public fields, no
builder, no whole-root validator. All reads go through a generatively branded
view:

```rust
let summary = dae.inspect(|view: DaeView<'_>| {
    let mut states = 0;
    for (_, variable) in view.variables() {
        if variable.role() == VariableRole::State { states += 1; }
    }
    states
});
```

**This is the single most important constraint on the pass framework.** The
`'dae` brand prevents any view, id, or borrow from escaping the closure — that
is enforced by `compile_fail` doctests in `rumoca-ir-dae/src/lib.rs`, and it
also prevents mixing ids between two different `Dae` values. Consequences:

- `AnalysisPass<Dae>::Output` must be **owned data**. It cannot contain
  `VariableId<'dae>`, `ExprId<'dae>`, `DaeView<'dae>`, or anything borrowed
  from the DAE.
- Our graph node/edge types must carry owned identity (`VarName`, role, index,
  resolved span), computed inside `inspect`.
- Cross-model comparison (Milestone 5) *cannot* use branded ids at all — two
  `Dae` values have incompatible brands by design. This is the type system
  enforcing the spec's own warning about not assuming numeric IDs are stable.

### 4.2 Variable model

```rust
enum VariableRole { Parameter, Constant, Input, State, Algebraic,
                    Output, DiscreteReal, DiscreteValue }
enum VariableCausality { Input, Output, Parameter, CalculatedParameter,
                         Independent, Local }
enum VariableOrigin { Source, Generated }
```

Per variable, `VariableAttributes` carries `component_ref` (with `DefId`),
`binding`, `start`, `fixed`, `min`, `max`, `nominal`, `unit`, `state_select`,
`description`, `causality`, `is_tunable`, `is_held`, `origin`.

Role and causality are **orthogonal**: role is the Appendix B partition,
causality is the interface annotation. Milestone 6's "inputs/outputs" come from
causality; "states/algebraics/parameters" come from role.

`VariableView::scalar_count()` and `scalar_name(flat_index)` handle arrays —
a DAE variable may be a tensor, and scalarization is a *view*, not a rewrite.

### 4.3 Expression arena

One DAE-wide dense arena, with parallel columns for nodes, provenance, and
types *(verified — `storage.expressions` has `nodes`, `provenance`, `operands`,
`subscripts`)*. Leaves are typed coordinates, never names:

```rust
enum CoordinateInput<'dae> {
    Parameter, Input, State, Derivative, Algebraic,
    DiscreteReal, DiscreteValue,
    PreDiscreteReal, PreDiscreteValue, PreState, PreAlgebraic,
    Time, ClockInterval, Condition, Previous, Terminal, FunctionParameter,
}
```

`der(x)` is `Coordinate::Derivative(state 0)`; `k` is
`Coordinate::Parameter(1)`. **No name lookup is ever required to find a
dependency.**

### 4.4 Event model

Exactly the structure Pass 4 needs, in four layers:

```text
relations        leaf comparisons, e.g. `x < 0`
   ↓
roots            zero-crossing surfaces: {relation, activation condition}
   ↓
conditions       boolean algebra: Initial | Always | Relation | Discrete
                 | Clock | Not | And | Or | AnyRise
   ↓
event_actions    {trigger, guard, Assert | Terminate | Reinitialize{state, value}}

time_events      Static(rational instant) | Dynamic(deadline expression)
```

**Verified — `Ball.mo`** (`when x < 0 then reinit(v, -0.8*pre(v))`) produces
exactly one relation (`x < 0`), one condition (`Relation(0)`), one root
(`{relation 0, activation 0}`, origin `generated: condition_lowering`), and one
event action (`Reinitialize { state: 1 /* v */, value: expr 17 }`).

**Verified — `SwitchedRLC.mo`** (`Vs = if time > 0.5 then Vb else 0`) produces
`time_events = 1` and **zero** relations. A time-dependent switch is a
*scheduled* event, not a state-crossing root. Pass 4 must report both kinds.

### 4.5 What DAE deliberately does *not* contain

Mass matrices, Jacobians, BLT orderings, tearing choices, state-selection
reports, scalarized variants. Those are structural results or Solve artifacts.
Do not look for them on the DAE.

---

## 5. Solve — `rumoca-ir-solve`

**What it is.** Typed register-machine programs plus tensor kernels. Unlike
DAE, `SolveProblem` is a plain public struct: `layout`, `solve_layout`,
`continuous`, `initialization`, `discrete`, `events`, `clocks`.

**Verified — `der(x) = -k*x` at Solve level:**

```json
"layout": { "bindings": { "x": {"Y": {"index": 0}}, "k": {"P": {"index": 0}} } }

"derivative_rhs": [ {"ScalarPrograms": {"programs": [[
  {"LoadP": {"dst":0,"index":0}},
  {"LoadY": {"dst":1,"index":0}},
  {"Binary":{"dst":2,"op":"Mul","lhs":0,"rhs":1}},
  {"Unary": {"dst":3,"op":"Neg","arg":2}},
  {"StoreOutput":{"src":3}}
]]}} ]
```

Names are gone; variables are `Y[]`/`P[]` slots recoverable only through
`layout.bindings`. `program_spans` still carries provenance.

**Rule of thumb.** Analyse at Solve only when the question is genuinely about
execution (register pressure, kernel selection, sparsity of the emitted
program). Every dependency/structure/event question is cheaper and more precise
at DAE.

---

## 6. Structural analysis — `rumoca-phase-structural`

This sits between DAE and Solve and already computes much of what Pass 2 wants.

```rust
pub struct Incidence<'dae> {
    pub n_eq: usize,
    pub n_var: usize,
    pub eq_unknowns: IncidenceRows,        // CSR bipartite equation → unknowns
    pub unknowns: Vec<UnknownId<'dae>>,
    pub unknown_spans: Vec<Span>,
    pub equation_refs: Vec<EquationRef>,
    pub equation_spans: Vec<Span>,
}

pub enum UnknownId<'dae> {
    Derivative { state, scalar }, Algebraic { variable, scalar },
    Solver(usize), Unmatched { equation },
}
```

Also exported: `BltBlock`, `SortedDae`, `AlgebraicLoop`, `TearingResult`,
`StructuralDiagnostics`, `analyze`, `sort`, `build_blt_from_incidence`,
`tear_algebraic_loop`.

**Important limitation.** `Incidence` relates equations to *unknowns* — states'
derivatives and algebraics. It does **not** include parameters, inputs, or
discrete variables, because matching does not need them. Its
`resolve_coordinate` explicitly returns `None` for `CoordinateView::Parameter`,
`Input`, `FunctionParameter`, and `pre()` reads
(`incidence.rs:311-346`). Our dependency graph does need them (the project
spec's own example has `V`, `R` → `equation_1`), so `Incidence` alone is
insufficient. See §7.

**Verified — `rumoca compile Circuit.mo --model Circuit.Test --inspect structure`:**

```text
structure: 20 equations, 20 unknowns | 20 BLT block(s), 0 coupled (largest 1)
  [   0] scalar   src.v <- f_x[3] (v = V)
  [   9] scalar   res.i <- f_x[8] (v = R*i)
  [  18] scalar   der(cap.v) <- f_x[12] (i = C*der(v))
```

This is effectively a prototype of the analysis we are being asked to
formalise: it already joins matching, BLT order, and source snippets — but it
prints text instead of returning structured data, and it is wired directly into
the CLI rather than being a reusable pass.

---

## 7. The dependency primitive we should build on

`rumoca-eval-dae` exposes the exact incidence proof, and it is a better
foundation than either `Incidence` or a hand-written expression walker:

```rust
pub fn for_each_scalar_coordinate<'dae>(
    view: DaeView<'dae>,
    root: ExprId<'dae>,
    scalar_index: usize,
    domain_point: Option<(DomainId<'dae>, &[i64])>,
    visit: impl FnMut(CoordinateView<'dae>, usize),
) -> Result<(), ProjectionError>;

// plus a cached variant sharing a ScalarCoordinateProjectionCache across rows
```

Per its doc comment and `SPEC_0040` row `DAE-C18`, it already handles:

- function calls, via exact parameter-scalar/record-field projection summaries;
- structured/tensor equation families over compact domains;
- runtime array selection, reporting *the exact union of every potentially
  selected base scalar plus its subscript dependencies*;
- function folds, following only the requested carried scalar.

It reports **all** coordinate kinds, including `Parameter`, `Input`, `Time`,
and `Pre*` — exactly the ones `Incidence` omits.

> **`DependencyGraphPass` should be a structured wrapper over
> `for_each_scalar_coordinate` plus the DAE's equation owners — not a new
> expression traversal.** Writing our own walker would duplicate a
> conservative-but-exact analysis that the compiler already maintains, and
> would silently get function calls and dynamic subscripts wrong.

This is not speculation about the right layering: `build_incidence` in
`rumoca-phase-structural` is itself implemented on
`for_each_scalar_coordinate_cached` (`incidence.rs:7,245`), discarding the
coordinate kinds matching does not need. Our dependency graph is the same walk
keeping all of them.

For raw expression walks where no scalar projection is needed,
`rumoca-ir-dae::expr_query` provides `for_each_expression`,
`for_each_expression_pruned`, `expr_refers_to_var`, `expr_contains_der_of`, and
a reusable `ExpressionTraversal` workspace.

For graph algorithms, `rumoca_core::dependency_graph::dependency_first_sccs`
already provides deterministic SCC computation over index adjacency lists.
SPEC_0029 §3b (single-source helpers) means we reuse it rather than write our
own Tarjan.

---

## 8. Provenance

Provenance is a first-class, non-optional property of the DAE — not a
best-effort annotation.

```rust
pub struct DaeProvenance { origin: DaeProvenanceOrigin, span: ProvenanceSpan }

pub enum DaeProvenanceOrigin { Source, Generated(DaeGeneration) }

pub enum DaeGeneration {           // 24 variants, including:
    BindingEquation, ConnectionEquation, FlowBalanceEquation,
    AlgorithmEquation, DiscreteUpdate, ConditionLowering, PreValueLowering,
    ClockLowering, DelayLowering, TerminalLowering, EventActionLowering,
    InitializationEquation, DefaultStart, ArrayEquationProjection,
    RecordEquationProjection, IndexReduction, AliasElimination, ...
}
```

Construction *refuses a dummy span*: `DaeProvenance::try_new` returns
`MissingProvenance` if the span is not source-backed. Every expression node,
variable declaration, equation, relation, condition, root, event action, and
value type carries one.

The `Dae` also owns its `SourceMap` (`Dae::source_map()` is public), and
`DaeView::source_text(provenance)` returns the original source snippet. So
provenance resolution needs nothing beyond the DAE itself.

To get `file:line:col`, `rumoca-compile` already provides:

```rust
rumoca_compile::compile::source_span_location(&source_map, span)
    -> Option<SourceSpanLocation { file_name, start: TextPosition, end }>
```

**Verified — `SympyDecay.mo` spans resolve exactly:**

| DAE object | Span | Source text |
|---|---|---|
| variable `x` declaration | `19..25` | `Real x` |
| variable `k` declaration | `48..54` | `Real k` |
| expression `der(x)` | `73..79` | `der(x)` |
| expression `k*x` | `83..86` | `k*x` |
| expression `-k*x` | `82..86` | `-k*x` |
| continuous residual #0 | `73..86` | `der(x) = -k*x` |

**Verified — generated equations in `Circuit.Test`.** Of 20 continuous
equations: 13 `source`, 3 `flow_balance_equation`, 4 `connection_equation`.

Two findings that matter for Pass 5 and for the matcher:

1. **Generated connection equations anchor to the connector member
   declaration, not the `connect(...)` statement.** The four connection
   equations all point at `Circuit.mo[33:39]` = `Real v` inside `Pin`, and the
   three flow balances at `Circuit.mo[58:64]` = `Real i`. A naive
   "DAE object → source line" report would say `Pin.v` where a user expects
   `connect(src.p, res.p)`. We should report the generation kind alongside the
   span rather than pretending the span is the whole answer.

2. **A span does not uniquely identify a DAE equation.** `TwoPin`'s
   `v = p.v - n.v` at `Circuit.mo[142:155]` appears three times in the DAE —
   once per instance (`src`, `res`, `cap`), because `extends` inlines it. The
   differential matcher must key on `(instance path, span, generation)`, never
   on span alone.

**Identity available for matching (Milestone 5).**

| Identity | Stability | Notes |
|---|---|---|
| `VarName` | stable text + precomputed segmentation | flattened path, interned in `rumoca-core`; accessors expose segments without string parsing |
| `Span.source` (`SourceId`) | derived from the source *name*, not a slot index | comparable across invocations when file names match |
| `Span.start/end` | byte offsets | shift when unrelated text above changes |
| `DefId`, `InstanceId` | allocation order | deterministic per compile; **not** comparable across two different models |
| Branded `VariableId<'dae>` etc. | brand-local | cannot even be named outside `inspect`; do not attempt |
| `VariableRole`, causality, `unit`, `value_type` | semantic | good matcher signal |

So: match on `VarName` + role + type + unit first, use provenance as
corroboration and as the explanation shown to the user, and never compare
numeric ids across compilations.

---

## 9. Code generation path

```text
proven-valid IR → typed read-only semantic view → target.toml + MiniJinja → artifacts
```

A target is a **directory**, not Rust code:

```toml
version = 1
ir = "dae"                    # ast | flat | dae | solve | fmi | algorithm-code
name = "checked-dae-report-example"
description = "..."
execution_mode = "source-transform"
deployment_class = "text"

[capabilities]
continuous_states = true
events = true
# ... ~20 flags

[[files]]
path = "{{ model_name }}_checked_dae.txt"
template = "checked_dae_report.txt.jinja"
```

Targets may also declare `[package]` (directory layout + optional zip archive),
`[[partials]]` (shared templates), `[[assets]]`, and `[integer]` domains.
`--target` accepts a built-in name, a directory path, or a bare `.jinja` file
(with `--phase` selecting the IR).

**The DAE template projection.** `rumoca-phase-codegen/src/codegen/dae_backend.rs`
turns the checked DAE into a `serde_json::Value` named
`rumoca.checked-dae-template`, currently schema version 5:

```text
schema, value_types, variables, functions, domains, expressions,
systems.{continuous, initialization, discrete_real, discrete_values,
         conditions, events, clocks, temporal}
```

This is explicitly **not** the wire format. There are two distinct DAE JSONs
and they must not be confused:

| | `--emit dae-json` | template projection |
|---|---|---|
| Producer | `Serialize for Dae` | `dae_backend::project` |
| Version | `DAE_SCHEMA_VERSION` = 33 | template schema 5 |
| Purpose | persistence / checked replay | semantic arrays for templates |
| Shape | `{schema_version, source_map, storage{...}}` | `{schema, variables[], systems{...}}` |

**Rendering is forbidden from doing semantic work.** Per SPEC_0007, a template
may not resolve names, infer types or shapes, lower to another IR, mutate its
input, or repair an invalid artifact. Unsupported constructs must fail with a
span-bearing error; `fail_at("unsupported-feature:...", provenance)` is the
primitive, and lossy placeholder text is explicitly prohibited.

**Capability gating is fail-closed** *(verified)*:

```
$ rumoca compile Ball.mo --model Ball --target c-ode
unsupported-feature:events: Target 'c-ode' does not support feature 'events':
event or condition partitions present
```

**Verified — an out-of-tree target works end to end.** Rendering
`examples/codegen/checked_dae_report` (a plain directory outside the codegen
crate) against `SwitchedRLC.mo`:

```text
checked-dae-report 5
model SwitchedRLC
variables 9
0 parameter Vb real unit=V scalars=1
2 parameter R  real unit=Ohm scalars=1
5 state     V  real unit=V scalars=1
6 state     i_L real unit=A scalars=1
7 algebraic i_R real unit=A scalars=1
...
continuous_owners 5
time_events 1
```

**This is the Milestone 6 mechanism, confirmed.** A `world-model` target can be
a directory with `target.toml` + templates emitting `manifest.toml` and
`model.json`, with `ir = "dae"`, and it requires no compiler change.

One known gap: the projection serializes `provenance` as
`{origin, span{source, start, end}}` — byte offsets and a `SourceId` hash, not
`file:line:col`. If the world-model artifact should carry human-readable source
locations (the spec's §13 asks for "source provenance"), we will need either a
template helper that resolves spans through the `SourceMap`, or to accept
offsets in the artifact and resolve them downstream. `rumoca-phase-codegen/src/views/source_trace.rs`
is the existing precedent for span→file resolution in a target (it builds
source legends for GALEC).

There are 17 built-in targets; `rumoca targets` prints the IR, mode,
deployment class, readiness level, and per-feature capability matrix for each.

---

## 10. Crate boundaries and where new work fits

Six tiers, dependencies flow downward, enforced by Cargo (SPEC_0029):

```text
Tier 6 — binaries & bindings: rumoca, bind-python, bind-wasm, contracts
Tier 5 — integration/runtime: codec / input / solver / sim / opt / tool-lsp
Tier 4 — orchestration:       rumoca-compile, tool-fmt, tool-lint
Tier 3 — phases & evaluation: rumoca-phase-*, rumoca-eval-*
Tier 2 — IR data:             rumoca-ir-*
Tier 1 — foundation:          rumoca-core
```

Within Tier 3, phases must compose through IR/evaluation crates rather than
depend on each other.

Relevant constraints for our crates:

- IR crates are **pure data**: no evaluation logic, no phase logic, no side
  effects. Read-only traversal/query helpers are allowed; anything needing
  typecheck state, solver layout, or incidence data is not an IR helper.
- Public collections must be deterministic (`IndexMap`); `HashMap`/`HashSet`
  may only be phase-internal. This matters directly for
  `StructuralFingerprint` — a nondeterministic iteration order would make
  fingerprints unstable.
- SPEC_0021 limits: functions ≤ 100 lines, nesting ≤ 4, ≤ 7 arguments, files
  under 2000 lines, all clippy-enforced (`too_many_lines = "deny"`).

**Natural placement** (to be confirmed against SPEC_0041's ownership catalog):

| Crate | Tier | Depends on |
|---|---|---|
| `rumoca-pass` — IR-generic framework | 2 | `rumoca-core` only (for `Diagnostic`) |
| `rumoca-analysis` — DAE/Flat analyses | 4 | `rumoca-ir-dae`, `rumoca-ir-flat`, `rumoca-eval-dae`, `rumoca-phase-structural` |
| `rumoca analyze` / `rumoca diff` | 6 | subcommands on the existing binary, like `lint` |

`rumoca-analysis` needs Tier 4 because it consumes `rumoca-phase-structural`
and `rumoca-eval-dae` (both Tier 3), and Tier 3 crates may not depend on other
phases.

**Existing precedent for a pass-like trait.** `rumoca-tool-lint` (Tier 4,
depends only on `rumoca-core` + `rumoca-compile`) already has a miniature
version of what we are building:

```rust
pub trait LintRule {
    fn check(&self, ctx: &LintContext<'_>) -> Vec<LintMessage>;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
}
```

`AnalysisPass<I>` is this generalised over the IR, with a typed `Output` and a
caching manager. Staying close to this shape keeps the new framework
idiomatic.

**There is no existing pass or analysis-manager infrastructure.** Grepping for
`trait .*Pass`, `PassManager`, and `AnalysisManager` across all 55 crates
returns nothing. This is greenfield.

---

## 11. Entry point for passes

```rust
pub struct CompilationResult {
    pub dae: Arc<Dae>,
    pub balance_detail: dae_analysis::BalanceDetail,
    pub flat: FlatModel,
    pub resolved: ResolvedTree,
    // + cached template renderers
}
```

`rumoca-compile` re-exports the pieces an analysis needs:

- `rumoca_compile::analysis` — `BalanceDetail`, `BalanceBreakdown`, `balance_detail`
- `rumoca_compile::phase_structural` — `Incidence`, `BltBlock`, `SortedDae`,
  `AlgebraicLoop`, `TearingResult`, `analyze`, `sort`, …
- `rumoca_compile::compile` — `Dae`, `DaeView`, `VariableRole`,
  `for_each_expression`, `source_span_location`, the session API

`compiler.rs` already contains a `dae_counts()` helper counting states,
algebraics, parameters, constants, inputs, outputs, and continuous equations —
effectively `ModelSummaryPass` written inline. Milestone 1 can start by lifting
that shape into the framework.

---

## 12. Test corpus — already in the repository

The four model classes the project spec asks for exist under `examples/models/`:

| Spec requirement | File | Exercises |
|---|---|---|
| Simple ODE | `SympyDecay.mo` | one state, one parameter, one residual |
| RLC circuit | `SwitchedRLC.mo` | 2 states, 3 algebraics, 4 parameters, units, one time event |
| Event-driven | `Ball.mo` | 2 states, relation root, `reinit`, `pre()` |
| Multi-component | `Circuit.mo` (`Circuit.Test`) | connectors, `extends`, `connect`, 20×20 system, generated equations |

`SwitchedRLC.mo` is particularly useful because it declares `unit` on every
type, which exercises the unit metadata the world-model artifact needs.

Models with `MSL` in the name (`SwitchedRLC_MSL.mo`, `PIDMSL.mo`) require
`cargo xtask repo modelica-deps ensure` first; without it, compiling anything
in `examples/models/` with `--emit ast-json` fails on unresolved
`Modelica.*` references, because source-root discovery loads every `.mo` in the
directory. Copy the four files above into an isolated directory to avoid this.

---

## 13. Process note

`AGENTS.md` is a routing index and states: *"All design rules live in `spec/`.
If a rule isn't in a spec, it's not a rule — propose a spec change first,"* and
*"If you cannot find the spec for what you're about to change, stop and ask
before coding."* `SPEC_0033 §1` adds that conflicting instructions must be
surfaced rather than silently bypassed.

Adding a pass framework introduces a new architectural layer and new crates,
which touches SPEC_0029 (crate boundaries and tiers) and its ownership catalog
SPEC_0041. Upstream process expects a `PROPOSED` spec before the code, and that
route was taken: see
[SPEC_0054](../spec/SPEC_0054_MODEL_ANALYSIS_PASSES.md) (`PROPOSED`), with
SPEC_0029 §3, SPEC_0041 §7, and the SPEC_0008 `EA0xx`/`WA0xx` ranges amended to
match. The spec stays `PROPOSED` until upstream maintainers vote, which cannot
happen inside this fork; `ACCEPTED` would claim an implementation that does not
exist yet.

SPEC_0054 §5 makes **out-of-tree drop-in plugins a first-class requirement**:
an installed Rumoca binary loads a pass artifact with no checkout, no rebuild,
and no toolchain match, over a WASM Component Model / WIT interface that never
exposes an internal Rust IR type. Two findings from these notes became
normative rules there — the `'dae` brand (§4.1), which is why the boundary is
typed queries over opaque IDs rather than a lent-out model, and the
Flat-versus-DAE connector gap (below), which is why the ABI is scoped per IR
stage.

---

## 14. Summary of decisions this milestone informs

1. **Analyse at DAE by default.** It is the canonical semantic contract, it has
   typed coordinates, complete provenance, and a full event model. Use Flat for
   Modelica-intent questions and Solve only for genuine execution questions.
2. **Pass outputs must be owned.** The `'dae` brand makes borrowing from a DAE
   impossible across the `inspect` boundary — design the result types for that
   from the start rather than fighting it later.
3. **Build `DependencyGraphPass` on `rumoca_eval_dae::for_each_scalar_coordinate`,**
   not a new expression walker and not `Incidence` alone (which omits
   parameters and inputs).
4. **The world-model backend is a target directory**, `ir = "dae"`, no compiler
   change required. The one gap is span→`file:line` resolution in templates.
5. **The matcher keys on `VarName` + role + type + unit**, with provenance as
   corroboration. Spans are not unique (`extends` duplicates them) and branded
   ids are not comparable across compilations.
6. **Reuse `dependency_first_sccs`, `ExpressionTraversal`, `Incidence`, and
   `source_span_location`** rather than reimplementing them; SPEC_0029 §3b
   makes single-source helpers a rule, not a preference.
7. **Connector structure is a Flat concept and is gone by DAE.**
   `rumoca-ir-flat` owns `ConnectionSet` (with `flow()`/`stream()` variants),
   `ConnectedVariable` (with `is_inside` and `sign()`), `ConnectionGraph`,
   `top_level_connectors`, and per-variable connection flags. `rumoca-ir-dae`
   retains **none** of it: the only survivors are the `ConnectionEquation` and
   `FlowBalanceEquation` provenance tags, which — as §8 shows — anchor to the
   connector *member declaration*, not to a connector set. Any analysis that
   asks "which connectors exist, which members are attached, which are flow
   versus potential" is a **Flat** consumer. An analysis that then instruments
   the corresponding runtime values spans Flat → DAE → Solve. This is why the
   plugin ABI is scoped per IR stage rather than being DAE-only.

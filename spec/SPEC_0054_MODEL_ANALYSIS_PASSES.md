# SPEC_0054: Model Analysis Pass Framework

## Status
PROPOSED

## Summary
Reusable analyses over compiler IR are passes, available both as in-process
Rust extensions and as drop-in plugins loaded by an installed Rumoca binary
through a stable, versioned ABI that never exposes Rumoca's internal IR types.

## Motivation

Rumoca's phases are a fixed sequence with one output each. Questions *about* a
compiled model — what depends on what, which variables an event touches, how
two versions differ — are not phases. They compose, and several consumers want
the same answer computed once. Without an owner they land wherever a caller
needs them: `compiler.rs::dae_counts` counts roles inline, and
`compile --inspect structure` prints text instead of returning data, reachable
only from the CLI.

A framework reachable only by contributors repeats that mistake at a larger
scale. Requiring a checkout, a matching toolchain, and a rebuild to ask a
question about a model is a barrier unrelated to the analysis being written.

## Specification

### 1. Two Extension Mechanisms

| Mechanism | Audience | Reaches | Distribution |
|---|---|---|---|
| In-process Rust pass | Rumoca contributors; deeply integrated compiler work | Internal crate APIs, live branded IR views | Compiled into the workspace or an out-of-tree crate linking Rumoca |
| Drop-in plugin | External pass authors | The stable Pass ABI only | One artifact file, loaded by an installed binary |

Neither replaces the other. Both honour the pass contract (§3); the drop-in
boundary (§5) is the normal choice for external authors.

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| A pass reads one checked IR and returns owned data | all passes | Analysis is observation; the pipeline keeps producing IR |
| A pass result MUST NOT re-enter compilation except through a checked request (§5.3) | framework and host | An unchecked result feeding a phase is a fifth IR stage, which SPEC_0007 owns |
| The framework and host MUST NOT depend on any consumer of their results | `rumoca-pass`, plugin host | A downstream runtime or scheduler cannot shape the analysis contract |

The host interface belongs to Rumoca and plugins depend on it. This is the
ordinary extension direction of SPEC_0031, not an inversion of it: the compiler
still depends on no individual pass.

### 2. Pass Categories

| Category | Reads | Produces | Plugin-exposed in v1 |
|---|---|---|---|
| Analysis | one checked IR | typed owned result | yes |
| Verification | one checked IR + analysis results | diagnostics | yes |
| Instrumentation | one checked IR | checked instrumentation requests (§5.3) | yes |
| Projection | one checked IR | a separate representation, or a new valid checked IR | **no** |

Projection passes remain in-process in v1; arbitrary IR rewriting across the
plugin boundary is PROHIBITED until a checked request vocabulary exists. An
unrestricted `fn run(&self, ir: &mut Dae)` is PROHIBITED in every category and
mechanism: it bypasses the guarantees SPEC_0036 establishes.

### 3. Pass Contract

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| A pass result MUST be owned, carrying no borrow of its input IR | `AnalysisPass::Output` | `rumoca-ir-dae` brands every view and id to one `inspect` call; a borrowing result cannot compile, and cross-model comparison cannot name a brand at all |
| A pass MUST target the highest IR that carries the information it needs | pass authors | Lowering past the answer discards structure; mirrors SPEC_0007's "codegen targets the lowest proven-valid IR it needs — no lower" |
| A pass MUST be deterministic for the same input | all passes | Structural fingerprints and model diffs are worthless otherwise; SPEC_0021's deterministic-collection rule applies to every public result field |
| A pass MUST NOT recover identity by parsing rendered names | all passes | SPEC_0001; structured identity and provenance already exist |
| A pass MUST NOT reimplement a SPEC_0029 §3b single-source helper | in-process passes | One implementation per helper, catalogued in SPEC_0041 §1 |
| Pass identity is a stable name | all passes | Pipelines and cache keys refer to passes by name; renaming one is a breaking change |

Analyses over DAE dependency structure MUST consume `rumoca-eval-dae`'s scalar
coordinate projection rather than walking the expression arena. It is the
compiler's own incidence proof — `rumoca-phase-structural::incidence` is built
on it — and already resolves function calls, structured families, and runtime
array selection.

### 4. Analysis Manager

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| A pass declares the analyses it requires; the manager runs missing ones | `rumoca-pass` | Callers do not hand-order pipelines |
| A required analysis MUST be computed at most once per IR instance | manager cache | The whole reason dependent passes exist |
| A cache is scoped to one IR instance and MUST NOT outlive it | manager | Results describe a specific compiled model |
| A requirement cycle MUST be rejected before any pass runs | manager | A cycle is an authoring bug, not a runtime condition |

Caching is the manager's obligation. A pass that memoizes its own result is
PROHIBITED: two callers would then disagree about which cache is authoritative.

### 5. Drop-in Plugin Boundary

#### 5.1 Requirement

Out-of-tree drop-in passes are a first-class requirement. Rumoca MUST support
passes distributed independently of the Rumoca source tree and loadable by an
installed Rumoca binary without recompiling Rumoca, without a source checkout,
and without matching Rumoca's pinned Rust toolchain.

The v1 implementation is a WebAssembly Component Model interface described in
WIT: Rust has no stable ABI, WIT gives an explicitly versioned
language-independent contract, and the sandbox makes §5.5's obligations
enforceable rather than advisory.

#### 5.2 What Crosses The Boundary

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| A plugin MUST NOT link against, or receive, any internal Rumoca Rust IR type | Pass ABI | Internal types have no stable ABI and carry lifetime brands that cannot survive a boundary |
| A plugin MUST NOT receive a pointer, handle, or serialized copy of an IR root | Pass ABI | A raw handle re-creates the unchecked access SPEC_0036 exists to prevent |
| Rumoca owns the IR and exposes read-only inspection through opaque IDs and typed queries | plugin host | The host answers questions; it does not hand over the model |
| The internal IR wire schema MUST NOT be the plugin ABI | plugin host | `DAE_SCHEMA_VERSION` changes freely by SPEC_0033; making it public would freeze internal representation and create a plugin-vs-compiler upgrade deadlock |
| Query interfaces are scoped per IR stage; a plugin declares the stage it inspects | Pass ABI, plugin manifest | Connector sets, flow/potential membership, and `is_inside` exist in Flat and are gone by DAE (only `ConnectionEquation`/`FlowBalanceEquation` provenance survives), so a connector-aware plugin is a Flat consumer; mirrors `target.toml`'s existing `ir` dimension |

A plugin that reasons about connectors and then instruments runtime values
therefore spans Flat, DAE, and Solve, and the ABI must let it say so.

#### 5.3 Transformation By Request

A transforming plugin does not mutate IR. It returns typed requests that the
host validates and applies through its own checked machinery.

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| An instrumentation plugin returns checked requests naming targets by opaque ID | Pass ABI | The host retains sole authority over IR construction |
| The host MUST validate every request against the live IR before applying it | plugin host | A stale or forged ID must fail, not corrupt the model |
| A rejected request MUST fail with a diagnostic, never be silently dropped | plugin host | Fail-closed, consistent with target capability gating |
| v1 request vocabulary is instrumentation only | Pass ABI | A wider vocabulary is added by extending this spec, not by widening plugin authority |

#### 5.4 Versioning And Capabilities

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| The Pass ABI is versioned independently of Rumoca's release version and of every internal IR schema | Pass ABI | One plugin must keep working across Rumoca releases; that is the point of the boundary |
| A plugin declares its required ABI version, IR stage, and capabilities | plugin manifest | The host can refuse before running anything |
| A capability the host does not grant MUST fail before the plugin runs | plugin host | Same fail-closed rule `target.toml` capabilities already follow |
| An ABI version MUST NOT change meaning after release; additions go in a new version | Pass ABI | Silent redefinition breaks distributed artifacts |

The concrete query surface, record types, and capability names are a catalog,
to be split into a REFERENCE annex once the interface is defined.

#### 5.5 Host Obligations

| Rule | Owner/Where | Brief Justification |
|---|---|---|
| The host MUST deny nondeterministic capabilities — wall clock, randomness, filesystem, network — unless explicitly granted | plugin host | §3 requires deterministic pass results; a sandbox makes that enforceable instead of advisory |
| The host MUST bound plugin execution and memory | plugin host | A buggy or hostile plugin cannot be allowed to hang or exhaust the compiler |
| Plugin support MUST be opt-in and MUST NOT enter the default `rumoca-bind-wasm` dependency graph | plugin host, bindings | Rumoca itself targets `wasm32`; a wasm runtime in the browser build is both unusable and large. Enforce with an architecture test mirroring `test_bind_wasm_default_graph_does_not_include_diffsol` |
| A plugin diagnostic MUST be attributed to its plugin | plugin host | Users must be able to tell a plugin finding from a compiler finding |

### 6. Crate Ownership

Placement and tiers are stated by
[SPEC_0029 §3](SPEC_0029_CRATE_BOUNDARIES.md) and catalogued in
[SPEC_0041 §7](SPEC_0041_CRATE_OWNERSHIP_CATALOG.md#7-analysis-pass-ownership-catalog-spec_0029-3);
shared helper ownership is
[SPEC_0041 §1](SPEC_0041_CRATE_OWNERSHIP_CATALOG.md#1-single-source-helper-catalog-spec_0029-3b).

The direction is the invariant worth repeating: **analysis consumes IR; IR
never depends on analysis; plugins depend on the host contract, never the
reverse.**

### 7. Diagnostics

Pass failures are `Diagnostic` values per
[SPEC_0008](SPEC_0008_PHASE_ERRORS.md), using the `EA0xx` / `WA0xx` ranges.
This spec reserves both ranges; they join the SPEC_0008 prefix table when the
first pass diagnostic ships.
A verification pass that cannot classify its input MUST report that rather than
return an empty finding list: silence and "nothing wrong" are different answers,
and only one of them is honest.

## Rationale

LLVM's pass manager is the obvious reference and the wrong thing to copy
literally: its passes mutate a shared module and its plugins inherit C++ ABI
coupling, binding each plugin to a build. Rumoca's IRs are valid by
construction and its DAE already refuses to hand out escaping references.
Taking that seriously yields a stricter, more portable model — the host answers
typed queries instead of lending out the model, transformations are requests
the host validates, and the artifact is a language-independent component rather
than a toolchain-matched shared object.

`rumoca-tool-lint`'s `LintRule` is the in-tree precedent for the in-process
shape; `target.toml` — declarative, versioned, capability-gated, fail-closed,
loaded from any directory with no rebuild — is the precedent for the drop-in
shape.

## References

- [SPEC_0007](SPEC_0007_IR_PIPELINE.md) — IR stage contracts; passes add no stage
- [SPEC_0029](SPEC_0029_CRATE_BOUNDARIES.md) — crate tiers and helper ownership
- [SPEC_0031](SPEC_0031_COMPILER_PHILOSOPHY.md) — core/extension split
- [SPEC_0033](SPEC_0033_DEVELOPMENT_PROCESS.md) — single-version internal wire formats
- [SPEC_0036](SPEC_0036_VALID_BY_CONSTRUCTION_IR.md) — why in-place mutation is prohibited
- [SPEC_0008](SPEC_0008_PHASE_ERRORS.md) — diagnostic and error-code contract
- [SPEC_0021](SPEC_0021_CODE_COMPLEXITY.md) — deterministic public collections

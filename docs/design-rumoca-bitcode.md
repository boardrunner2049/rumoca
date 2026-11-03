# Rumoca Bitcode: design and rationale

**Audience:** Rumoca maintainers.
**Status:** implemented; current v2 in `crates/rumoca-bitcode`.
**Companions:** [`SPEC_RUMOCA_BITCODE.md`](SPEC_RUMOCA_BITCODE.md) (normative
format), [`writing-a-bitcode-pass.md`](writing-a-bitcode-pass.md) (user guide),
[`rumoca_bitcode_notes.md`](rumoca_bitcode_notes.md) (findings this design rests
on).

---

## 1. What was added

An external compiler interface:

```
Modelica → Rumoca → .rbc → external tool → .rbc → Rumoca → checked DAE
```

```bash
rumoca compile Motor.mo --model Motor --emit-bitcode motor.rbc
python connector_logger.py motor.rbc -o motor-traced.rbc     # any language
rumoca compile-bitcode motor-traced.rbc --simulate --trace-out traces.csv
```

The tool in the middle needs no Rumoca checkout, no Rust, no matching compiler
version, and no linkage against Rumoca's crates.

Concretely:

| Component | Where |
|---|---|
| Public schema, export, import, validation, codec | `crates/rumoca-bitcode/` (new) |
| CLI: `--emit-bitcode`, `bitcode {inspect,dump,check,convert,round-trip}`, `compile-bitcode` | `crates/rumoca/src/bitcode_cli.rs` (new) |
| Zero-dependency Python SDK | `packages/rumoca-bitcode/` (new) |
| Four worked passes | `examples/bitcode-passes/` (new) |

## 2. Why this was needed

Rumoca already does the hard part. Parsing, resolution, instantiation,
flattening, and DAE lowering are solid, and the DAE is genuinely canonical:
typed coordinates, complete provenance, a real event model, and 70 construction
invariants enforced by the type system.

What was missing is the property that made LLVM's IR valuable — not the IR
itself, but the fact that *other people's tools can consume it*. Today, anyone
who wants to ask a question about a compiled Modelica model must become a
Rumoca contributor: clone the tree, learn the crate graph, match the pinned
nightly, and rebuild. That is a barrier with nothing to do with the question
being asked.

The consequence is visible in the tree. `compiler.rs::dae_counts` counts
variable roles inline because there was nowhere else to put it.
`compile --inspect structure` already computes matching, BLT order and source
snippets — real analysis — but prints text and is reachable only from the CLI.
Each of these is a small analysis that had to be built *inside* the compiler
because there was no outside.

Bitcode makes the outside exist.

## 3. Why not the existing serialization

Rumoca already serializes the DAE, and the obvious question is why that is not
the answer. Three reasons, all of them yours:

**It is single-version by design.** `SPEC_0007` states the current
`DAE_SCHEMA_VERSION` is the only supported version, and `SPEC_0033` requires a
format cutover to delete the superseded reader *in the same change*. The schema
is at 33 and rising. That is exactly right for an internal format and
disqualifying for a public one — a tool built against schema 14 would break on
schema 18 with no migration path, and the question "who upgrades whom" has no
good answer.

**It would freeze your internals.** Publishing the wire schema makes every Rust
struct a compatibility promise. You would lose the freedom to reorganise
variables, renumber ids, or restructure crates — the freedom `SPEC_0033`
explicitly protects.

**`SPEC_0041 §3` already forbids it.** Flat-IR, DAE-IR and Solve-IR artifacts
are in the *MUST NOT persist* column. Treating a dump as a durable artifact
would contradict a rule you already made.

So bitcode is a separate, curated projection with its own version. Internal
`DAE_SCHEMA_VERSION` may change freely; `RBC_VERSION` changes only when the
public contract does. There is precedent for this working: the template
projection in `codegen/dae_backend.rs` has held at schema 5 while the internal
wire went to 33.

## 4. Why a file, and not a plugin ABI

An earlier iteration of this design was a loadable plugin interface — a
versioned WIT/WASM boundary with the host answering typed queries over opaque
ids. It was abandoned in favour of a file, and the reasoning is worth recording
because the file looks like the less sophisticated option.

A query interface is the right shape when the consumer needs *laziness* or
*interactivity*: fetch three variables from a huge model, or stay attached while
state changes. Neither applies here. Every analysis we care about — dependency
graphs, connector topology, model diffing, instrumentation — wants the whole
model, once.

Against that, the query design carried real cost. An opaque-id API is chatty by
nature: `variable_role(id)`, `variable_unit(id)`, `variable_source(id)`. A
dependency pass over an MSL-scale model crosses the boundary 10⁵–10⁷ times. It
needed a wasm runtime embedded in a CLI that itself targets `wasm32`. And the
cross-language promise was softer than it sounded, since WIT bindings are
uneven outside Rust and C.

A file has none of that. The boundary is crossed once. A pass is *a program that
reads a file*, debuggable with an editor, writable in any language. The "plugin
API" becomes a documented file format — far easier to version, test, and explain
than an interface definition.

The parts of the plugin design that were actually good survived intact: opaque
ids rather than pointers, a curated projection rather than internal types, and
host-validated requests rather than direct mutation. Those are transport-
independent, and they are all in the current schema.

## 5. Design decisions

### 5.1 Import replays construction; it does not deserialize

This is the decision everything else rests on, and it came from reading
`model/wire.rs`. Your own deserializer does not fill in structs — it replays
operations through `Dae::construct`, the same constructors production
compilation uses. The wire comment says so explicitly: columns are
"operation-shaped", and constructor-derived facts are absent "because replay
re-issues them through the same checked operation".

Bitcode import does the same thing. It validates structure, then issues
`reserve_state`, `expressions.at(p).binary(...)`, `continuous.equation(...)`,
`events.reinitialize(...)` — real construction calls.

The consequence is the security property, and it is stronger than the brief
asked for. The requirement was "Rumoca MUST NOT trust imported bitcode", with a
four-stage validation pipeline. What we get instead is that **all 70 of your
construction invariants apply to imported bitcode automatically**, because there
is no other way in. An external pass that produces nonsense gets a rejection
naming the record. It cannot produce an invalid DAE, because `Dae` has no
unchecked constructor to reach.

We did not write a second validator. Writing one would have created a weaker,
divergent answer to a question your constructors already answer.

The bitcode-specific validator that does exist is deliberately narrow:
references, dense ids, duplicate names, topological ordering, summary
agreement. These are things construction *would* catch, but catching them first
produces "equation 0 references expression 999, which does not exist" instead of
an opaque construction error, and reports every problem at once rather than the
first.

### 5.2 Expression arenas are topologically ordered

Every operand references a strictly lower `ExprId`. This makes the arena a DAG
*by construction*: a cycle is unrepresentable rather than something to detect,
and a consumer builds the whole tree in one forward pass with no recursion. The
validator enforces it in one line; the Python SDK relies on it.

### 5.3 Dependency edges are exported, not re-derived

Each equation carries `reads` and `reads_derivative`, computed with
`rumoca_eval_dae::for_each_scalar_coordinate`.

This matters more than it looks. That projection is your own incidence proof —
`phase-structural::incidence` is built on it — and it already resolves function
calls, structured families, and runtime array selection. A Python pass walking
the expression tree would get all three wrong, silently. Exporting the edges
means the compiler's answer is the one consumers see.

### 5.4 Trace points are metadata, not equations

Instrumentation is a separate section, not an equation rewrite. Adding a trace
point cannot change what the model computes, so the common case — "observe this
quantity" — carries no risk of introducing a physics bug. Passes that genuinely
need to change behaviour still can, by editing equations; they just do not have
to for observation.

### 5.5 Execution reuses the ordinary simulation entry point

`rumoca_sim::simulate_with_diagnostics` takes `&dae::Dae` and nothing else.
That turned out to be the whole of Milestone 8: an imported DAE is a DAE, so a
reconstructed model simulates through exactly the path a freshly compiled one
does. No parallel runtime, no bitcode-specific solver, no special case.

Trace points then select which of the solver's reported columns are emitted,
joined with the label, unit, quantity and connection the external pass attached.
The result is structured rows; the table printed without `--trace-out` is one
presentation of them.

This is worth noting as evidence for the layering rather than as a feature:
because trace points are metadata and not equations, the model that ran is
bit-for-bit the model that was compiled. Instrumentation could not have changed
the physics even if the pass had been wrong.

### 5.6 CBOR with a JSON twin

CBOR is map-keyed and self-describing, so an unknown field is skipped rather
than shifting everything after it. That is the property your own
`wire_roundtrip_verification.rs` exists to protect: it tests under bincode
*specifically because* bincode tags variants by declaration index, and
documents two real near-misses where a mid-enum insertion would have silently
changed decoding. A public format must not have that failure mode, so every
bitcode enum is tagged by an explicit `kind` string.

JSON carries the identical schema for debugging. Encoding is detected from the
first byte, so a consumer is never told which it was handed.

Protocol Buffers were evaluated first, as the brief asked. CBOR won on build
friction: no `protoc`, no generated bindings, and a Python SDK with zero
dependencies. The one property protobuf has that CBOR lacks — automatic
unknown-field *preservation* on re-serialization — matters for a pass preserving fields it does not edit. The Python SDK retains
those raw fields within the supported version; semantic readers reject other
version headers. See the current [contract](SPEC_RUMOCA_BITCODE.md) for the v2
clock transport cutover.

### 5.7 Connector provenance is recovered at export, plus one compiler fix

The brief said: if Rumoca loses connector provenance, change Rumoca. We found
it loses it in two places, fixed one, and worked around the other.

**The fix.** `build_connection_sets` recorded the `connect(...)` span only for
direct endpoints. A connector-level `connect(a.p, b.p)` expands into member
variables (`a.p.v`, `a.p.i`) that are not themselves endpoints, so they fell
through to their own declaration span — pointing at the connector *type*. Every
generated connection equation in every model therefore reported `Pin.v` rather
than the connect. `ConnectionSet`'s own doc comment says the span "points at the
originating connect() statement", so this was a defect against stated intent,
not a missing feature. Recording the span for each expanded member is a
six-line change; all 620 flatten tests pass unchanged.

**The workaround.** The rest is structural:

`equation_generation()` in `phase-dae/src/construction.rs:1927` maps
`EquationOrigin::Connection { lhs, rhs }` to a fieldless
`DaeGeneration::ConnectionEquation`, discarding both endpoints. And
`ConnectionSet` / `ConnectionGraph` — which have exactly the right shape,
including `is_inside` and the connector type's `DefId` — are built and discarded
inside `phase-flatten/src/vcg.rs`; they are not fields of `flat::Model`.

Rather than change two crates and bump the wire schema, the exporter joins the
DAE with the Flat model, which `CompilationResult` already carries. Flat retains
per-equation `Connection { lhs, rhs }` and per-variable `flow`/`stream`/
`connected` flags — enough for endpoints, flow-versus-potential, connector
membership and component grouping. Enough, as it turns out, for the
connector-logging application to work.

This keeps the change additive. §7 records what it costs.

## 6. What changed in your tree

119 lines across 9 files, and most of that is spec text:

| File | Change |
|---|---|
| `Cargo.toml` | +3: workspace member, dependency entry, `ciborium` |
| `crates/rumoca/Cargo.toml` | +1: dependency |
| `crates/rumoca/src/lib.rs` | +4: module declaration |
| `crates/rumoca/src/cli.rs` | +29: two subcommands, three flags, one dispatch |
| `spec/*` | +46: SPEC_0052 registration and `EA0xx` diagnostic range |

No phase crate, IR crate, or existing test was modified. `cargo check
--workspace` passes; `cargo test -p rumoca-bitcode` is 21/21.

That this was possible is a compliment to the architecture. `Dae::construct` is
public, so a new crate can build a checked DAE without living inside
`rumoca-ir-dae`. The wire records are private, so bitcode *cannot* accidentally
couple to them. `CompilationResult` already carries both `dae` and `flat`, so
connector recovery needed no new plumbing. The boundaries were already in the
right places.

## 7. What is not done

| Gap | Consequence | Cost |
|---|---|---|
| `--target` covers DAE targets only | `dae-modelica` and custom `ir = "dae"` targets render from bitcode. FMI packaging is one contained refactor away: `ManifestRenderer::Fmi` already ignores `CompilationResult` entirely, so the change is threading `Option<&CompilationResult>` through `render`, `write_manifest_files` and `compile_manifest_package`. Left undone deliberately — it touches the path all 17 built-in targets use. | small, but needs your review |
| Elided function bodies and unrepresented event owners | Calls refuse import; model-event transactions, structured roots, previous, terminal and delay owner tables refuse export. Arrays, records, enumerations, exact clocks and String conversion are represented. | schema and checked mapping work per remaining feature |
| `connect(...)` statement spans | A connection reports the connector member's declaration, not `System.mo:31`. | retain a connection inventory on `flat::Model`; thread the span through `connections/equation_generation.rs` |
| Type aliases | `Voltage` and `Current` collapse to `Real` on import. Structure is preserved; alias identity is not. | carry alias names in `RbcType` |

The end-to-end instrumentation loop works today, including execution. Code
generation from bitcode does not; that is the remaining structural gap.

## 8. Findings you may want independently of this work

Three things surfaced that are about Rumoca, not about bitcode:

**Connector provenance is lost earlier than it looks.** `ConnectionSet` and
`ConnectionGraph` have the right shape but never reach `flat::Model`. Any
future connector-aware analysis — not just ours — has to recover them from
`EquationOrigin` strings. `EquationOrigin::Connection` holds `lhs: String,
rhs: String`, which is also in tension with SPEC_0001's rule against rendered
names as semantic identity.

**Generated-equation spans can mislead.** In `Circuit.Test`, all four connection
equations and all three flow balances point at declarations inside `Pin`
(`Real v`, `Real i`), not at the `connect(...)` statements. Any diagnostic that
prints a span without checking `DaeGeneration` will point users at the wrong
line.

**The dependency projection deserves more visibility.**
`for_each_scalar_coordinate` is the single most useful thing we found, and it is
reachable only through `rumoca-eval-dae`. `phase-structural::incidence` uses it
and then discards parameters, inputs and `pre()` reads because matching does not
need them — which means the obvious entry point for "what does this equation
depend on" gives an incomplete answer for any purpose other than matching.

## 9. Open item: SPEC_0054

`spec/SPEC_0054_MODEL_ANALYSIS_PASSES.md` was written during the earlier
plugin-ABI iteration and is registered as `PROPOSED`. Its §5 describes a
WASM/WIT drop-in boundary that this design supersedes. Its other sections — pass
categories, owned results, determinism, the analysis manager — remain
compatible with the bitcode direction and describe in-process Rust passes, which
bitcode does not replace.

It should be revised or narrowed before anyone treats it as current. That was
left alone deliberately rather than rewritten unasked.

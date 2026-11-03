# main v0.10.1 / bitcode integration evidence

The merge joins bitcode `fa911c9a7c8515391afaf78703d052f0928f5cf1` and
main `0cbfa46000705a577de9fcc5fbcd0f82743d7ba1` (v0.10.1). Main's checked
IR construction and native compiler pipeline take precedence. This snapshot
was tested in the uncommitted merge worktree; the candidate band artifact
records that working-tree digest and its pre-merge HEAD.

## Resolution choices

- Keep main's checked IR, typed public FMI variables, event/initialization
  semantics, algebraic projection, tensor lowering and tunable parameter
  dependency rules. Ordinary compilation uses main's native pipeline.
- Preserve bitcode import/export, text/binary/JSON transports, linking,
  optimization passes and explicit `--pass default` / `--pass round-trip`.
  Carry main's derivative annotations, initial Boolean parameters, observed
  definitions, per-element fixed attributes, StateSelect, causality and
  warning assertions through the checked import API.
- Retain branch fixes for record/empty-array bindings, named package
  constants, function loop assertions and event control dependencies while
  using main's occurrence and owner rules.
- Complete FMI parameter dependency analysis and shared C rendering for
  nested function folds, tensor updates, maps and reductions. Nested fold
  stores retain the enclosing output buffer. Loop counters do not overflow
  after the final iteration.
- Bitcode v2 still elides recursive bodies; an explicit bitcode pipeline can
  reject such reconstruction. Already-differentiated call provenance has no
  v2 encoding and export rejects it instead of silently discarding it.
  Function derivative annotations themselves round-trip successfully.

## Verification

All Rust commands used `CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=4
RAYON_NUM_THREADS=4`.

- `cargo test -p rumoca --test suite_core`: main baseline 1,010 passed;
  merged tree 1,113 passed, none failed.
- `cargo test -p rumoca-bitcode -p rumoca-phase-codegen -p rumoca-ir-solve`:
  all unit, integration and documentation tests passed. Unit totals are
  bitcode 80, Solve IR 415 and codegen 251.
- `cargo clippy -p rumoca-bitcode -p rumoca-phase-codegen
  -p rumoca-phase-dae -p rumoca-phase-flatten --lib`: passed.
- `cargo clippy -p rumoca --lib --bin rumoca`: passed.
- Rustfmt and whitespace checks on merge-specific content: passed. Existing
  whitespace in the unchanged main license notice and branch evidence CSVs
  is retained.
- FastDyn: `FASTDYN_TEST_RUMOCA_DIR=/home/mhcho/ws/rumoca-bitcode-integration
  fastdyn-env/bin/python -m pytest tests/unit/test_fmi3_package.py
  tests/unit/test_fmu_build.py tests/integration/test_rumoca_fmi3_package.py -q`:
  16 passed. Real compiled FMI3 libraries integrate a continuous input/state,
  a parameter matrix fold, and a nested matrix accumulation. Parameters are
  changed before initialization so pre-folded defaults cannot mask errors.

### Fixed 20-model MSL canary

Both worktrees ran:

```sh
cargo xtask verify msl-parity \
  --sim-targets-file infra/verification/msl-canary-20.json
```

The merge used `--results-dir /tmp/rumoca-canary-candidate`; main's original
results were copied before reuse of the build cache. OpenModelica is 1.26.1.
`main-canary.json`, `merged-canary.json` and `canary-delta.json` retain the
per-model evidence and the fixed target-list digest.

Both snapshots have 12 strict-high, 0 near, 0 deviation and 8 absent
(6 not attempted, 2 simulation failures), with no band changes. One absent
model, AutomaticSeed, reaches a different unsupported construct after the
branch's implicit-purity handling; it still cannot simulate. Comparator has
a floating-point rounding difference in its mean comparison score and stays
strict-high. This is a partial canary comparison, **not full MSL parity**.

### FIRE / FastDyn application check

The local FIRE ConfiguredQuad compiles to FMI3, its six declared C translation
units link, and the resulting FMU initializes and completes 100 co-simulation
steps of 1 ms with finite sensor/state outputs. Runtime getters confirm mass
1.5 and inertia diag(0.02, 0.02, 0.04) through aggregate, chassis and body.
OpenModelica's flattened model confirms the same parameter forwarding.

This application check also needs local changes outside this Rumoca commit:
FastDyn must compile every C source in `sources/buildDescription.xml`, and
FIRE's MultirotorPlant / ChassisAssembly record defaults use explicit record
constructor bindings. The latter avoids a default-modifier forwarding defect
also reproduced on pristine main. The already-applied explicit rigid-body
inertia inverse remains in FIRE. Full non-diagonal inertia, quaternion, hover
and tilted-contact tests passed in OpenModelica; a singular inertia remained
rejected by the positive-definiteness assertion.

The FMI target still does not advertise arbitrary `residual_equations`.
Main's checked algebraic-projection profile supports eligible coupled loops;
this merge does not claim unrestricted implicit-DAE support.

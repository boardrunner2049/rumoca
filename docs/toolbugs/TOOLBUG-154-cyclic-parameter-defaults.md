# TOOLBUG-154 — Mutually defaulted parameters reported as cyclic (ER007)

**Status:** fixed.
**Severity:** medium.

## What

Resolve checked each class's parameter bindings for cycles. A non-final
binding is only a default that a modification replaces, and libraries rely on
mutually defaulted pairs where every use sets one side:

```modelica
model PlugFlowPipeDiscretized
  parameter Length totLen = sum(segLen);
  parameter Length segLen[nSeg] = fill(totLen/nSeg, nSeg);
end PlugFlowPipeDiscretized;
// Example: PlugFlowPipeDiscretized pip(totLen = 100, ...)
```

(also AixLib `CHPEngDataBaseRecord`: `p = f_1/n0`, `n0 = f_1/p`). The class
check rejected every model using them with
`ER007 cyclic dependency in parameter binding for 'totLen'`.

Affected (cluster R2-fnbody): IBPSA and IDEAS
`FixedResistances.Examples.PlugFlowPipeDiscretized`; AixLib
`ModularCHP.BaseClasses.ExhaustHeatExchanger`, `GasolineEngineChp` — 4 models
(the CHP models then stop on ER002 `specificEntropyOfpTX`, not fixed here).
Not changed: OpenIPSL `GENTPJ` (`Zs`) is a genuine cycle through
`qsat0`/`id0`/`delta0` in the unmodified instance, and TRANSFORM `data_RCTR`
(`rs_ring_cell[i]` reads `rs_ring_cell[i-1]`) needs element-wise ordering.

## Fix

* `crates/rumoca-phase-resolve/src/semantic_checks/mod.rs`: only final
  bindings form edges in the class-level check (a final cycle can never be
  broken and stays ER007).
* `crates/rumoca-phase-dae/src/construction/analysis/parameter_cycles.rs`
  (new): after translation-time evaluation, the bindings left unevaluated are
  checked for a cycle on the instance; a surviving cycle is the new
  `ED025 cyclic dependency in parameter binding: p.totLen -> p.segLen -> p.totLen`.

## Test

`function_body_and_binding_regressions.rs::parameter_cycles_are_judged_on_the_instance`;
`rumoca-contracts/tests/inst_contracts.rs::inst_008_no_cyclic_binding` (now
ED025 for default bindings, ER007 for final ones) and
`inst_008_modification_breaks_a_default_cycle`.

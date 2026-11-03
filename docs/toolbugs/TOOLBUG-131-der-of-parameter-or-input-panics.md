# TOOLBUG-131 — `der()` of a parameter or of a top-level input panics in DAE construction

**Status:** fixed.
**Severity:** high (compiler panic).

## What

```modelica
block SimpleLead                       // OpenIPSL.NonElectrical.Continuous
  extends Modelica.Blocks.Interfaces.SISO;
equation
  T*der(u) = K*y - u;                  // u is a top-level input
end SimpleLead;

model E parameter Real p = 2; Real x; equation der(x) = der(p) + 1; end E;
```

```
panicked at crates/rumoca-phase-dae/src/construction/expression/temporal.rs:15:10:
analysis proves derivative role: UnsupportedFlatSemantics { feature: "derivative target", ... }
```

Role analysis collected every `der` operand as a state but then classified
external inputs and parameters first, so the operand had no state coordinate
and the constructor's `expect` fired.

- `der(parameter)` / `der(constant)`: MLS §3.7.4.2 defines the result as zero.
- `der(top-level input)`: the input is supplied by the environment, not a
  state; its derivative is not part of the canonical DAE. OpenModelica also
  refuses to simulate this ("The model requires derivatives of some inputs").

Affected (cluster G): OpenIPSL `SimpleLead`, Modelica_DeviceDrivers
`EmbeddedTargets.AVR.Examples.SBHS.Controller` (`der(degC)` of an input) —
2 models; both now end in the ED024 diagnostic instead of a panic.

## Fix

- `rumoca-phase-flatten/src/postprocess/parameter_derivatives.rs` (new pass,
  run after constant substitution): folds `der(p)` of a parameter/constant
  variable (whole or scalar-subscripted) or a numeric literal to a zero of the
  same shape.
- `rumoca-phase-dae/src/construction/analysis/model_roles.rs`: a top-level input
  used under `der` is rejected with the new `ED024` (`ToDaeError::DerivativeOfInput`),
  labelled at the input declaration.

## Test

`crates/rumoca-contracts/tests/expr_contracts.rs::der_of_parameter_is_zero`
(simulates) and `::der_of_top_level_input_is_diagnosed` (ED024 in ToDae).

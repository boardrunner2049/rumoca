# TOOLBUG-121 — expandable-connector member source count (EF037)

**Status:** fixed.
**Severity:** high (every hierarchical bus model in the IBPSA/AixLib/IDEAS/
Buildings heat-pump family, ThermoPower plant controls, VehicleInterfaces).

## What

`check_augmented_member_sources` (MLS §9.1.3 with §9.3) had two defects.

1. **Occurrences, not connectors.** Sources were counted once per connect
   *endpoint*, so an output named by three connects counted three times, and an
   output that merely forwards an inner source counted as a source of its own:

   ```modelica
   block Wrap  RO iceFac; Src s; equation connect(s.y, iceFac); end Wrap;
   model M  Bus sigBus; Wrap w; RO out;
   equation
     connect(w.iceFac, sigBus.iceFacMea);   // EF037: "has 5 sources"
     connect(w.iceFac, out);
   end M;
   ```

   Now distinct endpoints are counted, and an output that is the *outside*
   connector of a connect (the connect is written in the output's own
   component, `connect(s.y, iceFac)` inside `Wrap`) is a forwarder, not a
   source (MLS §9.3 inside/outside connectors).

2. **Zero sources was an error.** A bus member that is only read reported
   "has 0 sources; exactly one is required". MLS §9.1.3 says an input member
   *should* appear as a non-input elsewhere in its augmentation set, and for
   the model's own (top-level) expandable connector the member is supplied by
   whoever connects the bus: §9.1.3 gives it the causality of the input it
   feeds. OpenModelica accepts all of these. The member of a top-level bus
   left undriven is now marked `input` (and its bus recorded as a top-level
   connector), so DAE construction treats it as a model input; an undriven
   member of a *nested* bus stays an unknown and the balance check reports it.

Affected (cluster F, EF037, 18 models): AixLib `NoCooling`, `Safety.OnOff`,
`Frosting.Validation.WetterAfjei1997`; Buildings `DualFanDualDuct.Controls.
{Cooling,Heating}CoilTemperatureSetpoint`, `RefrigerantCycleConditional`;
IBPSA `Safety.AntiFreeze`, `Safety.MinimalFlowRate`, `FunctionalIcingFactor`;
IDEAS `RefrigerantCycle`, `NoHeating`, `WetterAfjei1997`; ThermoPower
`HRSG.Control.levelsControl`, `ST3L_bypass`, `ST_2LRhHU`; VehicleInterfaces
`MinimalExample`, `DriveByWireAutomaticExternalDriver`, `MinimalEngine`.

## Fix

- `rumoca-phase-flatten/src/connections/expandable.rs`: `EndpointSets`
  union-find, `EndpointRole` (outside/inside per connect scope), distinct
  drivers minus forwarders; top-level undriven member becomes an input.
- `rumoca-phase-dae/src/construction/analysis/model_roles.rs`: an augmented
  member has no component reference, so its root is read off its bus path when
  deciding whether an input is external.

## Test

`crates/rumoca-contracts/tests/conn_frontend_regressions.rs`:
`forwarded_output_on_a_bus_member_is_one_source`,
`read_only_member_of_a_top_level_bus_is_a_model_input`,
`two_outputs_on_one_bus_member_are_rejected` (the genuine conflict still fails).

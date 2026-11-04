//! Regressions for library models whose array dimensions or structural
//! parameters Rumoca could not evaluate although the model is valid
//! (TOOLBUG-165..179). Each fixture is the minimal form of the library
//! construct; tests compile through the CLI and, where the value matters,
//! simulate and read the result.

use std::fs;
use std::process::{Command, Output};

use tempfile::tempdir;

fn run(fixture: &str, model: &str, extra: &[&str], simulate: bool) -> (Output, Option<String>) {
    let work = tempdir().expect("temp dir");
    let source = work.path().join("Fixture.mo");
    fs::write(&source, fixture).expect("fixture written");
    let csv = work.path().join("result.csv");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rumoca"));
    if simulate {
        command.args([
            "sim",
            source.to_str().expect("utf-8 path"),
            "-m",
            model,
            "--t-end",
            "1",
            "-o",
            csv.to_str().expect("utf-8 path"),
        ]);
    } else {
        command.args([
            "compile",
            source.to_str().expect("utf-8 path"),
            "--model",
            model,
        ]);
    }
    let output = command.args(extra).output().expect("rumoca runs");
    let result = fs::read_to_string(&csv).ok();
    (output, result)
}

fn assert_compiles(fixture: &str, model: &str) {
    let (output, _) = run(fixture, model, &["--emit", "dae-json"], false);
    assert!(
        output.status.success(),
        "{model} must compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Simulate and return the first data row's value of column `column`.
fn initial_value(fixture: &str, model: &str, column: &str) -> f64 {
    let (output, csv) = run(fixture, model, &[], true);
    assert!(
        output.status.success(),
        "{model} must simulate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let csv = csv.expect("result csv written");
    let mut lines = csv.lines();
    let header = csv_cells(lines.next().expect("header"));
    let row = csv_cells(lines.next().expect("first row"));
    let index = header
        .iter()
        .position(|name| name == column)
        .unwrap_or_else(|| panic!("column {column} in {header:?}"));
    row[index].parse().expect("numeric value")
}

/// Split one CSV line, honoring quoted cells (`"x[1,1]"`).
fn csv_cells(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            ',' if !quoted => cells.push(String::new()),
            _ => cells.last_mut().expect("one cell").push(ch),
        }
    }
    cells
}

/// TOOLBUG-165: a record parameter bound to a function call whose record
/// output fixes a flexible field through its declaration modifier; the body
/// writes the fields inside loops and branches (Buildings/IBPSA/IDEAS/AixLib
/// `Movers.Data.Generic.motorEfficiency_yMot_generic` via
/// `motorEfficiencyCurve`).
#[test]
fn record_field_extent_comes_from_function_output_modifier() {
    const FIXTURE: &str = "\
package D
  record R
    parameter Real y[:];
    parameter Real eta[size(y, 1)];
  end R;
  function f
    input Real p;
    output R res(y={0, 0.5, 1}, eta=zeros(n));
  protected
    constant Integer n = 3;
  algorithm
    if p > 1 then
      for j in 1:n loop
        res.eta[j] := p*j;
      end for;
    else
      res.eta := fill(7, n);
    end if;
    res.eta[1] := -1;
  end f;
  model M
    parameter Real p = 0.5;
    final parameter R r = D.f(p);
    final parameter R r2 = D.f(2);
    Real x;
  equation
    x = r.eta[1] + 10*r.eta[2] + 100*r.y[2] + 1000*r2.eta[2];
  end M;
end D;
";
    // r.eta = {-1, 7, 7}, r.y[2] = 0.5, r2.eta = {-1, 4, 6}.
    let x = initial_value(FIXTURE, "D.M", "x");
    assert!((x - (-1.0 + 70.0 + 50.0 + 4000.0)).abs() < 1e-9, "x = {x}");
}

/// TOOLBUG-165: a record input whose later field's extent reads an earlier
/// field (`Real P[size(V_flow, 1)]`), decomposed into scalar-ABI inputs
/// (IDEAS `Movers.BaseClasses.Characteristics.flowParameters`).
#[test]
fn decomposed_record_input_extent_reads_sibling_field() {
    const FIXTURE: &str = "\
package B
  record R
    parameter Real y[:];
    parameter Real eta[size(y, 1)];
  end R;
  function g
    input R r;
    input Real u;
    output Real s;
  algorithm
    s := u*sum(r.eta) + size(r.y, 1);
  end g;
  model M
    parameter R r(y={1,2,3}, eta={4,5,6});
    Real x;
  equation
    x = g(r, 1);
  end M;
end B;
";
    let x = initial_value(FIXTURE, "B.M", "x");
    assert!((x - 18.0).abs() < 1e-9, "x = {x}");
}

/// TOOLBUG-166: extents instantiation recorded for a literal-size array must
/// stay readable after constant collection clears a redeclared component's
/// alias scope (IDEAS `Movers.Validation.Pump_stratos`:
/// `powEu(V_flow = powEu_internal.V_flow)` inside a redeclared mover).
#[test]
fn literal_extents_survive_component_redeclaration() {
    const FIXTURE: &str = "\
package H
  record PW
    parameter Real V_flow[3];
  end PW;
  record PP
    parameter Real V_flow[:];
    parameter Real P[size(V_flow, 1)];
  end PP;
  partial model Iface
    parameter Real k = 1;
  end Iface;
  model Impl
    extends Iface;
    final parameter PW inter(V_flow = {k, 2*k, 3*k});
    final parameter PP powEu(V_flow = inter.V_flow, P = 2*inter.V_flow);
    Real x;
  equation
    x = powEu.P[3];
  end Impl;
  model Base
    replaceable Iface floMacSta;
  end Base;
  model M
    extends Base(redeclare Impl floMacSta(k = 2));
  end M;
end H;
";
    assert_compiles(FIXTURE, "H.M");
    let x = initial_value(FIXTURE, "H.M", "floMacSta.x");
    assert!((x - 12.0).abs() < 1e-9, "x = {x}");
}

/// TOOLBUG-167: a component's dimension reads a sibling package constant by a
/// relative name (`C.n`), which typecheck never collected.
#[test]
fn component_dimension_reads_sibling_package_constant() {
    const FIXTURE: &str = "\
package L
  package C
    constant Integer n = 2;
  end C;
  model A
    Real x[C.n] = {1, 2};
  end A;
  model B
    A a;
  end B;
end L;
";
    assert_compiles(FIXTURE, "L.B");
}

/// TOOLBUG-167: a component's class redeclares its replaceable package in an
/// `extends` clause with a relative name; the dimension must use the
/// redeclared package's constant, set by an extends modifier
/// (IBPSA/Buildings `Electrical.Interfaces.Source.S[PhaseSystem.n]`).
#[test]
fn component_dimension_uses_extends_redeclared_package() {
    const FIXTURE: &str = "\
package P
  package PhaseSystems
    partial package PartialPhaseSystem
      constant Integer n;
    end PartialPhaseSystem;
    package OnePhase
      extends PartialPhaseSystem(n=2);
    end OnePhase;
    package ThreePhase
      extends PartialPhaseSystem(n=3);
    end ThreePhase;
  end PhaseSystems;
  package Interfaces
    partial model Source
      replaceable package PhaseSystem = P.PhaseSystems.OnePhase
        constrainedby P.PhaseSystems.PartialPhaseSystem;
      Real S[PhaseSystem.n];
    end Source;
  end Interfaces;
  package AC
    package Sources
      model FixedVoltage
        extends P.Interfaces.Source(
          redeclare package PhaseSystem = PhaseSystems.ThreePhase);
      equation
        S = {1, 2, 3};
      end FixedVoltage;
    end Sources;
    model M
      Sources.FixedVoltage E;
      Real s3 = E.S[3];
    end M;
  end AC;
end P;
";
    let s3 = initial_value(FIXTURE, "P.AC.M", "s3");
    assert!((s3 - 3.0).abs() < 1e-9, "s3 = {s3}");
}

/// TOOLBUG-168: a structural Integer parameter reduces an array parameter
/// through a comprehension over its elements (AixLib/IDEAS/Buildings
/// `ReducedOrder.RC.OneElement.dimension`).
#[test]
fn structural_parameter_reduces_array_parameter_elements() {
    const FIXTURE: &str = "\
package S
  model Z
    parameter Real AExt[2] = {1, 2};
    parameter Real A1 = sum(AExt);
    parameter Real A2 = 0;
    parameter Real A3 = 5;
    final parameter Real AArray[3] = {A1, A2, A3};
    parameter Integer dimension = sum({if A > 0 then 1 else 0 for A in AArray});
    parameter Integer k = if AArray[2] > 0 then 3 else 1;
    Real x[dimension, k];
    Real n = dimension;
  equation
    x = fill(1.0, dimension, k);
  end Z;
end S;
";
    let n = initial_value(FIXTURE, "S.Z", "n");
    assert!((n - 2.0).abs() < 1e-9, "n = {n}");
}

/// TOOLBUG-169: `size()` of a literal-shaped array is a literal extent once
/// constant substitution exposes it (MSL `PartialMedium.nX =
/// size(substanceNames, 1)`, `X_default = fill(1/nX, nX)`).
#[test]
fn size_of_literal_array_is_a_literal_extent() {
    const FIXTURE: &str = "\
package Z
  package Med
    constant String substanceNames[:] = {\"a\", \"b\"};
    constant Integer nX = size(substanceNames, 1);
    constant Real X_default[nX] = fill(1/nX, nX);
  end Med;
  model M
    replaceable package Medium = Med;
    parameter Real X_start[Medium.nX] = Medium.X_default;
    Real y = X_start[2];
  end M;
end Z;
";
    let y = initial_value(FIXTURE, "Z.M", "y");
    assert!((y - 0.5).abs() < 1e-9, "y = {y}");
}

/// TOOLBUG-170: a modifier written as an if-expression over the writing
/// scope's parameters decides a nested instance's conditional component
/// (IBPSA/TRANSFORM `LimPID`: `I(final reset = if reset == Reset.Disabled
/// then reset else Reset.Input)` and `IntegratorWithReset.y_reset_in`).
#[test]
fn if_expression_modifier_decides_nested_conditional_component() {
    const FIXTURE: &str = "\
package E
  connector RealInput = input Real;
  type Reset = enumeration(Disabled, Parameter, Input);
  block Integ
    parameter E.Reset reset = E.Reset.Disabled;
    RealInput y_reset_in if reset == E.Reset.Input;
    Real y = time;
  end Integ;
  block PID
    parameter E.Reset res = E.Reset.Disabled;
    RealInput y_reset_in if res == E.Reset.Input;
    Integ I(final reset = if res == E.Reset.Disabled then res else E.Reset.Input);
  equation
    connect(y_reset_in, I.y_reset_in);
  end PID;
  model M
    PID pid(res = E.Reset.Input, y_reset_in = 2);
  end M;
end E;
";
    assert_compiles(FIXTURE, "E.M");
}

/// TOOLBUG-171: an extends modifier carrying attribute modifiers together
/// with a binding, `x(each final unit = \"kg/s\") = e`, keeps the binding
/// (IBPSA/IDEAS/AixLib `Movers.FlowControlled_*.stageInputs`).
/// TOOLBUG-172: a component array sized `size(x, 1)` where `x` is bound to
/// another array, itself a scaled comprehension.
#[test]
fn extends_attribute_modifier_keeps_binding_and_sizes_component_array() {
    const FIXTURE: &str = "\
package G
  connector RealOutput = output Real;
  connector RealInput = input Real;
  block Const
    parameter Real k;
    RealOutput y = k;
  end Const;
  block Extract
    parameter Integer nin = 1;
    RealInput u[nin];
    RealOutput y = sum(u);
  end Extract;
  record Gen
    parameter Real[:] speeds(each final min = 0) = {0.3, 0.6, 1};
  end Gen;
  partial model PFM
    replaceable parameter Gen per;
    parameter Real stageInputs[:];
    Const[size(stageInputs, 1)] stageValues(final k = stageInputs);
    Extract extractor(final nin = size(stageInputs, 1));
  equation
    connect(stageValues.y, extractor.u);
  end PFM;
  model M
    extends PFM(final stageInputs(each final unit=\"kg/s\") = massFlowRates);
    parameter Real m_flow_nominal = 10;
    parameter Real[:] massFlowRates =
      m_flow_nominal*{per.speeds[i]/per.speeds[end] for i in 1:size(per.speeds, 1)};
  end M;
end G;
";
    let y = initial_value(FIXTURE, "G.M", "extractor.y");
    assert!((y - 19.0).abs() < 1e-9, "y = {y}");
}

/// TOOLBUG-173: a connect-statement slice whose extent is a constant of the
/// component's medium package (Buildings/IDEAS/AixLib `Fluid.Sources`:
/// `connect(X_in_internal[1:Medium.nXi], Xi_in_internal)`).
#[test]
fn connect_slice_reads_package_constant() {
    const FIXTURE: &str = "\
package C
  connector RI = input Real;
  connector RO = output Real;
  package Med
    constant Integer nX = 3;
    constant Integer nXi = nX - 1;
  end Med;
  block Src
    replaceable package Medium = Med;
    RO X_in_internal[Medium.nX];
    RI Xi_in_internal[Medium.nXi];
    Real s = sum(Xi_in_internal);
  equation
    X_in_internal = {1, 2, 3};
    connect(X_in_internal[1:Medium.nXi], Xi_in_internal);
  end Src;
  model M
    Src src;
  end M;
end C;
";
    let s = initial_value(FIXTURE, "C.M", "src.s");
    assert!((s - 3.0).abs() < 1e-9, "s = {s}");
}

/// A dimension naming a package constant several packages deep
/// (`Modelica.Math.Random.Generators.Xorshift128plus.nState` in the MSL random
/// generators) while another constant of the same root package is already
/// known: the deepest named class is seeded, not only the root, whose
/// extraction reaches one nested level (MLS §5.3.2, §10.1; ET004 before).
#[test]
fn dimension_reads_a_constant_several_packages_deep() {
    const FIXTURE: &str = "\
package P
  constant Integer m = 2;
  package A
    package B
      package C
        constant Integer n = 3;
      end C;
    end B;
  end A;
end P;
model Deep
  Real y[P.m];
  Real x[P.A.B.C.n];
  Real total = sum(x) + sum(y);
equation
  y = ones(P.m);
  x = {1, 2, 3};
end Deep;
";
    let total = initial_value(FIXTURE, "Deep", "total");
    assert!((total - 8.0).abs() < 1e-12, "total = {total}");
}

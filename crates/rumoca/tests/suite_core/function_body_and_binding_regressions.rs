//! Regressions for library models the DAE construction refused although the
//! model is valid (TOOLBUG-150..154). Each fixture is the minimal form of the
//! library construct; every test compiles it to the DAE through the CLI.

use std::fs;
use std::process::{Command, Output};

use tempfile::tempdir;

fn compile(fixture: &str, model: &str) -> Output {
    let work = tempdir().expect("temp dir");
    let source = work.path().join("Fixture.mo");
    fs::write(&source, fixture).expect("fixture written");
    Command::new(env!("CARGO_BIN_EXE_rumoca"))
        .args([
            "compile",
            source.to_str().expect("utf-8 path"),
            "--model",
            model,
            "--emit",
            "dae-json",
        ])
        .output()
        .expect("rumoca runs")
}

fn assert_compiles(fixture: &str, model: &str) -> String {
    let output = compile(fixture, model);
    assert!(
        output.status.success(),
        "{model} must compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// TOOLBUG-150: a Real package constant that an equation reads is declared as
/// a model constant; a function body reading the same constant must still
/// fold it (AixLib `EquivalentOpeningArea` via `Jiang`).
#[test]
fn function_body_folds_a_package_constant_the_model_also_declares() {
    const FIXTURE: &str = "\
package P
  package C
    constant Real eps = 1e-15;
  end C;
  function F
    input Real a;
    output Real y;
  algorithm
    if a < C.eps then
      y := 0;
    else
      y := 1/a;
    end if;
  end F;
  model M
    Real x = time + 1;
    Real y;
    Real z;
  equation
    y = F(x);
    z = if noEvent(x > C.eps) then x else 0;
  end M;
end P;
";
    assert_compiles(FIXTURE, "P.M");
}

/// TOOLBUG-151: an assertion condition may compare against an enumeration
/// literal, and an initial-algorithm assertion may pass an explicit level
/// (PowerGrids `System`, Buildings `RefrigerantCycleConditional`).
#[test]
fn assertions_resolve_enumeration_literals() {
    const FIXTURE: &str = "\
package EA
  type RF = enumeration(fixed, adaptive);
  model M
    parameter RF referenceFrequency = RF.fixed;
    parameter Boolean allow = false;
    parameter String a = \"x\";
    Real y = time;
  initial equation
    assert(not referenceFrequency == RF.adaptive, \"not implemented\");
  initial algorithm
    if not allow then
      assert(a == \"x\", \"In \" + a + \": not equal\", AssertionLevel.error);
    end if;
  equation
    assert(referenceFrequency <> RF.adaptive, \"not implemented\");
  end M;
end EA;
";
    let dae = assert_compiles(FIXTURE, "EA.M");
    assert!(
        dae.contains("\"assert\""),
        "the assertions reach the DAE: {dae}"
    );
}

/// TOOLBUG-152: element-wise `.-`/`.+` with one scalar operand in a parameter
/// binding (IBPSA `SelectCable_low`: `{65, 95} .- 10`), and a binding that
/// divides by a parameter left at its default zero (IBPSA
/// `PlugFlowTransportDelay`) stays a runtime binding instead of failing.
#[test]
fn parameter_bindings_broadcast_and_defer_division_by_zero() {
    const FIXTURE: &str = "\
package PB
  model M
    parameter Real I[:] = {65, 95, 110} .- 10;
    parameter Real P[3] = 2 .+ I;
    parameter Real length;
    parameter Real conUM = 4/length;
    Real x(start = 1, fixed = true);
  equation
    der(x) = -x*sum(P);
  end M;
end PB;
";
    assert_compiles(FIXTURE, "PB.M");
}

/// TOOLBUG-153: mutually defaulted parameters are legal when the instance
/// modifies one of them (IBPSA `PlugFlowPipeDiscretized`); a cycle that
/// survives in the instance is still rejected, now with the instance path.
#[test]
fn parameter_cycles_are_judged_on_the_instance() {
    const FIXTURE: &str = "\
package Cy
  model Pipe
    parameter Integer nSeg = 3;
    parameter Real totLen = sum(segLen);
    parameter Real segLen[nSeg] = fill(totLen/nSeg, nSeg);
    Real x(start = 1, fixed = true);
  equation
    der(x) = -x*totLen;
  end Pipe;
  model Ok
    Pipe p(totLen = 100);
  end Ok;
  model Bad
    Pipe p;
  end Bad;
end Cy;
";
    assert_compiles(FIXTURE, "Cy.Ok");
    let bad = compile(FIXTURE, "Cy.Bad");
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(!bad.status.success(), "the unbroken cycle is rejected");
    assert!(
        stderr.contains("ED025") && stderr.contains("p.totLen -> p.segLen"),
        "the cycle is named on the instance: {stderr}"
    );
}

/// TOOLBUG-155: `String(x)` in a package-constant modifier is the predefined
/// String conversion (IDEAS/AixLib `PropyleneGlycolWater`:
/// `mediumName = "PropyleneGlycolWater(X_a = " + String(X_a) + ")"`).
#[test]
fn package_constant_string_conversion_is_the_predefined_operator() {
    const FIXTURE: &str = "\
package Str
  package Base
    constant String mediumName = \"unknown\";
    constant Real c = 1;
  end Base;
  package Med
    constant Real X_a = 0.4;
    extends Base(mediumName = \"PG(X_a = \" + String(X_a) + \")\");
  end Med;
  model M
    package Medium = Med;
    parameter String n = Medium.mediumName;
    Real y = time*Medium.c;
  equation
    assert(y < 10, \"medium \" + Medium.mediumName);
  end M;
end Str;
";
    assert_compiles(FIXTURE, "Str.M");
}

/// TOOLBUG-156: package constants bound by a signed symbolic expression
/// (IDEAS `Types.Azimuth`: `W = +pi/2`, `E = -pi/2`) fold into component
/// modifiers.
#[test]
fn signed_symbolic_package_constants_fold_into_modifiers() {
    const FIXTURE: &str = "\
package Az
  package Azimuth
    constant Real pi = 3.141592653589793;
    constant Real S = 0;
    constant Real W = +pi/2;
    constant Real E = -pi/2;
  end Azimuth;
  block Constant
    parameter Real k;
    output Real y = k;
  end Constant;
  model M
    Constant azi[3](k = {Azimuth.S, Azimuth.W, Azimuth.E});
    Constant c(k = Azimuth.E);
  end M;
end Az;
";
    assert_compiles(FIXTURE, "Az.M");
}

/// TOOLBUG-157: the `array(...)` constructor (MLS §10.4.1) in a binding and
/// in a function's protected constant (ThermoSysPro `BaseIF97`:
/// `constant Real[42] nn = array(...)`).
#[test]
fn array_constructor_call_is_an_array() {
    const FIXTURE: &str = "\
package Arr
  function f
    input Real x;
    output Real y;
  protected
    constant Real nn[3] = array(1, 2, 3);
  algorithm
    y := x*nn[2];
  end f;
  model M
    Real z = f(time);
    parameter Real p[2] = array(4, 5);
    Real w = p[2]*time;
  end M;
end Arr;
";
    assert_compiles(FIXTURE, "Arr.M");
}

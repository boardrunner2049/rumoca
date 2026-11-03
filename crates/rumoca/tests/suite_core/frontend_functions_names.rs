//! Function and name-resolution regressions reduced from real libraries
//! (TOOLBUG-100..109). Each model here compiled in OpenModelica but was
//! refused by Rumoca's front end.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae_with_diagnostics};

fn compile(source: &str, model: &str) -> rumoca::CompilationResult {
    Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
        .unwrap_or_else(|error| panic!("{model} compiles: {error}"))
}

// TOOLBUG-100: `AssertionLevel.error` inside a function body is the default
// severity (MLS §8.3.7); it resolved at the model level but not in a function.
const FUNCTION_ASSERTION_LEVEL: &str = r#"
model FunctionAssertionLevel
  function f
    input Real w;
    output Real a;
  algorithm
    assert(w <= 2, "too big", AssertionLevel.error);
    a := 2*w;
  end f;
  Real x(start = 0, fixed = true);
equation
  der(x) = f(time);
end FunctionAssertionLevel;
"#;

const FUNCTION_ASSERTION_WARNING: &str = r#"
model FunctionAssertionWarning
  function f
    input Real w;
    output Real a;
  algorithm
    assert(w <= 2, "too big", level = AssertionLevel.warning);
    a := 2*w;
  end f;
  Real x(start = 0, fixed = true);
equation
  der(x) = f(time);
end FunctionAssertionWarning;
"#;

#[test]
fn function_assertion_with_error_level_is_the_default_assertion() {
    let compiled = compile(FUNCTION_ASSERTION_LEVEL, "FunctionAssertionLevel");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("the assertion holds on [0, 1]");
    let x = result.names.iter().position(|name| name == "x").unwrap();
    let last = *result.data[x].last().unwrap();
    assert!(
        (last - 1.0).abs() < 1e-4,
        "der(x) = 2*time gives x(1) = 1, got {last}"
    );
}

#[test]
fn function_assertion_with_warning_level_keeps_running() {
    let compiled = compile(FUNCTION_ASSERTION_WARNING, "FunctionAssertionWarning");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 3.0,
            ..Default::default()
        },
    )
    .expect("main supports warning-level assertions without turning them into errors");
    assert!(result.termination.is_none());
    assert_eq!(result.times.last().copied(), Some(3.0));
}

// TOOLBUG-101: a call inside an array constructor whose argument reads the
// iterator (MLS §10.4.2) was shape-checked in the enclosing scope, where the
// iterator is not bound, and failed with `unresolved Flat reference i`.
const CALL_IN_COMPREHENSION: &str = r#"
model CallInComprehension
  function g
    input Real a;
    output Real r;
  algorithm
    r := 2*a;
  end g;
  constant Real v[2] = {3, 4};
  parameter Real s = sum({g(v[i]) for i in 1:2});
  Real x(start = 0, fixed = true);
equation
  der(x) = s;
end CallInComprehension;
"#;

#[test]
fn call_reading_a_comprehension_iterator_is_shaped_in_the_iterator_scope() {
    let compiled = compile(CALL_IN_COMPREHENSION, "CallInComprehension");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("the model simulates");
    let x = result.names.iter().position(|name| name == "x").unwrap();
    let last = *result.data[x].last().unwrap();
    assert!(
        (last - 14.0).abs() < 1e-6,
        "s = g(3) + g(4) = 14, got x(1) = {last}"
    );
}

// TOOLBUG-102: a replaceable package that no modification redeclares denotes
// its declared default (MLS §7.3), so `Medium.nX` in a dimension names the
// default's constant. Post-materialization skipped every scope without
// redeclarations and left the member without identity (EF024).
const DEFAULT_REPLACEABLE_PACKAGE_DIMENSION: &str = r#"
package DefaultReplaceable
  partial package PartialMedium
    constant Integer nX = 2;
  end PartialMedium;
  package Air
    extends PartialMedium(nX = 3);
  end Air;
  model Decl
    replaceable package Medium = PartialMedium;
    parameter Real X_start[Medium.nX] = fill(0.5, Medium.nX);
    Real s(start = 0, fixed = true);
  equation
    der(s) = sum(X_start);
  end Decl;
  model Redeclared
    Decl d(redeclare package Medium = Air);
  end Redeclared;
end DefaultReplaceable;
"#;

fn final_value(compiled: &rumoca::CompilationResult, name: &str) -> f64 {
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .expect("the model simulates");
    let column = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("no result column {name}"));
    *result.data[column].last().unwrap()
}

#[test]
fn unredeclared_replaceable_package_dimension_uses_the_default() {
    let default = compile(
        DEFAULT_REPLACEABLE_PACKAGE_DIMENSION,
        "DefaultReplaceable.Decl",
    );
    let s = final_value(&default, "s");
    assert!(
        (s - 1.0).abs() < 1e-6,
        "default nX = 2 gives s(1) = 1, got {s}"
    );
    let redeclared = compile(
        DEFAULT_REPLACEABLE_PACKAGE_DIMENSION,
        "DefaultReplaceable.Redeclared",
    );
    let s = final_value(&redeclared, "d.s");
    assert!(
        (s - 1.5).abs() < 1e-6,
        "redeclared nX = 3 gives s(1) = 1.5, got {s}"
    );
}

// TOOLBUG-103: a function input dimension written over a field of a package
// record constant (`input Real a[sum(proCoe.nT)]`, IDEAS/AixLib antifreeze
// media) names the field's shared record declaration, which has no value of
// its own; the value belongs to the record constant (EF023).
const RECORD_CONSTANT_FIELD_IN_FUNCTION_SHAPE: &str = r#"
package RecordConstantShape
  record Coef
    parameter Integer n;
    Integer nT[n];
  end Coef;
  package Med
    constant Coef proCoe(n = 2, nT = {2, 1});
    function total
      input Real a[sum(proCoe.nT)];
      output Real f;
    algorithm
      f := sum(a);
    end total;
  end Med;
  model M
    Real s(start = 0, fixed = true);
  equation
    der(s) = Med.total({1, 2, 3});
  end M;
end RecordConstantShape;
"#;

#[test]
fn function_shape_over_a_record_constant_field_reads_the_constant() {
    let compiled = compile(
        RECORD_CONSTANT_FIELD_IN_FUNCTION_SHAPE,
        "RecordConstantShape.M",
    );
    let s = final_value(&compiled, "s");
    assert!(
        (s - 6.0).abs() < 1e-6,
        "sum over 3 coefficients gives s(1) = 6, got {s}"
    );
}

// TOOLBUG-104: a constant whose value is a call (MSL `PartialMedium`
// `T_default = Modelica.Units.Conversions.from_degC(20)`) substituted into a
// binding introduced the call after the call graph was collected, so the
// callee was missing and the DAE reported `ED008 unresolved Flat reference`.
const CALL_INTRODUCED_BY_CONSTANT: &str = r#"
package CallByConstant
  package Conv
    function from_degC
      input Real c;
      output Real k;
    algorithm
      k := c + 273.15;
    end from_degC;
  end Conv;
  package Medium
    constant Real T_default = CallByConstant.Conv.from_degC(20);
  end Medium;
  model M
    parameter Real T_start = Medium.T_default;
    Real s(start = 0, fixed = true);
  equation
    der(s) = T_start;
  end M;
end CallByConstant;
"#;

#[test]
fn call_introduced_by_constant_substitution_is_collected() {
    let compiled = compile(CALL_INTRODUCED_BY_CONSTANT, "CallByConstant.M");
    let s = final_value(&compiled, "s");
    assert!(
        (s - 293.15).abs() < 1e-6,
        "T_start = 293.15 K gives s(1) = 293.15, got {s}"
    );
}

// TOOLBUG-106: `function hl_p = Base.hl_p` (MSL `IF97_Utilities`) called
// unqualified from a sibling function rendered the alias exposure but kept the
// target's structured path, and flatten refused it with EF019.
const SHORT_FUNCTION_ALIAS_CALLED_BY_SIBLING: &str = r#"
package ShortAlias
  package Util
    package Base
      function hl_p
        input Real p;
        output Real h;
      algorithm
        h := 2*p;
      end hl_p;
    end Base;
    function hl_p = Base.hl_p;
    function twice
      input Real p;
      output Real h;
    algorithm
      h := hl_p(p);
    end twice;
  end Util;
  model M
    Real s(start = 0, fixed = true);
  equation
    der(s) = Util.twice(1.5) + Util.hl_p(0.5);
  end M;
end ShortAlias;
"#;

#[test]
fn short_function_alias_called_by_a_sibling_keeps_one_exposure() {
    let compiled = compile(SHORT_FUNCTION_ALIAS_CALLED_BY_SIBLING, "ShortAlias.M");
    let s = final_value(&compiled, "s");
    assert!((s - 4.0).abs() < 1e-6, "3 + 1 gives s(1) = 4, got {s}");
}

// TOOLBUG-107: `extends .Lib.Icons.Package` inside an `encapsulated package`
// (Modelica_DeviceDrivers AVR functions) must look `Lib` up in the global
// scope (MLS §5.3.3); the parser dropped the leading dot and lookup stopped at
// the encapsulated boundary (ER003).
const GLOBAL_EXTENDS_IN_ENCAPSULATED: &str = r#"
package GlobalExtends
  package Icons
    partial package Package end Package;
    partial function Function end Function;
  end Icons;
  encapsulated package Fns
    extends .GlobalExtends.Icons.Package;
    function f
      extends .GlobalExtends.Icons.Function;
      input Real x;
      output Real y;
    algorithm
      y := 2*x;
    end f;
  end Fns;
  model M
    Real s(start = 0, fixed = true);
  equation
    der(s) = Fns.f(1.5);
  end M;
end GlobalExtends;
"#;

#[test]
fn leading_dot_extends_crosses_an_encapsulated_boundary() {
    let compiled = compile(GLOBAL_EXTENDS_IN_ENCAPSULATED, "GlobalExtends.M");
    let s = final_value(&compiled, "s");
    assert!((s - 3.0).abs() < 1e-6, "f(1.5) = 3 gives s(1) = 3, got {s}");
}

// TOOLBUG-108: a call through a replaceable package in the middle of a
// qualified name (`ThermoSysPro.Properties.WaterSteam.IF97.Water_Ph`, where
// `replaceable package IF97 = ...`) was reported unresolved (ER002), and the
// member was never proved in the selected class (EF024).
const CALL_THROUGH_INNER_REPLACEABLE_PACKAGE: &str = r#"
package InnerReplaceable
  package Props
    package Impl
      function f
        input Real x;
        output Real y;
      algorithm
        y := 2*x;
      end f;
    end Impl;
    replaceable package P = InnerReplaceable.Props.Impl;
  end Props;
  model M
    Real s(start = 0, fixed = true);
  equation
    der(s) = InnerReplaceable.Props.P.f(1.5);
  end M;
end InnerReplaceable;
"#;

#[test]
fn call_through_an_inner_replaceable_package_uses_its_default() {
    let compiled = compile(CALL_THROUGH_INNER_REPLACEABLE_PACKAGE, "InnerReplaceable.M");
    let s = final_value(&compiled, "s");
    assert!((s - 3.0).abs() < 1e-6, "f(1.5) = 3 gives s(1) = 3, got {s}");
}

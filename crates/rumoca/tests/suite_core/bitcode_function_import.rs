//! Importing an artifact that carries function bodies.
//!
//! Until bodies were carried, import refused every call, so any model that
//! used a function could be exported but never rebuilt. These tests pin the
//! other direction: a body with a nested loop, a conditional join, an
//! assertion and a multi-output call comes back as a checked DAE, re-exports
//! to the same artifact function for function, and simulates.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

const FIXTURE: &str = "\
model FunctionImport
  function sq
    input Real x;
    output Real y;
  algorithm
    y := x*x;
  end sq;
  function acc
    input Real a[3];
    input Real k;
    output Real s;
    output Real m;
  protected
    Real t;
  algorithm
    s := 0;
    m := -1e9;
    for i in 1:3 loop
      t := 1;
      for j in 1:2 loop
        t := t * sq(a[i]) * k;
      end for;
      s := s + t;
      if t > m then
        m := t;
      end if;
    end for;
    assert(s >= 0, \"negative\");
  end acc;
  Real x(start = 1, fixed = true);
  Real s;
  Real m;
equation
  (s, m) = acc({x, 2*x, 3*x}, 0.5);
  der(x) = -s * 0.01 + m * 0.001;
end FunctionImport;
";

fn rumoca(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rumoca"))
        .args(args)
        .output()
        .expect("rumoca runs")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn emit(dir: &Path) -> String {
    let source = dir.join("FunctionImport.mo");
    fs::write(&source, FIXTURE).expect("write fixture");
    let artifact = dir.join("fixture.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--model",
        "FunctionImport",
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    artifact.to_str().unwrap().to_string()
}

#[test]
fn a_carried_body_is_emitted_with_its_loops_and_values() {
    let work = tempdir().expect("temp dir");
    let artifact = emit(work.path());
    let dump = rumoca(&["bitcode", "dump", &artifact]);
    assert!(dump.status.success(), "{}", text(&dump));
    let json: serde_json::Value = serde_json::from_slice(&dump.stdout).expect("json");
    let functions = json["model"]["functions"].as_array().expect("functions");
    let acc = functions
        .iter()
        .find(|f| f["name"].as_str().is_some_and(|n| n.ends_with("acc")))
        .expect("acc is exported");
    assert_eq!(acc["body"]["kind"], "modelica", "the body is carried");
    let folds = acc["folds"].as_array().expect("folds");
    // A statement between the loops keeps them from fusing into one 2-D
    // fold, so the inner loop is a fold nested in the outer one.
    assert_eq!(folds.len(), 2, "two loops: {folds:?}");
    assert!(
        folds.iter().any(|fold| fold["parent"].is_number()),
        "the inner loop names the outer as its parent: {folds:?}"
    );
    assert!(
        acc["values"].as_array().is_some_and(|v| v.len() >= 3),
        "outputs s, m and local t are in the value table"
    );
}

#[test]
fn a_carried_body_round_trips_function_for_function() {
    let work = tempdir().expect("temp dir");
    let artifact = emit(work.path());
    // `round-trip` imports through the checked constructors, re-exports, and
    // compares every expression node and every function, statements,
    // values, folds and provenance included.
    let trip = rumoca(&["bitcode", "round-trip", &artifact]);
    assert!(trip.status.success(), "{}", text(&trip));
    assert!(
        text(&trip).contains("round-trip preserves"),
        "{}",
        text(&trip)
    );
}

#[test]
fn a_model_that_calls_functions_rebuilds_and_simulates_from_bitcode() {
    let work = tempdir().expect("temp dir");
    let artifact = emit(work.path());
    let run = rumoca(&[
        "compile-bitcode",
        &artifact,
        "--simulate",
        "--check",
        "--t-end",
        "1",
    ]);
    assert!(run.status.success(), "{}", text(&run));
    assert!(
        String::from_utf8_lossy(&run.stdout).trim() == "[]",
        "no runtime violation expected: {}",
        text(&run)
    );
}

const QUOTIENTS: &str = "\
model QuotientImport
  function wrap
    input Real x;
    output Real y;
  algorithm
    y := mod(x, 0.7);
  end wrap;
  parameter Integer n = 3;
  Real v[n] = fill(time, n);
  Real r;
  Real w;
equation
  r = rem(time, 0.3);
  w = wrap(time);
end QuotientImport;
";

#[test]
fn quotients_and_parameter_extents_rebuild_from_bitcode() {
    // A varying `rem` in an equation carries an event owner (seven nodes, a
    // relation, an activation, a root); one in a function body is owned by
    // the function; `fill(time, n)` needs `n` defined before it is built.
    // Each was a way import refused a model the compiler had accepted.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("QuotientImport.mo");
    fs::write(&source, QUOTIENTS).expect("write fixture");
    let artifact = work.path().join("quotients.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--model",
        "QuotientImport",
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let artifact = artifact.to_str().unwrap();
    let trip = rumoca(&["bitcode", "round-trip", artifact]);
    assert!(trip.status.success(), "{}", text(&trip));
    let run = rumoca(&[
        "compile-bitcode",
        artifact,
        "--simulate",
        "--check",
        "--t-end",
        "1",
    ]);
    assert!(run.status.success(), "{}", text(&run));
}

#[test]
fn linking_two_artifacts_relocates_their_function_bodies() {
    // A carried body names arena expressions, types, domains and sources.
    // Linking once moved only the function's id and signature, so the second
    // instance's body still pointed into the first one's expressions and the
    // result failed to rebuild ("parameter from function 0 cannot be used in
    // function 2").
    let work = tempdir().expect("temp dir");
    let artifact = emit(work.path());
    let linked = work.path().join("linked.rbc");
    let link = rumoca(&[
        "bitcode",
        "link",
        "-o",
        linked.to_str().unwrap(),
        &format!("a={artifact}"),
        &format!("b={artifact}"),
    ]);
    assert!(link.status.success(), "{}", text(&link));
    let rebuilt = rumoca(&["compile-bitcode", linked.to_str().unwrap(), "--summary"]);
    assert!(rebuilt.status.success(), "{}", text(&rebuilt));
}

#[test]
fn elementwise_operators_cross_the_interchange_boundary() {
    // `.+ .- .* ./ .^` were missing from `RbcBinaryOp`, so any model using
    // one exported an `Unsupported` node and could not be rebuilt.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("Elementwise.mo");
    fs::write(
        &source,
        "model Elementwise\n  Real x[3](each start = 1, each fixed = true);\n  \
         parameter Real k[3] = {1, 2, 3};\nequation\n  \
         der(x) = -(k .* x) ./ (k .+ x) + (x .^ k) .- x .* x;\nend Elementwise;\n",
    )
    .expect("write fixture");
    let artifact = work.path().join("elementwise.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let trip = rumoca(&["bitcode", "round-trip", artifact.to_str().unwrap()]);
    assert!(trip.status.success(), "{}", text(&trip));
}

#[test]
fn temporal_owners_cross_the_interchange_boundary() {
    // `delay` (fixed and bounded), `previous` and `terminal()` each have an
    // owner table beside the coordinate that reads it. Export used to refuse
    // all three; they are carried and rebuilt now.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("Temporal.mo");
    fs::write(
        &source,
        "model Temporal\n  Real x(start = 1, fixed = true);\n  Real y;\n  Real z;\n  \
         parameter Real tau = 0.1;\n  Boolean done;\n  Clock c = Clock(0.1);\n  \
         discrete Real s(start = 0);\nequation\n  der(x) = -x;\n  y = delay(x, tau);\n  \
         z = delay(x, 0.05 + 0.01 * x, 0.2);\n  done = terminal();\n  when c then\n    \
         s = previous(s) + sample(x, c);\n  end when;\nend Temporal;\n",
    )
    .expect("write fixture");
    let artifact = work.path().join("temporal.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let artifact = artifact.to_str().unwrap();
    let trip = rumoca(&["bitcode", "round-trip", artifact]);
    assert!(trip.status.success(), "{}", text(&trip));
    let run = rumoca(&[
        "compile-bitcode",
        artifact,
        "--simulate",
        "--check",
        "--t-end",
        "1",
    ]);
    assert!(run.status.success(), "{}", text(&run));
}

const PURE_CALL: &str = "\
model PureCall
  function tri
    input Integer n;
    input Real w;
    output Real s;
  algorithm
    s := 0;
    for i in 1:n loop
      if i > 2 then
        s := s + i * w;
      end if;
    end for;
  end tri;
  parameter Real p = tri(4, 0.5);
  Real x(start = p, fixed = true);
equation
  der(x) = -x;
end PureCall;
";

fn disassemble_compiled(dir: &Path, extra: &[&str]) -> String {
    let source = dir.join("PureCall.mo");
    fs::write(&source, PURE_CALL).expect("write fixture");
    let artifact = dir.join(format!("pure{}.rbc", extra.len()));
    let mut args = vec![
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let compiled = rumoca(&args);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let listing = rumoca(&["bitcode", "disasm", artifact.to_str().unwrap()]);
    String::from_utf8_lossy(&listing.stdout).into_owned()
}

#[test]
fn pure_call_folding_is_a_pass_not_a_frontend_step() {
    // The frontend no longer folds `tri(4, 0.5)` into the binding: that is
    // an optimization, and it lives in `fold-pure-calls`. Evaluating the
    // body -- a loop carrying `s` through a conditional -- gives
    // 3 * 0.5 + 4 * 0.5 = 3.5.
    let work = tempdir().expect("temp dir");
    let frontend = disassemble_compiled(work.path(), &["--pass", "none"]);
    assert!(
        frontend.contains("binding=tri(4, 0.5)"),
        "the frontend leaves the call in place:\n{frontend}"
    );
    let folded = disassemble_compiled(work.path(), &["--pass", "fold-pure-calls"]);
    assert!(
        folded.contains("binding=3.5"),
        "the pass evaluates the body:\n{folded}"
    );
    let default = disassemble_compiled(work.path(), &[]);
    assert!(
        default.contains("binding=3.5"),
        "a plain compile runs the default group, which folds it:\n{default}"
    );
}

#[test]
fn a_settled_binding_that_indexes_out_of_bounds_is_still_refused() {
    // Moving pure-call folding out of the frontend kept its one diagnostic:
    // a parameter binding whose evaluation provably indexes out of bounds is
    // a translation error (EF036), not something to discover at run time.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("OutOfBounds.mo");
    fs::write(
        &source,
        "model OutOfBounds\n  function pick\n    input Real v[3];\n    input Integer k;\n    \
         output Real y;\n  algorithm\n    y := v[k];\n  end pick;\n  \
         parameter Real b = pick({1, 2, 3}, 4);\n  Real x(start = b, fixed = true);\n\
         equation\n  der(x) = -x;\nend OutOfBounds;\n",
    )
    .expect("write fixture");
    let compiled = rumoca(&["compile", source.to_str().unwrap()]);
    assert!(
        !compiled.status.success(),
        "must be refused: {}",
        text(&compiled)
    );
    assert!(text(&compiled).contains("EF036"), "{}", text(&compiled));
}

#[test]
fn the_text_profile_round_trips_a_carried_body_exactly() {
    // `emit-text` printed a carried body as "N statements not in text" and a
    // function-value node as a placeholder, so `assemble` refused its own
    // output for any model with a function. Bodies, value tables, folds and
    // call edges are written as function records now, and the reassembled
    // artifact is the original.
    let work = tempdir().expect("temp dir");
    let artifact = emit(work.path());
    let text = work.path().join("fixture.rbt");
    let back = work.path().join("back.rbc");
    let emitted = rumoca(&[
        "bitcode",
        "emit-text",
        &artifact,
        "-o",
        text.to_str().unwrap(),
        "--sources",
    ]);
    assert!(emitted.status.success(), "{}", text_of(&emitted));
    let assembled = rumoca(&[
        "bitcode",
        "assemble",
        text.to_str().unwrap(),
        "-o",
        back.to_str().unwrap(),
    ]);
    assert!(assembled.status.success(), "{}", text_of(&assembled));
    let dump = |path: &str| -> serde_json::Value {
        let out = rumoca(&["bitcode", "dump", path]);
        let mut value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
        value["producer"] = serde_json::Value::Null;
        value
    };
    assert_eq!(dump(&artifact), dump(back.to_str().unwrap()));
}

fn text_of(output: &Output) -> String {
    text(output)
}

#[test]
fn a_model_with_an_unconnected_input_runs_when_its_inputs_are_held() {
    // A block tested standalone has top-level inputs nothing drives, and the
    // simulator refused it ("has neither a checked default nor a runtime
    // value"). `--input` holds one at a value; `--free-inputs start` holds
    // every unbound one at its start value, else 0, and says which.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("Held.mo");
    fs::write(
        &source,
        "model Held\n  input Real u;\n  Real x(start = 1, fixed = true, max = 1.5);\n\
         equation\n  der(x) = -x + u;\nend Held;\n",
    )
    .expect("write fixture");
    let artifact = work.path().join("held.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let artifact = artifact.to_str().unwrap();
    let simulate = |extra: &[&str]| {
        let mut args = vec![
            "compile-bitcode",
            artifact,
            "--simulate",
            "--check",
            "--t-end",
            "1",
        ];
        args.extend_from_slice(extra);
        rumoca(&args)
    };
    let refused = simulate(&[]);
    assert!(
        text(&refused).contains("has neither a checked default nor a runtime value"),
        "{}",
        text(&refused)
    );
    let held = simulate(&["--free-inputs", "start"]);
    assert!(held.status.success(), "{}", text(&held));
    assert!(
        text(&held).contains("holding 1 free input(s) constant"),
        "{}",
        text(&held)
    );
    // Held at 2 for the whole run, x crosses its max near t = ln 2; held
    // only at t = 0 it would not.
    let driven = simulate(&["--input", "u=2"]);
    assert!(text(&driven).contains("above-max"), "{}", text(&driven));
}

#[test]
fn check_reports_each_array_element_against_its_declared_bound() {
    // The solver reports `y[1]`, `y[2]`, ... while bounds are declared on
    // `y`, so `--check` never checked an array element. A scalar bound now
    // applies to every element, a literal array bound element-wise.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("ArrayBounds.mo");
    fs::write(
        &source,
        "model ArrayBounds\n  Real x[3](each start = 1, each fixed = true);\n  \
         Real y[3](each max = 2.9);\n  Real w[3](max = {5, 2.5, 5});\nequation\n  \
         der(x) = -x;\n  y = 2 .+ x;\n  w = {2, 2, 2} + x;\nend ArrayBounds;\n",
    )
    .expect("write fixture");
    let artifact = work.path().join("bounds.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let checked = rumoca(&[
        "compile-bitcode",
        artifact.to_str().unwrap(),
        "--simulate",
        "--check",
        "--t-end",
        "0.1",
    ]);
    let report: serde_json::Value = serde_json::from_slice(&checked.stdout).expect("json");
    let mut broken: Vec<String> = report
        .as_array()
        .expect("violations")
        .iter()
        .map(|v| v["variable"].as_str().unwrap_or_default().to_string())
        .collect();
    broken.sort();
    assert_eq!(broken, ["w[2]", "y[1]", "y[2]", "y[3]"]);
}

#[test]
fn a_zero_size_array_with_a_scalar_attribute_simulates() {
    // `Xi[nXi]` with `nXi = 0` and a scalar `nominal`/`start` has nothing to
    // apply them to. A single value spread only over N > 1 elements, so this
    // was refused as "nominal must contain 0 finite positive values".
    let work = tempdir().expect("temp dir");
    let source = work.path().join("ZeroSize.mo");
    fs::write(
        &source,
        "model ZeroSize\n  parameter Integer n = 0;\n  Real v[n](each nominal = 2, each start = 1);\n  \
         Real x(start = 1, fixed = true);\nequation\n  v = fill(x, n);\n  der(x) = -x;\n\
         end ZeroSize;\n",
    )
    .expect("write fixture");
    let artifact = work.path().join("zero.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let run = rumoca(&[
        "compile-bitcode",
        artifact.to_str().unwrap(),
        "--simulate",
        "--check",
        "--t-end",
        "0.1",
    ]);
    assert!(run.status.success(), "{}", text(&run));
}

#[test]
fn a_start_written_over_an_input_evaluates_from_the_inputs_value() {
    // `x(start = 3 * u)` with `input Real u = 2`: the input has a value at the
    // initial instant, but the evaluator refused any input coordinate as
    // "a runtime coordinate". It now resolves the input as it does anywhere
    // else -- binding, or the host-supplied value -- so x(0) = 6.
    let work = tempdir().expect("temp dir");
    let source = work.path().join("InputStart.mo");
    fs::write(
        &source,
        "model InputStart\n  input Real u = 2;\n  Real x(start = 3 * u, fixed = true, max = 5.9);\n\
         equation\n  der(x) = -x;\nend InputStart;\n",
    )
    .expect("write fixture");
    let artifact = work.path().join("input_start.rbc");
    let compiled = rumoca(&[
        "compile",
        source.to_str().unwrap(),
        "--emit-bitcode",
        artifact.to_str().unwrap(),
    ]);
    assert!(
        artifact.exists(),
        "fixture must compile: {}",
        text(&compiled)
    );
    let run = rumoca(&[
        "compile-bitcode",
        artifact.to_str().unwrap(),
        "--simulate",
        "--check",
        "--t-end",
        "0.01",
    ]);
    let report: serde_json::Value = serde_json::from_slice(&run.stdout).expect("json");
    assert_eq!(report[0]["kind"], "above-max", "{}", text(&run));
    assert_eq!(report[0]["value"], 6.0, "{}", text(&run));
}

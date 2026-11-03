//! Assertions of calls a function reaches inside a whole-array loop
//! (MLS 3.7 §8.3.7, §11.2.8.1).
//!
//! DAE construction lowers `for i in 1:3 loop R[i] := elem(q, i); end for;`
//! to one comprehension, `R := {elem(q, i) for i in 1:3}`. Every element runs
//! `elem`, so every element can fail its assertion, and the first failing
//! element ends the function. The Solve map cannot publish those assertion
//! slots itself; a fold over the same domain carries them. The shape is the
//! FIRE library's `quaternionToRotationMatrix`, whose
//! `quaternionRotationElement` checks its row and column.

use std::path::Path;
use std::process::Command;

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

const SOURCE: &str = r#"
model Elements
  function elem
    input Real q[3];
    input Integer i;
    input Integer j;
    output Real y;
  algorithm
    assert(q[i] < LIMIT, "element " + String(i) + "," + String(j) + " too large");
    y := q[i]*j;
  end elem;
  function column
    input Real q[3];
    output Real r[3];
  algorithm
    for i in 1:3 loop
      r[i] := elem(q, i, 1);
    end for;
  end column;
  function grid
    input Real q[3];
    output Real m[3, 2];
  algorithm
    for i in 1:3 loop
      for j in 1:2 loop
        m[i, j] := elem(q, i, j);
      end for;
    end for;
  end grid;
  Real x(start = 1, fixed = true);
  Real r[3];
  Real m[3, 2];
initial equation
  assert(max(abs(grid({1, 2, 3}) - [1, 2; 2, 4; 3, 6])) < 1e-12, "initial grid");
equation
  der(x) = -x;
  r = column({x, 2*x, 3*x});
  m = grid({x, 2*x, 3*x});
end Elements;
"#;

fn compiled(limit: &str) -> rumoca::CompilationResult {
    Compiler::new()
        .model("Elements")
        .compile_str(&SOURCE.replace("LIMIT", limit), "Elements.mo")
        .unwrap_or_else(|error| panic!("Elements compiles: {error:?}"))
}

fn simulate(limit: &str) -> Result<SimResult, String> {
    simulate_dae_with_diagnostics(
        &compiled(limit).dae,
        &SimOptions {
            t_end: 1.0,
            ..SimOptions::default()
        },
    )
    .map_err(|error| format!("{error:?}"))
}

fn column<'a>(result: &'a SimResult, name: &str) -> &'a [f64] {
    let index = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("{name} is recorded"));
    &result.data[index]
}

#[test]
fn asserting_calls_in_a_loop_comprehension_keep_their_values() {
    let result = simulate("100").expect("Elements simulates");
    for sample in 0..result.times.len() {
        let x = column(&result, "x")[sample];
        for i in 1..=3 {
            let r = column(&result, &format!("r[{i}]"))[sample];
            assert!((r - i as f64 * x).abs() < 1e-9, "r[{i}] = {r} at x = {x}");
            for j in 1..=2 {
                let m = column(&result, &format!("m[{i},{j}]"))[sample];
                let expected = (i * j) as f64 * x;
                assert!((m - expected).abs() < 1e-9, "m[{i},{j}] = {m} at x = {x}");
            }
        }
    }
}

#[test]
fn the_first_failing_comprehension_element_ends_the_call() {
    // At t = 0, q = {1, 2, 3}: elements 2 and 3 both fail, and the message
    // must be the one of element 2, the first in iteration order.
    let error = simulate("1.5").expect_err("elem fails inside the comprehension");
    assert!(error.contains("element 2,1 too large"), "{error}");
}

#[test]
fn asserting_calls_in_a_loop_comprehension_export_to_fmi3() {
    let files = rumoca::render_target_files(&compiled("100"), "Elements", "fmi3", None)
        .unwrap_or_else(|error| panic!("Elements exports to fmi3: {error:#}"));
    assert!(
        files.iter().any(|file| file.path == "sources/model.c"),
        "the fmi3 export renders its C model"
    );
}

/// A C FMI 3 Co-Simulation driver for the growing variant (`der(x) = x`,
/// limit 4): every step checks `m[i,j] = i*j*x`, and the first failing step
/// must report element 3,1, the first of the two elements that cross the
/// limit together when `3*x` reaches 4 (t = ln(4/3) = 0.2877).
const CS_DRIVER: &str = r#"
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "fmi3Functions.h"

static char first_error[512];

static void logger(fmi3InstanceEnvironment env, fmi3Status status, fmi3String category,
                   fmi3String message) {
    (void)env; (void)category;
    if (status >= fmi3Error && first_error[0] == '\0') {
        strncpy(first_error, message, sizeof first_error - 1);
    }
}

int main(int argc, char **argv) {
    if (argc != 4) return 90;
    fmi3Instance cs = fmi3InstantiateCoSimulation(
        "cs", argv[1], NULL, fmi3False, fmi3True, fmi3False, fmi3False, NULL, 0, NULL,
        logger, NULL);
    if (!cs) return 91;
    if (fmi3EnterInitializationMode(cs, fmi3False, 0.0, 0.0, fmi3True, 1.0) != fmi3OK) return 92;
    if (fmi3ExitInitializationMode(cs) != fmi3OK) return 93;
    const fmi3ValueReference x_vr = (fmi3ValueReference)atoi(argv[2]);
    const fmi3ValueReference m_vr = (fmi3ValueReference)atoi(argv[3]);
    double t = 0.0;
    for (int k = 0; k < 100; ++k) {
        fmi3Boolean event_needed, terminate, early_return;
        fmi3Float64 last;
        if (fmi3DoStep(cs, t, 0.01, fmi3True, &event_needed, &terminate, &early_return,
                       &last) != fmi3OK) {
            printf("failed %.6f %s\n", t, first_error);
            return 0;
        }
        t += 0.01;
        double x, m[6];
        if (fmi3GetFloat64(cs, &x_vr, 1, &x, 1) != fmi3OK) return 94;
        if (fmi3GetFloat64(cs, &m_vr, 1, m, 6) != fmi3OK) return 95;
        for (int i = 0; i < 3; ++i)
            for (int j = 0; j < 2; ++j)
                if (fabs(m[2 * i + j] - (i + 1) * (j + 1) * x) > 1e-9 * fabs(x)) return 96;
        if (fabs(x - exp(t)) > 1e-4 * exp(t)) return 97;
    }
    printf("completed\n");
    return 0;
}
"#;

fn description_attribute<'a>(description: &'a str, anchor: &str, attribute: &str) -> &'a str {
    let start = description
        .find(anchor)
        .unwrap_or_else(|| panic!("modelDescription.xml names {anchor}"));
    let tail = &description[start..];
    let key = format!("{attribute}=\"");
    let value = &tail[tail.find(&key).expect("attribute is present") + key.len()..];
    &value[..value.find('"').expect("attribute is closed")]
}

#[test]
fn the_generated_c_component_raises_the_first_failing_element() {
    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("skipping: no C compiler");
        return;
    }
    let source = SOURCE
        .replace("LIMIT", "4")
        .replace("der(x) = -x;", "der(x) = x;");
    let compiled = Compiler::new()
        .model("Elements")
        .compile_str(&source, "Elements.mo")
        .unwrap_or_else(|error| panic!("Elements compiles: {error:?}"));
    let files = rumoca::render_target_files(&compiled, "Elements", "fmi3", None)
        .unwrap_or_else(|error| panic!("Elements exports to fmi3: {error:#}"));
    let work = tempfile::tempdir().expect("C harness work directory");
    let mut sources = Vec::new();
    let mut description = String::new();
    for file in &files {
        let path = work.path().join(&file.path);
        std::fs::create_dir_all(path.parent().expect("rendered files have a parent"))
            .expect("create rendered directory");
        std::fs::write(&path, &file.content).expect("write rendered file");
        if file.path.starts_with("sources/") && file.path.ends_with(".c") {
            sources.push(path);
        } else if file.path == "modelDescription.xml" {
            description = file.content.clone();
        }
    }
    let token = description_attribute(&description, "<fmiModelDescription", "instantiationToken");
    let x_vr = description_attribute(&description, "name=\"x\"", "valueReference");
    let m_vr = description_attribute(&description, "name=\"m\"", "valueReference");
    let driver = work.path().join("driver.c");
    std::fs::write(&driver, CS_DRIVER).expect("write the C driver");
    let headers = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let executable = work.path().join("elements_cs");
    let build = Command::new("cc")
        .args([
            "-std=c11",
            "-O1",
            "-ffp-contract=off",
            "-DFMI3_OVERRIDE_FUNCTION_PREFIX=1",
        ])
        .arg("-I")
        .arg(work.path().join("sources"))
        .arg("-I")
        .arg(&headers)
        .args(&sources)
        .arg(&driver)
        .args(["-lm", "-o"])
        .arg(&executable)
        .output()
        .expect("run cc");
    assert!(
        build.status.success(),
        "the generated C compiles: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&executable)
        .args([token, x_vr, m_vr])
        .output()
        .expect("run the C driver");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "driver status {:?}: {stdout}",
        run.status.code()
    );
    let failed = stdout
        .strip_prefix("failed ")
        .unwrap_or_else(|| panic!("the component raises the assertion: {stdout}"));
    let (time, message) = failed.trim().split_once(' ').expect("time and message");
    let time: f64 = time.parse().expect("failure time");
    assert!(
        (time - 0.28).abs() < 1e-9,
        "the step containing t = ln(4/3) = 0.2877 fails, not {time}"
    );
    assert!(
        message.contains("element 3,1 too large"),
        "the first failing element's message: {message}"
    );
}

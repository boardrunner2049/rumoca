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

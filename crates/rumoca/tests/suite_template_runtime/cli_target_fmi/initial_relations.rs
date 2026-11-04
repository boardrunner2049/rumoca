//! SPEC_0044 ME-EVENT-011: relations outside continuous integration take the
//! values of their operands. At initialization a relation is evaluated from
//! the start values (not its relation-memory seed) and the initialization
//! system is solved under relations consistent with its solution; an input
//! written in Event or Step Mode moves the relations before the next read or
//! event iteration, so no equation the written value deselects is evaluated.
//!
//! A C harness compiled against the packaged `model.c` drives the FMI 3
//! interfaces directly, as `time_events.rs` does.

use super::time_events::{
    assert_close, attribute, field, find_line, harness_tools_available, value_reference,
    vendored_fmi3_headers,
};
use super::*;

const SOURCE: &str = r#"
model InitialRelation
  parameter Real d = 0;
  Real den;
  Boolean parallel;
  output Real x;
  output Integer status;
equation
  den = d * (1 + 0 * time);
  parallel = abs(den) < 1e-9;
  x = if parallel then 0 else 1 / den;
  status = if parallel or x < 0 then 2 else 1;
end InitialRelation;

model InputRelation
  input Real u(start = 1);
  Real area(start = 0, fixed = true);
  output Real y;
equation
  y = if u > 0.5 then sqrt(u - 0.5) else 0;
  der(area) = y;
end InputRelation;

model ArrayInputRelation
  input Real v[2](start = {1, 0});
  output Real y;
equation
  assert(abs(v[1]^2 + v[2]^2 - 1) < 1e-9, "v must be a unit vector");
  y = if v[1] > 0.5 then 1 else 0;
end ArrayInputRelation;
"#;

/// `MODEL` selects the scenario; `D`, `X`, `U`, `V`, `AREA`, and `Y` are value
/// references (no variable is named like a unit, which the description also
/// lists by name). Every failed call exits with a distinct status.
const HARNESS: &str = r#"#include "model.c"
#include <stdio.h>

static void quiet(fmi3InstanceEnvironment e, fmi3Status s, fmi3String c, fmi3String m) { (void)e; (void)s; (void)c; fprintf(stderr, "%s\n", m); }
static double get_f(fmi3Instance i, fmi3ValueReference vr) { fmi3Float64 v = NAN; if (fmi3GetFloat64(i, &vr, 1, &v, 1) != fmi3OK) exit(4); return v; }
static void set_f(fmi3Instance i, fmi3ValueReference vr, double v) { if (fmi3SetFloat64(i, &vr, 1, &v, 1) != fmi3OK) exit(5); }
static void step(fmi3Instance i, double t, double h) {
    fmi3Boolean ev, term, early; fmi3Float64 last;
    if (fmi3DoStep(i, t, h, fmi3True, &ev, &term, &early, &last) != fmi3OK) exit(3);
}
static void me_update(fmi3Instance i) {
    fmi3Boolean need = fmi3True, term, nominals, values, defined; fmi3Float64 next;
    while (need) if (fmi3UpdateDiscreteStates(i, &need, &term, &nominals, &values, &defined, &next) != fmi3OK) exit(6);
}

int main(void) {
#if MODEL == 0
    /* The relation reads an algebraic its parameter makes zero. */
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK) return 2;
    printf("cs init x %.17g\n", get_f(i, X));
    if (fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    printf("cs start x %.17g\n", get_f(i, X));
    step(i, 0.0, 0.5);
    printf("cs end x %.17g\n", get_f(i, X));
    fmi3FreeInstance(i);
    /* A parameter written in Initialization Mode after a read. */
    i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK) return 2;
    printf("cs set before %.17g\n", get_f(i, X));
    set_f(i, D, 4.0);
    printf("cs set init x %.17g\n", get_f(i, X));
    if (fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    printf("cs set start x %.17g\n", get_f(i, X));
    fmi3FreeInstance(i);
    i = fmi3InstantiateModelExchange("h", TOKEN, NULL, fmi3False, fmi3False, NULL, quiet);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    me_update(i);
    printf("me start x %.17g\n", get_f(i, X));
#elif MODEL == 2
    /* A host writes an array input element by element, reading the array
       back between the writes (as a co-simulation master merging element
       signals does); the half-written array violates the model's assert, so
       reading it back must not evaluate the model. */
    fmi3ValueReference v = V; fmi3Float64 values[2];
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK) return 2;
    values[0] = 1.0; values[1] = 0.0;
    if (fmi3SetFloat64(i, &v, 1, values, 2) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    printf("cs start y %.17g\n", get_f(i, Y));
    step(i, 0.0, 0.1);
    if (fmi3GetFloat64(i, &v, 1, values, 2) != fmi3OK) return 7;
    values[0] = 0.0;
    if (fmi3SetFloat64(i, &v, 1, values, 2) != fmi3OK) return 5;
    if (fmi3GetFloat64(i, &v, 1, values, 2) != fmi3OK) return 8;
    printf("cs half v %.17g %.17g\n", values[0], values[1]);
    values[1] = 1.0;
    if (fmi3SetFloat64(i, &v, 1, values, 2) != fmi3OK) return 5;
    printf("cs written y %.17g\n", get_f(i, Y));
    step(i, 0.1, 0.1);
    printf("cs end y %.17g\n", get_f(i, Y));
#else
    /* The input crosses the relation between communication steps; every
       read between the write and the step sees the written side. */
    const double inputs[4] = { 0.0, 1.0, 0.0, 2.5 };
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK) return 2;
    set_f(i, U, 0.0);
    printf("cs init y %.17g\n", get_f(i, Y));
    set_f(i, U, 1.0);
    if (fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    printf("cs start y %.17g\n", get_f(i, Y));
    for (int k = 0; k < 4; ++k) {
        step(i, 0.1 * k, 0.1);
        set_f(i, U, inputs[k]);
        printf("cs %d written y %.17g\n", k, get_f(i, Y));
    }
    step(i, 0.4, 0.1);
    printf("cs end area %.17g y %.17g\n", get_f(i, AREA), get_f(i, Y));
    fmi3FreeInstance(i);
    /* Model Exchange: an input written in Event Mode, before and after the
       initial event. */
    i = fmi3InstantiateModelExchange("h", TOKEN, NULL, fmi3False, fmi3False, NULL, quiet);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    set_f(i, U, 0.0);
    printf("me initial written y %.17g\n", get_f(i, Y));
    me_update(i);
    printf("me initial y %.17g\n", get_f(i, Y));
    set_f(i, U, 3.0);
    printf("me written y %.17g\n", get_f(i, Y));
    me_update(i);
    printf("me updated y %.17g\n", get_f(i, Y));
#endif
    return 0;
}
"#;

#[test]
fn initialization_evaluates_relations_from_their_operands() {
    if !harness_tools_available() {
        return;
    }
    let output = run_harness("InitialRelation", 0);
    for prefix in [
        "cs init x ",
        "cs start x ",
        "cs end x ",
        "cs set before ",
        "me start x ",
    ] {
        let line = find_line(&output, prefix);
        assert_close(field(&line, 3), 0.0, 0.0, &line);
    }
    // d = 4 written in Initialization Mode moves the relation to `1 / den`.
    for prefix in ["cs set init x ", "cs set start x "] {
        let line = find_line(&output, prefix);
        assert_close(field(&line, 4), 0.25, 1e-15, &line);
    }
}

#[test]
fn an_input_written_across_a_relation_moves_it_before_the_next_read() {
    if !harness_tools_available() {
        return;
    }
    let output = run_harness("InputRelation", 1);
    let half = 0.5_f64.sqrt();
    let init = find_line(&output, "cs init y ");
    assert_close(field(&init, 3), 0.0, 0.0, &init);
    let start = find_line(&output, "cs start y ");
    assert_close(field(&start, 3), half, 1e-15, &start);
    for (k, y) in [(0, 0.0), (1, half), (2, 0.0), (3, 2.0_f64.sqrt())] {
        let line = find_line(&output, &format!("cs {k} written y "));
        assert_close(field(&line, 4), y, 1e-15, &line);
    }
    // u held at 1, 0, 1, 0, 2.5 over five steps of 0.1.
    let end = find_line(&output, "cs end area ");
    assert_close(
        field(&end, 3),
        0.1 * (2.0 * half + 2.0_f64.sqrt()),
        1e-9,
        &end,
    );
    assert_close(field(&end, 5), 2.0_f64.sqrt(), 1e-15, &end);
    for (prefix, y) in [
        ("me initial written y ", 0.0),
        ("me initial y ", 0.0),
        ("me written y ", 2.5_f64.sqrt()),
        ("me updated y ", 2.5_f64.sqrt()),
    ] {
        let line = find_line(&output, prefix);
        assert_close(
            field(&line, prefix.split_whitespace().count()),
            y,
            1e-15,
            &line,
        );
    }
}

#[test]
fn an_input_reads_back_its_written_value_without_an_evaluation() {
    if !harness_tools_available() {
        return;
    }
    let output = run_harness("ArrayInputRelation", 2);
    let start = find_line(&output, "cs start y ");
    assert_close(field(&start, 3), 1.0, 0.0, &start);
    let half = find_line(&output, "cs half v ");
    assert_close(field(&half, 3), 0.0, 0.0, &half);
    assert_close(field(&half, 4), 0.0, 0.0, &half);
    for prefix in ["cs written y ", "cs end y "] {
        let line = find_line(&output, prefix);
        assert_close(field(&line, 3), 0.0, 0.0, &line);
    }
}

/// Package `model`, compile the harness scenario against it, and return its
/// standard output.
fn run_harness(model: &str, scenario: u32) -> String {
    let compiled = rumoca::Compiler::new()
        .model(model)
        .compile_str(SOURCE, "InitialRelations.mo")
        .unwrap_or_else(|error| panic!("compile {model}: {error:?}"));
    let work = tempdir().expect("initial-relation harness work directory");
    let fmu = build_named_fmu(work.path(), &compiled, "fmi3", model);
    let sources = fmu.root.join("sources");
    let description = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the packaged model description");
    let token = attribute(&description, "instantiationToken");
    let mut command = Command::new("cc");
    command
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-Wno-unused-function")
        .arg("-DFMI3_OVERRIDE_FUNCTION_PREFIX=1")
        .arg(format!("-DMODEL={scenario}"))
        .arg(format!("-DTOKEN=\"{token}\""));
    for (define, name) in [
        ("D", "d"),
        ("X", "x"),
        ("U", "u"),
        ("V", "v"),
        ("AREA", "area"),
        ("Y", "y"),
    ] {
        let reference = value_reference(&description, name).unwrap_or(0);
        command.arg(format!("-D{define}={reference}"));
    }
    let harness = work.path().join("initial_relations_harness.c");
    fs::write(&harness, HARNESS).expect("write the initial-relation harness");
    let binary = work.path().join("initial_relations_harness");
    checked_output(
        command
            .arg(format!("-I{}", sources.display()))
            .arg(format!("-I{}", vendored_fmi3_headers().display()))
            .arg(&harness)
            .args(
                [
                    "rmc_assign.c",
                    "rmc_rows.c",
                    "rmc_jacobian.c",
                    "rmc_isolators.c",
                    "rmc_functions.c",
                ]
                .map(|unit| sources.join(unit)),
            )
            .arg("-lm")
            .arg("-o")
            .arg(&binary),
        &format!("compile the {model} initial-relation harness"),
    );
    let output = checked_output(
        &mut Command::new(&binary),
        &format!("run the {model} initial-relation harness"),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

//! SPEC_0044 ME-EVENT-010: an MLS §3.7.5 `when sample(start, interval)` runs
//! in the generated FMI 3 C component as a time event, not a synchronous
//! clock partition. Each tick is announced and stopped at, computed from its
//! tick counter (no drift), consumed exactly once however the communication
//! points fall, and its body runs once per tick. A start or interval over a
//! parameter without a translation-time value is read at Exit
//! Initialization Mode.

use super::*;

const SOURCE: &str = r#"
model Exact
  Integer n(start = 0, fixed = true);
  discrete Real ts(start = -1, fixed = true);
equation
  when sample(0, 0.1) then
    n = pre(n) + 1;
    ts = time;
  end when;
end Exact;

model Parametric
  parameter Real frequency;
  Integer n(start = 0, fixed = true);
  discrete Real ts(start = -1, fixed = true);
equation
  when sample(0.05, 1 / frequency) then
    n = pre(n) + 1;
    ts = time;
  end when;
end Parametric;

model Order
  discrete Real a(start = 0, fixed = true);
  discrete Real b(start = 0, fixed = true);
  Real u[2];
equation
  when sample(0, 0.1) then
    b = u[1] + u[2];
  end when;
  u[1] = 2*a;
  u[2] = a;
  when sample(0, 0.1) then
    a = pre(a) + 1;
  end when;
end Order;

model Tunable
  parameter Real period = 0.1;
  parameter Real offset = 0.05;
  Real twice = 2*period;
  discrete Real n(start = 0, fixed = true);
  discrete Real ts(start = -1, fixed = true);
equation
  when sample(offset, period) then
    n = pre(n) + 1;
    ts = time;
  end when;
end Tunable;
"#;

/// Prints `cs <step> <value>...` for the `VARS` value references after each
/// Co-Simulation step of `STEP`; `PERIOD_VALUE > 0` sets the `PERIOD` value
/// reference before initialization.
const STEP_HARNESS: &str = r#"#include "model.c"
#include <stdio.h>

static void quiet(fmi3InstanceEnvironment e, fmi3Status s, fmi3String c, fmi3String m) { (void)e; (void)s; (void)c; fprintf(stderr, "%s\n", m); }
static const fmi3ValueReference vars[] = { VARS };
#define NVARS (sizeof(vars) / sizeof(vars[0]))

int main(void) {
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i) return 2;
    if (PERIOD_VALUE > 0.0) { fmi3ValueReference vr = PERIOD; fmi3Float64 v = PERIOD_VALUE; if (fmi3SetFloat64(i, &vr, 1, &v, 1) != fmi3OK) return 2; }
    if (fmi3EnterInitializationMode(i, fmi3False, 0.0, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    for (int k = 0; k <= STEPS; ++k) {
        if (k > 0) { fmi3Boolean ev, term, early; fmi3Float64 last; if (fmi3DoStep(i, STEP * (k - 1), STEP, fmi3True, &ev, &term, &early, &last) != fmi3OK) return 3; }
        fmi3Float64 values[NVARS];
        if (fmi3GetFloat64(i, vars, NVARS, values, NVARS) != fmi3OK) return 4;
        printf("cs %d", k);
        for (size_t v = 0; v < NVARS; ++v) printf(" %.17g", values[v]);
        printf("\n");
    }
    fmi3FreeInstance(i);
    return 0;
}
"#;

/// The harness prints `<interface> <time> n <n> ts <ts>` lines. `N`, `TS` and
/// `FREQUENCY` are value references; `MODEL` selects the scenario.
const HARNESS: &str = r#"#include "model.c"
#include <stdio.h>

static void quiet(fmi3InstanceEnvironment e, fmi3Status s, fmi3String c, fmi3String m) { (void)e; (void)s; (void)c; fprintf(stderr, "%s\n", m); }
static double get_f(fmi3Instance i, fmi3ValueReference vr) { fmi3Float64 v = NAN; if (fmi3GetFloat64(i, &vr, 1, &v, 1) != fmi3OK) exit(4); return v; }
static int get_i(fmi3Instance i, fmi3ValueReference vr) { fmi3Int32 v = -1; if (fmi3GetInt32(i, &vr, 1, &v, 1) != fmi3OK) exit(4); return v; }

static fmi3Instance cs(double frequency) {
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i) exit(2);
    if (frequency > 0.0) { fmi3ValueReference vr = FREQUENCY; if (fmi3SetFloat64(i, &vr, 1, &frequency, 1) != fmi3OK) exit(2); }
    if (fmi3EnterInitializationMode(i, fmi3False, 0.0, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) exit(2);
    return i;
}
static void run_cs(double frequency, double h, int steps) {
    fmi3Instance i = cs(frequency);
    printf("cs 0 n %d ts %.17g\n", get_i(i, N), get_f(i, TS));
    for (int k = 0; k < steps; ++k) {
        fmi3Boolean ev, term, early; fmi3Float64 last;
        if (fmi3DoStep(i, h * k, h, fmi3True, &ev, &term, &early, &last) != fmi3OK) exit(3);
        printf("cs %d n %d ts %.17g\n", k + 1, get_i(i, N), get_f(i, TS));
    }
    fmi3FreeInstance(i);
}
static double me_update(fmi3Instance i) {
    fmi3Boolean need = fmi3True, term, nominals, values, defined = fmi3False; fmi3Float64 next = 0.0;
    while (need) if (fmi3UpdateDiscreteStates(i, &need, &term, &nominals, &values, &defined, &next) != fmi3OK) exit(5);
    printf("me next %.17g n %d\n", defined ? next : INFINITY, get_i(i, N));
    return defined ? next : INFINITY;
}

int main(void) {
#if MODEL == 0
    run_cs(0.0, 0.07, 5);
    run_cs(0.0, 0.1, 3);
    fmi3Instance i = fmi3InstantiateModelExchange("h", TOKEN, NULL, fmi3False, fmi3False, NULL, quiet);
    if (!i || fmi3EnterInitializationMode(i, fmi3False, 0.0, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    double next = me_update(i);
    for (int k = 0; k < 3; ++k) {
        if (fmi3EnterContinuousTimeMode(i) != fmi3OK || fmi3SetTime(i, next) != fmi3OK || fmi3EnterEventMode(i) != fmi3OK) return 6;
        next = me_update(i);
    }
#else
    run_cs(8.0, 0.07, 5);
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3False, 0.0, 0.0, fmi3True, 1.0) != fmi3OK) return 2;
    printf("unset exit %d\n", (int)fmi3ExitInitializationMode(i));
#endif
    return 0;
}
"#;

#[test]
fn generated_c_component_runs_sample_ticks_as_time_events() {
    if !harness_tools_available() {
        return;
    }
    let exact = run_harness("Exact", 0);
    // Step 0.07: ticks 0, 0.1, 0.2, 0.3 land between communication points;
    // the tick at the start runs as its own event after initialization.
    for line in [
        "cs 0 n 1 ts 0",
        "cs 1 n 1 ts 0",
        "cs 2 n 2 ts 0.10000000000000001",
        "cs 3 n 3 ts 0.20000000000000001",
        "cs 4 n 3 ts 0.20000000000000001",
        "cs 5 n 4 ts 0.29999999999999999",
    ] {
        assert!(exact.contains(line), "missing `{line}` in:\n{exact}");
    }
    // Step 0.1: a tick on each communication point runs once, at the step's
    // end, and the next step does not run it again.
    for line in [
        "cs 1 n 2 ts 0.10000000000000001",
        "cs 2 n 3 ts 0.20000000000000001",
        "cs 3 n 4 ts 0.29999999999999999",
    ] {
        assert!(exact.contains(line), "missing `{line}` in:\n{exact}");
    }
    // Model Exchange announces each tick on the exact grid.
    for line in [
        "me next 0.10000000000000001 n 1",
        "me next 0.20000000000000001 n 2",
        "me next 0.29999999999999999 n 3",
        "me next 0.40000000000000002 n 4",
    ] {
        assert!(exact.contains(line), "missing `{line}` in:\n{exact}");
    }

    // sample(0.05, 1 / frequency) with frequency = 8 set before
    // initialization: ticks at 0.05, 0.175, 0.3.
    let parametric = run_harness("Parametric", 1);
    for line in [
        "cs 0 n 0 ts -1",
        "cs 1 n 1 ts 0.050000000000000003",
        "cs 3 n 2 ts 0.17499999999999999",
        "cs 5 n 3 ts 0.29999999999999999",
    ] {
        assert!(
            parametric.contains(line),
            "missing `{line}` in:\n{parametric}"
        );
    }
    // Without a frequency the interval is not positive: initialization fails.
    assert!(parametric.contains("unset exit 3"), "{parametric}");
}

#[test]
fn generated_c_component_orders_same_tick_sample_clauses_by_data() {
    if !harness_tools_available() {
        return;
    }
    // `b` reads `u`, which the other clause's `a` determines through element
    // equations, at the same tick: tick k (t = 0.1 k) gives a = k + 1 and
    // b = 3 (k + 1), although the reading clause comes first.
    let rows = step_rows("Order", &["a", "b"], (0.0, 0.05, 7));
    for (step, a) in [(0, 1.0), (2, 2.0), (4, 3.0), (6, 4.0)] {
        assert_eq!(rows[step], vec![a, 3.0 * a], "step {step}: {rows:?}");
    }
}

#[test]
fn generated_c_component_schedules_tunable_sample_operands_at_initialization() {
    if !harness_tools_available() {
        return;
    }
    // Defaults: sample(0.05, 0.1) on its exact lattice.
    let rows = step_rows("Tunable", &["n", "ts", "twice"], (0.0, 0.07, 5));
    assert_eq!(rows[1], vec![1.0, 0.05, 0.2], "{rows:?}");
    assert_eq!(rows[3], vec![2.0, 0.15, 0.2], "{rows:?}");
    assert_eq!(rows[4], vec![3.0, 0.25, 0.2], "{rows:?}");
    // period = 0.04 set before initialization: the derived `twice` and the
    // tick schedule 0.05 + 0.04 k both follow it.
    let rows = step_rows("Tunable", &["n", "ts", "twice"], (0.04, 0.035, 5));
    let tick = |k: f64| k.mul_add(0.04, 0.05);
    assert_eq!(rows[0], vec![0.0, -1.0, 0.08], "{rows:?}");
    assert_eq!(rows[1], vec![0.0, -1.0, 0.08], "{rows:?}");
    for (step, k) in [(2, 0.0), (3, 1.0), (4, 2.0), (5, 3.0)] {
        assert_eq!(rows[step], vec![k + 1.0, tick(k), 0.08], "{rows:?}");
    }
}

/// Run [`STEP_HARNESS`] on `model` with `(period, step, steps)` and return
/// the values of `names` after each step.
fn step_rows(model: &str, names: &[&str], (period, step, steps): (f64, f64, u32)) -> Vec<Vec<f64>> {
    let compiled = rumoca::Compiler::new()
        .model(model)
        .compile_str(SOURCE, "SampleEvents.mo")
        .unwrap_or_else(|error| panic!("compile {model}: {error:?}"));
    let work = tempdir().expect("sample-event harness work directory");
    let fmu = build_named_fmu(work.path(), &compiled, "fmi3", model);
    let description = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the packaged model description");
    let references = names
        .iter()
        .map(|name| {
            value_reference(&description, name)
                .unwrap_or_else(|| panic!("{model} publishes no {name}"))
                .to_string()
        })
        .collect::<Vec<_>>();
    let mut command = Command::new("cc");
    command
        .arg(format!("-DVARS={}", references.join(",")))
        .arg(format!(
            "-DPERIOD={}",
            value_reference(&description, "period").unwrap_or(0)
        ))
        .arg(format!("-DPERIOD_VALUE={period:?}"))
        .arg(format!("-DSTEP={step:?}"))
        .arg(format!("-DSTEPS={steps}"));
    let output = compile_and_run(
        command,
        work.path(),
        &fmu,
        &description,
        STEP_HARNESS,
        model,
    );
    output
        .lines()
        .filter_map(|line| line.strip_prefix("cs "))
        .map(|line| {
            line.split_whitespace()
                .skip(1)
                .map(|value| value.parse().expect("a printed value"))
                .collect()
        })
        .collect()
}

fn harness_tools_available() -> bool {
    let tools = [("cc", command_is_available("cc"))];
    super::super::template_runtime_policy::prerequisites_are_available(
        "FMI C sample-event harness",
        &tools,
    )
}

/// Package `model`, compile the harness scenario against it, and return its
/// standard output.
fn run_harness(model: &str, scenario: u32) -> String {
    let compiled = rumoca::Compiler::new()
        .model(model)
        .compile_str(SOURCE, "SampleEvents.mo")
        .unwrap_or_else(|error| panic!("compile {model}: {error:?}"));
    let work = tempdir().expect("sample-event harness work directory");
    let fmu = build_named_fmu(work.path(), &compiled, "fmi3", model);
    let description = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the packaged model description");
    let mut command = Command::new("cc");
    command.arg(format!("-DMODEL={scenario}"));
    for (define, name) in [("N", "n"), ("TS", "ts"), ("FREQUENCY", "frequency")] {
        let reference = value_reference(&description, name).unwrap_or(0);
        command.arg(format!("-D{define}={reference}"));
    }
    compile_and_run(command, work.path(), &fmu, &description, HARNESS, model)
}

/// Compile `harness` against the packaged sources with the defines already
/// on `command`, run it, and return its standard output.
fn compile_and_run(
    mut command: Command,
    work: &Path,
    fmu: &BuiltFmu,
    description: &str,
    harness: &str,
    model: &str,
) -> String {
    let sources = fmu.root.join("sources");
    let token = attribute(description, "instantiationToken");
    command
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-Wno-unused-function")
        .arg("-DFMI3_OVERRIDE_FUNCTION_PREFIX=1")
        .arg(format!("-DTOKEN=\"{token}\""));
    let source = work.join("sample_events_harness.c");
    fs::write(&source, harness).expect("write the sample-event harness");
    let binary = work.join("sample_events_harness");
    checked_output(
        command
            .arg(format!("-I{}", sources.display()))
            .arg(format!("-I{}", vendored_fmi3_headers().display()))
            .arg(&source)
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
        &format!("compile the {model} sample-event harness"),
    );
    let output = checked_output(
        &mut Command::new(&binary),
        &format!("run the {model} sample-event harness"),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn vendored_fmi3_headers() -> PathBuf {
    workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers")
}

fn attribute(xml: &str, name: &str) -> String {
    let start = xml
        .find(&format!("{name}=\""))
        .unwrap_or_else(|| panic!("model description has no {name}"))
        + name.len()
        + 2;
    xml[start..start + xml[start..].find('"').expect("terminated attribute")].to_string()
}

fn value_reference(xml: &str, variable: &str) -> Option<u32> {
    let at = xml.find(&format!(" name=\"{variable}\""))?;
    let tail = &xml[at..];
    let start = tail.find("valueReference=\"")? + "valueReference=\"".len();
    tail[start..start + tail[start..].find('"')?].parse().ok()
}

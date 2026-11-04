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
    for (define, name) in [("N", "n"), ("TS", "ts"), ("FREQUENCY", "frequency")] {
        let reference = value_reference(&description, name).unwrap_or(0);
        command.arg(format!("-D{define}={reference}"));
    }
    let harness = work.path().join("sample_events_harness.c");
    fs::write(&harness, HARNESS).expect("write the sample-event harness");
    let binary = work.path().join("sample_events_harness");
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

//! A Co-Simulation substep whose trial stage the component cannot evaluate is
//! rejected and retried shorter, as `rumoca-solver-rk45` retries a refused
//! trial point, instead of failing the communication step.
//!
//! `z * z = y` has no real solution once `y < 0`, and `der(y) = -k (z - 1/2)`
//! drives `y` from 1 toward 1/4 fast enough that the first Dormand-Prince
//! stage of a 0.01 s substep lands at `y = 1 - 0.002 * 2000 * 0.5 = -1`.
//! The trajectory itself never leaves `y > 1/4`. The component's own error
//! control used to see that stage only as a failed derivative evaluation and
//! returned Error from the first `fmi3DoStep` (the FIRE
//! `ESC.AverageThreePhase` harness failed at t = 0 the same way: the
//! floating phase's leakage path makes its first trial stage leave the
//! physical manifold, where the algebraic projection cannot converge).
//!
//! The source FMU's C sources are compiled with an FMI 3 Co-Simulation driver
//! against the FMI 3 headers Rumoca vendors, so this runs without the pinned
//! conformance tree.

use super::*;

const MODEL: &str = "RefusedTrial";
const SOURCE: &str = r#"
model RefusedTrial
  parameter Real k = 2000;
  Real y(start = 1, fixed = true);
  Real z(start = 1, min = 0);
equation
  z * z = y;
  der(y) = -k * (z - 0.5);
end RefusedTrial;
"#;

/// Static FMI 3 Co-Simulation driver; the instantiation token and the value
/// references of `y` and `z` arrive on the command line. Every log message
/// of Error severity is counted.
const DRIVER: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include "fmi3Functions.h"

static int errors = 0;
static void logger(fmi3InstanceEnvironment env, fmi3Status status, fmi3String category,
    fmi3String message) {
    (void)env;
    if (status >= fmi3Error) errors += 1;
    fprintf(stderr, "[%d %s] %s\n", (int)status, category, message);
}

int main(int argc, char** argv) {
    if (argc != 4) {
        return 2;
    }
    const fmi3ValueReference vr[2] = {
        (fmi3ValueReference)strtoul(argv[2], NULL, 10),
        (fmi3ValueReference)strtoul(argv[3], NULL, 10)};
    fmi3Instance instance = fmi3InstantiateCoSimulation("refused", argv[1], NULL, fmi3False,
        fmi3True, fmi3False, fmi3False, NULL, 0, NULL, logger, NULL);
    if (instance == NULL) {
        return 1;
    }
    if (fmi3EnterInitializationMode(instance, fmi3False, 0.0, 0.0, fmi3True, 1.0) != fmi3OK
        || fmi3ExitInitializationMode(instance) != fmi3OK) {
        return 1;
    }
    fmi3Float64 time = 0.0, values[2];
    for (int step = 0; step < 100; ++step) {
        fmi3Boolean event = fmi3False, terminate = fmi3False, early = fmi3False;
        fmi3Float64 reached = time;
        fmi3Status status = fmi3DoStep(instance, time, 0.01, fmi3True, &event, &terminate, &early, &reached);
        if (status != fmi3OK) {
            printf("step %d status %d\n", step, (int)status);
            return 1;
        }
        time = reached;
        if (fmi3GetFloat64(instance, vr, 2, values, 2) != fmi3OK) {
            return 1;
        }
        printf("t %.17g y %.17g z %.17g\n", time, values[0], values[1]);
    }
    printf("errors %d\n", errors);
    fmi3Terminate(instance);
    fmi3FreeInstance(instance);
    return 0;
}
"#;

fn value_reference(xml: &str, name: &str) -> String {
    let at = xml
        .find(&format!(" name=\"{name}\" valueReference=\""))
        .unwrap_or_else(|| panic!("`{name}` is published"));
    let rest = &xml[at..];
    rest.split_once("valueReference=\"")
        .and_then(|(_, tail)| tail.split_once('"'))
        .expect("value reference")
        .0
        .to_string()
}

fn compile_driver(work: &Path, fmu: &BuiltFmu) -> PathBuf {
    let headers =
        workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let sources = fmu.root.join("sources");
    let driver = work.join("refused_trial_driver.c");
    fs::write(&driver, DRIVER).expect("write the co-simulation driver");
    let binary = work.join("refused_trial_driver");
    checked_output(
        Command::new("cc")
            .arg("-std=c99")
            .arg("-ffp-contract=off")
            .arg("-DFMI3_OVERRIDE_FUNCTION_PREFIX=1")
            .arg(format!("-I{}", sources.display()))
            .arg(format!("-I{}", headers.display()))
            .arg(&driver)
            .args(
                [
                    "model.c",
                    "rmc_rows.c",
                    "rmc_isolators.c",
                    "rmc_functions.c",
                    "rmc_assign.c",
                    "rmc_jacobian.c",
                ]
                .map(|unit| sources.join(unit)),
            )
            .arg("-lm")
            .arg("-o")
            .arg(&binary),
        "compile the refused-trial co-simulation driver",
    );
    binary
}

/// `y(t)` of `der(y) = -k (sqrt(y) - 1/2)`, `y(0) = 1`: with `s = sqrt(y)`,
/// `ds/dt = -k (s - 1/2) / (2 s)`, integrated by classical RK4 at a step far
/// below its time scale.
fn reference_y(t_end: f64) -> f64 {
    let rate = |s: f64| -2000.0 * (s - 0.5) / (2.0 * s);
    let steps = 200_000;
    let h = t_end / f64::from(steps);
    let mut s = 1.0_f64;
    for _ in 0..steps {
        let k1 = rate(s);
        let k2 = rate(s + 0.5 * h * k1);
        let k3 = rate(s + 0.5 * h * k2);
        let k4 = rate(s + h * k3);
        s += h / 6.0 * (k1 + 2.0 * k2 + 2.0 * k3 + k4);
    }
    s * s
}

#[test]
fn generated_fmi3_co_simulation_rejects_a_substep_whose_trial_stage_is_refused() {
    if !super::super::template_runtime_policy::prerequisites_are_available(
        "FMI 3 refused-trial co-simulation",
        &[("C compiler `cc`", command_is_available("cc"))],
    ) {
        return;
    }
    let work = tempdir().expect("refused-trial work directory");
    let compiled = rumoca::Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "RefusedTrial.mo")
        .expect("compile the refused-trial model");
    let fmu = build_named_fmu(work.path(), &compiled, "fmi3", MODEL);
    let xml = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the model description");
    let token = xml
        .split_once("instantiationToken=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .expect("instantiation token")
        .0;
    let binary = compile_driver(work.path(), &fmu);
    let output = checked_output(
        Command::new(&binary)
            .arg(token)
            .arg(value_reference(&xml, "y"))
            .arg(value_reference(&xml, "z")),
        "step the refused-trial component",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let rows = stdout
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            match fields.as_slice() {
                ["t", t, "y", y, "z", z] => Some((
                    t.parse::<f64>().ok()?,
                    y.parse::<f64>().ok()?,
                    z.parse::<f64>().ok()?,
                )),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 100, "{stdout}");
    for &(t, y, z) in &rows {
        assert!((z * z - y).abs() < 1e-9 * y.max(1.0), "t={t}: {stdout}");
        if [0.01, 0.02, 0.05]
            .iter()
            .any(|probe| (t - probe).abs() < 1e-12)
        {
            let exact = reference_y(t);
            assert!(
                (y - exact).abs() < 1e-6,
                "t={t}: y={y}, reference {exact}\n{stdout}"
            );
        }
    }
    let (t_end, y_end, _) = rows[rows.len() - 1];
    assert!((t_end - 1.0).abs() < 1e-12, "{stdout}");
    assert!((y_end - 0.25).abs() < 1e-5, "{stdout}");
    // A discarded trial point is not a failure the importer is told about.
    assert!(stdout.lines().any(|line| line == "errors 0"), "{stdout}");
}

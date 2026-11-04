//! MLS §8.6 initialization rows over states, inputs and parameters only, and
//! initial-equation assertions, in the generated C component.
//!
//! `x = f/(m*w^2)` solves the state `x` from the input written during
//! initialization. The row reads no algebraic coordinate, so its residual
//! over the declaration seeds and over the settled view is the same row the C
//! kernel evaluates (`ModelicaCPSLibrary.Physical.Sensors.MEMS.
//! AccelerometerMechanics`). `assert(time <= tLimit)` is an initial equation:
//! it restricts the start time only, so stepping past `tLimit` is not a
//! violation. The source FMU's C sources are compiled with an FMI 3
//! Co-Simulation driver that writes `f = 3` during initialization.

use super::*;

const MODEL: &str = "SeedRow";
const SOURCE: &str = r#"
block SeedRow
  parameter Real m = 2;
  parameter Real w = 10;
  parameter Real tLimit = 0.1;
  input Real f(start = 0);
  output Real x;
  output Real v(start = 0, fixed = true);
initial equation
  assert(time <= tLimit, "starts after tLimit");
  x = f/(m*w^2);
equation
  der(x) = v;
  m*der(v) + m*w*v + m*w^2*x = f;
end SeedRow;
"#;

/// Static FMI 3 Co-Simulation driver; value references and the
/// instantiation token arrive on the command line.
const DRIVER: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include "fmi3Functions.h"

static void logger(fmi3InstanceEnvironment env, fmi3Status status, fmi3String category,
    fmi3String message) {
    (void)env;
    fprintf(stderr, "[%d %s] %s\n", (int)status, category, message);
}

#define CHECK(call) do { if ((call) > fmi3Warning) { fprintf(stderr, "%s failed\n", #call); return 1; } } while (0)

int main(int argc, char** argv) {
    if (argc != 5) {
        return 2;
    }
    fmi3ValueReference vr[3];
    for (int n = 0; n < 3; ++n) {
        vr[n] = (fmi3ValueReference)strtoul(argv[n + 2], NULL, 10);
    }
    fmi3Instance instance = fmi3InstantiateCoSimulation("seed_row", argv[1], NULL, fmi3False,
        fmi3True, fmi3False, fmi3False, NULL, 0, NULL, logger, NULL);
    if (instance == NULL) {
        return 1;
    }
    CHECK(fmi3EnterInitializationMode(instance, fmi3False, 0.0, 0.0, fmi3True, 1.0));
    const fmi3Float64 three = 3.0;
    CHECK(fmi3SetFloat64(instance, &vr[0], 1, &three, 1));
    CHECK(fmi3ExitInitializationMode(instance));
    fmi3Float64 values[2];
    CHECK(fmi3GetFloat64(instance, &vr[1], 2, values, 2));
    printf("initial x %.9f v %.9f\n", values[0], values[1]);
    fmi3Float64 time = 0.0;
    for (int step = 0; step < 50; ++step) {
        fmi3Boolean event = fmi3False, terminate = fmi3False, early = fmi3False;
        fmi3Float64 reached = time;
        CHECK(fmi3DoStep(instance, time, 0.01, fmi3True, &event, &terminate, &early, &reached));
        time = reached;
    }
    CHECK(fmi3GetFloat64(instance, &vr[1], 2, values, 2));
    printf("final t %.3f x %.9f v %.9f\n", time, values[0], values[1]);
    CHECK(fmi3Terminate(instance));
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
        .and_then(|(_, rest)| rest.split_once('"'))
        .unwrap_or_else(|| panic!("`{name}` has a value reference"))
        .0
        .to_string()
}

fn compile_driver(work: &Path, fmu: &BuiltFmu) -> PathBuf {
    let headers =
        workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let sources = fmu.root.join("sources");
    let driver = work.join("seed_row_driver.c");
    fs::write(&driver, DRIVER).expect("write the co-simulation driver");
    let binary = work.join("seed_row_driver");
    checked_output(
        Command::new("cc")
            .arg("-std=c99")
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
        "compile the seed-row co-simulation driver",
    );
    binary
}

#[test]
fn generated_fmi3_component_solves_initial_rows_over_seeds() {
    if !super::super::template_runtime_policy::prerequisites_are_available(
        "FMI 3 initialization rows over declaration seeds",
        &[("C compiler `cc`", command_is_available("cc"))],
    ) {
        return;
    }
    let work = tempdir().expect("seed-row work directory");
    let compiled = rumoca::Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "SeedRow.mo")
        .expect("compile the seed-row block");
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
            .args(["f", "x", "v"].map(|name| value_reference(&xml, name))),
        "drive the seed-row component",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // x = 3/(2*10^2) at rest, and a constant force keeps it there past tLimit.
    for line in [
        "initial x 0.015000000 v 0.000000000",
        "final t 0.500 x 0.015000000 v 0.000000000",
    ] {
        assert!(stdout.lines().any(|seen| seen == line), "{line}\n{stdout}");
    }
}

//! MLS §8.6 initial equations over `pre` in the generated C component.
//!
//! `pre(r) = 0` of a discrete input defines the value its history enters the
//! initial event with, whatever the importer wrote; `pre(u) = u` of an input
//! and the steady state `pre(v) = v` of a discrete equation make `pre` read
//! the initialization value itself. A `when initial()` body records what it
//! reads. The source FMU's C sources are compiled with an FMI 3 Co-Simulation
//! driver that writes `r = 3` and `u = true` during initialization.

use super::*;

const MODEL: &str = "InitialPre";
const SOURCE: &str = r#"
block InitialPre
  input Integer r(start = 0);
  input Boolean u;
  output Integer j(start = -1, fixed = true);
  output Integer i(start = 0, fixed = true);
  output Integer k(start = 0, fixed = true);
  output Boolean v;
initial equation
  pre(r) = 0;
  pre(u) = u;
  pre(v) = v;
equation
  v = time >= 0;
algorithm
  when initial() then
    j := pre(r);
    i := if pre(u) then 1 else 2;
    k := if pre(v) then 1 else 2;
  end when;
end InitialPre;
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
    if (argc != 7) {
        return 2;
    }
    fmi3ValueReference vr[5];
    for (int n = 0; n < 5; ++n) {
        vr[n] = (fmi3ValueReference)strtoul(argv[n + 2], NULL, 10);
    }
    fmi3Instance instance = fmi3InstantiateCoSimulation("initial_pre", argv[1], NULL, fmi3False,
        fmi3True, fmi3False, fmi3False, NULL, 0, NULL, logger, NULL);
    if (instance == NULL) {
        return 1;
    }
    CHECK(fmi3EnterInitializationMode(instance, fmi3False, 0.0, 0.0, fmi3True, 1.0));
    const fmi3Int32 three = 3;
    const fmi3Boolean yes = fmi3True;
    CHECK(fmi3SetInt32(instance, &vr[0], 1, &three, 1));
    CHECK(fmi3SetBoolean(instance, &vr[1], 1, &yes, 1));
    CHECK(fmi3ExitInitializationMode(instance));
    fmi3Int32 values[3];
    CHECK(fmi3GetInt32(instance, &vr[2], 3, values, 3));
    printf("initial j %d i %d k %d\n", (int)values[0], (int)values[1], (int)values[2]);
    fmi3Boolean event = fmi3False, terminate = fmi3False, early = fmi3False;
    fmi3Float64 reached = 0.0;
    CHECK(fmi3DoStep(instance, 0.0, 0.5, fmi3True, &event, &terminate, &early, &reached));
    CHECK(fmi3GetInt32(instance, &vr[2], 3, values, 3));
    printf("stepped j %d i %d k %d\n", (int)values[0], (int)values[1], (int)values[2]);
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
    let driver = work.join("initial_pre_driver.c");
    fs::write(&driver, DRIVER).expect("write the co-simulation driver");
    let binary = work.join("initial_pre_driver");
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
        "compile the initial-pre co-simulation driver",
    );
    binary
}

#[test]
fn generated_fmi3_component_holds_initial_pre_equations() {
    if !super::super::template_runtime_policy::prerequisites_are_available(
        "FMI 3 initial pre equations",
        &[("C compiler `cc`", command_is_available("cc"))],
    ) {
        return;
    }
    let work = tempdir().expect("initial-pre work directory");
    let compiled = rumoca::Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "InitialPre.mo")
        .expect("compile the initial-pre block");
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
            .args(["r", "u", "j", "i", "k"].map(|name| value_reference(&xml, name))),
        "drive the initial-pre component",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `pre(r) = 0` although `r = 3` was written; `pre(u) = u = true`; and
    // `pre(v) = v = (0 >= 0)`, which the initial event settles.
    for line in ["initial j 0 i 1 k 1", "stepped j 0 i 1 k 1"] {
        assert!(stdout.lines().any(|seen| seen == line), "{line}\n{stdout}");
    }
}

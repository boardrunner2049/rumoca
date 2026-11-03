//! An exported input declared without `start` holds its type's default start
//! (MLS §4.9) in the generated C component until the importer writes it, and
//! stays settable.
//!
//! The source FMU's C sources are compiled with an FMI 3 Co-Simulation driver
//! against the FMI 3 headers Rumoca vendors, so this runs without the pinned
//! conformance tree. The driver reads every input before writing any, then
//! writes `u = 1` during initialization and steps `der(x) = 1 + u` to t = 1.
//! An enumeration input refuses an ordinal that names no literal and accepts
//! one that does.

use super::*;

const MODEL: &str = "UnstartedInputs";
const SOURCE: &str = include_str!("../../fixtures/fmi/UnstartedInputs.mo");

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
    if (argc != 10) {
        return 2;
    }
    fmi3ValueReference vr[8];
    for (int i = 0; i < 8; ++i) {
        vr[i] = (fmi3ValueReference)strtoul(argv[i + 2], NULL, 10);
    }
    const fmi3ValueReference u = vr[0], w = vr[1], k = vr[2], b = vr[3], m = vr[4], s = vr[5],
        x = vr[6], y = vr[7];
    fmi3Instance instance = fmi3InstantiateCoSimulation("unstarted", argv[1], NULL, fmi3False,
        fmi3True, fmi3False, fmi3False, NULL, 0, NULL, logger, NULL);
    if (instance == NULL) {
        return 1;
    }
    CHECK(fmi3EnterInitializationMode(instance, fmi3False, 0.0, 0.0, fmi3True, 1.0));
    fmi3Float64 real[3];
    fmi3Int32 integer;
    fmi3Boolean boolean;
    CHECK(fmi3GetFloat64(instance, &u, 1, &real[0], 1));
    printf("start u %g\n", real[0]);
    CHECK(fmi3GetFloat64(instance, &w, 1, real, 2));
    printf("start w %g %g\n", real[0], real[1]);
    CHECK(fmi3GetInt32(instance, &k, 1, &integer, 1));
    printf("start k %d\n", (int)integer);
    CHECK(fmi3GetBoolean(instance, &b, 1, &boolean, 1));
    printf("start b %d\n", (int)boolean);
    CHECK(fmi3GetInt32(instance, &m, 1, &integer, 1));
    printf("start m %d\n", (int)integer);
    CHECK(fmi3GetFloat64(instance, &s, 1, &real[0], 1));
    printf("start s %g\n", real[0]);
    /* An enumeration ordinal below the first literal names no literal. */
    const fmi3Int32 no_literal = 0, second_literal = 2;
    printf("set m 0 %s\n", fmi3SetInt32(instance, &m, 1, &no_literal, 1) == fmi3Error ? "refused" : "accepted");
    CHECK(fmi3GetInt32(instance, &m, 1, &integer, 1));
    printf("kept m %d\n", (int)integer);
    CHECK(fmi3SetInt32(instance, &m, 1, &second_literal, 1));
    CHECK(fmi3GetInt32(instance, &m, 1, &integer, 1));
    printf("set m %d\n", (int)integer);
    const fmi3Float64 one = 1.0;
    CHECK(fmi3SetFloat64(instance, &u, 1, &one, 1));
    CHECK(fmi3ExitInitializationMode(instance));
    CHECK(fmi3GetFloat64(instance, &y, 1, &real[0], 1));
    printf("initial y %.12g\n", real[0]);
    fmi3Float64 time = 0.0;
    for (int step = 0; step < 100; ++step) {
        fmi3Boolean event = fmi3False, terminate = fmi3False, early = fmi3False;
        fmi3Float64 reached = time;
        CHECK(fmi3DoStep(instance, time, 0.01, fmi3True, &event, &terminate, &early, &reached));
        time = reached;
    }
    CHECK(fmi3GetFloat64(instance, &x, 1, &real[0], 1));
    CHECK(fmi3GetFloat64(instance, &y, 1, &real[1], 1));
    printf("final t %.12g x %.12g y %.12g\n", time, real[0], real[1]);
    CHECK(fmi3Terminate(instance));
    fmi3FreeInstance(instance);
    return 0;
}
"#;

/// The value of `attribute` on the published variable `name`.
fn variable_attribute(xml: &str, name: &str, attribute: &str) -> String {
    let at = xml
        .find(&format!(" name=\"{name}\" valueReference="))
        .unwrap_or_else(|| panic!("`{name}` is published"));
    let element = &xml[at..at + xml[at..].find('>').expect("element ends")];
    element
        .split_once(&format!(" {attribute}=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .unwrap_or_else(|| panic!("`{name}` has no {attribute}: {element}"))
        .0
        .to_string()
}

fn compile_driver(work: &Path, fmu: &BuiltFmu) -> PathBuf {
    let headers =
        workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let sources = fmu.root.join("sources");
    let driver = work.join("input_start_driver.c");
    fs::write(&driver, DRIVER).expect("write the co-simulation driver");
    let binary = work.join("input_start_driver");
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
        "compile the unstarted-input co-simulation driver",
    );
    binary
}

#[test]
fn generated_fmi3_component_holds_default_input_starts_until_written() {
    if !super::super::template_runtime_policy::prerequisites_are_available(
        "FMI 3 unstarted-input co-simulation",
        &[("C compiler `cc`", command_is_available("cc"))],
    ) {
        return;
    }
    let work = tempdir().expect("input-start work directory");
    let compiled = rumoca::Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "UnstartedInputs.mo")
        .expect("compile the unstarted-input block");
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
        Command::new(&binary).arg(token).args(
            ["u", "w", "k", "b", "m", "s", "x", "y"]
                .map(|name| variable_attribute(&xml, name, "valueReference")),
        ),
        "drive the unstarted-input component",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in [
        "start u 0",
        "start w 0 0",
        "start k 0",
        "start b 0",
        "start m 1",
        "start s 3",
        "set m 0 refused",
        "kept m 1",
        "set m 2",
        "initial y 3",
    ] {
        assert!(stdout.lines().any(|seen| seen == line), "{line}\n{stdout}");
    }
    // `der(x) = 1 + u` with the written `u = 1`: x(1) = 2, y(1) = x + s = 5.
    let last = stdout
        .lines()
        .find_map(|line| line.strip_prefix("final "))
        .unwrap_or_else(|| panic!("final line\n{stdout}"));
    let field = |key: &str| -> f64 {
        last.split_whitespace()
            .skip_while(|part| *part != key)
            .nth(1)
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("{key} in {last}"))
    };
    assert!((field("t") - 1.0).abs() < 1e-12, "{last}");
    assert!((field("x") - 2.0).abs() < 1e-9, "{last}");
    assert!((field("y") - 5.0).abs() < 1e-9, "{last}");
}

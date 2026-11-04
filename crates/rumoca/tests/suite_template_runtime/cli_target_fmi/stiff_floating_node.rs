//! A node held only by leakage conductances is a stiff mode whose algebraic
//! voltage has a large gain on the state: `G (V - v) = 2 G v + i` with
//! `L der(i) = v - R i - E` has the eigenvalue `-(R + 1 / (3 G)) / L`
//! (-6.7e7 1/s at G = 1e-5 S) and `dv/di = -1 / (3 G)` (-33 kOhm). An explicit
//! Co-Simulation substep is held at its stability boundary there, where the
//! stiff component keeps a local error of the order of the tolerance that
//! never decays, and the gain turns it into millivolts on `v` (the FIRE
//! AverageHalfBridge floating phase sat 7 mV low with `L di/dt` = -7 mV and
//! `i` flat). The step rule detects the stiff mode and substeps it with the
//! L-stable Rosenbrock 2(3) pair, so the floating voltage is the exact DC
//! value; closing the node to the rail (`Gh = 100`) makes the mode non-stiff
//! and the rule returns to Dormand-Prince 5(4), and opening it again finds the
//! stiff mode again.
//!
//! The source FMU's C sources are compiled with an FMI 3 Co-Simulation driver
//! against the FMI 3 headers Rumoca vendors, so this runs without the pinned
//! conformance tree. The importer defines no tolerance (the plan default
//! applies), as the FIRE harness runtime does.

use super::*;

const MODEL: &str = "FloatingNode";
const SOURCE: &str = r#"
model FloatingNode
  parameter Real G = 1e-5 "leakage conductance of each off device";
  parameter Real V = 16 "rail voltage";
  parameter Real R = 1;
  parameter Real L = 0.5e-3;
  parameter Real E = 2 "back-EMF";
  input Real Gh(start = 1e-5) "conductance from the node to the rail";
  Real i(start = 0, fixed = true);
  output Real v "node voltage";
equation
  Gh * (V - v) = 2 * G * v + i;
  L * der(i) = v - R * i - E;
end FloatingNode;
"#;

/// Steps of 10 us: the node floats to 2 ms, is closed to the rail to 12 ms
/// (20 load time constants), and floats again to 14 ms. Each line is
/// `t v i`.
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
    const fmi3ValueReference gh = (fmi3ValueReference)strtoul(argv[2], NULL, 10);
    const fmi3ValueReference v = (fmi3ValueReference)strtoul(argv[3], NULL, 10);
    const fmi3ValueReference i = (fmi3ValueReference)strtoul(argv[4], NULL, 10);
    fmi3Instance instance = fmi3InstantiateCoSimulation("floating", argv[1], NULL, fmi3False,
        fmi3True, fmi3False, fmi3False, NULL, 0, NULL, logger, NULL);
    if (instance == NULL) {
        return 1;
    }
    CHECK(fmi3EnterInitializationMode(instance, fmi3False, 0.0, 0.0, fmi3False, 0.0));
    CHECK(fmi3ExitInitializationMode(instance));
    for (int step = 0; step < 1400; ++step) {
        const fmi3Float64 time = step * 1e-5;
        const fmi3Float64 conductance = step >= 200 && step < 1200 ? 100.0 : 1e-5;
        CHECK(fmi3SetFloat64(instance, &gh, 1, &conductance, 1));
        fmi3Boolean event = fmi3False, terminate = fmi3False, early = fmi3False;
        fmi3Float64 reached = time;
        CHECK(fmi3DoStep(instance, time, 1e-5, fmi3True, &event, &terminate, &early, &reached));
        fmi3Float64 values[2];
        CHECK(fmi3GetFloat64(instance, &v, 1, &values[0], 1));
        CHECK(fmi3GetFloat64(instance, &i, 1, &values[1], 1));
        printf("%.17g %.17g %.17g\n", (step + 1) * 1e-5, values[0], values[1]);
    }
    CHECK(fmi3Terminate(instance));
    fmi3FreeInstance(instance);
    return 0;
}
"#;

fn value_reference(xml: &str, name: &str) -> String {
    let at = xml
        .find(&format!(" name=\"{name}\" valueReference=\""))
        .unwrap_or_else(|| panic!("`{name}` is published"));
    let rest = &xml[at + format!(" name=\"{name}\" valueReference=\"").len()..];
    rest[..rest.find('"').expect("reference ends")].to_string()
}

fn compile_driver(work: &Path, fmu: &BuiltFmu) -> PathBuf {
    let headers =
        workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let sources = fmu.root.join("sources");
    let driver = work.join("floating_node_driver.c");
    fs::write(&driver, DRIVER).expect("write the co-simulation driver");
    let binary = work.join("floating_node_driver");
    let units = fs::read_dir(&sources)
        .expect("list the FMU sources")
        .map(|entry| entry.expect("source entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"));
    checked_output(
        Command::new("cc")
            .arg("-std=c99")
            .arg("-DFMI3_OVERRIDE_FUNCTION_PREFIX=1")
            .arg(format!("-I{}", sources.display()))
            .arg(format!("-I{}", headers.display()))
            .arg(&driver)
            .args(units)
            .arg("-lm")
            .arg("-o")
            .arg(&binary),
        "compile the floating-node co-simulation driver",
    );
    binary
}

/// DC operating point with the rail conductance `gh`: `v = E + R i` and
/// `i = gh V - (gh + 2 G) v`.
fn steady_voltage(gh: f64) -> f64 {
    let (g, rail, r, emf) = (1e-5, 16.0, 1.0, 2.0);
    (emf + r * gh * rail) / (1.0 + r * (gh + 2.0 * g))
}

#[test]
fn generated_fmi3_co_simulation_holds_a_stiff_floating_node_at_its_dc_value() {
    if !super::super::template_runtime_policy::prerequisites_are_available(
        "FMI 3 stiff floating-node co-simulation",
        &[("C compiler `cc`", command_is_available("cc"))],
    ) {
        return;
    }
    let work = tempdir().expect("floating-node work directory");
    let compiled = rumoca::Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "FloatingNode.mo")
        .expect("compile the floating node");
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
            .args(["Gh", "v", "i"].map(|name| value_reference(&xml, name))),
        "drive the floating-node component",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let rows: Vec<[f64; 3]> = stdout
        .lines()
        .map(|line| {
            let fields: Vec<f64> = line
                .split_whitespace()
                .map(|field| field.parse().expect("numeric trace field"))
                .collect();
            [fields[0], fields[1], fields[2]]
        })
        .collect();
    assert_eq!(rows.len(), 1400, "{stdout}");
    let floating = steady_voltage(1e-5);
    let closed = steady_voltage(100.0);
    for [t, v, i] in rows {
        // The leakage time constant is 15 ns, so every floating sample after
        // the first step is at the DC point; the closed node settles with
        // L / (R + 1 / (Gh + 2 G)) = 0.495 ms and is checked after 20 of them.
        let expected = if !(2e-3 + 1e-9..=12e-3 + 1.5e-5).contains(&t) {
            floating
        } else if t > 12e-3 - 1e-9 {
            continue;
        } else if t > 11.9e-3 {
            closed
        } else {
            continue;
        };
        assert!(
            (v - expected).abs() <= 1e-6 * expected.abs(),
            "t = {t}: v = {v}, expected {expected} (i = {i})"
        );
    }
}

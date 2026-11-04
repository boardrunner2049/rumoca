//! MLS §12.9 external functions in the FMI C profile: an MSL random
//! generator (`ModelicaRandom_xorshift128plus`) runs the shipped MSL C source,
//! linked from `sources/` and listed in the build description, and draws the
//! same states and numbers as the native runtime; an `external "builtin"`
//! math function is the C math library builtin; an external with no shipped
//! source is refused by name.
//!
//! A C harness drives the packaged FMI 3 component directly; it needs only a
//! C compiler.

use super::*;

const FIXTURE: &str = include_str!("../../fixtures/external_functions/Random.mo");

/// The MSL C source's `Xorshift128plus` states and draws from
/// `initialState(1, 2)`, as in `suite_core::external_functions`.
const STATES: [[i64; 4]; 3] = [
    [2_049_721_705, -416_146_649, 2_001_124_099, 1_729_396_250],
    [2_001_124_099, 1_729_396_250, 1_908_300_452, 2_058_340_188],
    [1_908_300_452, 2_058_340_188, 870_918_268, -1_959_790_993],
];
const DRAWS: [f64; 2] = [0.979_244_670_934_116_3, 0.043_700_601_719_966_26];

/// Prints `t s1 s2 s3 s4 r` after each 0.05 s Co-Simulation step to 0.25,
/// or `t angle crossings` to 1.0 when `ATAN` is defined.
const HARNESS: &str = r#"#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include "model.h"

static void quiet(fmi3InstanceEnvironment e, fmi3Status s, fmi3String c, fmi3String m) { (void)e; (void)s; (void)c; fprintf(stderr, "%s\n", m); }

int main(void) {
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
#ifdef ATAN
    const int steps = 20;
#else
    const int steps = 5;
#endif
    for (int k = 0; k <= steps; ++k) {
        if (k > 0) {
            fmi3Boolean ev, term, early; fmi3Float64 last;
            if (fmi3DoStep(i, 0.05 * (k - 1), 0.05, fmi3True, &ev, &term, &early, &last) != fmi3OK) return 3;
        }
#ifdef ATAN
        fmi3ValueReference a = ANGLE, c = CROSSINGS; fmi3Float64 angle; fmi3Int32 crossings;
        if (fmi3GetFloat64(i, &a, 1, &angle, 1) != fmi3OK || fmi3GetInt32(i, &c, 1, &crossings, 1) != fmi3OK) return 4;
        printf("%d %.17g %d\n", k, angle, (int)crossings);
#else
        fmi3ValueReference s = STATE, r = DRAW; fmi3Int32 state[4]; fmi3Float64 draw;
        if (fmi3GetInt32(i, &s, 1, state, 4) != fmi3OK || fmi3GetFloat64(i, &r, 1, &draw, 1) != fmi3OK) return 4;
        printf("%d %d %d %d %d %.17g\n", k, (int)state[0], (int)state[1], (int)state[2], (int)state[3], draw);
#endif
    }
    fmi3FreeInstance(i);
    return 0;
}
"#;

#[test]
fn msl_random_generator_runs_the_shipped_c_source() {
    let work = tempdir().expect("external-function harness work directory");
    let fmu = package(work.path(), "RngTicks", "fmi3");
    let sources = fmu.root.join("sources");
    let build = fs::read_to_string(sources.join("buildDescription.xml"))
        .expect("read the build description");
    for listed in ["ModelicaRandom.c", "rmc_modelica_external.c"] {
        assert!(
            build.contains(&format!("<SourceFile name=\"{listed}\"/>")),
            "{build}"
        );
    }
    let vendored = workspace_root()
        .join("crates/rumoca-phase-codegen/src/templates/fmi3/msl_ModelicaRandom.c.jinja");
    assert_eq!(
        fs::read(sources.join("ModelicaRandom.c")).expect("shipped MSL source"),
        fs::read(vendored).expect("vendored MSL source"),
        "the MSL source ships unmodified"
    );
    let licence = fs::read_to_string(
        fmu.root
            .join("documentation/licenses/ModelicaExternalC.txt"),
    )
    .expect("the MSL licences ship with their sources");
    assert!(licence.contains("Modelica Association"), "{licence}");
    if !harness_tools_available() {
        return;
    }
    let output = run_harness(&fmu, &[("STATE", "s"), ("DRAW", "r")], false);
    // Communication points 1, 3 and 5 (t = 0.05, 0.15, 0.25) lie after the
    // initial state and the draws at t = 0.1 and t = 0.2.
    for (point, state, draw) in [
        (1, STATES[0], 0.0),
        (3, STATES[1], DRAWS[0]),
        (5, STATES[2], DRAWS[1]),
    ] {
        let line = output
            .lines()
            .find(|line| line.starts_with(&format!("{point} ")))
            .unwrap_or_else(|| panic!("no communication point {point} in:\n{output}"));
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let states = fields[1..5]
            .iter()
            .map(|field| field.parse::<i64>().expect("state element"))
            .collect::<Vec<_>>();
        assert_eq!(states, state, "{line}");
        let drawn: f64 = fields[5].parse().expect("draw");
        assert_eq!(drawn.to_bits(), f64::to_bits(draw), "{line}");
    }
}

#[test]
fn builtin_external_math_function_ships_no_foreign_source() {
    let work = tempdir().expect("builtin harness work directory");
    let fmu = package(work.path(), "AtanEvents", "fmi3");
    let sources = fmu.root.join("sources");
    assert!(!sources.join("ModelicaRandom.c").exists());
    assert!(!sources.join("rmc_modelica_external.c").exists());
    assert!(!fmu.root.join("documentation").exists());
    if !harness_tools_available() {
        return;
    }
    let output = run_harness(
        &fmu,
        &[("ANGLE", "angle"), ("CROSSINGS", "crossings")],
        true,
    );
    let last = output.lines().last().expect("a communication point");
    let fields = last.split_whitespace().collect::<Vec<_>>();
    let angle: f64 = fields[1].parse().expect("angle");
    assert!(
        (angle - std::f64::consts::FRAC_PI_4).abs() < 1e-9,
        "{output}"
    );
    assert_eq!(fields[2], "1", "one crossing of angle > 0.5: {output}");
}

#[test]
fn fmi2_lists_and_ships_the_msl_random_source() {
    let work = tempdir().expect("fmi2 package work directory");
    let fmu = package(work.path(), "RngTicks", "fmi2");
    let description = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the FMI 2 model description");
    assert_eq!(
        description
            .matches("<File name=\"ModelicaRandom.c\"/>")
            .count(),
        2,
        "both interfaces list the source: {description}"
    );
    for shipped in [
        "ModelicaRandom.c",
        "ModelicaRandom.h",
        "rmc_modelica_external.c",
    ] {
        assert!(
            fmu.root.join("sources").join(shipped).is_file(),
            "{shipped}"
        );
    }
}

#[test]
fn external_without_a_shipped_source_is_refused_by_name() {
    let compiled = rumoca::Compiler::new()
        .model("UserExternal")
        .compile_str(FIXTURE, "Random.mo")
        .expect("UserExternal compiles");
    let work = tempdir().expect("refusal work directory");
    for target in ["fmi3", "fmi2"] {
        let error = rumoca::compile_packaged_target(
            &compiled,
            "UserExternal",
            target,
            work.path().join(target),
        )
        .expect_err("an external C function without a shipped source cannot link");
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("unsupported-feature:external_functions")
                && rendered.contains("`myGain`")
                && rendered.contains("`my_gain`"),
            "{target}: {rendered}"
        );
    }
}

fn package(work: &Path, model: &str, target: &'static str) -> BuiltFmu {
    let compiled = rumoca::Compiler::new()
        .model(model)
        .compile_str(FIXTURE, "Random.mo")
        .unwrap_or_else(|error| panic!("compile {model}: {error:?}"));
    build_named_fmu(work, &compiled, target, model)
}

fn harness_tools_available() -> bool {
    let tools = [("cc", command_is_available("cc"))];
    super::super::template_runtime_policy::prerequisites_are_available(
        "FMI C external-function harness",
        &tools,
    )
}

/// Compile every source the build description lists, the generated units
/// warning-free and the shipped MSL sources as distributed, then run the
/// harness against them.
fn run_harness(fmu: &BuiltFmu, references: &[(&str, &str)], atan: bool) -> String {
    let sources = fmu.root.join("sources");
    let description = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the packaged model description");
    let build = fs::read_to_string(sources.join("buildDescription.xml"))
        .expect("read the build description");
    let work = fmu.root.parent().expect("package parent");
    let headers =
        workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let mut objects = Vec::new();
    let mut units = build
        .split("<SourceFile name=\"")
        .skip(1)
        .map(|tail| tail[..tail.find('"').expect("terminated name")].to_string())
        .collect::<Vec<_>>();
    units.push("harness.c".to_string());
    fs::write(sources.join("harness.c"), HARNESS).expect("write the harness");
    let token = attribute(&description, "instantiationToken");
    for unit in &units {
        let object = work.join(format!("{unit}.o"));
        let mut command = Command::new("cc");
        command.args(["-std=c99", "-c", "-DFMI3_OVERRIDE_FUNCTION_PREFIX=1"]);
        if !unit.starts_with("Modelica") {
            command.args(["-Wall", "-Wextra", "-Werror", "-Wno-unused-function"]);
        }
        command.arg(format!("-DTOKEN=\"{token}\""));
        if atan {
            command.arg("-DATAN");
        }
        for (define, name) in references {
            let reference = value_reference(&description, name)
                .unwrap_or_else(|| panic!("{name} has a value reference"));
            command.arg(format!("-D{define}={reference}"));
        }
        checked_output(
            command
                .arg(format!("-I{}", sources.display()))
                .arg(format!("-I{}", headers.display()))
                .arg(sources.join(unit))
                .arg("-o")
                .arg(&object),
            &format!("compile {unit}"),
        );
        objects.push(object);
    }
    let binary = work.join("external_functions_harness");
    checked_output(
        Command::new("cc")
            .args(&objects)
            .arg("-lm")
            .arg("-o")
            .arg(&binary),
        "link the external-function harness",
    );
    let output = checked_output(
        &mut Command::new(&binary),
        "run the external-function harness",
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn attribute(xml: &str, name: &str) -> String {
    let start = xml
        .find(&format!("{name}=\""))
        .unwrap_or_else(|| panic!("model description has no {name}"))
        + name.len()
        + 2;
    xml[start..start + xml[start..].find('"').expect("terminated attribute")].to_string()
}

/// The value reference of model variable `variable` (a unit may share its
/// name, so the reference must follow the name directly).
fn value_reference(xml: &str, variable: &str) -> Option<u32> {
    let marker = format!(" name=\"{variable}\" valueReference=\"");
    let start = xml.find(&marker)? + marker.len();
    xml[start..start + xml[start..].find('"')?].parse().ok()
}

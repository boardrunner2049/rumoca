//! The FMI C profile executes a retained state-manifold projection
//! (SPEC_0044 ME-PROJ-007).
//!
//! A floating star of three inductor branches keeps `i[1] + i[2] + i[3] = 0`
//! (here with a cubic term, so the constraint is nonlinear) as a lower-order
//! constraint over three states: index reduction differentiates it to find
//! the star potential and retains the original row as a state manifold. The
//! linked kernel projects every accepted point back onto it; the generated C
//! component must do the same instead of refusing the model. The
//! Co-Simulation component is driven from C and compared with the native
//! simulation, and a Model-Exchange component handed a state off the manifold
//! must request Event Mode at the completed step and settle the state on the
//! minimum-norm correction there.

use std::path::{Path, PathBuf};
use std::process::Command;

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae_with_diagnostics};

const STAR: &str = r#"
model Star
  parameter Real R[3] = {0.2, 0.3, 0.5};
  parameter Real L[3] = {1e-3, 2e-3, 1.5e-3};
  parameter Real w = 50;
  Real i[3](start = {1, -0.4, -0.6}, fixed = {true, true, false});
  Real vn "star point potential";
  Real v[3];
equation
  for j in 1:3 loop
    v[j] = 10*sin(w*time - 2.0943951023931953*(j - 1));
    L[j]*der(i[j]) = v[j] - R[j]*i[j] - vn;
  end for;
  CONSTRAINT;
end Star;
"#;

const LINEAR: &str = "i[1] + i[2] + i[3] = 0";
const CUBIC: &str = "i[1] + i[2] + i[3] + 0.1*i[3]^3 = 0";

fn compiled(constraint: &str) -> rumoca::CompilationResult {
    Compiler::new()
        .model("Star")
        .compile_str(&STAR.replace("CONSTRAINT", constraint), "Star.mo")
        .unwrap_or_else(|error| panic!("Star compiles: {error:?}"))
}

/// The rendered files of one target as (path, content) pairs.
fn rendered(compiled: &rumoca::CompilationResult, target: &str) -> Vec<(String, String)> {
    rumoca::render_target_files(compiled, "Star", target, None)
        .unwrap_or_else(|error| panic!("Star exports to {target}: {error:#}"))
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files
        .iter()
        .find(|(candidate, _)| candidate == path)
        .unwrap_or_else(|| panic!("{path} is rendered"))
        .1
}

#[test]
fn a_retained_state_manifold_exports_to_both_fmi_targets() {
    let compiled = compiled(CUBIC);
    let prepared =
        rumoca_phase_structural::prepare_for_solve(&compiled.dae).expect("Star prepares for Solve");
    prepared.inspect(|system| {
        assert_eq!(system.manifold.len(), 1, "the star constraint is retained");
    });
    for target in ["fmi2", "fmi3"] {
        let files = rendered(&compiled, target);
        let model = file(&files, "sources/model.c");
        assert!(model.contains("#define RMC_MF_BLOCKS 1"), "{target}");
        assert!(
            model.contains("rmc_manifold_accept(m, &projected)"),
            "{target}"
        );
    }
    let description = file(&rendered(&compiled, "fmi3"), "modelDescription.xml").to_string();
    assert!(
        description.contains("needsCompletedIntegratorStep=\"true\""),
        "the ME component corrects completed steps: {description}"
    );
}

/// A Co-Simulation run (`cs TOKEN VR`, 200 steps of 1 ms at tolerance 1e-8)
/// printing `t i1 i2 i3` per step, or a Model-Exchange probe (`me TOKEN VR`)
/// of the completed-step request and the Event-Mode correction.
const DRIVER: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "fmi3Functions.h"

static void logger(fmi3InstanceEnvironment env, fmi3Status status, fmi3String category, fmi3String message) {
    (void)env; (void)status; printf("log %s %s\n", category, message);
}

static int settle(fmi3Instance m) {
    fmi3Boolean need = fmi3True, terminate, nominals, values = fmi3False, any = fmi3False, defined;
    fmi3Float64 next;
    while (need) {
        if (fmi3UpdateDiscreteStates(m, &need, &terminate, &nominals, &values, &defined, &next) != fmi3OK) return -1;
        any = any || values;
    }
    return any ? 1 : 0;
}

int main(int argc, char** argv) {
    if (argc != 4) return 90;
    const fmi3ValueReference vr = (fmi3ValueReference)atoi(argv[3]);
    double i[3];
    if (strcmp(argv[1], "cs") == 0) {
        fmi3Instance m = fmi3InstantiateCoSimulation("cs", argv[2], NULL, fmi3False, fmi3True, fmi3False, fmi3False, NULL, 0, NULL, logger, NULL);
        if (!m) return 91;
        if (fmi3EnterInitializationMode(m, fmi3True, 1e-8, 0.0, fmi3True, 0.2) != fmi3OK || fmi3ExitInitializationMode(m) != fmi3OK) return 92;
        double t = 0.0;
        for (int k = 0; k <= 200; ++k) {
            if (fmi3GetFloat64(m, &vr, 1, i, 3) != fmi3OK) return 93;
            printf("row %.17g %.17g %.17g %.17g\n", t, i[0], i[1], i[2]);
            if (k == 200) break;
            fmi3Boolean event, terminate, early; fmi3Float64 last;
            if (fmi3DoStep(m, t, 1e-3, fmi3True, &event, &terminate, &early, &last) != fmi3OK) return 94;
            t = (k + 1) * 1e-3;
        }
        return 0;
    }
    fmi3Instance m = fmi3InstantiateModelExchange("me", argv[2], NULL, fmi3False, fmi3True, NULL, logger);
    if (!m) return 95;
    if (fmi3EnterInitializationMode(m, fmi3True, 1e-8, 0.0, fmi3True, 0.2) != fmi3OK || fmi3ExitInitializationMode(m) != fmi3OK) return 96;
    if (settle(m) < 0 || fmi3EnterContinuousTimeMode(m) != fmi3OK) return 97;
    double x[3];
    if (fmi3GetContinuousStates(m, x, 3) != fmi3OK) return 98;
    fmi3Boolean enter = fmi3False, terminate = fmi3False;
    if (fmi3CompletedIntegratorStep(m, fmi3True, &enter, &terminate) != fmi3OK) return 99;
    printf("initial %.17g %.17g %.17g request %d\n", x[0], x[1], x[2], (int)enter);
    const double off[3] = {1.0, -0.4, -0.5};
    if (fmi3SetTime(m, 1e-3) != fmi3OK || fmi3SetContinuousStates(m, off, 3) != fmi3OK) return 100;
    if (fmi3CompletedIntegratorStep(m, fmi3True, &enter, &terminate) != fmi3OK) return 101;
    if (fmi3GetContinuousStates(m, x, 3) != fmi3OK) return 102;
    printf("completed %.17g %.17g %.17g request %d\n", x[0], x[1], x[2], (int)enter);
    if (fmi3EnterEventMode(m) != fmi3OK) return 103;
    int changed = settle(m);
    if (changed < 0 || fmi3EnterContinuousTimeMode(m) != fmi3OK || fmi3GetContinuousStates(m, x, 3) != fmi3OK) return 104;
    if (fmi3GetFloat64(m, &vr, 1, i, 3) != fmi3OK) return 105;
    printf("event %.17g %.17g %.17g changed %d\n", x[0], x[1], x[2], changed);
    return 0;
}
"#;

fn attribute<'a>(description: &'a str, anchor: &str, name: &str) -> &'a str {
    let tail = &description[description
        .find(anchor)
        .unwrap_or_else(|| panic!("modelDescription.xml names {anchor}"))..];
    let key = format!("{name}=\"");
    let value = &tail[tail.find(&key).expect("attribute is present") + key.len()..];
    &value[..value.find('"').expect("attribute is closed")]
}

/// The generated fmi3 component compiled with the C driver, and the
/// arguments naming its token and the `i` value reference.
fn build_driver(compiled: &rumoca::CompilationResult, work: &Path) -> (PathBuf, [String; 2]) {
    let files = rendered(compiled, "fmi3");
    let mut sources = Vec::new();
    for (relative, content) in &files {
        let path = work.join(relative);
        std::fs::create_dir_all(path.parent().expect("rendered files have a parent"))
            .expect("create rendered directory");
        std::fs::write(&path, content).expect("write rendered file");
        if relative.starts_with("sources/") && relative.ends_with(".c") {
            sources.push(path);
        }
    }
    let description = file(&files, "modelDescription.xml");
    let token = attribute(description, "<fmiModelDescription", "instantiationToken");
    let vr = attribute(description, "name=\"i\"", "valueReference");
    let driver = work.join("driver.c");
    std::fs::write(&driver, DRIVER).expect("write the C driver");
    let headers = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers");
    let executable = work.join("star");
    let build = Command::new("cc")
        .args([
            "-std=c11",
            "-O1",
            "-ffp-contract=off",
            "-DFMI3_OVERRIDE_FUNCTION_PREFIX=1",
        ])
        .arg("-I")
        .arg(work.join("sources"))
        .arg("-I")
        .arg(&headers)
        .args(&sources)
        .arg(&driver)
        .args(["-lm", "-o"])
        .arg(&executable)
        .output()
        .expect("run cc");
    assert!(
        build.status.success(),
        "the generated C compiles: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    (executable, [token.to_string(), vr.to_string()])
}

fn run(executable: &Path, mode: &str, args: &[String; 2]) -> String {
    let output = Command::new(executable)
        .arg(mode)
        .args(args)
        .output()
        .expect("run the C driver");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{mode} driver status {:?}: {stdout}",
        output.status.code()
    );
    stdout
}

fn numbers(line: &str, skip: usize) -> Vec<f64> {
    line.split_whitespace()
        .skip(skip)
        .filter_map(|value| value.parse().ok())
        .collect()
}

fn cc_available() -> bool {
    let available = Command::new("cc").arg("--version").output().is_ok();
    if !available {
        eprintln!("skipping: no C compiler");
    }
    available
}

#[test]
fn the_co_simulation_component_stays_on_the_manifold_and_tracks_native() {
    if !cc_available() {
        return;
    }
    let compiled = compiled(CUBIC);
    let native = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 0.2,
            dt: Some(1e-3),
            rtol: 1e-10,
            atol: 1e-12,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("Star simulates natively: {error:?}"));
    let columns = (1..=3)
        .map(|j| {
            let name = format!("i[{j}]");
            let index = native
                .names
                .iter()
                .position(|candidate| *candidate == name)
                .unwrap_or_else(|| panic!("{name} is recorded"));
            &native.data[index]
        })
        .collect::<Vec<_>>();
    let work = tempfile::tempdir().expect("C driver work directory");
    let (executable, args) = build_driver(&compiled, work.path());
    let stdout = run(&executable, "cs", &args);
    let mut compared = 0;
    for row in stdout.lines().filter(|line| line.starts_with("row ")) {
        let values = numbers(row, 1);
        let (t, i) = (values[0], &values[1..4]);
        // A correction is kept once it moves a state by more than the
        // component tolerance relative to its magnitude, so the residual
        // stays within that tolerance under the row's scale.
        let residual = i[0] + i[1] + i[2] + 0.1 * i[2].powi(3);
        let magnitude = i
            .iter()
            .fold(1.0_f64, |scale, value| scale.max(value.abs()));
        assert!(
            residual.abs() <= 3.0e-8 * magnitude * (1.0 + 0.3 * i[2] * i[2]),
            "t = {t}: the state leaves the manifold by {residual:e}"
        );
        let Some(sample) = native
            .times
            .iter()
            .position(|time| (time - t).abs() <= 1e-12)
        else {
            continue;
        };
        for (j, column) in columns.iter().enumerate() {
            let expected = column[sample];
            assert!(
                (i[j] - expected).abs() <= 1e-5 * expected.abs().max(1.0),
                "t = {t}: i[{}] = {} against native {expected}",
                j + 1,
                i[j]
            );
        }
        compared += 1;
    }
    assert!(compared >= 150, "compared {compared} points\n{stdout}");
}

#[test]
fn model_exchange_requests_the_correction_and_applies_it_in_event_mode() {
    if !cc_available() {
        return;
    }
    let work = tempfile::tempdir().expect("C driver work directory");
    let (executable, args) = build_driver(&compiled(LINEAR), work.path());
    let stdout = run(&executable, "me", &args);
    let line = |prefix: &str| {
        stdout
            .lines()
            .find(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("the driver reports {prefix}\n{stdout}"))
    };
    let initial = numbers(line("initial "), 1);
    for (actual, expected) in initial[..3].iter().zip([1.0, -0.4, -0.6]) {
        assert!((actual - expected).abs() <= 1e-12, "{stdout}");
    }
    assert_eq!(initial[3], 0.0, "a point on the manifold requests nothing");
    let completed = numbers(line("completed "), 1);
    assert_eq!(
        completed[..3],
        [1.0, -0.4, -0.5],
        "Continuous-Time Mode leaves the importer's states alone"
    );
    assert_eq!(
        completed[3], 1.0,
        "a point off the manifold requests Event Mode"
    );
    // The scaled minimum-norm correction of a unit-weight row moves each
    // current by a third of the 0.1 A residual.
    let event = numbers(line("event "), 1);
    for (actual, expected) in event[..3].iter().zip([1.0, -0.4, -0.5]) {
        assert!((actual - (expected - 0.1 / 3.0)).abs() <= 1e-12, "{stdout}");
    }
    assert_eq!(event[3], 1.0, "the update reports changed state values");
}

//! A top-level input declared without `start` exports with its type's default
//! start (MLS §4.9: `0`, `false`, or the first enumeration literal).
//!
//! FMI requires every input to publish a start, and an importer may run a
//! component without writing an input at all, so the exported component holds
//! that start until the importer's first write. Headless simulation, which has
//! no importer to write the input, still refuses the undriven input.

use rumoca::Compiler;
use rumoca_sim::SimOptions;

const MODEL: &str = "UnstartedInputs";
const SOURCE: &str = include_str!("../fixtures/fmi/UnstartedInputs.mo");

/// Every input and the start its exported declaration carries.
const INPUT_STARTS: [(&str, &[f64]); 6] = [
    ("u", &[0.0]),
    ("w", &[0.0, 0.0]),
    ("k", &[0.0]),
    ("b", &[0.0]),
    ("m", &[1.0]),
    ("s", &[3.0]),
];

fn compile() -> rumoca::CompilationResult {
    Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "UnstartedInputs.mo")
        .expect("compile the unstarted-input block")
}

#[test]
fn fmi_component_seeds_inputs_declared_without_start_with_their_type_default() {
    let compiled = compile();
    let component = rumoca_sim::lower_fmi_component(&compiled.dae)
        .expect("an input without `start` is exported with its type's default start");
    for (name, start) in INPUT_STARTS {
        let variable = component
            .variables()
            .iter()
            .find(|variable| variable.name() == name)
            .unwrap_or_else(|| panic!("`{name}` is published"));
        assert_eq!(variable.start(), Some(start), "start of `{name}`");
    }

    // The linked kernel holds those starts until an importer writes, so with
    // every default in place `der(x) = 1`.
    let artifact = rumoca_solver::fmi_me::MeModelArtifact::new(component);
    let run = rumoca_solver::fmi_me::fixed_step::fixed_step_rk4(&artifact, 0.1, 1.0)
        .expect("drive the component without writing any input");
    let last = run.states.last().expect("the drive records states");
    assert!((last[0] - 1.0).abs() < 1e-9, "x(1) = {last:?}");
}

#[test]
fn headless_simulation_still_refuses_an_undriven_input() {
    let compiled = compile();
    let error = rumoca_sim::lower_dae_for_simulation(&compiled.dae, &SimOptions::default())
        .expect_err("headless simulation has no importer to write the input");
    assert!(
        error
            .to_string()
            .contains("has neither a checked default nor a runtime value"),
        "{error}"
    );
}

/// The `start="..."` attribute the FMI 3 description publishes for `name`.
fn published_start(xml: &str, name: &str) -> String {
    let at = xml
        .find(&format!(" name=\"{name}\" valueReference="))
        .unwrap_or_else(|| panic!("`{name}` is published:\n{xml}"));
    let element = &xml[at..at + xml[at..].find('>').expect("element ends")];
    let start = element
        .split_once(" start=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .unwrap_or_else(|| panic!("FMI requires a start on input `{name}`: {element}"));
    start.0.to_string()
}

#[test]
fn fmi3_description_publishes_the_default_start_of_every_input() {
    let compiled = compile();
    let work = tempfile::tempdir().expect("FMI work directory");
    rumoca::compile_packaged_target(&compiled, MODEL, "fmi3", work.path().to_path_buf())
        .unwrap_or_else(|error| panic!("export fmi3: {error:#}"));
    let xml = std::fs::read_to_string(work.path().join(MODEL).join("modelDescription.xml"))
        .expect("read the model description");
    for (name, start) in [
        ("u", "0.0"),
        ("w", "0.0 0.0"),
        ("k", "0"),
        ("b", "false"),
        ("m", "1"),
        ("s", "3.0"),
    ] {
        assert_eq!(published_start(&xml, name), start, "start of `{name}`");
    }
}

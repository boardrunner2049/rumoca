//! A top-level input declared without `start` exports with its type's default
//! start (MLS §4.9: `0`, `false`, or the first enumeration literal).
//!
//! FMI requires every input to publish a start with `min <= start <= max`, and
//! an importer may run a component without writing an input at all, so the
//! exported component holds that start until the importer's first write. A
//! language default start the declared range excludes is moved to the nearest
//! bound; a start written outside the range is refused. GPU preparation and
//! the bitcode simulator's `--free-inputs start` seed the same value. Headless
//! simulation, which has no importer to write the input, still refuses the
//! undriven input.

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

/// Inputs whose declared range excludes their language default start (`a`,
/// `c`, `r`, `ki`), a start declared inside the range (`p`), and enumerations
/// whose first literal bounds them below.
const RANGED_MODEL: &str = "RangedInputs";
const RANGED_SOURCE: &str = include_str!("../fixtures/fmi/RangedInputs.mo");

/// Every ranged input and the start it holds before an importer writes it:
/// the declared start, else the type default moved to the nearest bound.
const RANGED_STARTS: [(&str, &[f64]); 7] = [
    ("a", &[1.0]),
    ("c", &[-1.0]),
    ("r", &[1.0, 1.0]),
    ("p", &[5.0]),
    ("ki", &[3.0]),
    ("m", &[1.0]),
    ("mh", &[3.0]),
];

fn compile() -> rumoca::CompilationResult {
    compile_model(MODEL, SOURCE)
}

fn compile_model(model: &str, source: &str) -> rumoca::CompilationResult {
    Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
        .unwrap_or_else(|error| panic!("compile `{model}`: {error:#}"))
}

/// The start the checked FMI component holds for every scalar of `name`.
fn component_start(component: &rumoca_ir_solve::fmi::FmiComponent, name: &str) -> Vec<f64> {
    component
        .variables()
        .iter()
        .find(|variable| variable.name() == name)
        .unwrap_or_else(|| panic!("`{name}` is published"))
        .start()
        .unwrap_or_else(|| panic!("`{name}` publishes a start"))
        .to_vec()
}

#[test]
fn fmi_component_seeds_inputs_declared_without_start_with_their_type_default() {
    let compiled = compile();
    let component = rumoca_sim::lower_fmi_component(&compiled.dae)
        .expect("an input without `start` is exported with its type's default start");
    for (name, start) in INPUT_STARTS {
        assert_eq!(
            component_start(&component, name),
            start,
            "start of `{name}`"
        );
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

#[test]
fn gpu_preparation_seeds_inputs_declared_without_start_like_fmi_export() {
    // GPU preparation shares the host-driven seeding with FMI export: before
    // the browser's first write an input holds the start its FMU publishes.
    let compiled = compile();
    rumoca_sim::lower_dae_for_gpu_preparation(&compiled.dae, &SimOptions::default())
        .expect("GPU preparation seeds an input without `start` with its type's default");
    let seeds = rumoca_sim::host_driven_input_starts(&compiled.dae).expect("input seeds");
    let expected: Vec<(String, f64)> = [
        ("u", 0.0),
        ("w[1]", 0.0),
        ("w[2]", 0.0),
        ("k", 0.0),
        ("b", 0.0),
        ("m", 1.0),
        ("s", 3.0),
    ]
    .map(|(name, value)| (name.to_string(), value))
    .to_vec();
    assert_eq!(seeds, expected);
}

#[test]
fn a_default_input_start_outside_its_declared_range_moves_to_the_nearest_bound() {
    let compiled = compile_model(RANGED_MODEL, RANGED_SOURCE);
    let component =
        rumoca_sim::lower_fmi_component(&compiled.dae).expect("export the ranged inputs");
    let seeds = rumoca_sim::host_driven_input_starts(&compiled.dae).expect("input seeds");
    for (name, start) in RANGED_STARTS {
        assert_eq!(
            component_start(&component, name),
            start,
            "start of `{name}`"
        );
        // FMI export, GPU preparation and `--free-inputs start` all read one
        // pre-write value per input scalar.
        let seeded: Vec<f64> = seeds
            .iter()
            .filter(|(scalar, _)| scalar == name || scalar.starts_with(&format!("{name}[")))
            .map(|(_, value)| *value)
            .collect();
        assert_eq!(seeded, start, "seed of `{name}`");
    }
    // The kernel holds those values until written: der(x) = a + c + p + ki.
    let artifact = rumoca_solver::fmi_me::MeModelArtifact::new(component);
    let run = rumoca_solver::fmi_me::fixed_step::fixed_step_rk4(&artifact, 0.1, 1.0)
        .expect("drive the component without writing any input");
    let last = run.states.last().expect("the drive records states");
    assert!((last[0] - 8.0).abs() < 1e-9, "x(1) = {last:?}");
}

/// The value of `attribute` on the FMI 3 variable `name`, if published.
fn fmi3_attribute(xml: &str, name: &str, attribute: &str) -> Option<String> {
    let at = xml
        .find(&format!(" name=\"{name}\" valueReference="))
        .unwrap_or_else(|| panic!("`{name}` is published:\n{xml}"));
    let element = &xml[at..at + xml[at..].find('>').expect("element ends")];
    element
        .split_once(&format!(" {attribute}=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(value, _)| value.to_string())
}

/// The `start="..."` attribute the FMI 3 description publishes for `name`.
fn published_start(xml: &str, name: &str) -> String {
    fmi3_attribute(xml, name, "start")
        .unwrap_or_else(|| panic!("FMI requires a start on input `{name}`"))
}

/// The `<Real|Integer|Boolean .../>` element of the FMI 2 scalar `name`.
fn fmi2_type_element<'xml>(xml: &'xml str, name: &str) -> &'xml str {
    let at = xml
        .find(&format!("<ScalarVariable name=\"{name}\" "))
        .unwrap_or_else(|| panic!("`{name}` is published:\n{xml}"));
    let body = &xml[at..];
    let open = body.find(">\n").expect("scalar variable opens") + 2;
    let close = open + body[open..].find("/>").expect("type element closes");
    body[open..close].trim()
}

fn exported_description(compiled: &rumoca::CompilationResult, model: &str, target: &str) -> String {
    let work = tempfile::tempdir().expect("FMI work directory");
    rumoca::compile_packaged_target(compiled, model, target, work.path().to_path_buf())
        .unwrap_or_else(|error| panic!("export {target}: {error:#}"));
    std::fs::read_to_string(work.path().join(model).join("modelDescription.xml"))
        .expect("read the model description")
}

#[test]
fn fmi_descriptions_publish_input_starts_inside_their_declared_range() {
    let compiled = compile_model(RANGED_MODEL, RANGED_SOURCE);
    let xml = exported_description(&compiled, RANGED_MODEL, "fmi3");
    for (name, start, min) in [
        ("a", "1.0", Some("1.0")),
        ("c", "-1.0", None),
        ("r", "1.0 1.0", Some("1.0")),
        ("p", "5.0", Some("1.0")),
        ("ki", "3", Some("3")),
        // An enumeration's ordinals start at its first literal.
        ("m", "1", Some("1")),
        ("mh", "3", Some("1")),
    ] {
        assert_eq!(published_start(&xml, name), start, "start of `{name}`");
        assert_eq!(
            fmi3_attribute(&xml, name, "min").as_deref(),
            min,
            "min of `{name}`"
        );
    }
    let xml = exported_description(&compiled, RANGED_MODEL, "fmi2");
    for (name, element) in [
        ("a", "<Real start=\"1.0\" min=\"1.0\" max=\"2.0\""),
        ("c", "<Real start=\"-1.0\" max=\"-1.0\""),
        ("r[2]", "<Real start=\"1.0\" min=\"1.0\" max=\"4.0\""),
        ("ki", "<Integer start=\"3\" min=\"3\""),
        ("m", "<Integer start=\"1\" min=\"1\""),
        ("mh", "<Integer start=\"3\" min=\"1\""),
    ] {
        assert_eq!(fmi2_type_element(&xml, name), element, "FMI 2 `{name}`");
    }
}

#[test]
fn fmi2_description_publishes_the_default_start_of_every_input() {
    let compiled = compile();
    let xml = exported_description(&compiled, MODEL, "fmi2");
    for (name, element) in [
        ("u", "<Real start=\"0.0\""),
        ("w[2]", "<Real start=\"0.0\""),
        ("k", "<Integer start=\"0\""),
        ("b", "<Boolean start=\"false\""),
        ("m", "<Integer start=\"1\" min=\"1\""),
        ("s", "<Real start=\"3.0\""),
    ] {
        assert_eq!(fmi2_type_element(&xml, name), element, "FMI 2 `{name}`");
    }
}

#[test]
fn fmi3_description_publishes_the_default_start_of_every_input() {
    let compiled = compile();
    let xml = exported_description(&compiled, MODEL, "fmi3");
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

const OUT_OF_RANGE_MODEL: &str = "OutOfRangeInputStart";
const OUT_OF_RANGE_SOURCE: &str = include_str!("../fixtures/fmi/OutOfRangeInputStart.mo");
const OUT_OF_RANGE_MESSAGE: &str =
    "the declared start 5 of input `a` lies outside its range [-inf, 2]";

#[test]
fn a_declared_input_start_outside_its_declared_range_is_refused() {
    let compiled = compile_model(OUT_OF_RANGE_MODEL, OUT_OF_RANGE_SOURCE);
    let error = rumoca_sim::lower_fmi_component(&compiled.dae)
        .expect_err("FMI export refuses a written start outside [min, max]");
    assert!(
        format!("{error:#}").contains(OUT_OF_RANGE_MESSAGE),
        "{error:#}"
    );
    // GPU preparation and `--free-inputs start` read the same rule.
    let error = rumoca_sim::host_driven_input_starts(&compiled.dae)
        .expect_err("the pre-write value of `a` is refused, not moved");
    assert!(error.to_string().contains(OUT_OF_RANGE_MESSAGE), "{error}");
    rumoca_sim::lower_dae_for_gpu_preparation(&compiled.dae, &SimOptions::default())
        .expect_err("GPU preparation refuses the out-of-range start");
    for target in ["fmi2", "fmi3"] {
        let work = tempfile::tempdir().expect("FMI work directory");
        let error = rumoca::compile_packaged_target(
            &compiled,
            OUT_OF_RANGE_MODEL,
            target,
            work.path().to_path_buf(),
        )
        .expect_err("export refuses the out-of-range start");
        assert!(
            format!("{error:#}").contains(OUT_OF_RANGE_MESSAGE),
            "{target}: {error:#}"
        );
    }
}

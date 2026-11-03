//! Fields of a top-level `input` or `output` record component export with the
//! component's causality (MLS §4.4.2.2: the prefix applies to every field), in
//! FMI 2 and FMI 3 alike, and every output field is listed in
//! `<ModelStructure>`. A record output of a nested component stays local, a
//! `parameter` field stays a parameter, and an input field (or scalar input)
//! with a declaration equation publishes that equation's value as its start:
//! the value it holds until the importer's first write.

use rumoca::Compiler;
use rumoca_ir_solve::fmi::FmiCausality;

const MODEL: &str = "RecordPorts";
const SOURCE: &str = include_str!("../fixtures/fmi/RecordPorts.mo");

const CAUSALITIES: [(&str, FmiCausality); 11] = [
    ("u.x", FmiCausality::Input),
    ("u.y", FmiCausality::Input),
    ("w.a", FmiCausality::Input),
    ("w.b", FmiCausality::Input),
    ("v", FmiCausality::Input),
    ("r.x", FmiCausality::Output),
    ("r.y", FmiCausality::Output),
    ("r.k", FmiCausality::Parameter),
    ("z", FmiCausality::Output),
    ("sub.q.x", FmiCausality::Local),
    ("sub.q.y", FmiCausality::Local),
];

/// The `causality` attribute FMI 2 and FMI 3 publish for `causality`.
fn exported_causality(causality: FmiCausality) -> &'static str {
    match causality {
        FmiCausality::Input => "input",
        FmiCausality::Output => "output",
        FmiCausality::Parameter => "parameter",
        FmiCausality::Local => "local",
        other => panic!("unexpected causality {other:?}"),
    }
}

fn compile() -> rumoca::CompilationResult {
    Compiler::new()
        .model(MODEL)
        .compile_str(SOURCE, "RecordPorts.mo")
        .unwrap_or_else(|error| panic!("compile `{MODEL}`: {error:#}"))
}

fn exported_description(target: &str) -> String {
    let compiled = compile();
    let work = tempfile::tempdir().expect("FMI work directory");
    rumoca::compile_packaged_target(&compiled, MODEL, target, work.path().to_path_buf())
        .unwrap_or_else(|error| panic!("export {target}: {error:#}"));
    std::fs::read_to_string(work.path().join(MODEL).join("modelDescription.xml"))
        .expect("read the model description")
}

/// The opening tag of the variable `name`, from `<` up to its first `>`.
fn variable_tag<'xml>(xml: &'xml str, open: &str, name: &str) -> &'xml str {
    let at = xml
        .find(&format!("<{open} name=\"{name}\" "))
        .unwrap_or_else(|| panic!("`{name}` is published:\n{xml}"));
    &xml[at..at + xml[at..].find('>').expect("tag ends")]
}

fn attribute<'tag>(tag: &'tag str, attribute: &str) -> &'tag str {
    tag.split_once(&format!(" {attribute}=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(value, _)| value)
        .unwrap_or_else(|| panic!("`{attribute}` on {tag}"))
}

#[test]
fn record_port_fields_take_the_record_component_causality() {
    let compiled = compile();
    let component = rumoca_sim::lower_fmi_component(&compiled.dae).expect("export record ports");
    for (name, causality) in CAUSALITIES {
        let variable = component
            .variables()
            .iter()
            .find(|variable| variable.name() == name)
            .unwrap_or_else(|| panic!("`{name}` is published"));
        assert_eq!(variable.causality(), causality, "causality of `{name}`");
    }
}

#[test]
fn a_bound_input_publishes_its_declaration_equation_value_as_start() {
    let compiled = compile();
    let component = rumoca_sim::lower_fmi_component(&compiled.dae).expect("export record ports");
    for (name, start) in [("w.a", 0.0), ("w.b", 7.0), ("v", 3.0)] {
        let variable = component
            .variables()
            .iter()
            .find(|variable| variable.name() == name)
            .unwrap_or_else(|| panic!("`{name}` is published"));
        assert_eq!(variable.start(), Some(&[start][..]), "start of `{name}`");
    }
    let xml = exported_description("fmi3");
    assert_eq!(
        attribute(variable_tag(&xml, "Float64", "w.b"), "start"),
        "7.0"
    );
    assert_eq!(
        attribute(variable_tag(&xml, "Float64", "v"), "start"),
        "3.0"
    );
}

#[test]
fn fmi3_description_lists_record_output_fields_as_outputs() {
    let xml = exported_description("fmi3");
    let mut outputs = Vec::new();
    for (name, causality) in CAUSALITIES {
        let tag = variable_tag(&xml, "Float64", name);
        assert_eq!(
            attribute(tag, "causality"),
            exported_causality(causality),
            "{tag}"
        );
        if causality == FmiCausality::Parameter {
            // Settable by the importer, as before it was a record field.
            assert_eq!(attribute(tag, "variability"), "tunable", "{tag}");
        }
        if causality == FmiCausality::Output {
            outputs.push(format!(
                "<Output valueReference=\"{}\"",
                attribute(tag, "valueReference")
            ));
        }
    }
    let structure = &xml[xml.find("<ModelStructure>").expect("model structure")..];
    for output in &outputs {
        assert!(
            structure.contains(output.as_str()),
            "{output}:\n{structure}"
        );
    }
    assert_eq!(
        structure.matches("<Output ").count(),
        outputs.len(),
        "{structure}"
    );
}

#[test]
fn fmi2_description_lists_record_output_fields_as_outputs() {
    let xml = exported_description("fmi2");
    let variables = &xml[..xml.find("<ModelStructure>").expect("model structure")];
    let mut outputs = Vec::new();
    for (index, (name, causality)) in variables
        .match_indices("<ScalarVariable name=\"")
        .enumerate()
        .map(|(index, (at, _))| {
            let tag = &variables[at..at + variables[at..].find('>').expect("tag ends")];
            (
                index + 1,
                (attribute(tag, "name"), attribute(tag, "causality")),
            )
        })
    {
        let declaration = name.split_once('[').map_or(name, |(base, _)| base);
        if let Some((_, expected)) = CAUSALITIES.iter().find(|(known, _)| *known == declaration) {
            assert_eq!(
                causality,
                exported_causality(*expected),
                "causality of `{name}`"
            );
        }
        if causality == "output" {
            outputs.push(format!("<Unknown index=\"{index}\""));
        }
    }
    // r.x, r.y[1], r.y[2], z.
    assert_eq!(outputs.len(), 4, "{variables}");
    let structure = &xml[xml.find("<Outputs>").expect("outputs")..];
    let structure = &structure[..structure.find("</Outputs>").expect("outputs end")];
    for output in &outputs {
        assert!(
            structure.contains(output.as_str()),
            "{output}:\n{structure}"
        );
    }
}

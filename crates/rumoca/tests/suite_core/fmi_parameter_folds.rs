//! Tunable parameter bindings containing nested matrix folds must pass the
//! dependency proof and render through both shared FMI C targets.

const SOURCE: &str = r#"model NestedParameterFold
  function accumulate
    input Real scale;
    output Real values[2,2];
  protected
    Real total;
  algorithm
    values := zeros(2,2);
    total := 0;
    for part in 1:2 loop
      total := total + scale;
      for i in 1:2 loop
        for j in 1:2 loop
          values[i,j] := values[i,j] + total*(i+j);
        end for;
      end for;
    end for;
    assert(values[1,1] >= 0, "negative accumulated entry");
  end accumulate;
  parameter Real scale = 2;
  parameter Real values[2,2] = accumulate(scale);
  parameter Real gain = values[1,1]+values[1,2]+values[2,1]+values[2,2];
  output Real x(start=0,fixed=true);
equation
  der(x) = gain;
end NestedParameterFold;
"#;

#[test]
fn nested_parameter_folds_render_for_both_fmi_targets() {
    let compiled = rumoca::Compiler::new()
        .model("NestedParameterFold")
        .compile_str(SOURCE, "NestedParameterFold.mo")
        .expect("compile nested parameter folds");
    for target in ["fmi3", "fmi2"] {
        let files = rumoca::render_target_files(&compiled, "NestedParameterFold", target, None)
            .unwrap_or_else(|error| panic!("render {target}: {error:#}"));
        assert!(files.iter().any(|file| file.path == "sources/model.c"));
        assert!(files.iter().any(|file| file.path == "modelDescription.xml"));
    }
}

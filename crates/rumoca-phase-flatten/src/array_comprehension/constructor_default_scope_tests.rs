//! A record-constructor default unrolled as a comprehension keeps the record
//! instance's scope (MLS §12.6, §10.4.1).

use rumoca_ir_ast as ast;
use rumoca_ir_flat as flat;

const SOURCE: &str = "
package Lib
  record Geometry
    parameter Integer nArms = 4;
    parameter Real armMount[nArms, 3] = {{i, 0, 0} for i in 1:nArms};
    parameter Real p[4, 3] = zeros(4, 3);
  end Geometry;

  record X8
    extends Geometry(p = {armMount[i, :] for i in 1:4});
  end X8;

  model Base
    parameter Geometry g;
    Real y = g.p[1, 1];
  end Base;

  model M
    extends Base(g = X8());
  end M;

  model Top
    M m;
  end Top;
end Lib;
";

fn flatten_source(model: &str) -> flat::Model {
    let file_name = "<constructor_default_scope_tests>";
    let stored = rumoca_phase_parse::parse_to_ast(SOURCE, file_name).expect("fixture should parse");
    let mut tree = ast::ClassTree::from_parsed(stored);
    tree.source_map.add(file_name, SOURCE);
    let resolved =
        rumoca_phase_resolve::resolve(ast::ParsedTree::new(tree)).expect("fixture should resolve");
    let instanced =
        rumoca_phase_instantiate::instantiate(resolved, model).expect("fixture should instantiate");
    let ast::InstancedTree { tree, mut overlay } = instanced;
    rumoca_phase_typecheck::typecheck_instanced(&tree, &mut overlay, model)
        .expect("fixture should typecheck");
    crate::flatten_ref(&tree, &overlay, model).expect("fixture should flatten")
}

fn binding_refs(model: &flat::Model, name: &str) -> Vec<String> {
    let binding = model
        .variables
        .get(&rumoca_core::VarName::new(name))
        .and_then(|variable| variable.binding.as_ref())
        .unwrap_or_else(|| panic!("`{name}` should carry a binding"));
    let mut references = Vec::new();
    binding.collect_var_refs(&mut references);
    references
        .iter()
        .map(|name| name.as_str().to_string())
        .collect()
}

#[test]
fn extends_modifier_default_names_fields_of_the_record_instance() {
    // `g = X8()` takes `p` from `X8`'s extends modifier; its `armMount` is the
    // constructor's own input, i.e. the field `g.armMount`, not a name beside
    // the `g = X8()` modifier (MLS §12.6).
    for (model, field, expected) in [
        ("Lib.M", "g.p", "g.armMount"),
        ("Lib.Top", "m.g.p", "m.g.armMount"),
    ] {
        let references = binding_refs(&flatten_source(model), field);
        assert!(
            !references.is_empty(),
            "{model}: `{field}` should read armMount"
        );
        assert!(
            references.iter().all(|reference| reference == expected),
            "{model}: `{field}` should read only `{expected}`, got {references:?}"
        );
    }
}

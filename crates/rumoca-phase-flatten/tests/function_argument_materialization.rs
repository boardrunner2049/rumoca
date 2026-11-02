//! Final Flat function calls have declaration-order executable argument slots.
//! Named/default syntax is a source concern and does not cross the DAE boundary.

use rumoca_core::Expression;
use rumoca_ir_ast as ast;

const SOURCE: &str = r#"
package Constants
  constant Real defaultOffset = 1.0;
end Constants;

function addOffset
  input Real value;
  input Real offset = value + Constants.defaultOffset;
  output Real result;
algorithm
  result := value + offset;
end addOffset;

model UsesDefault
  // `time`, not a literal: a call whose arguments are all settled at
  // translation is folded to its result before the final Flat model, and this
  // test inspects the surviving call's argument slots.
  Real result = addOffset(value = time);
end UsesDefault;
"#;

fn flatten_source() -> rumoca_ir_flat::Model {
    let file_name = "<function_argument_materialization>";
    let stored = rumoca_phase_parse::parse_to_ast(SOURCE, file_name).expect("source parses");
    let mut tree = ast::ClassTree::from_parsed(stored);
    tree.source_map.add(file_name, SOURCE);
    let resolved =
        rumoca_phase_resolve::resolve(ast::ParsedTree::new(tree)).expect("source resolves");
    let instanced =
        rumoca_phase_instantiate::instantiate(resolved, "UsesDefault").expect("model instantiates");
    let ast::InstancedTree { tree, mut overlay } = instanced;
    rumoca_phase_typecheck::typecheck_instanced(&tree, &mut overlay, "UsesDefault")
        .expect("model typechecks");
    rumoca_phase_flatten::flatten_ref(&tree, &overlay, "UsesDefault").expect("model flattens")
}

#[test]
fn named_and_dependent_default_arguments_are_positional_in_final_flat() {
    let model = flatten_source();
    let binding = model
        .variables
        .get(&rumoca_core::VarName::new("result"))
        .and_then(|variable| variable.binding.as_ref())
        .expect("result binding");
    let Expression::FunctionCall { name, args, .. } = binding else {
        panic!("result binding is a function call");
    };
    let resolved = name
        .resolved_function()
        .expect("call carries exact Flat function instance identity");
    assert_eq!(
        model
            .functions
            .values()
            .find(|function| function.instance_id == Some(resolved.instance_id))
            .map(|function| function.name.as_str()),
        Some("addOffset")
    );
    // Two positional slots: the named argument and the materialized default,
    // in declaration order. The supplied argument is `time` rather than a
    // literal so the call survives pure-constant folding; what is asserted is
    // unchanged, namely that the dependent default carries the *actual*
    // argument and the package constant is resolved (to the model constant the
    // frontend declares for it; `inline-constants` replaces it by the value).
    assert_eq!(args.len(), 2);
    assert!(
        matches!(&args[0], Expression::VarRef { name, .. } if name.as_str() == "time"),
        "the named argument becomes the first positional slot, found {:?}",
        &args[0]
    );
    assert!(
        matches!(
            &args[1],
            Expression::Binary { lhs, rhs, .. }
                if matches!(lhs.as_ref(), Expression::VarRef { name, .. } if name.as_str() == "time")
                && real_value(&model, rhs) == Some(1.0)
        ),
        "the dependent default materializes as `value + defaultOffset` with the \
         actual argument and the constant resolved, found {:?}",
        &args[1]
    );
}

/// A Real literal, or a reference to a declared constant with a Real literal
/// binding.
fn real_value(model: &rumoca_ir_flat::Model, expression: &Expression) -> Option<f64> {
    match expression {
        Expression::Literal {
            value: rumoca_core::Literal::Real(value),
            ..
        } => Some(*value),
        Expression::VarRef {
            name, subscripts, ..
        } if subscripts.is_empty() => {
            let constant = model
                .variables
                .get(&rumoca_core::VarName::new(name.as_str()))?;
            matches!(constant.variability, rumoca_core::Variability::Constant(_)).then_some(())?;
            real_value(model, constant.binding.as_ref()?)
        }
        _ => None,
    }
}

//! An `if` inside an equation family that parameter values decide.
//!
//! DAE construction lowers the whole conditional; the default pass group's
//! `select-family-branches` replaces it by the branch taken, which is what
//! construction itself used to do (docs/design/minimal-frontend.md).

const FAMILY_IF: &str = "\
model FamilyIf
  parameter Real k = 2;
  Real x[4];
equation
  for i in 1:4 loop
    der(x[i]) = if k > 1 then -x[i] else x[i];
  end for;
end FamilyIf;
";

/// Whether any equation family body reaches a conditional.
fn family_has_conditional(passes: &[&str]) -> bool {
    let passes: Vec<String> = passes.iter().map(ToString::to_string).collect();
    let compiled = rumoca::Compiler::new()
        .model("FamilyIf")
        .passes(&passes)
        .compile_str(FAMILY_IF, "FamilyIf.mo")
        .expect("model compiles");
    let file = rumoca_bitcode::export(
        &compiled.dae,
        None,
        "FamilyIf",
        &rumoca_bitcode::ExportOptions::default(),
    )
    .expect("compiled model exports");
    let model = &file.model;
    assert!(
        !model.equation_families.is_empty(),
        "the for-equation should stay one family"
    );
    let mut pending: Vec<_> = model
        .equation_families
        .iter()
        .flat_map(|family| family.bodies.iter().copied())
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = &model.expressions[id.0 as usize].node;
        if matches!(
            node,
            rumoca_bitcode::schema::RbcExprNode::Conditional { .. }
        ) {
            return true;
        }
        pending.extend(rumoca_bitcode::build::references(id, node));
    }
    false
}

#[test]
fn construction_keeps_the_conditional() {
    assert!(family_has_conditional(&["none"]));
}

#[test]
fn the_default_group_selects_the_branch() {
    assert!(!family_has_conditional(&[]));
}

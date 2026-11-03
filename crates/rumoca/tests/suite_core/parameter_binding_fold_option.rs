//! Derived parameter bindings: the frontend keeps them as written, the
//! `fold-parameter-bindings` pass folds them.
//!
//! `parameter Real d = k * 10` with `k = 2` reaches Flat and the frontend's
//! DAE as `k * 10`: the dependency on `k` is a fact about the model, and no
//! later phase could recover it once the arithmetic had been carried out.
//! When `k` cannot change after translation (`Evaluate = true`), a plain
//! `rumoca compile` emits `d = 20.0`, the value a solver wants, because its
//! default pass group folds it; `--no-fold-parameter-bindings` runs the group
//! without that pass. A tunable `k` is never folded away.
//!
//! What lowering must *not* do is leave a binding unresolved that decides the
//! model's shape. MLS §18.3 structural parameters decide array extents and
//! branch selection, and Integer/Boolean/String parameters seed those
//! decisions for nested components, so the frontend resolves those. The
//! resolved value decides the shape; Flat still renders the binding as
//! written (instantiation keeps it as the binding source), so a legal parent
//! override recomputes the dependent value.

use rumoca_compile::compile::{Session, SessionConfig};

fn session() -> Session {
    Session::new(SessionConfig::default())
}

/// The rendered binding of a flat variable, or `"<none>"` when it has none.
fn binding_of(flat: &rumoca_compile::compile::FlatModel, name: &str) -> String {
    let interned = rumoca_core::VarName::intern(name);
    let variable = flat
        .variables
        .get(&interned)
        .unwrap_or_else(|| panic!("flat model has no variable `{name}`"));
    variable
        .binding
        .as_ref()
        .map_or_else(|| "<none>".to_string(), |expr| format!("{expr:?}"))
}

/// The binding node of `name` in the bitcode of a full `rumoca compile`.
fn compiled_binding(source: &str, model: &str, name: &str, no_fold: bool) -> String {
    let compiled = rumoca::Compiler::new()
        .model(model)
        .no_fold_parameter_bindings(no_fold)
        .compile_str(source, &format!("{model}.mo"))
        .expect("model compiles");
    let file = rumoca_bitcode::export(
        &compiled.dae,
        None,
        model,
        &rumoca_bitcode::ExportOptions::default(),
    )
    .expect("compiled model exports");
    let variable = file
        .model
        .variables
        .iter()
        .find(|variable| variable.name == name)
        .unwrap_or_else(|| panic!("artifact has no variable `{name}`"));
    let binding = variable.binding.expect("variable keeps a binding");
    format!("{:?}", file.model.expressions[binding.0 as usize].node)
}

const DERIVED: &str = "\
model Derived
  parameter Real k = 2;
  parameter Real d = k * 10;
  Real x;
equation
  der(x) = 1 / d;
end Derived;
";

#[test]
fn the_frontend_keeps_a_derived_real_binding_as_written() {
    let mut session = session();
    session
        .add_document("derived.mo", DERIVED)
        .expect("fixture parses");
    let compiled = session.compile_model("Derived").expect("model compiles");

    let binding = binding_of(&compiled.flat, "d");
    assert!(
        binding.contains('k'),
        "the frontend should keep the reference to `k`, got {binding}"
    );
    // The point of keeping it is that the dependency is readable, not merely
    // that some expression survived: `d` must still be the product.
    assert!(
        binding.contains("Mul") || binding.contains("Multiply") || binding.contains('*'),
        "the binding should still be the product, got {binding}"
    );
}

#[test]
fn the_default_pass_group_folds_a_binding_settled_at_translation() {
    let binding = compiled_binding(EVALUATED, "Derived", "d", false);
    assert!(
        binding.contains("Literal") && binding.contains("20.0"),
        "`fold-parameter-bindings` should write the value, got {binding}"
    );
}

#[test]
fn a_binding_that_reads_a_tunable_parameter_keeps_reading_it() {
    // `k` can be changed on the artifact without recompiling; `d` must follow.
    let binding = compiled_binding(DERIVED, "Derived", "d", false);
    assert!(
        binding.contains("Binary"),
        "a tunable input must not be folded away, got {binding}"
    );
}

#[test]
fn no_fold_parameter_bindings_skips_the_pass() {
    let binding = compiled_binding(EVALUATED, "Derived", "d", true);
    assert!(
        binding.contains("Binary"),
        "without the pass the binding should still be the product, got {binding}"
    );
}

/// `k` is not tunable, so `d`'s value is settled at translation.
const EVALUATED: &str = "\
model Derived
  parameter Real k = 2 annotation(Evaluate = true);
  parameter Real d = k * 10;
  Real x;
equation
  der(x) = 1 / d;
end Derived;
";

const STRUCTURAL: &str = "\
model Structural
  parameter Integer base = 2;
  parameter Integer n = base + 1;
  parameter Boolean useExtra = base > 1;
  parameter Real gain = base * 4;
  Real y[n];
  Real extra if useExtra;
equation
  for i in 1:n loop
    der(y[i]) = gain * i;
  end for;
  if useExtra then
    extra = gain;
  end if;
end Structural;
";

#[test]
fn the_frontend_still_resolves_structural_and_discrete_parameters() {
    let mut session = session();
    session
        .add_document("structural.mo", STRUCTURAL)
        .expect("fixture parses");
    let compiled = session
        .compile_model("Structural")
        .unwrap_or_else(|error| panic!("should compile: {error:?}"));

    // Shape is the real claim: the array extent and the conditional component
    // are decided from the resolved values. The Flat bindings of `n` and
    // `useExtra` stay as written (`base + 1`, `base > 1`): instantiation keeps
    // the declaration as the binding source once it resolves a different value.
    let has = |name: &str| {
        compiled
            .flat
            .variables
            .contains_key(&rumoca_core::VarName::intern(name))
    };
    assert!(has("extra"), "the conditional component should exist");
    let extent = compiled
        .flat
        .variables
        .get(&rumoca_core::VarName::intern("y"))
        .map(|variable| variable.dims.clone());
    assert!(
        extent == Some(vec![3]) || (has("y[3]") && !has("y[4]")),
        "the array should have extent 3: {:?}",
        compiled.flat.variables.keys().collect::<Vec<_>>()
    );
    // And the Real parameter keeps its dependency.
    assert!(binding_of(&compiled.flat, "gain").contains("base"));
}

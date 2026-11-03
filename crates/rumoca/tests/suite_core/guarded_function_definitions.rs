//! A function-body definition read inside one arm of a conditional expression
//! keeps its value outside that arm.
//!
//! A call with no directional derivative (here because the body uses `max`)
//! is inlined by the scalar Solve lowerer. That lowerer caches each SSA
//! definition of the body once per call frame. It used to lower the
//! definition under whatever activation guards were live at the first read,
//! so a definition first demanded from `if c then d else 0` was computed by a
//! loop that only ran when `c` held. The same register then served every
//! later read, and reads with `c` false saw the loop's unwritten output
//! (zero). In the FIRE library's `combineMassProperties` that turned the
//! parallel-axis products of inertia into products about the origin, since
//! the combined centre of mass read as zero off the diagonal.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimResult, simulate_dae_with_diagnostics};

fn simulate(source: &str, model: &str) -> SimResult {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(source, &format!("{model}.mo"))
        .unwrap_or_else(|error| panic!("{model} must compile: {error:?}"));
    simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 0.01,
            ..SimOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{model} must simulate: {error:?}"))
}

fn last(result: &SimResult, name: &str) -> f64 {
    let index = result
        .names
        .iter()
        .position(|candidate| candidate == name)
        .unwrap_or_else(|| panic!("simulation result has no column {name}: {:?}", result.names));
    *result.data[index]
        .last()
        .expect("the simulation reports at least one sample")
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0),
        "{what}: expected {expected}, got {actual}"
    );
}

/// The loop result `m` is first read in the untaken arm of the conditional.
const ARM_FIRST_READ: &str = "\
package ArmFirstRead
  function f
    input Real x[:];
    input Real s;
    output Real y;
  protected
    Real m;
  algorithm
    m := 0;
    for i in 1:size(x, 1) loop
      m := m + x[i] + 0 * max(x);
    end for;
    y := (if s > 0 then m else 0) + 10 * m;
  end f;
  model M
    parameter Real x[3] = {1, 2, 3};
    parameter Real s = -1;
    parameter Real y = f(x, s);
    Real z = y;
  end M;
end ArmFirstRead;
";

#[test]
fn definition_first_read_in_an_untaken_arm_keeps_its_value() {
    let result = simulate(ARM_FIRST_READ, "ArmFirstRead.M");
    assert_close(last(&result, "z"), 60.0, "0 + 10 * (1 + 2 + 3)");
}

/// The shape of the FIRE library's `combineMassProperties`: record-array
/// parts, a validating loop whose assertion uses `max(abs(...))`, and the
/// parallel-axis term `(if j == k then d * d else 0) - d[j] * d[k]`.
const MASS_PROPERTIES: &str = "\
package MassProperties
  record Part
    Real mass = 0;
    Real r_C[3] = {0, 0, 0};
    Real R_Cj[3, 3] = identity(3);
    Real inertia[3, 3] = zeros(3, 3);
  end Part;
  function combine
    input Part parts[:];
    output Part combined;
  protected
    Real offset[3];
    Real rotated[3, 3];
    Real scale;
  algorithm
    combined.mass := 0;
    combined.r_C := zeros(3);
    combined.R_Cj := identity(3);
    combined.inertia := zeros(3, 3);
    for i in 1:size(parts, 1) loop
      scale := max(abs(parts[i].inertia));
      assert(parts[i].mass > 0 or scale == 0, \"massless part with inertia\");
      combined.mass := combined.mass + parts[i].mass;
      combined.r_C := combined.r_C + parts[i].mass * parts[i].r_C;
    end for;
    if combined.mass > 0 then
      combined.r_C := combined.r_C / combined.mass;
    end if;
    for i in 1:size(parts, 1) loop
      offset := parts[i].r_C - combined.r_C;
      rotated := parts[i].R_Cj * parts[i].inertia * transpose(parts[i].R_Cj);
      for j in 1:3 loop
        for k in 1:3 loop
          combined.inertia[j, k] := combined.inertia[j, k] + rotated[j, k]
            + parts[i].mass * ((if j == k then offset * offset else 0) - offset[j] * offset[k]);
        end for;
      end for;
    end for;
  end combine;
  model M
    parameter Part core(mass = 2, r_C = {1, -2, 3}, inertia = diagonal({2, 3, 4}));
    parameter Part arm(mass = 1, r_C = {3, -1, 4},
      R_Cj = [0, -1, 0; 1, 0, 0; 0, 0, 1], inertia = diagonal({0.1, 0.2, 0.3}));
    parameter Part payload(mass = 1, r_C = {-1, -3, 2},
      inertia = [0.4, 0.05, 0; 0.05, 0.5, 0; 0, 0, 0.6]);
    final parameter Part total = combine({core, arm, payload});
    final parameter Part single = combine({payload});
    Real cg[3] = total.r_C;
    Real J[3, 3] = total.inertia;
    Real K[3, 3] = single.inertia;
  end M;
end MassProperties;
";

#[test]
fn record_array_mass_properties_use_the_combined_centre_of_mass() {
    let result = simulate(MASS_PROPERTIES, "MassProperties.M");
    for (index, expected) in [1.0, -2.0, 3.0].into_iter().enumerate() {
        let name = format!("cg[{}]", index + 1);
        assert_close(last(&result, &name), expected, &name);
    }
    // sum R J R' + m (|d|^2 E - d d') about the combined centre {1, -2, 3},
    // computed independently with numpy. About the origin the off-diagonal
    // entries would be 4.05, -16 and 22 instead.
    let expected = [[6.6, -3.95, -4.0], [-3.95, 13.6, -2.0], [-4.0, -2.0, 14.9]];
    for (row, values) in expected.iter().enumerate() {
        for (column, expected) in values.iter().enumerate() {
            let name = format!("J[{},{}]", row + 1, column + 1);
            assert_close(last(&result, &name), *expected, &name);
        }
    }
    // One part is already about its own centre of mass, so its tensor passes
    // through untranslated (the off-origin part used to gain -m r_j r_k).
    let expected = [[0.4, 0.05, 0.0], [0.05, 0.5, 0.0], [0.0, 0.0, 0.6]];
    for (row, values) in expected.iter().enumerate() {
        for (column, expected) in values.iter().enumerate() {
            let name = format!("K[{},{}]", row + 1, column + 1);
            assert_close(last(&result, &name), *expected, &name);
        }
    }
}

//! Affine derivative coefficients read through aggregates.
//!
//! Solve lowering proves each matched derivative's affine coefficient is not
//! declared zero. `Electrical.Analog.Lines.OLine` gives segment `k` the
//! inductance `lm[k]` of a comprehension binding
//! `{if i == 1 or i == N + 1 then l/(2*N) else l/N for i in 1:N + 1}`, so the
//! probe must select element `k` through the index, the comprehension point,
//! and the binder, as the runtime program does. The coefficient itself stays
//! symbolic and reads the runtime parameter vector.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, simulate_dae_with_diagnostics};

const MODEL: &str = r#"
model Segments
  parameter Integer N = 2;
  parameter Real l = 1;
  parameter Real lm[N + 1] = {if i == 1 or i == N + 1 then l/(2*N) else l/N for i in 1:N + 1};
  Real x[N + 1](each start = 1, each fixed = true);
equation
  for k in 1:N + 1 loop
    lm[k]*der(x[k]) = -x[k];
  end for;
end Segments;
"#;

#[test]
fn a_comprehension_coefficient_is_selected_per_segment() {
    let compiled = Compiler::new()
        .model("Segments")
        .compile_str(MODEL, "Segments.mo")
        .expect("the segmented model compiles");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            t_end: 0.5,
            ..SimOptions::default()
        },
    )
    .expect("the segment coefficients are proven nonzero");
    let last = |name: &str| {
        let index = result
            .names
            .iter()
            .position(|candidate| candidate == name)
            .expect("the result records the column");
        *result.data[index].last().expect("a sample")
    };
    // lm = {1/4, 1/2, 1/4}: x[k](t) = exp(-t/lm[k]).
    assert!((last("x[1]") - (-2.0f64).exp()).abs() < 1e-4);
    assert!((last("x[2]") - (-1.0f64).exp()).abs() < 1e-4);
    assert!((last("x[3]") - (-2.0f64).exp()).abs() < 1e-4);
}

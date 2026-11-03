//! A supply star earthed only through an unconnected insulation resistor.
//!
//! The resistor's far pin is an unconnected flow, so `pin.i = 0` pins its
//! current to zero, and the star's flow balance makes the three inductor
//! currents sum to zero: an index-two constraint on states. The connection
//! sets write that balance as a sum of negated flows, and after the alias
//! quotient it reads the inductor currents through double negations and the
//! pinned current. The reducer differentiates it directly, so a model that
//! also resets a state with `reinit` (which the formal-derivative route does
//! not carry) still reduces, and the currents keep summing to zero.

use rumoca::Compiler;
use rumoca_sim::{SimOptions, SimSolverMode, simulate_dae_with_diagnostics};

const EARTHED_STAR: &str = "
package Circuit
  connector Pin
    Real v;
    flow Real i;
  end Pin;
  partial model TwoPin
    Pin p;
    Pin n;
    Real v;
    Real i;
  equation
    v = p.v - n.v;
    0 = p.i + n.i;
    i = p.i;
  end TwoPin;
  model Resistor
    extends TwoPin;
    parameter Real R = 1;
  equation
    v = R*i;
  end Resistor;
  model Inductor
    extends TwoPin;
    parameter Real L = 0.01;
  equation
    L*der(i) = v;
  end Inductor;
  model Source
    extends TwoPin;
    parameter Real phase = 0;
  equation
    v = sin(314*time + phase);
  end Source;
  model Ground
    Pin p;
  equation
    p.v = 0;
  end Ground;
  model Earthing
    Pin p;
    Pin pin;
    Resistor insulation(R = 1e6);
  equation
    connect(p, insulation.p);
    connect(insulation.n, pin);
  end Earthing;
  model Mean
    input Real u;
    Real x(start = 0, fixed = true);
  equation
    der(x) = u;
    when sample(0, 0.02) then
      reinit(x, 0);
    end when;
  end Mean;
  model Star
    Source s1(phase = 0);
    Source s2(phase = -2.0943951023931953);
    Source s3(phase = -4.1887902047863905);
    Earthing earthing;
    Resistor r1;
    Resistor r2;
    Resistor r3;
    Inductor l1(i(start = 0, fixed = true));
    Inductor l2(i(start = 0, fixed = true));
    Inductor l3;
    Ground ground;
    Mean mean(u = l1.v);
  equation
    connect(s1.n, earthing.p);
    connect(s2.n, earthing.p);
    connect(s3.n, earthing.p);
    connect(s1.p, r1.p);
    connect(s2.p, r2.p);
    connect(s3.p, r3.p);
    connect(r1.n, l1.p);
    connect(r2.n, l2.p);
    connect(r3.n, l3.p);
    connect(l1.n, ground.p);
    connect(l2.n, ground.p);
    connect(l3.n, ground.p);
  end Star;
  model StarPoint
    Pin a;
    Pin b;
    Pin c;
    Pin n;
  equation
    connect(a, n);
    connect(b, n);
    connect(c, n);
  end StarPoint;
  model Floating
    Source s1(phase = 0);
    Source s2(phase = -2.0943951023931953);
    Source s3(phase = -4.1887902047863905);
    Resistor r1;
    Resistor r2;
    Resistor r3;
    Inductor l1(i(start = 0, fixed = true));
    Inductor l2(i(start = 0, fixed = true));
    Inductor l3;
    StarPoint star;
    Ground ground;
    Mean mean(u = l1.v);
  equation
    connect(s1.n, ground.p);
    connect(s2.n, ground.p);
    connect(s3.n, ground.p);
    connect(s1.p, r1.p);
    connect(s2.p, r2.p);
    connect(s3.p, r3.p);
    connect(r1.n, l1.p);
    connect(r2.n, l2.p);
    connect(r3.n, l3.p);
    connect(l1.n, star.a);
    connect(l2.n, star.b);
    connect(l3.n, star.c);
  end Floating;
end Circuit;
";

#[test]
fn an_earthed_star_with_a_reset_state_reduces_and_balances_its_currents() {
    assert_balanced("Circuit.Star");
}

/// The star point's own connection set writes `-l1.i - n.i - l2.i - l3.i = 0`
/// with the unconnected `n.i` pinned to zero; the reducer reads a state out of
/// that negated sum directly.
#[test]
fn a_floating_star_point_with_a_reset_state_reduces_and_balances_its_currents() {
    assert_balanced("Circuit.Floating");
}

fn assert_balanced(model: &str) {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(EARTHED_STAR, "Circuit.mo")
        .expect("the circuit is well formed");
    let result = simulate_dae_with_diagnostics(
        &compiled.dae,
        &SimOptions {
            solver_mode: SimSolverMode::Bdf,
            t_end: 0.1,
            dt: Some(1e-3),
            ..SimOptions::default()
        },
    )
    .expect("the model simulates");
    let last = |name: &str| {
        let index = result
            .names
            .iter()
            .position(|candidate| candidate == name)
            .expect("the result records the column");
        *result.data[index].last().expect("a sample")
    };
    let balance = last("l1.i") + last("l2.i") + last("l3.i");
    assert!(balance.abs() < 1e-9, "{model}: currents sum to {balance}");
    assert!(last("l1.i").abs() > 1e-3, "the phases carry current");
}

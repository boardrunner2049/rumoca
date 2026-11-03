//! SPEC_0044 ME-EVENT-002: packaged FMI 2 and 3 components locate and apply
//! state events in Model Exchange and Co-Simulation.

use super::*;

const SOURCE: &str = r#"
model RelationSwitch
  Real x(start = 1.0, fixed = true);
  Boolean low;
equation
  der(x) = if x > 0.5 then -1.0 else -0.2;
  low = x < 0.3;
end RelationSwitch;

model BouncingBall
  parameter Real e = 0.8;
  parameter Real g = 9.81;
  Real h(start = 1.0, fixed = true);
  Real v(start = 0.0, fixed = true);
equation
  der(h) = v;
  der(v) = -g;
  when h < 0 then
    reinit(v, -e * pre(v));
  end when;
end BouncingBall;
"#;

/// The relation switch is piecewise linear, so both integrators reproduce it
/// to roundoff; the ball's first bounce and rebound speed are analytic.
const DRIVER: &str = r#"
import math, sys
from fmpy import simulate_fmu
archive, model = sys.argv[1], sys.argv[2]
for interface in ["ModelExchange", "CoSimulation"]:
    if model == "RelationSwitch":
        r = simulate_fmu(archive, fmi_type=interface, stop_time=3.0, output_interval=0.25, output=["x", "low"])
        for t, x, low in r:
            expected = 1.0 - t if t <= 0.5 else 0.5 - 0.2 * (t - 0.5)
            assert abs(x - expected) < 1e-9, (interface, t, x, expected)
            if t > 1.5 + 1e-9:
                assert low, (interface, t, low)
            if t < 1.5 - 1e-9:
                assert not low, (interface, t, low)
    else:
        r = simulate_fmu(archive, fmi_type=interface, stop_time=0.8, output_interval=0.01, output=["h", "v"])
        bounce = math.sqrt(2.0 / 9.81)
        after = [(t, h, v) for t, h, v in r if t > bounce + 0.02]
        assert after, interface
        t, h, v = after[0]
        v_expected = 0.8 * 9.81 * bounce - 9.81 * (t - bounce)
        assert abs(v - v_expected) < 1e-3, (interface, t, v, v_expected)
        assert min(h for _, h, _ in r) > -1e-6, interface
"#;

#[test]
fn packaged_fmi_state_events_match_the_analytic_traces() {
    if !conformance_prerequisites_are_available() {
        return;
    }
    assert_pinned_fmpy();
    let standards = standard_roots();
    let work = tempdir().expect("state-event FMI work directory");
    let driver = work.path().join("state_events.py");
    fs::write(&driver, DRIVER).expect("write the state-event driver");
    for model in ["RelationSwitch", "BouncingBall"] {
        let compiled = rumoca::Compiler::new()
            .model(model)
            .compile_str(SOURCE, "StateEvents.mo")
            .expect("compile the state-event model");
        for (target, standard) in [("fmi2", &standards.0), ("fmi3", &standards.1)] {
            let fmu = build_named_fmu(&work.path().join(model), &compiled, target, model);
            validate_source_package(&fmu, standard);
            checked_output(
                Command::new("python3")
                    .arg(&driver)
                    .arg(&fmu.archive)
                    .arg(model),
                &format!("{target} {model} state-event traces"),
            );
        }
    }
}

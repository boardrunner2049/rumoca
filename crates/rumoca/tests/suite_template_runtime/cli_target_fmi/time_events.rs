//! SPEC_0044 ME-EVENT-002: the generated FMI 3 C component schedules time
//! events (static instants, dynamic deadlines), holds relation memory across
//! events without any continuously searched root, runs the `initial()` event,
//! treats a changed discrete input as an event, and checks a continuously
//! changing assertion at every settled evaluation. After a time event every
//! read at its instant takes the right limit; the `when` bodies active at one
//! instant settle together, also when a state root coincides with a time
//! event; a body that moves its own deadline still runs whole; and an input
//! written in Continuous-Time Mode across a relation requests a step event.
//!
//! A C harness compiled against the packaged `model.c` and the vendored FMI 3
//! headers drives both interfaces directly; it needs only a C compiler, not
//! the standards-conformance configuration.

use super::*;

const SOURCE: &str = r#"
model TimeSwitch
  Real x(start = 0.0, fixed = true);
  Boolean late;
equation
  der(x) = if time < 0.5 then 1.0 else -1.0;
  late = time >= 0.25;
end TimeSwitch;

model Ticker
  discrete Real next(start = 0.1, fixed = true);
  Integer count(start = 0, fixed = true);
  Integer started(start = 0, fixed = true);
equation
  when initial() then
    started = 1;
  end when;
  when time >= pre(next) then
    next = pre(next) + 0.1;
    count = pre(count) + 1;
  end when;
end Ticker;

function valid
  input Real value;
  output Boolean accepted;
algorithm
  accepted := value > -1;
end valid;

model ChangingAssertion
  Real x(start = 0.0, fixed = true);
equation
  der(x) = 1.0;
  assert(valid(1.0 - x), "changing predicate");
end ChangingAssertion;

model InputLatch
  input Integer request(start = 0);
  Integer seen(start = 0, fixed = true);
equation
  when change(request) then
    seen = pre(seen) + 1;
  end when;
end InputLatch;

model InputRelation
  input Real gap(start = 1.0);
  Boolean contact;
  Integer hits(start = 0, fixed = true);
equation
  contact = gap <= 0;
  when contact then
    hits = pre(hits) + 1;
  end when;
end InputRelation;

model StrictTime
  Real x(start = 0.0, fixed = true);
  Real y;
  Integer n(start = 0, fixed = true);
equation
  y = if time > 0.5 then 1 else 0;
  der(x) = if time > 0.5 then 1.0 else -1.0;
  when time > 0.5 then
    n = pre(n) + 1;
  end when;
end StrictTime;

model SameInstant
  Real x(start = 0.0, fixed = true);
  discrete Real a(start = 0.0, fixed = true);
  discrete Real b(start = 0.0, fixed = true);
equation
  der(x) = 1.0;
  when time >= 0.3 then
    b = 1.0;
  end when;
  when x > 0.3 - 3e-15 then
    a = b + 10.0;
  end when;
end SameInstant;

model Deadline
  discrete Real T(start = 0.3, fixed = true);
  Integer n(start = 0, fixed = true);
  Real x(start = 0.0, fixed = true);
equation
  der(x) = 1.0;
  when time >= T then
    T = pre(T) + 0.25;
    n = pre(n) + 1;
    reinit(x, 0.0);
  end when;
end Deadline;
"#;

/// The harness prints `name time value` lines. `X`, `Y`, `N`, `A`, `B`,
/// `LATE`, `COUNT`, `STARTED`, `REQUEST`, `SEEN`, `GAP`, `CONTACT` and `HITS`
/// are value references the test passes as defines; `MODEL` selects the
/// scenario.
const HARNESS: &str = r#"#include "model.c"
#include <stdio.h>

static void quiet(fmi3InstanceEnvironment e, fmi3Status s, fmi3String c, fmi3String m) { (void)e; (void)s; (void)c; fprintf(stderr, "%s\n", m); }

static double get_f(fmi3Instance i, fmi3ValueReference vr) { fmi3Float64 v = NAN; if (fmi3GetFloat64(i, &vr, 1, &v, 1) != fmi3OK) exit(4); return v; }
static int get_i(fmi3Instance i, fmi3ValueReference vr) { fmi3Int32 v = -1; if (fmi3GetInt32(i, &vr, 1, &v, 1) != fmi3OK) exit(4); return v; }
static int get_b(fmi3Instance i, fmi3ValueReference vr) { fmi3Boolean v = 0; if (fmi3GetBoolean(i, &vr, 1, &v, 1) != fmi3OK) exit(4); return v; }

static fmi3Instance cs(void) {
    fmi3Instance i = fmi3InstantiateCoSimulation("h", TOKEN, NULL, fmi3False, fmi3False, fmi3False, fmi3False, NULL, 0, NULL, quiet, NULL);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 2.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) exit(2);
    return i;
}
static void step(fmi3Instance i, double t, double h) {
    fmi3Boolean ev, term, early; fmi3Float64 last;
    if (fmi3DoStep(i, t, h, fmi3True, &ev, &term, &early, &last) != fmi3OK) exit(3);
}

/* Model Exchange: forward Euler is exact for the piecewise-constant
   derivative of TimeSwitch; the importer stops at every announced time. */
static double me_update(fmi3Instance i) {
    fmi3Boolean need = fmi3True, term, nominals, values, defined = fmi3False; fmi3Float64 next = 0.0;
    while (need) if (fmi3UpdateDiscreteStates(i, &need, &term, &nominals, &values, &defined, &next) != fmi3OK) exit(5);
    printf("next %.17g\n", defined ? next : INFINITY);
    return defined ? next : INFINITY;
}

int main(void) {
#if MODEL == 0
    fmi3Instance i = cs();
    for (int k = 0; k < 3; ++k) { step(i, 0.3 * k, 0.3); printf("cs %.17g x %.17g late %d\n", 0.3 * (k + 1), get_f(i, X), get_b(i, LATE)); }
    fmi3FreeInstance(i);
    i = fmi3InstantiateModelExchange("h", TOKEN, NULL, fmi3False, fmi3False, NULL, quiet);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    double t = 0.0, next = me_update(i), x = 0.0;
    if (fmi3EnterContinuousTimeMode(i) != fmi3OK) return 6;
    while (t < 1.0) {
        double h = fmin(0.01, fmin(next, 1.0) - t), dx;
        if (fmi3GetContinuousStateDerivatives(i, &dx, 1) != fmi3OK) return 7;
        t = fabs(t + h - next) < 1e-12 ? next : t + h; x += h * dx;
        fmi3Boolean enter, term;
        if (fmi3SetTime(i, t) != fmi3OK || fmi3SetContinuousStates(i, &x, 1) != fmi3OK || fmi3CompletedIntegratorStep(i, fmi3True, &enter, &term) != fmi3OK) return 8;
        if (t == next) {
            if (fmi3EnterEventMode(i) != fmi3OK) return 9;
            next = me_update(i);
            if (fmi3EnterContinuousTimeMode(i) != fmi3OK) return 10;
            printf("me %.17g x %.17g late %d\n", t, get_f(i, X), get_b(i, LATE));
        }
    }
    printf("me %.17g x %.17g late %d\n", t, get_f(i, X), get_b(i, LATE));
#elif MODEL == 1
    fmi3Instance i = cs();
    printf("cs 0 count %d started %d\n", get_i(i, COUNT), get_i(i, STARTED));
    step(i, 0.0, 0.35);
    printf("cs 0.35 count %d started %d\n", get_i(i, COUNT), get_i(i, STARTED));
    step(i, 0.35, 0.6);
    printf("cs 0.95 count %d started %d\n", get_i(i, COUNT), get_i(i, STARTED));
#elif MODEL == 3
    fmi3Instance i = cs();
    step(i, 0.0, 1.5);
    printf("cs 1.5 x %.17g\n", get_f(i, X));
    fmi3Boolean ev, term, early; fmi3Float64 last;
    printf("cs 3 status %d\n", (int)fmi3DoStep(i, 1.5, 1.5, fmi3True, &ev, &term, &early, &last));
#elif MODEL == 4
    /* A relation on an input with relation memory and no searched root: each
       crossing is a host event whose when-edge reads pre(contact). */
    fmi3ValueReference vr = GAP;
    const double gaps[4] = { -1.0, -2.0, 1.0, -0.5 };
    fmi3Instance i = cs();
    for (int k = 0; k < 4; ++k) {
        if (fmi3SetFloat64(i, &vr, 1, &gaps[k], 1) != fmi3OK) return 12;
        step(i, 0.1 * k, 0.1);
        printf("cs %d contact %d hits %d\n", k, get_b(i, CONTACT), get_i(i, HITS));
    }
    fmi3FreeInstance(i);
    i = fmi3InstantiateModelExchange("h", TOKEN, NULL, fmi3False, fmi3False, NULL, quiet);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    me_update(i);
    for (int k = 0; k < 4; ++k) {
        if (fmi3SetFloat64(i, &vr, 1, &gaps[k], 1) != fmi3OK) return 12;
        me_update(i);
        printf("me %d contact %d hits %d\n", k, get_b(i, CONTACT), get_i(i, HITS));
    }
    /* No indicator watches the input, so a write in Continuous-Time Mode
       that crosses the relation is a step event at the completed step. */
    const double away = 2.0;
    fmi3Boolean enter = fmi3False, term = fmi3False;
    if (fmi3EnterContinuousTimeMode(i) != fmi3OK || fmi3SetTime(i, 0.1) != fmi3OK || fmi3CompletedIntegratorStep(i, fmi3True, &enter, &term) != fmi3OK) return 13;
    printf("me ct quiet enter %d\n", (int)enter);
    if (fmi3SetFloat64(i, &vr, 1, &away, 1) != fmi3OK || fmi3SetTime(i, 0.2) != fmi3OK || fmi3CompletedIntegratorStep(i, fmi3True, &enter, &term) != fmi3OK) return 13;
    printf("me ct crossed enter %d contact %d\n", (int)enter, get_b(i, CONTACT));
    if (fmi3EnterEventMode(i) != fmi3OK) return 13;
    me_update(i);
    printf("me ct event contact %d hits %d\n", get_b(i, CONTACT), get_i(i, HITS));
#elif MODEL == 5
    /* Reads at a time event's instant after Event Mode take its right
       limit: y, der(x), and n agree there. */
    fmi3Instance i = cs();
    for (int k = 0; k < 4; ++k) { step(i, 0.25 * k, 0.25); printf("cs %.17g x %.17g y %.17g n %d\n", 0.25 * (k + 1), get_f(i, X), get_f(i, Y), get_i(i, N)); }
    fmi3FreeInstance(i);
    i = fmi3InstantiateModelExchange("h", TOKEN, NULL, fmi3False, fmi3False, NULL, quiet);
    if (!i || fmi3EnterInitializationMode(i, fmi3True, 1e-10, 0.0, fmi3True, 1.0) != fmi3OK || fmi3ExitInitializationMode(i) != fmi3OK) return 2;
    double t = 0.0, next = me_update(i), x = 0.0, dx = 0.0;
    if (fmi3EnterContinuousTimeMode(i) != fmi3OK) return 6;
    while (t < 1.0) {
        double h = fmin(0.01, fmin(next, 1.0) - t);
        if (fmi3GetContinuousStateDerivatives(i, &dx, 1) != fmi3OK) return 7;
        t = fabs(t + h - next) < 1e-12 ? next : t + h; x += h * dx;
        fmi3Boolean enter, term;
        if (fmi3SetTime(i, t) != fmi3OK || fmi3SetContinuousStates(i, &x, 1) != fmi3OK || fmi3CompletedIntegratorStep(i, fmi3True, &enter, &term) != fmi3OK) return 8;
        if (t == next) {
            if (fmi3GetContinuousStateDerivatives(i, &dx, 1) != fmi3OK) return 7;
            printf("me before %.17g der %.17g y %.17g\n", t, dx, get_f(i, Y));
            if (fmi3EnterEventMode(i) != fmi3OK) return 9;
            next = me_update(i);
            if (fmi3EnterContinuousTimeMode(i) != fmi3OK || fmi3GetContinuousStateDerivatives(i, &dx, 1) != fmi3OK) return 10;
            printf("me after %.17g der %.17g y %.17g n %d\n", t, dx, get_f(i, Y), get_i(i, N));
        }
    }
    printf("me end %.17g x %.17g\n", t, x);
#elif MODEL == 6
    fmi3Instance i = cs();
    for (int k = 0; k < 4; ++k) { step(i, 0.1 * k, 0.1); printf("cs %d a %.17g b %.17g\n", k + 1, get_f(i, A), get_f(i, B)); }
#elif MODEL == 7
    fmi3Instance i = cs();
    for (int k = 0; k < 20; ++k) { step(i, 0.05 * k, 0.05); printf("cs %d n %d x %.17g\n", k + 1, get_i(i, N), get_f(i, X)); }
#else
    fmi3Instance i = cs();
    step(i, 0.0, 0.1);
    printf("cs 0.1 seen %d\n", get_i(i, SEEN));
    fmi3ValueReference vr = REQUEST; fmi3Int32 one = 1;
    if (fmi3SetInt32(i, &vr, 1, &one, 1) != fmi3OK) return 11;
    step(i, 0.1, 0.1);
    printf("cs 0.2 seen %d\n", get_i(i, SEEN));
    step(i, 0.2, 0.1);
    printf("cs 0.3 seen %d\n", get_i(i, SEEN));
#endif
    return 0;
}
"#;

#[test]
fn generated_c_component_executes_time_events_and_input_events() {
    if !harness_tools_available() {
        return;
    }
    let time_switch = run_harness("TimeSwitch", 0);
    for (time, x) in [
        ("0.29999999999999999", 0.3),
        ("0.59999999999999998", 0.4),
        ("0.89999999999999991", 0.1),
    ] {
        let line = find_line(&time_switch, &format!("cs {time} x "));
        assert_close(field(&line, 3), x, 1e-12, &line);
    }
    assert!(find_line(&time_switch, "cs 0.29999999999999999").ends_with("late 1"));
    // Model Exchange announces both instants, and the relations flip there.
    assert!(time_switch.contains("next 0.25\n"), "{time_switch}");
    assert!(time_switch.contains("next 0.5\n"), "{time_switch}");
    let at_quarter = find_line(&time_switch, "me 0.25 x ");
    assert_close(field(&at_quarter, 3), 0.25, 1e-12, &at_quarter);
    assert!(at_quarter.ends_with("late 1"), "{at_quarter}");
    let end = find_line(&time_switch, "me 1 x ");
    assert_close(field(&end, 3), 0.0, 1e-9, &end);

    let ticker = run_harness("Ticker", 1);
    assert!(ticker.contains("cs 0 count 0 started 1"), "{ticker}");
    assert!(ticker.contains("cs 0.35 count 3 started 1"), "{ticker}");
    assert!(ticker.contains("cs 0.95 count 9 started 1"), "{ticker}");

    // A predicate without relations changes continuously: the component
    // checks it at every settled evaluation and fails the step past x = 2.
    let assertion = run_harness("ChangingAssertion", 3);
    let before = find_line(&assertion, "cs 1.5 x ");
    assert_close(field(&before, 3), 1.5, 1e-12, &before);
    assert!(assertion.contains("cs 3 status 3"), "{assertion}");

    let latch = run_harness("InputLatch", 2);
    assert!(latch.contains("cs 0.1 seen 0"), "{latch}");
    assert!(latch.contains("cs 0.2 seen 1"), "{latch}");
    assert!(latch.contains("cs 0.3 seen 1"), "{latch}");

    // Relation memory on an input without any searched root: the when-edge
    // fires on each new crossing only, in both interfaces.
    let relation = run_harness("InputRelation", 4);
    for interface in ["cs", "me"] {
        for (k, contact, hits) in [(0, 1, 1), (1, 1, 1), (2, 0, 1), (3, 1, 2)] {
            let line = format!("{interface} {k} contact {contact} hits {hits}");
            assert!(relation.contains(&line), "missing `{line}` in:\n{relation}");
        }
    }
    // An input written in Continuous-Time Mode: nothing until it crosses the
    // relation, then a step event whose iteration moves the relation.
    for line in [
        "me ct quiet enter 0",
        "me ct crossed enter 1 contact 1",
        "me ct event contact 0 hits 2",
    ] {
        assert!(relation.contains(line), "missing `{line}` in:\n{relation}");
    }
}

/// After a time event at 0.5, a read at 0.5 is the right limit: `time > 0.5`
/// holds there for `y` and `der(x)` as it does for the counter.
#[test]
fn reads_at_a_time_event_instant_take_its_right_limit() {
    if !harness_tools_available() {
        return;
    }
    let output = run_harness("StrictTime", 5);
    let at_event = find_line(&output, "cs 0.5 x ");
    assert!(at_event.ends_with("y 1 n 1"), "{at_event}");
    let end = find_line(&output, "cs 1 x ");
    assert_close(field(&end, 3), 0.0, 1e-9, &end);
    let before = find_line(&output, "me before 0.5 ");
    assert_close(field(&before, 4), -1.0, 0.0, &before);
    let after = find_line(&output, "me after 0.5 ");
    assert!(after.ends_with("der 1 y 1 n 1"), "{after}");
    let me_end = find_line(&output, "me end 1 x ");
    assert_close(field(&me_end, 4), 0.0, 1e-9, &me_end);
}

/// The `when` bodies active at one instant settle together (MLS 8.5): `a`
/// reads the `b` assigned at the same instant, also when the state root of
/// `a` lies 3e-15 s before the time event of `b`. A body that moves its own
/// deadline still runs whole at each deadline.
#[test]
fn simultaneous_when_bodies_settle_together() {
    if !harness_tools_available() {
        return;
    }
    let same = run_harness("SameInstant", 6);
    assert!(same.contains("cs 2 a 0 b 0"), "{same}");
    assert!(same.contains("cs 3 a 11 b 1"), "{same}");
    assert!(same.contains("cs 4 a 11 b 1"), "{same}");
    let deadline = run_harness("Deadline", 7);
    for (k, n, x) in [
        (5, 0, 0.25),
        (6, 1, 0.0),
        (7, 1, 0.05),
        (12, 2, 0.05),
        (20, 3, 0.2),
    ] {
        let line = find_line(&deadline, &format!("cs {k} n "));
        assert_eq!(field(&line, 3), f64::from(n), "{line}");
        assert_close(field(&line, 5), x, 1e-9, &line);
    }
}

fn harness_tools_available() -> bool {
    let tools = [("cc", command_is_available("cc"))];
    super::super::template_runtime_policy::prerequisites_are_available(
        "FMI C time-event harness",
        &tools,
    )
}

fn find_line(output: &str, prefix: &str) -> String {
    output
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starting with `{prefix}` in:\n{output}"))
        .to_string()
}

fn field(line: &str, index: usize) -> f64 {
    line.split_whitespace()
        .nth(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("field {index} of `{line}`"))
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, context: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: expected {expected}, got {actual}"
    );
}

/// Package `model`, compile the harness scenario against it, and return its
/// standard output.
fn run_harness(model: &str, scenario: u32) -> String {
    let compiled = rumoca::Compiler::new()
        .model(model)
        .compile_str(SOURCE, "TimeEvents.mo")
        .unwrap_or_else(|error| panic!("compile {model}: {error:?}"));
    let work = tempdir().expect("time-event harness work directory");
    let fmu = build_named_fmu(work.path(), &compiled, "fmi3", model);
    let sources = fmu.root.join("sources");
    let description = fs::read_to_string(fmu.root.join("modelDescription.xml"))
        .expect("read the packaged model description");
    let token = attribute(&description, "instantiationToken");
    let mut command = Command::new("cc");
    command
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-Wno-unused-function")
        .arg("-DFMI3_OVERRIDE_FUNCTION_PREFIX=1")
        .arg(format!("-DMODEL={scenario}"))
        .arg(format!("-DTOKEN=\"{token}\""));
    for (define, name) in [
        ("X", "x"),
        ("Y", "y"),
        ("N", "n"),
        ("A", "a"),
        ("B", "b"),
        ("LATE", "late"),
        ("COUNT", "count"),
        ("STARTED", "started"),
        ("REQUEST", "request"),
        ("SEEN", "seen"),
        ("GAP", "gap"),
        ("CONTACT", "contact"),
        ("HITS", "hits"),
    ] {
        let reference = value_reference(&description, name).unwrap_or(0);
        command.arg(format!("-D{define}={reference}"));
    }
    let harness = work.path().join("time_events_harness.c");
    fs::write(&harness, HARNESS).expect("write the time-event harness");
    let binary = work.path().join("time_events_harness");
    checked_output(
        command
            .arg(format!("-I{}", sources.display()))
            .arg(format!("-I{}", vendored_fmi3_headers().display()))
            .arg(&harness)
            .args(
                [
                    "rmc_assign.c",
                    "rmc_rows.c",
                    "rmc_jacobian.c",
                    "rmc_isolators.c",
                    "rmc_functions.c",
                ]
                .map(|unit| sources.join(unit)),
            )
            .arg("-lm")
            .arg("-o")
            .arg(&binary),
        &format!("compile the {model} time-event harness"),
    );
    let output = checked_output(
        &mut Command::new(&binary),
        &format!("run the {model} time-event harness"),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn vendored_fmi3_headers() -> PathBuf {
    workspace_root().join("crates/rumoca-phase-codegen/src/templates/fmi-ls-wasm/fmi3-headers")
}

fn attribute(xml: &str, name: &str) -> String {
    let start = xml
        .find(&format!("{name}=\""))
        .unwrap_or_else(|| panic!("model description has no {name}"))
        + name.len()
        + 2;
    xml[start..start + xml[start..].find('"').expect("terminated attribute")].to_string()
}

fn value_reference(xml: &str, variable: &str) -> Option<u32> {
    let at = xml.find(&format!(" name=\"{variable}\""))?;
    let tail = &xml[at..];
    let start = tail.find("valueReference=\"")? + "valueReference=\"".len();
    tail[start..start + tail[start..].find('"')?].parse().ok()
}

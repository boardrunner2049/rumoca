//! Connection and balance regressions found compiling real libraries
//! (AixLib, Buildings, IBPSA, IDEAS, VehicleInterfaces, ThermoPower, ...).
//!
//! Each test is the minimal form of a model OpenModelica checks and this
//! compiler used to refuse; the TOOLBUG named in each comment has the story.

use rumoca_compile::compile::FailedPhase;
use rumoca_contracts::test_support::{expect_balanced, expect_failure_in_phase_with_code};

// TOOLBUG-120: a discrete external input connected to a nested discrete
// input. The connection equation defines the nested input from the
// environment's value; it was neither counted as a discrete definition nor
// accepted as a continuous row (`ED001` / `ED020`).
#[test]
fn boolean_top_level_input_drives_nested_boolean_input() {
    expect_balanced(
        r#"
        model M
            connector BI = input Boolean;
            connector RO = output Real;
            block Timer
                BI u;
                RO y;
            protected
                discrete Real entryTime;
            equation
                when u then
                    entryTime = time;
                end when;
                y = if u then time - entryTime else 0.0;
            end Timer;
            BI u;
            RO y;
            Timer tim;
        equation
            connect(u, tim.u);
            connect(tim.y, y);
        end M;
    "#,
        "M",
    );
}

#[test]
fn integer_top_level_input_drives_nested_integer_input() {
    expect_balanced(
        r#"
        model M
            connector II = input Integer;
            block B
                II k;
                Integer j;
            equation
                j = 2 * k;
            end B;
            II k;
            B b;
        equation
            connect(k, b.k);
        end M;
    "#,
        "M",
    );
}

// TOOLBUG-121: expandable-connector member source counting.
const BUS_LIBRARY: &str = r#"
    connector RI = input Real;
    connector RO = output Real;
    expandable connector Bus
    end Bus;
    block Src
        RO y;
    equation
        y = 1;
    end Src;
    block Wrap
        RO iceFac;
        Src s;
    equation
        connect(s.y, iceFac);
    end Wrap;
    block Gain
        RI u;
        RO y;
    equation
        y = 2 * u;
    end Gain;
"#;

fn bus_package(model: &str) -> String {
    format!("package P\n{BUS_LIBRARY}\n{model}\nend P;\n")
}

/// An output forwarded from an inner block (`connect(s.y, iceFac)` inside
/// `Wrap`) is one source, however many connects name it. Counting connect
/// occurrences reported this member as driven five times.
#[test]
fn forwarded_output_on_a_bus_member_is_one_source() {
    expect_balanced(
        &bus_package(
            r#"
            model M
                Bus sigBus;
                Wrap w;
                RO out;
            equation
                connect(w.iceFac, sigBus.iceFacMea);
                connect(w.iceFac, out);
            end M;
            "#,
        ),
        "P.M",
    );
}

/// A member of the model's own expandable connector that is only read is an
/// input of the model (MLS §9.1.3: it takes the causality of the input it is
/// connected to), supplied by whoever connects the bus.
#[test]
fn read_only_member_of_a_top_level_bus_is_a_model_input() {
    expect_balanced(
        &bus_package(
            r#"
            model M
                Bus sigBus;
                Gain g;
                RO y;
            equation
                connect(g.u, sigBus.onOffMea);
                connect(g.y, y);
            end M;
            "#,
        ),
        "P.M",
    );
}

/// Two distinct outputs on one member remain a conflict.
#[test]
fn two_outputs_on_one_bus_member_are_rejected() {
    expect_failure_in_phase_with_code(
        &bus_package(
            r#"
            model M
                Bus bus;
                Src a;
                Src b;
            equation
                connect(a.y, bus.x);
                connect(b.y, bus.x);
            end M;
            "#,
        ),
        "P.M",
        FailedPhase::Flatten,
        "EF037",
    );
}

// TOOLBUG-122: a component-array dimension taken from another array's size.
// `size(tp, 1)` did not evaluate during instantiation, so `sub[n]` was
// instantiated as a scalar and the vector connection failed (`EF002`,
// `dims: [5]` vs `dims: []`).
#[test]
fn component_array_sized_by_size_of_parameter_array() {
    expect_balanced(
        r#"
        package P
            connector RI = input Real;
            connector RO = output Real;
            block Rep
                parameter Integer n = 1;
                RI u;
                RO y[n];
            equation
                y = fill(u, n);
            end Rep;
            block Sub
                RI u1;
                RI u2;
                RO y;
            equation
                y = u1 - u2;
            end Sub;
            model M
                parameter Real tp[5] = {1, 2, 3, 4, 5};
                RI u;
                RI v[n];
                Rep rep(n = n);
                Sub sub[n];
                RO y[n];
            protected
                parameter Integer n = size(tp, 1);
            equation
                connect(u, rep.u);
                connect(rep.y, sub.u1);
                connect(v, sub.u2);
                connect(sub.y, y);
            end M;
        end P;
        "#,
        "P.M",
    );
}

/// A `:` dimension is sized by its binding; a comprehension over an empty
/// range gives zero, and the arrays sized from it vanish (OpenModelica: 0
/// equations, 0 variables).
#[test]
fn component_array_sized_by_empty_comprehension_binding() {
    let result = expect_balanced(
        r#"
        package P
            connector RI = input Real;
            connector RO = output Real;
            block Add
                RI u1;
                RI u2;
                RO y;
            equation
                y = u1 + u2;
            end Add;
            model M
                parameter Integer nin = 0;
                parameter Integer idx[:] = {i for i in 1:nin};
                final parameter Integer m = size(idx, 1);
                RI a[m];
                Add add[m];
                RO y[m];
            equation
                connect(a, add.u1);
                connect(a, add.u2);
                connect(add.y, y);
            end M;
        end P;
        "#,
        "P.M",
    );
    assert_eq!(result.balance_detail.equations_unknowns(), (0, 0));
}

// TOOLBUG-123: a parameter declared in an expandable connector, read by a
// block input through a connect. OpenModelica generates `g.u = bus.p`.
#[test]
fn expandable_bus_parameter_feeds_block_input() {
    expect_balanced(
        r#"
        package P
            connector RI = input Real;
            connector RO = output Real;
            expandable connector Bus
                parameter Real p = 2;
            end Bus;
            block G
                RI u;
                RO y;
            equation
                y = u;
            end G;
            model M
                Bus bus;
                G g;
            equation
                connect(g.u, bus.p);
            end M;
        end P;
        "#,
        "P.M",
    );
}

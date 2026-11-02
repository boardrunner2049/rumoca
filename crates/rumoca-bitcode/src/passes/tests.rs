use super::*;
use crate::build::Builder;

/// der(x) = -(x * (2 + 3)), with a dead node left in the arena.
fn model() -> RbcModel {
    let mut b = Builder::new("Folding");
    let x = b.state("x", 1.0);
    let _dead = b.real(42.0);
    let two = b.real(2.0);
    let three = b.real(3.0);
    let sum = b.binary(RbcBinaryOp::Add, two, three);
    let xr = b.state_ref(x);
    let product = b.binary(RbcBinaryOp::Multiply, xr, sum);
    let rhs = b.negate_of(product);
    b.derivative_equation(x, rhs);
    b.finish()
}

fn file(model: RbcModel) -> RbcFile {
    RbcFile {
        execution: None,
        magic: RBC_MAGIC.into(),
        bitcode_version: RBC_VERSION,
        producer: "test".into(),
        model,
    }
}

#[test]
fn dead_expressions_are_dropped_and_references_renumbered() {
    let mut file = file(model());
    let before = file.model.expressions.len();
    let reports = run(&mut file, &["dead-expressions".into()]).expect("pass runs");
    assert_eq!(reports[0].rewrites, 1, "only the unreferenced literal goes");
    assert_eq!(file.model.expressions.len(), before - 1);
    for (index, expression) in file.model.expressions.iter().enumerate() {
        assert_eq!(expression.id.0 as usize, index, "ids stay dense");
    }
}

#[test]
fn literal_operators_fold_and_their_operands_become_dead() {
    let mut file = file(model());
    let reports = run(&mut file, &["fold-constants".into()]).expect("pass runs");
    assert_eq!(reports[0].rewrites, 1, "2 + 3 folds; x * 5 does not");
    assert!(
        file.model
            .expressions
            .iter()
            .any(|e| matches!(e.node, RbcExprNode::Literal { value: RbcLiteral::Real { value } } if value == 5.0)),
        "the sum is now the literal 5"
    );
    assert_eq!(
        reports[0].removed_expressions, 3,
        "the two operands and the dead literal are dropped"
    );
}

#[test]
fn folding_stops_where_the_runtime_would_report() {
    use RbcBinaryOp as Op;
    assert_eq!(
        constants_binary(Op::Divide, 1.0, 0.0),
        None,
        "division by zero is the runtime's to report"
    );
    let mut b = Builder::new("Overflow");
    let x = b.state("x", 1.0);
    let big = b.expr(RbcExprNode::Literal {
        value: RbcLiteral::Integer { value: i64::MAX },
    });
    let one = b.expr(RbcExprNode::Literal {
        value: RbcLiteral::Integer { value: 1 },
    });
    let sum = b.binary(Op::Add, big, one);
    let xr = b.state_ref(x);
    let rhs = b.binary(Op::Multiply, xr, sum);
    b.derivative_equation(x, rhs);
    let mut file = file(b.finish());
    let reports = run(&mut file, &["fold-constants".into()]).expect("pass runs");
    assert_eq!(reports[0].rewrites, 0, "an overflowing sum is left alone");
}

fn constants_binary(op: RbcBinaryOp, a: f64, b: f64) -> Option<f64> {
    let mut builder = Builder::new("One");
    let x = builder.state("x", 1.0);
    let lhs = builder.real(a);
    let rhs = builder.real(b);
    let node = builder.binary(op, lhs, rhs);
    let xr = builder.state_ref(x);
    let use_it = builder.binary(RbcBinaryOp::Add, xr, node);
    builder.derivative_equation(x, use_it);
    let mut model = builder.finish();
    constants::FoldConstants.run(&mut model).ok()?;
    match model.expressions[node.0 as usize].node {
        RbcExprNode::Literal {
            value: RbcLiteral::Real { value },
        } => Some(value),
        _ => None,
    }
}

#[test]
fn an_unknown_pass_is_refused_by_name() {
    let mut file = file(model());
    let error = run(&mut file, &["no-such-pass".into()]).unwrap_err();
    assert!(error.to_string().contains("unknown pass `no-such-pass`"));
}

/// der(x) = -(x * (c * p)) with `c` a constant and `p` a parameter, both
/// bound to literals.
fn constant_model() -> RbcModel {
    let mut b = Builder::new("Constants");
    let x = b.state("x", 1.0);
    let c = b.parameter("P.c", 3.0);
    let p = b.parameter("p", 2.0);
    let c_ref = b.parameter_ref(c);
    let p_ref = b.parameter_ref(p);
    let product = b.binary(RbcBinaryOp::Multiply, c_ref, p_ref);
    let xr = b.state_ref(x);
    let scaled = b.binary(RbcBinaryOp::Multiply, xr, product);
    let rhs = b.negate_of(scaled);
    b.derivative_equation(x, rhs);
    let mut model = b.finish();
    let contract = model.variables[c.0 as usize]
        .contract
        .as_mut()
        .expect("builder variables carry a contract");
    contract.variability = RbcVariability::Constant;
    model
}

#[test]
fn constants_inline_parameters_stay_tunable() {
    let mut file = file(constant_model());
    let reports = run(
        &mut file,
        &["inline-constants".into(), "fold-constants".into()],
    )
    .expect("passes run");
    assert_eq!(
        reports[0].rewrites, 1,
        "only the constant's reference is inlined"
    );
    assert!(
        file.model.expressions.iter().any(|e| matches!(
            e.node,
            RbcExprNode::Coordinate {
                coordinate: RbcCoordinate::Parameter { .. }
            }
        )),
        "the parameter is still read: it can be changed without recompiling"
    );
    assert_eq!(
        reports[1].rewrites, 0,
        "3 * p does not fold while p is a parameter"
    );
}

/// `assert(c > 1)` over a constant and `assert(p > 1)` over a parameter.
fn assert_model() -> RbcModel {
    let mut b = Builder::new("Asserts");
    let x = b.state("x", 1.0);
    let c = b.parameter("c", 3.0);
    let p = b.parameter("p", 3.0);
    let xr = b.state_ref(x);
    let rhs = b.negate_of(xr);
    b.derivative_equation(x, rhs);
    for variable in [c, p] {
        let reference = b.parameter_ref(variable);
        let one = b.real(1.0);
        let holds = b.binary(RbcBinaryOp::Greater, reference, one);
        let relation = b.relation(holds);
        let condition = b.condition(RbcConditionNode::Relation { relation });
        let fails = b.condition(RbcConditionNode::Not { operand: condition });
        let always = b.always();
        let message = b.expr(RbcExprNode::Literal {
            value: RbcLiteral::String {
                value: "must exceed 1".into(),
            },
        });
        b.event(
            always,
            fails,
            RbcAction::Assert {
                message,
                level: None,
            },
        );
    }
    let mut model = b.finish();
    model.variables[c.0 as usize]
        .contract
        .as_mut()
        .expect("builder variables carry a contract")
        .variability = RbcVariability::Constant;
    model
}

#[test]
fn an_assertion_that_folds_true_is_dropped_one_over_a_parameter_stays() {
    let mut file = file(assert_model());
    let reports = run(
        &mut file,
        &["inline-constants".into(), "fold-asserts".into()],
    )
    .expect("passes run");
    assert_eq!(
        reports[1].rewrites, 1,
        "only the constant's assertion is decided"
    );
    assert_eq!(file.model.events.len(), 1);
    assert_eq!(file.model.events[0].id.0, 0, "event ids stay dense");
    assert!(
        file.model.relations.iter().all(|relation| matches!(
            file.model.expressions[relation.expression.0 as usize].node,
            RbcExprNode::Binary { .. }
        )),
        "relation roots stay relational"
    );
}

#[test]
fn an_assertion_over_a_structural_parameter_folds_without_inlining_it() {
    let mut model = assert_model();
    // `p` becomes `annotation(Evaluate = true)`: frozen for translation, but
    // still a parameter every backend may read as one.
    model.variables[2]
        .contract
        .as_mut()
        .expect("builder variables carry a contract")
        .structural = true;
    let mut file = file(model);
    let reports = run(
        &mut file,
        &["inline-constants".into(), "fold-asserts".into()],
    )
    .expect("passes run");
    assert_eq!(reports[0].rewrites, 1, "only the constant is inlined");
    assert_eq!(reports[1].rewrites, 2, "both assertions are decided");
    assert!(file.model.events.is_empty());
}

#[test]
fn a_pipeline_parses_groups_lists_and_fixpoints() {
    let steps =
        pipeline::parse(&["inline-constants,fixpoint(fold-constants, fold-pure-calls)".into()])
            .expect("parses");
    assert_eq!(
        format!("{steps:?}"),
        "[inline-constants, fixpoint([fold-constants, fold-pure-calls])]"
    );
    let default = pipeline::parse(&["O1".into()]).expect("parses");
    assert_eq!(default.len(), catalog().len());
    assert!(
        pipeline::parse(&["O0,none,round-trip".into()])
            .expect("parses")
            .is_empty()
    );
    let external =
        pipeline::parse(&["fold-constants,exec:tool --keep a,b".into()]).expect("parses");
    assert_eq!(
        format!("{external:?}"),
        "[fold-constants, exec:tool --keep a,b]"
    );
    assert!(pipeline::parse(&["fixpoint(fold-constants".into()]).is_err());
    assert!(pipeline::parse(&["exec:".into()]).is_err());
}

#[test]
fn a_pass_that_changed_nothing_is_not_rerun_on_the_same_model() {
    let mut file = file(model());
    let steps = pipeline::parse(&["fold-constants,fold-constants,fold-constants".into()]).unwrap();
    let mut scheduler = pipeline::Scheduler::default();
    scheduler.run(&mut file, &steps).expect("runs");
    // The first run folds `2 + 3`; the second finds nothing; the third is
    // skipped because nothing changed in between.
    assert_eq!(scheduler.reports.len(), 2);
    assert!(scheduler.reports[0].changed());
    assert!(!scheduler.reports[1].changed());
    assert_eq!(scheduler.skipped, 1);
}

#[test]
fn a_fixpoint_stops_once_a_round_changes_nothing() {
    let mut file = file(model());
    let steps = pipeline::parse(&["fixpoint(fold-constants,dead-expressions)".into()]).unwrap();
    let mut scheduler = pipeline::Scheduler::default();
    scheduler.run(&mut file, &steps).expect("converges");
    // Round one: folding changes the model (the dead operands go with it),
    // then `dead-expressions` finds nothing. Round two: folding reruns on the
    // changed model and finds nothing; `dead-expressions` is skipped, since
    // nothing changed after it last ran. There is no round three.
    assert_eq!(scheduler.reports.len(), 3);
    assert!(scheduler.reports[0].changed());
    assert!(
        scheduler.reports[1..]
            .iter()
            .all(|report| !report.changed())
    );
    assert_eq!(scheduler.skipped, 1);
}

#[cfg(unix)]
fn script(directory: &std::path::Path, name: &str, body: &str) -> String {
    let path = directory.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    format!("sh {}", path.display())
}

#[cfg(unix)]
#[test]
fn an_external_pass_runs_in_the_pipeline_and_its_output_is_validated() {
    let directory = tempfile::tempdir().unwrap();
    let copy = script(directory.path(), "copy.sh", r#"cp "$1" "$3""#);
    let mut file = file(model());
    let reports = run(&mut file, &[format!("fold-constants,exec:{copy}")]).expect("runs");
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[1].pass, format!("exec:{copy}"));
    assert!(!reports[1].changed(), "a copy changes nothing");

    let fails = script(directory.path(), "fails.sh", "echo broken >&2; exit 3");
    let error = run(&mut file, &[format!("exec:{fails}")]).unwrap_err();
    assert!(error.to_string().contains("exited with"), "{error}");

    let silent = script(directory.path(), "silent.sh", "true");
    let error = run(&mut file, &[format!("exec:{silent}")]).unwrap_err();
    assert!(
        error.to_string().contains("wrote no output artifact"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn an_external_pass_that_writes_another_version_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let mut future = file(model());
    future.bitcode_version = RBC_VERSION + 1;
    let path = directory.path().join("future.rbc");
    std::fs::write(&path, serde_json::to_vec(&future).unwrap()).unwrap();
    let writes_future = script(
        directory.path(),
        "future.sh",
        &format!(r#"cp "{}" "$3""#, path.display()),
    );
    let mut file = file(model());
    let error = run(&mut file, &[format!("exec:{writes_future}")]).unwrap_err();
    assert!(error.to_string().contains("version"), "{error}");
}

#[test]
fn trace_points_carry_onto_a_fresh_export_by_variable_name() {
    use crate::observations::carry_observations;
    use crate::schema::{RbcTracePoint, TracePointId};
    let mut traced = file(model());
    let x = traced.model.variables[0].id;
    traced.model.trace_points.push(RbcTracePoint {
        id: TracePointId(0),
        variable: x,
        label: "x".into(),
        connection: None,
        connection_set: None,
        quantity: None,
        unit: None,
        added_by: None,
    });
    let mut fresh = file(model());
    carry_observations(&traced, &mut fresh).expect("carries");
    assert_eq!(fresh.model.trace_points.len(), 1);
    assert_eq!(fresh.model.summary.trace_points, 1);
    assert_eq!(fresh.model.trace_points[0].variable, x);

    // A variable the rebuild no longer has is named, not silently dropped.
    let mut renamed = file(model());
    renamed.model.variables[0].name = "y".into();
    let error = carry_observations(&traced, &mut renamed).unwrap_err();
    assert!(error.to_string().contains("`x`"), "{error}");
}

/// `parameter k = 2; parameter d = k * 10; der(x) = -x / d`.
fn derived_parameter_model() -> (RbcModel, VariableId) {
    let mut b = Builder::new("Derived");
    let x = b.state("x", 1.0);
    let k = b.parameter("k", 2.0);
    let d = b.parameter("d", 0.0);
    let k_ref = b.parameter_ref(k);
    let ten = b.real(10.0);
    let product = b.binary(RbcBinaryOp::Multiply, k_ref, ten);
    let xr = b.state_ref(x);
    let d_ref = b.parameter_ref(d);
    let quotient = b.binary(RbcBinaryOp::Divide, xr, d_ref);
    let rhs = b.negate_of(quotient);
    b.derivative_equation(x, rhs);
    let mut model = b.finish();
    model.variables[d.0 as usize].binding = Some(product);
    (model, d)
}

#[test]
fn a_derived_parameter_binding_folds_to_its_value() {
    let (model, d) = derived_parameter_model();
    let mut file = file(model);
    let reports = run(&mut file, &["fold-parameter-bindings".into()]).expect("pass runs");
    assert_eq!(reports[0].rewrites, 1, "only `d` is derived");
    let binding = file.model.variables[d.0 as usize]
        .binding
        .expect("the binding stays");
    assert!(matches!(
        file.model.expressions[binding.0 as usize].node,
        RbcExprNode::Literal {
            value: RbcLiteral::Real { value }
        } if value == 20.0
    ));
    assert!(
        file.model.expressions.iter().any(|e| matches!(
            e.node,
            RbcExprNode::Coordinate {
                coordinate: RbcCoordinate::Parameter { variable }
            } if variable == d
        )),
        "the equation still reads `d`: only its binding is folded"
    );
}

#[test]
fn a_modifier_binding_and_an_initialization_parameter_are_not_folded() {
    for edit in [
        (|variable: &mut RbcVariable| {
            variable
                .contract
                .as_mut()
                .expect("builder variables carry a contract")
                .binding_from_modification = true;
        }) as fn(&mut RbcVariable),
        |variable: &mut RbcVariable| variable.fixed = Some(false),
    ] {
        let (mut model, d) = derived_parameter_model();
        edit(&mut model.variables[d.0 as usize]);
        let mut file = file(model);
        let reports = run(&mut file, &["fold-parameter-bindings".into()]).expect("pass runs");
        assert_eq!(reports[0].rewrites, 0);
    }
}

#[test]
fn a_binding_that_reads_a_tunable_parameter_is_not_folded() {
    let (mut model, _) = derived_parameter_model();
    let k = model
        .variables
        .iter_mut()
        .find(|variable| variable.name == "k")
        .expect("k is declared");
    k.tunable = true;
    let mut file = file(model);
    let reports = run(&mut file, &["fold-parameter-bindings".into()]).expect("pass runs");
    assert_eq!(reports[0].rewrites, 0, "`k` may change on the artifact");
}

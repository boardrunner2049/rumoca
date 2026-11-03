//! MLS §§11.2.2, 11.2.8.1; SOLVE-C39/C51/C52: assertions read the
//! iteration's exact definitions and retain failures across compact folds.

use rumoca::Compiler;
use rumoca_eval_solve::{EventActionRequest, RowEvalContext, TypedValue};
use rumoca_ir_solve::{SolveOperation, SolveUnaryOperator, SolveValueKind, TypedProgram};
use rumoca_phase_solve::LoweredSolvePackage;

fn compile(functions: &str) -> LoweredSolvePackage {
    let source = format!(
        "{functions}\nmodel Probe\n Real x(start=0, fixed=true);\n equation der(x)=checked(time);\nend Probe;"
    );
    let compiled = Compiler::new()
        .model("Probe")
        .compile_str(&source, "FunctionLoopAssertions.mo")
        .expect("loop assertions must construct DAE");
    rumoca_phase_solve::lower_solve_package(&compiled.dae)
        .expect("loop assertions must lower to typed Solve owners")
}

fn evaluate(package: &LoweredSolvePackage, input: f64, value: f64, predicates: &[bool]) {
    let owner = package.pure_calls.owners().last().unwrap();
    let argument = TypedValue::construct(
        owner.inputs()[0].clone(),
        vec![SolveValueKind::Real64(input.to_bits())],
    )
    .unwrap();
    let output = rumoca_eval_solve::eval_pure_call(&package.pure_calls, owner.id(), &[argument])
        .expect("numeric result and assertion predicates must be evaluable");
    assert_eq!(
        output[0].elements(),
        [SolveValueKind::Real64(value.to_bits())]
    );
    assert_eq!(output.len(), 1 + predicates.len());
    for (actual, expected) in output[1..].iter().zip(predicates) {
        assert_eq!(actual.elements(), [SolveValueKind::Boolean(*expected)]);
    }
}

fn request(package: &LoweredSolvePackage, time: f64) -> EventActionRequest {
    let problem = &package.problem;
    rumoca_eval_solve::eval_event_action_request(
        &problem.events,
        &vec![0.0; problem.layout.y_scalars()],
        &vec![0.0; problem.layout.p_scalars()],
        time,
        RowEvalContext {
            pure_calls: Some(&package.pure_calls),
            ..Default::default()
        },
    )
    .unwrap()
}

const SHARED_LOCAL: &str = r#"
function checked
  input Real limit;
  output Real result;
protected
  Real magnitude;
algorithm
  result := 0;
  for i in 1:3 loop
    magnitude := abs(limit - i);
    assert(magnitude < 3, "magnitude exceeds limit");
    result := result + magnitude;
  end for;
end checked;
"#;

#[test]
fn shared_local_failure_survives_later_success_and_reaches_event_action() {
    let package = compile(SHARED_LOCAL);
    evaluate(&package, 2.0, 2.0, &[true]);
    evaluate(&package, 0.0, 6.0, &[false]); // Last iteration fails.
    evaluate(&package, 4.0, 6.0, &[false]); // Only first iteration fails.
    assert_eq!(request(&package, 2.0), EventActionRequest::Continue);
    assert!(
        matches!(request(&package, 4.0), EventActionRequest::AssertionFailed { message }
        if message == "magnitude exceeds limit")
    );

    let owner = package.pure_calls.owners().last().unwrap();
    let (folds, maps, absolute_values) = operation_counts(owner.body());
    assert_eq!(
        (folds, maps, absolute_values),
        (1, 0, 1),
        "one compact loop and one shared local computation, independent of extent"
    );
}

#[test]
fn assertions_observe_loop_carried_values_before_and_after_assignment() {
    let package = compile(
        r#"
function checked
  input Real limit;
  output Real result;
algorithm
  assert(limit > 0, "entry");
  result := 0;
  for i in 1:3 loop
    assert(result < limit, "before update");
    result := result + i;
    assert(result < limit, "after update");
  end for;
  assert(result == 6, "exit");
end checked;
"#,
    );
    evaluate(&package, 7.0, 6.0, &[true, true, true, true]);
    evaluate(&package, 6.0, 6.0, &[true, true, false, true]);
    evaluate(&package, 3.0, 6.0, &[true, false, false, true]);
    assert!(
        matches!(request(&package, 6.0), EventActionRequest::AssertionFailed { message }
        if message == "after update")
    );
}

#[test]
fn assertion_only_loop_can_share_a_local_without_a_numeric_carry() {
    let package = compile(
        r#"
function checked
  input Real limit;
  output Real result;
protected
  Real magnitude;
algorithm
  result := 1;
  for i in 1:3 loop
    magnitude := abs(limit - i);
    assert(magnitude < 3, "upper");
    assert(magnitude >= 0, "lower");
  end for;
end checked;
"#,
    );
    evaluate(&package, 2.0, 1.0, &[true, true]);
    evaluate(&package, 4.0, 1.0, &[false, true]);
}

#[test]
fn nested_and_sibling_loops_capture_outer_locals_and_keep_predicate_order() {
    let package = compile(
        r#"
function checked
  input Real limit;
  output Real result;
protected
  Real magnitude;
algorithm
  result := 0;
  for i in 1:3 loop
    magnitude := abs(limit - i);
    result := result + magnitude;
    for j in 1:2 loop
      assert(magnitude < 3 and j <= 2, "nested upper");
    end for;
    for k in 1:2 loop
      assert(magnitude >= 0 and k <= 2, "nested lower");
    end for;
  end for;
  assert(result >= 0, "after loops");
end checked;
"#,
    );
    evaluate(&package, 2.0, 2.0, &[true, true, true]);
    evaluate(&package, 4.0, 6.0, &[false, true, true]);
}

#[test]
fn empty_loop_returns_true_predicates_without_evaluating_invalid_body() {
    let package = compile(
        r#"
function checked
  input Real divisor;
  output Real result;
algorithm
  result := 0;
  for i in 1:0 loop
    result := result + 1 / divisor;
    assert(result < 0, "unreachable");
  end for;
end checked;
"#,
    );
    evaluate(&package, 0.0, 0.0, &[true]);
    assert_eq!(request(&package, 0.0), EventActionRequest::Continue);
}

#[test]
fn nested_call_assertions_are_accumulated_inside_the_calling_loop() {
    let package = compile(
        r#"
function checkedMagnitude
  input Real value;
  output Real result;
algorithm
  result := abs(value);
  assert(result < 3, "callee magnitude");
end checkedMagnitude;
function checked
  input Real limit;
  output Real result;
algorithm
  result := 0;
  for i in 1:3 loop
    result := 2 * result + checkedMagnitude(limit - i);
  end for;
end checked;
"#,
    );
    evaluate(&package, 2.0, 5.0, &[true]);
    evaluate(&package, 4.0, 17.0, &[false]);
    assert!(
        matches!(request(&package, 4.0), EventActionRequest::AssertionFailed { message }
        if message == "callee magnitude")
    );
}

#[test]
fn inactive_nested_calls_contribute_true_predicates() {
    let package = compile(
        r#"
function checkedMagnitude
  input Real value;
  output Real result;
algorithm
  result := abs(value);
  assert(result < 3, "active callee");
end checkedMagnitude;
function checked
  input Real enabled;
  output Real result;
algorithm
  result := 0;
  for i in 1:3 loop
    if enabled > 0 then
      result := 2 * result + checkedMagnitude(i);
    else
      result := 2 * result + 1;
    end if;
  end for;
end checked;
"#,
    );
    evaluate(&package, 0.0, 7.0, &[true]);
    evaluate(&package, 1.0, 11.0, &[false]);
}

#[test]
fn array_local_is_shared_by_numeric_update_and_assertion() {
    let package = compile(
        r#"
function checked
  input Real limit;
  output Real result;
protected
  Real tensor[2, 2];
algorithm
  result := 0;
  for i in 1:3 loop
    tensor := [limit - i, 0; 0, 1];
    assert(tensor[1, 1] < 3, "tensor element");
    result := result + sum(tensor);
  end for;
end checked;
"#,
    );
    evaluate(&package, 2.0, 3.0, &[true]);
    evaluate(&package, 4.0, 9.0, &[false]);
}

fn operation_counts(program: &TypedProgram) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    for operation in program.operations() {
        let nested = match operation.operation() {
            SolveOperation::Fold { transition, .. } => {
                counts.0 += 1;
                operation_counts(transition.body())
            }
            SolveOperation::Map { body, .. } => {
                counts.1 += 1;
                operation_counts(body.body())
            }
            SolveOperation::Conditional {
                if_true, if_false, ..
            } => {
                let a = operation_counts(if_true.body());
                let b = operation_counts(if_false.body());
                (a.0 + b.0, a.1 + b.1, a.2 + b.2)
            }
            SolveOperation::Unary {
                operator: SolveUnaryOperator::Abs,
                ..
            } => (0, 0, 1),
            _ => (0, 0, 0),
        };
        counts = (
            counts.0 + nested.0,
            counts.1 + nested.1,
            counts.2 + nested.2,
        );
    }
    counts
}

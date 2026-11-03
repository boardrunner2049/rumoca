//! MLS §11.5, §11.2.2 and §11.2.8.1 / SPEC_0007 DAE-C11:
//! assertion conditions retain branch priority, reaching definitions and loop scope.

use rumoca::Compiler;
use rumoca_eval_solve::{EventActionRequest, RowEvalContext, eval_event_action_request};

const BRANCHES: &str = r#"
function checkBranch
  input Real u;
  output Real y;
algorithm
  y := u;
  if u > 0 then
    y := y + 1;
    assert(y < 3, "positive branch");
    y := 2*y;
  elseif u <> 0 then
    assert(u < 0 and u > -2, "negative branch");
    y := -u;
  else
    assert(false, "zero branch");
  end if;
end checkBranch;
model Branches
  Real y;
equation
  y = checkBranch(time);
end Branches;
"#;

// The direct action evaluator below does not run relation-memory updates.
// Keep the enabling expression event-free to test the function assertion itself.
const NESTED: &str = r#"
function checkIds
  input Real ids[:];
  input Boolean enabled;
  output Real result;
algorithm
  result := sum(ids);
  for i in 1:size(ids,1) loop
    if enabled and ids[i] <> 0 then
      for previous in 1:(i-1) loop
        assert(ids[previous] <> ids[i], "duplicate id");
      end for;
    end if;
  end for;
end checkIds;
model Duplicate
  Real y;
equation
  y = checkIds({1,2,1}, noEvent(time > 0));
end Duplicate;
model Unique
  Real y;
equation
  y = checkIds({1,2,3,4}, noEvent(time > 0));
end Unique;
model Single
  Real y;
equation
  y = checkIds({1}, noEvent(time > 0));
end Single;
"#;

fn lower(source: &str, model: &str) -> rumoca_phase_solve::LoweredSolvePackage {
    let compiled = Compiler::new()
        .model(model)
        .compile_str(source, "nested_assertions.mo")
        .expect("nested assertions must retain checked DAE owners");
    rumoca_phase_solve::lower_solve_package(&compiled.dae).unwrap_or_else(|error| {
        panic!("{model}: nested assertions must have executable schedules: {error}")
    })
}

fn request(package: &rumoca_phase_solve::LoweredSolvePackage, time: f64) -> EventActionRequest {
    let solve = &package.problem;
    eval_event_action_request(
        &solve.events,
        &vec![0.0; solve.layout.y_scalars()],
        &vec![0.0; solve.layout.p_scalars()],
        time,
        RowEvalContext {
            pure_calls: Some(&package.pure_calls),
            ..Default::default()
        },
    )
    .expect("assertion predicates must evaluate without inactive-branch effects")
}

#[test]
fn conditional_assertions_keep_branch_priority_and_statement_values() {
    let package = lower(BRANCHES, "Branches");
    for time in [0.5, 1.0, -1.0] {
        assert_eq!(request(&package, time), EventActionRequest::Continue);
    }
    for (time, expected) in [
        (2.0, "positive branch"),
        (-3.0, "negative branch"),
        (0.0, "zero branch"),
    ] {
        assert!(
            matches!(request(&package,time), EventActionRequest::AssertionFailed { message } if message == expected)
        );
    }
}

#[test]
fn nested_dependent_loop_assertion_fires_only_for_an_active_duplicate() {
    let package = lower(NESTED, "Duplicate");
    assert_eq!(request(&package, -1.0), EventActionRequest::Continue);
    assert!(
        matches!(request(&package,1.0), EventActionRequest::AssertionFailed { message } if message == "duplicate id")
    );
}

#[test]
fn unique_and_empty_inner_loop_domains_do_not_fail() {
    for model in ["Unique", "Single"] {
        let package = lower(NESTED, model);
        assert_eq!(request(&package, 1.0), EventActionRequest::Continue);
    }
}

#[test]
fn nested_branches_preserve_lazy_selection_and_converted_messages() {
    let source = r#"
function checkNested
  input Real u;
  output Real y;
algorithm
  y := u;
  if u > 0 then
    y := 2*u;
    if y > 1 then
      assert(y < 3, "nested input=" + String(u));
      y := y + 10;
    else
      assert(false, "inner else");
    end if;
  elseif sqrt(-u) > 1 then
    assert(false, "outer elseif");
  end if;
end checkNested;
model NestedBranches
  Real y;
equation
  y = checkNested(time);
end NestedBranches;
"#;
    let package = lower(source, "NestedBranches");
    for time in [1.0, -0.5, 0.0] {
        assert_eq!(request(&package, time), EventActionRequest::Continue);
    }
    for (time, expected) in [
        (2.0, "nested input=2"),
        (0.25, "inner else"),
        (-2.0, "outer elseif"),
    ] {
        assert!(
            matches!(request(&package,time), EventActionRequest::AssertionFailed { message } if message == expected)
        );
    }
}

#[test]
fn record_array_with_string_message_has_checked_dae() {
    // Preserve the source shape of FIRE's combineMassProperties, including
    // record projection and a loop-indexed String message. Runtime numeric
    // predicates are covered separately above; this pins the Flat -> DAE edge.
    let source = r#"
record Part
  Real mass;
  String componentId;
end Part;
function combine
  input Part parts[:];
  output Real mass;
algorithm
  mass := 0;
  for i in 1:size(parts,1) loop
    assert(parts[i].mass >= 0, "negative mass");
    if parts[i].componentId <> "" then
      for previous in 1:(i-1) loop
        assert(parts[previous].componentId <> parts[i].componentId,
          "Duplicate physical mass componentId: " + parts[i].componentId);
      end for;
    end if;
    mass := mass + parts[i].mass;
  end for;
end combine;
model MassParts
  Real y;
equation
  y = combine({Part(time, "body"), Part(1, "arm"), Part(2, "arm")});
end MassParts;
"#;
    Compiler::new()
        .model("MassParts")
        .compile_str(source, "mass_parts.mo")
        .expect("nested record assertions must have checked DAE owners");
}

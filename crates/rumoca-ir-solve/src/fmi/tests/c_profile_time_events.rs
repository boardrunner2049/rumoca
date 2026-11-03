//! The C profile admits time events, relation memory without a searched root,
//! and the `initial()` event, carrying the Solve IR facts its generated
//! component schedules (SPEC_0044 ME-EVENT-002).

use super::*;
use crate::{
    LinearOp, RootRelationRefreshRole, RootZeroDomain, ScalarProgramBlock, ScalarSlot,
    ScheduledRootCondition, SolveDelayPartition,
};

fn span() -> rumoca_core::Span {
    rumoca_core::Span::from_offsets(
        rumoca_core::SourceId::from_source_name("fmi_c_profile_time_events.mo"),
        0,
        1,
    )
}

fn rows(programs: Vec<Vec<LinearOp>>) -> ScalarProgramBlock {
    ScalarProgramBlock::with_source_span(
        programs,
        span()
            .require_provenance("C profile time-event fixture")
            .unwrap(),
    )
    .unwrap()
}

/// `p[index] - time`: a direct time root the schedule announces.
fn time_root(index: usize) -> Vec<LinearOp> {
    vec![
        LinearOp::LoadP { dst: 0, index },
        LinearOp::LoadTime { dst: 1 },
        LinearOp::Binary {
            dst: 2,
            op: crate::BinaryOp::Sub,
            lhs: 0,
            rhs: 1,
        },
        LinearOp::StoreOutput { src: 2 },
    ]
}

fn load_p(index: usize) -> Vec<LinearOp> {
    vec![
        LinearOp::LoadP { dst: 0, index },
        LinearOp::StoreOutput { src: 0 },
    ]
}

/// The serialized C view of the one-parameter kernel after `mutate`.
fn admitted(mutate: impl FnOnce(&mut SolveModel)) -> serde_json::Value {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = SolveDelayPartition::default();
    mutate(&mut model);
    let view = FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .unwrap_or_else(|error| panic!("the C profile admits the partition: {error}"));
    serde_json::to_value(&view).expect("the C view serializes")
}

fn time_events(view: &serde_json::Value) -> &serde_json::Value {
    &view["scalar_events"]["time_events"]
}

#[test]
fn static_instants_are_scheduled_sorted_and_merged() {
    let view = admitted(|model| {
        model.problem.events.scheduled_time_events = vec![0.5, 0.1, 0.1 + 1.0e-14, f64::INFINITY];
    });
    assert_eq!(
        time_events(&view)["instants"],
        serde_json::json!([0.1, 0.5])
    );
    assert_eq!(view["event_indicator_roots"], serde_json::json!([]));
}

#[test]
fn a_parameter_deadline_is_a_time_event_not_an_indicator() {
    let view = admitted(|model| {
        model.problem.events.dynamic_time_event_rhs = rows(vec![load_p(0)]);
    });
    assert_eq!(time_events(&view)["deadlines"], 1);
    assert_eq!(view["event_indicator_roots"], serde_json::json!([]));
}

#[test]
fn a_direct_time_root_with_relation_memory_is_announced_not_searched() {
    let view = admitted(|model| {
        let events = &mut model.problem.events;
        events.root_conditions = rows(vec![time_root(0)]);
        events.root_relation_memory_targets = vec![Some(ScalarSlot::P {
            index: 1,
            byte_offset: 8,
        })];
        events.root_zero_domains = vec![RootZeroDomain::Previous];
        events.root_relation_refresh_roles = vec![RootRelationRefreshRole::Frozen];
    });
    assert_eq!(time_events(&view)["announced"], serde_json::json!([0]));
    let root = &view["scalar_events"]["roots"][0];
    assert_eq!(root["role"], "announced_time");
    assert_eq!(root["memory"], 1);
    assert_eq!(view["event_indicator_roots"], serde_json::json!([]));
}

/// No Solve lowering produces a periodic sample root (`sample()` lowers to a
/// clock partition), so the C profile has no executed path for one and
/// refuses it by name instead of rendering untested tick handling.
#[test]
fn a_periodic_sample_root_is_refused() {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = SolveDelayPartition::default();
    let events = &mut model.problem.events;
    events.root_conditions = rows(vec![load_p(0)]);
    events.root_relation_memory_targets = vec![Some(ScalarSlot::P {
        index: 1,
        byte_offset: 8,
    })];
    events.root_zero_domains = vec![RootZeroDomain::Previous];
    events.root_relation_refresh_roles = vec![RootRelationRefreshRole::Frozen];
    events.scheduled_root_conditions = vec![ScheduledRootCondition {
        root_index: 0,
        period_seconds: 0.25,
        phase_seconds: 0.0,
    }];
    let refused = FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .expect_err("the C profile refuses a periodic sample root")
        .to_string();
    assert!(refused.contains("periodic sample roots"), "{refused}");
}

#[test]
fn the_initial_event_flag_is_carried() {
    let view = admitted(|model| {
        model.problem.events.scheduled_time_events = vec![1.0];
        model.problem.solve_layout.initial_event_parameter_index = Some(1);
    });
    assert_eq!(time_events(&view)["initial_flag"], 1);
}

#[test]
fn a_periodic_root_outside_the_root_table_is_not_constructible() {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = SolveDelayPartition::default();
    model.problem.events.scheduled_root_conditions = vec![ScheduledRootCondition {
        root_index: 0,
        period_seconds: 0.25,
        phase_seconds: 0.0,
    }];
    // The shape contract already rejects a scheduled root outside the root
    // table, so the component never reaches the C profile.
    assert!(FmiComponent::construct(model, vec![input]).is_err());
}

/// A deadline that moves with a state needs an FMI event indicator the C
/// component does not render; the refusal says so instead of dropping it.
#[test]
fn a_state_dependent_deadline_is_refused() {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_state();
    model.problem.events.delays = SolveDelayPartition::default();
    model.problem.events.dynamic_time_event_rhs = rows(vec![vec![
        LinearOp::LoadY { dst: 0, index: 0 },
        LinearOp::StoreOutput { src: 0 },
    ]]);
    let refused = FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .expect_err("the C profile refuses a state-dependent deadline")
        .to_string();
    assert!(
        refused.contains("state-dependent dynamic time-event indicator"),
        "{refused}"
    );
}

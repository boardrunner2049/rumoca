//! Every C-profile refusal of an initialization or event partition the shared
//! kernel cannot execute names its exact reason (SPEC_0044 ME-PARAM-001 and
//! ME-EVENT-003).

use super::*;
use crate::{
    ComputeBlock, DiscreteRowRole, InitializationRowRole, LinearOp, PeriodicEventSchedule,
    PreParamBinding, PreParamSource, ScalarProgramBlock, ScalarSlot, SolveDelayPartition,
};

fn span() -> rumoca_core::Span {
    rumoca_core::Span::from_offsets(
        rumoca_core::SourceId::from_source_name("fmi_c_profile_refusals.mo"),
        0,
        1,
    )
}

fn rows(programs: Vec<Vec<LinearOp>>) -> ScalarProgramBlock {
    ScalarProgramBlock::with_source_span(
        programs,
        span()
            .require_provenance("C profile refusal fixture")
            .unwrap(),
    )
    .unwrap()
}

fn constant_row(value: f64) -> Vec<LinearOp> {
    vec![
        LinearOp::Const { dst: 0, value },
        LinearOp::StoreOutput { src: 0 },
    ]
}

/// The refusal of the one-parameter kernel after `mutate`.
fn refusal(mutate: impl FnOnce(&mut SolveModel)) -> String {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = SolveDelayPartition::default();
    mutate(&mut model);
    FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .expect_err("the C profile refuses the partition")
        .to_string()
}

fn assert_refused(message: &str, mutate: impl FnOnce(&mut SolveModel)) {
    let refused = refusal(mutate);
    assert!(
        refused.contains(message),
        "expected `{message}`, got `{refused}`"
    );
}

/// One initialization residual row with the given role.
fn one_initial_row(model: &mut SolveModel, role: InitializationRowRole) {
    one_initial_program(model, role, constant_row(0.0));
}

/// One initialization residual row computing `program`, with the given role.
fn one_initial_program(
    model: &mut SolveModel,
    role: InitializationRowRole,
    program: Vec<LinearOp>,
) {
    let mut initialization = model.problem.initialization.clone().into_input();
    initialization.residual = ComputeBlock::from_scalar_program_block(rows(vec![program]));
    initialization.row_roles = vec![role];
    model.problem.initialization = crate::InitializationSolveSystem::construct(initialization)
        .expect("one surplus row is a checked initialization");
}

const SETTLED: InitializationRowRole = InitializationRowRole::SurplusAlgebraicCheck;

const SEEDS: &str =
    "C initialization evaluates residual rows only on the settled algebraic refresh";

/// A row over the declaration seed of an algebraic `y[0]` (no state precedes
/// it) reads a value the settled view replaces.
#[test]
fn initialization_rows_over_algebraic_declaration_seeds_are_refused() {
    assert_refused(SEEDS, |model| {
        model.problem.layout = crate::VarLayout::from_parts(Default::default(), 1, 2);
        one_initial_program(
            model,
            InitializationRowRole::SurplusCheck,
            vec![
                LinearOp::LoadY { dst: 0, index: 0 },
                LinearOp::StoreOutput { src: 0 },
            ],
        );
    });
}

/// A row that reads no coordinate the algebraic refresh writes has the same
/// residual over the seeds and over the settled view, so it is not refused
/// for reading seeds (SPEC_0044 ME-PARAM-001).
#[test]
fn initialization_rows_over_parameters_run_on_the_settled_view() {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = SolveDelayPartition::default();
    one_initial_program(
        &mut model,
        InitializationRowRole::SurplusCheck,
        vec![
            LinearOp::LoadP { dst: 0, index: 1 },
            LinearOp::StoreOutput { src: 0 },
        ],
    );
    let outcome = FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .map(|_| ())
        .map_err(|error| error.to_string());
    assert!(
        outcome
            .as_ref()
            .err()
            .is_none_or(|refused| !refused.contains(SEEDS)),
        "a parameter row is not a row over seeds: {outcome:?}"
    );
}

#[test]
fn homotopy_initialization_is_refused() {
    assert_refused(
        "C initialization cannot run the homotopy continuation",
        |model| {
            one_initial_row(model, SETTLED);
            model.problem.solve_layout.initial_homotopy_parameter_index = Some(0);
        },
    );
}

#[test]
fn delay_histories_in_initialization_are_refused() {
    assert_refused(
        "C initialization cannot synchronize delay histories",
        |model| {
            one_initial_row(model, SETTLED);
            let (delayed, _) = super::max_step_duration_local::delay_bearing_model_with_one_run();
            model.problem.events.delays = delayed.problem.events.delays;
        },
    );
}

#[test]
fn solver_coordinates_that_do_not_fill_the_y_storage_are_refused() {
    assert_refused(
        "C initialization needs the solver coordinates to fill the Y storage",
        |model| {
            one_initial_row(model, SETTLED);
            model.problem.layout = crate::VarLayout::from_parts(Default::default(), 1, 2);
        },
    );
}

/// A static event partition: one condition memory in parameter storage, so
/// the component has a discrete class without continuous indicators.
fn static_partition(model: &mut SolveModel) {
    model.problem.events.condition_memory_parameter_indices = vec![0];
}

/// One discrete row writing `p[0]` with `role`.
fn one_discrete_row(model: &mut SolveModel, role: DiscreteRowRole) {
    let discrete = &mut model.problem.discrete;
    discrete.rhs = rows(vec![constant_row(1.0)]);
    discrete.update_targets = vec![ScalarSlot::P {
        index: 0,
        byte_offset: 0,
    }];
    discrete.row_roles = vec![role];
    discrete.pre_modes = vec![crate::DiscreteEventPreMode::default()];
    discrete.observation_refresh = vec![false];
    discrete.integrator_history_effects = vec![crate::IntegratorHistoryEffect::default()];
    discrete.clock_owners = vec![None];
}

#[test]
fn event_edge_actions_are_refused() {
    // Not parameter-determined, so the event-iteration profile owns the
    // refusal: it executes discrete equations and condition memories only.
    assert_refused(
        "the C profile executes only discrete equations and condition memories",
        |model| {
            static_partition(model);
            one_discrete_row(model, DiscreteRowRole::EventAction);
        },
    );
}

#[test]
fn clocked_previous_history_is_refused() {
    assert_refused(
        "the C profile cannot execute delays, terminal events, or synchronous clocks",
        |model| {
            static_partition(model);
            model.problem.solve_layout.pre_param_bindings = vec![PreParamBinding {
                dest_p_index: 0,
                source: PreParamSource::P { index: 1 },
                clock_schedule: Some(PeriodicEventSchedule::from_seconds(0.1, 0.0).unwrap()),
            }];
        },
    );
}

/// A pre() value the continuous kernel reads is not parameter-determined, so
/// the static profile does not own it; the event-iteration profile keeps every
/// pre lane committed at each event and admits it.
#[test]
fn a_pre_value_read_by_the_continuous_kernel_runs_the_event_iteration() {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = crate::SolveDelayPartition::default();
    static_partition(&mut model);
    model.problem.solve_layout.pre_param_bindings = vec![PreParamBinding {
        dest_p_index: 0,
        source: PreParamSource::P { index: 1 },
        clock_schedule: None,
    }];
    model.problem.continuous.residual = ComputeBlock::from_scalar_program_block(rows(vec![vec![
        LinearOp::LoadP { dst: 0, index: 0 },
        LinearOp::StoreOutput { src: 0 },
    ]]));
    assert!(super::super::static_assertions::validate(&model).is_err());
    let view = FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .expect("the event-iteration profile admits a committed pre() read");
    let view = serde_json::to_value(&view).expect("the C view serializes");
    assert_eq!(view["scalar_events"]["pre_bindings"][0]["dest"], 0);
}

#[test]
fn a_projection_plan_without_residual_rows_is_refused() {
    assert_refused(
        "C initialization projects blocks without residual rows",
        |model| {
            let mut initialization = model.problem.initialization.clone().into_input();
            initialization.projection_plan = crate::InitializationProjectionPlan {
                blocks: vec![crate::InitializationProjectionBlock::default()],
            };
            model.problem.initialization =
                crate::InitializationSolveSystem::construct(initialization)
                    .expect("an empty block over an empty residual is a checked shape");
        },
    );
}

/// The published causality of the one parameter after `mutate`.
fn published_causality(mutate: impl FnOnce(&mut SolveModel)) -> serde_json::Value {
    let (mut model, input) = super::max_step_duration_local::delay_bearing_model_with_one_run();
    model.problem.events.delays = SolveDelayPartition::default();
    mutate(&mut model);
    let view = FmiComponent::construct(model, vec![input])
        .expect("the fixture is a checked component")
        .into_codegen_view()
        .try_c()
        .expect("a parameter-determined assertion is admitted");
    serde_json::to_value(&view).unwrap()["variables"][0]["causality"].clone()
}

/// An error assertion whose condition reads the parameter `p` (storage index
/// 1) and nothing else reads it.
fn assertion_on_parameter(model: &mut SolveModel) {
    let events = &mut model.problem.events;
    events.actions = vec![crate::SolveEventAction {
        kind: crate::SolveEventActionKind::Assert,
        message: crate::SolveEventMessage {
            parts: vec![crate::SolveEventMessagePart::Text(
                "p must be positive".into(),
            )],
        },
        span: span(),
        origin: "assertion fixture".into(),
        clock_owner: None,
    }];
    events.action_conditions = rows(vec![vec![
        LinearOp::LoadP { dst: 0, index: 1 },
        LinearOp::Const { dst: 1, value: 0.0 },
        LinearOp::Compare {
            dst: 2,
            op: crate::CompareOp::Le,
            lhs: 0,
            rhs: 1,
        },
        LinearOp::StoreOutput { src: 2 },
    ]]);
}

/// SPEC_0044 ME-PARAM-001: an assertion condition is a read that keeps a
/// parameter settable; a parameter no program reads keeps its parameter
/// variability too (MLS §4.5), since whether a program reads it does not
/// change what it is.
#[test]
fn a_parameter_read_only_by_an_assertion_condition_stays_settable() {
    assert_eq!(published_causality(assertion_on_parameter), "parameter");
    assert_eq!(
        published_causality(|model| {
            assertion_on_parameter(model);
            model.problem.events.action_conditions = rows(vec![constant_row(0.0)]);
        }),
        "parameter"
    );
}

/// A continuous residual row reading the parameter `p` (storage index 1) only
/// inside a nested sub-program.
fn nested_read(model: &mut SolveModel, row: Vec<LinearOp>) {
    model.problem.continuous.residual = ComputeBlock::from_scalar_program_block(rows(vec![row]));
}

fn load_parameter_row() -> Vec<LinearOp> {
    vec![
        LinearOp::LoadP { dst: 0, index: 1 },
        LinearOp::StoreOutput { src: 0 },
    ]
}

/// SPEC_0044 ME-PARAM-001: reads inside fold updates and conditional arms are
/// reads; the parameter stays settable.
#[test]
fn a_parameter_read_only_inside_a_nested_program_stays_settable() {
    let fold = crate::FunctionFoldProgram::checked(
        rumoca_core::StructuredIndexDomain {
            binders: vec![rumoca_core::StructuredIndexBinder {
                id: 0,
                display_name: "i".to_string(),
                lower: 1,
                upper: 2,
                step: 1,
            }],
        },
        1,
        0,
        load_parameter_row(),
    )
    .expect("a one-carried fold is checked");
    assert_eq!(
        published_causality(|model| nested_read(
            model,
            vec![
                LinearOp::Const { dst: 0, value: 0.0 },
                LinearOp::FunctionFold {
                    dst_start: 1,
                    initial_start: 0,
                    capture_start: 0,
                    program: std::sync::Arc::new(fold),
                },
                LinearOp::StoreOutput { src: 1 },
            ],
        )),
        "parameter",
        "a fold body read keeps the parameter settable"
    );
    let conditional = crate::FunctionConditionalProgram::checked(
        0,
        vec![1],
        [(constant_row(1.0), load_parameter_row())],
        constant_row(0.0),
    )
    .expect("a one-arm conditional is checked");
    assert_eq!(
        published_causality(|model| nested_read(
            model,
            vec![
                LinearOp::FunctionConditional {
                    dst_start: 0,
                    capture_start: 0,
                    program: std::sync::Arc::new(conditional),
                },
                LinearOp::StoreOutput { src: 0 },
            ],
        )),
        "parameter",
        "a conditional arm read keeps the parameter settable"
    );
}

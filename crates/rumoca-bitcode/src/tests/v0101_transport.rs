//! Metadata introduced by main must survive the public interchange boundary.
use super::*;

fn literal(model: &mut RbcModel, scalar: RbcScalar, value: RbcLiteral) -> ExprId {
    let value_type = TypeId(model.types.len() as u32);
    model.types.push(RbcType {
        id: value_type,
        scalar,
        dimensions: vec![],
        record: None,
    });
    let id = ExprId(model.expressions.len() as u32);
    model.expressions.push(RbcExpr {
        id,
        value_type,
        node: RbcExprNode::Literal { value },
        provenance: source_provenance(),
    });
    id
}

fn warning_model(legacy: bool) -> RbcFile {
    let mut model = decay_model();
    let condition = literal(
        &mut model,
        RbcScalar::Boolean,
        RbcLiteral::Boolean { value: false },
    );
    let message = literal(
        &mut model,
        RbcScalar::String,
        RbcLiteral::String {
            value: "keep running".into(),
        },
    );
    let action = if legacy {
        let level = literal(
            &mut model,
            RbcScalar::Enumeration,
            RbcLiteral::Enumeration { ordinal: 2 },
        );
        RbcAction::Assert {
            message,
            level: Some(level),
        }
    } else {
        RbcAction::Warning { condition, message }
    };
    model.conditions.push(RbcCondition {
        id: ConditionId(0),
        node: RbcConditionNode::Always,
        provenance: source_provenance(),
    });
    model.events.push(RbcEventAction {
        id: EventId(0),
        trigger: ConditionId(0),
        guard: ConditionId(0),
        action,
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);
    file(model)
}

#[test]
fn warnings_remain_nonterminating_actions_through_every_transport() {
    for legacy in [false, true] {
        let original = warning_model(legacy);
        let text = crate::text::print_text(&original).unwrap();
        let text = crate::text::parse_text(&text).unwrap();
        for encoding in [Encoding::Json, Encoding::Cbor] {
            let (decoded, _) = decode(&encode(&text, encoding).unwrap()).unwrap();
            let dae = crate::import(&decoded).expect("warning imports without coercing to error");
            dae.inspect(|view| {
                let action = view.event_action(view.event_action_id(0).unwrap()).unwrap();
                assert!(matches!(
                    action.operation(),
                    rumoca_ir_dae::EventActionOperation::Warning { .. }
                ));
                assert_eq!(view.root_count(), 0, "a warning introduces no event root");
            });
            let again = crate::export(&dae, None, "Warning", &Default::default()).unwrap();
            assert!(matches!(
                again.model.events[0].action,
                RbcAction::Warning { .. }
            ));
        }
    }
}

#[test]
fn per_element_fixed_and_evaluable_metadata_survive_reconstruction() {
    let mut model = decay_model();
    model.equations.clear();
    model.expressions.clear();
    for variable in &mut model.variables {
        variable.binding = None;
        variable.start = None;
    }
    let array_type = TypeId(model.types.len() as u32);
    model.types.push(RbcType {
        id: array_type,
        scalar: RbcScalar::Real,
        dimensions: vec![2],
        record: None,
    });
    model.variables[0].value_type = array_type;
    model.variables[0].scalar_count = 2;
    model.variables[0].state_select = RbcStateSelect::Prefer;
    model.variables[0].declared_causality = Some(RbcDeclaredCausality::Output);
    model.variables[0].fixed = None;
    model.variables[0].fixed_elements = Some(vec![true, false]);
    let binding = literal(&mut model, RbcScalar::Real, RbcLiteral::Real { value: 1.0 });
    model.variables[1].binding = Some(binding);
    model.variables[1].tunable = false;
    model.variables[1].evaluable = true;
    recompute_summary(&mut model);
    let original = file(model);
    let text = crate::text::print_text(&original).unwrap();
    let parsed = crate::text::parse_text(&text).unwrap();
    let dae = crate::import(&parsed).expect("per-element fixed attributes reconstruct");
    let again = crate::export(&dae, None, "FixedArray", &Default::default()).unwrap();
    assert_eq!(
        again.model.variables[0].fixed_elements,
        Some(vec![true, false])
    );
    assert!(again.model.variables[1].evaluable);
    assert_eq!(
        again.model.variables[0].state_select,
        RbcStateSelect::Prefer
    );
    assert_eq!(
        again.model.variables[0].declared_causality,
        Some(RbcDeclaredCausality::Output)
    );
}

#[test]
fn a_shifted_clock_cannot_reference_itself() {
    let mut model = decay_model();
    model.conditions.push(RbcCondition {
        id: ConditionId(0),
        node: RbcConditionNode::Always,
        provenance: source_provenance(),
    });
    model.clocks.push(RbcClock {
        id: ClockId(0),
        node: RbcClockNode::Shifted {
            base: ClockId(0),
            counter: 1,
            condition: ConditionId(0),
        },
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);
    assert!(
        errors(&model)
            .iter()
            .any(|error| matches!(error, ValidationError::Clock(_)))
    );
}

//! Public transport preserves activation, ownership and predefined conversion.
use super::*;

fn rational(numerator: &str, denominator: &str) -> RbcClockRational {
    RbcClockRational {
        numerator: numerator.into(),
        denominator: denominator.into(),
    }
}

fn clock_model(sampled: bool, anchor: RbcClockAnchor) -> RbcFile {
    let mut model = decay_model();
    model.clocks.push(RbcClock {
        id: ClockId(0),
        node: RbcClockNode::Periodic {
            period: rational("1", "20"),
            phase: rational("-5", "4"),
            anchor,
            sample_event: None,
        },
        provenance: source_provenance(),
    });
    model.conditions.push(RbcCondition {
        id: ConditionId(0),
        node: RbcConditionNode::ClockActivation { clock: ClockId(0) },
        provenance: source_provenance(),
    });
    let target = VariableId(model.variables.len() as u32);
    model
        .variables
        .push(variable(target.0, "held", RbcRole::DiscreteReal));
    model.clock_ownerships.push(RbcClockOwnership {
        variable: target,
        clock: ClockId(0),
        sampled,
        provenance: source_provenance(),
    });
    let residual = ExprId(model.expressions.len() as u32);
    model.expressions.push(RbcExpr {
        id: residual,
        value_type: TypeId(0),
        node: RbcExprNode::Coordinate {
            coordinate: RbcCoordinate::DiscreteReal { variable: target },
        },
        provenance: source_provenance(),
    });
    model.discrete_real_equations.push(RbcDiscreteRealEquation {
        id: EquationId(0),
        residual,
        activation: RbcDiscreteRealActivation::When {
            trigger: ConditionId(0),
            guard: ConditionId(0),
        },
        reads: vec![target],
        reads_derivative: vec![],
        reads_previous: vec![],
        provenance: source_provenance(),
    });
    let string_type = TypeId(model.types.len() as u32);
    model.types.push(RbcType {
        id: string_type,
        scalar: RbcScalar::String,
        dimensions: vec![],
        record: None,
    });
    let string = ExprId(model.expressions.len() as u32);
    model.expressions.push(RbcExpr {
        id: string,
        value_type: string_type,
        node: RbcExprNode::StringConversion {
            value: ExprId(0),
            format: RbcStringConversionFormat::Options {
                minimum_length: None,
                left_justified: None,
                significant_digits: None,
            },
        },
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);
    file(model)
}

fn round_trip(original: &RbcFile) -> RbcFile {
    let dae = crate::import(original).expect("checked public transport imports");
    crate::export(&dae, None, "test", &Default::default()).expect("re-export")
}

#[test]
fn clocks_preserve_negative_phase_start_anchor_and_sampled_ownership() {
    for anchor in [RbcClockAnchor::Absolute, RbcClockAnchor::SimulationStart] {
        for sampled in [false, true] {
            let original = clock_model(sampled, anchor);
            for encoding in [Encoding::Json, Encoding::Cbor] {
                let (decoded, _) = decode(&encode(&original, encoding).unwrap()).unwrap();
                let again = round_trip(&decoded);
                assert_eq!(
                    serde_json::to_value(&original.model.clocks).unwrap(),
                    serde_json::to_value(&again.model.clocks).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(&original.model.clock_ownerships).unwrap(),
                    serde_json::to_value(&again.model.clock_ownerships).unwrap()
                );
                assert!(again.model.expressions.iter().any(|expression| matches!(
                    expression.node,
                    RbcExprNode::StringConversion { .. }
                )));
                let text = crate::text::print_text_with(
                    &again,
                    crate::text::TextOptions { sources: true },
                )
                .unwrap();
                let parsed = crate::text::parse_text(&text).unwrap();
                assert_eq!(
                    serde_json::to_value(&again.model).unwrap(),
                    serde_json::to_value(&parsed.model).unwrap()
                );
            }
        }
    }
}

#[test]
fn linking_relocates_clock_identities_and_keeps_both_owners() {
    let a = clock_model(false, RbcClockAnchor::Absolute);
    let b = clock_model(true, RbcClockAnchor::SimulationStart);
    let linked = crate::link::link(
        "pair",
        &[
            crate::link::LinkInput {
                namespace: "a",
                file: &a,
            },
            crate::link::LinkInput {
                namespace: "b",
                file: &b,
            },
        ],
        false,
    )
    .unwrap();
    assert_eq!(linked.model.clock_ownerships[1].clock, ClockId(1));
    assert_eq!(linked.model.clock_ownerships[1].variable.0, 5);
    assert_eq!(round_trip(&linked).model.clocks.len(), 2);
}

#[test]
fn malformed_schedules_and_clock_ownership_fail_closed() {
    let original = clock_model(false, RbcClockAnchor::Absolute);
    for (numerator, denominator) in [
        ("1", "0"),
        ("0", "1"),
        ("bad", "1"),
        ("999999999999999999999999999999999999999999999999", "1"),
    ] {
        let mut bad = original.clone();
        if let RbcClockNode::Periodic { period, .. } = &mut bad.model.clocks[0].node {
            *period = rational(numerator, denominator);
        }
        assert!(crate::import(&bad).is_err());
    }
    let mut bad = original.clone();
    bad.model.clock_ownerships[0].clock = ClockId(8);
    assert!(crate::import(&bad).is_err());
    let mut bad = original.clone();
    bad.model.clock_ownerships[0].variable = VariableId(0);
    assert!(crate::import(&bad).is_err());
    let mut bad = original.clone();
    bad.model
        .clock_ownerships
        .push(bad.model.clock_ownerships[0].clone());
    recompute_summary(&mut bad.model);
    assert!(crate::import(&bad).is_err());
}

#[test]
fn string_format_operands_are_typed_and_topologically_checked() {
    let mut bad = clock_model(false, RbcClockAnchor::Absolute);
    let expression = bad.model.expressions.last_mut().unwrap();
    if let RbcExprNode::StringConversion { format, .. } = &mut expression.node {
        *format = RbcStringConversionFormat::Options {
            minimum_length: None,
            left_justified: Some(ExprId(0)),
            significant_digits: None,
        };
    }
    assert!(
        crate::import(&bad).is_err(),
        "Real is not Boolean leftJustified"
    );
    if let RbcExprNode::StringConversion { format, .. } =
        &mut bad.model.expressions.last_mut().unwrap().node
    {
        *format = RbcStringConversionFormat::Format {
            value: ExprId(10000),
        };
    }
    assert!(
        crate::import(&bad).is_err(),
        "forward format operand must reject"
    );
}

#[test]
fn superseded_payload_free_clock_artifacts_are_not_accepted() {
    let mut original = clock_model(false, RbcClockAnchor::Absolute);
    original.bitcode_version = 1;
    assert!(crate::import(&original).is_err());
    assert!(
        serde_json::from_value::<RbcConditionNode>(serde_json::json!({"kind":"clock"})).is_err()
    );
}

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

#[test]
fn string_options_and_format_string_round_trip_and_relocate() {
    let mut original = clock_model(false, RbcClockAnchor::Absolute);
    let model = &mut original.model;
    let minimum_length = literal(model, RbcScalar::Integer, RbcLiteral::Integer { value: 8 });
    let significant_digits = literal(model, RbcScalar::Integer, RbcLiteral::Integer { value: 6 });
    let left_justified = literal(
        model,
        RbcScalar::Boolean,
        RbcLiteral::Boolean { value: true },
    );
    let format = literal(
        model,
        RbcScalar::String,
        RbcLiteral::String {
            value: ".3f".into(),
        },
    );
    for options in [
        RbcStringConversionFormat::Options {
            minimum_length: Some(minimum_length),
            significant_digits: Some(significant_digits),
            left_justified: Some(left_justified),
        },
        RbcStringConversionFormat::Format { value: format },
    ] {
        let id = ExprId(model.expressions.len() as u32);
        model.expressions.push(RbcExpr {
            id,
            value_type: TypeId(1),
            node: RbcExprNode::StringConversion {
                value: ExprId(0),
                format: options,
            },
            provenance: source_provenance(),
        });
    }
    recompute_summary(model);
    let again = round_trip(&original);
    let formats = |file: &RbcFile| {
        file.model
            .expressions
            .iter()
            .filter_map(|expression| {
                if let RbcExprNode::StringConversion { format, .. } = &expression.node {
                    Some(serde_json::to_value(format).unwrap())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(formats(&original), formats(&again));
    let linked = crate::link::link(
        "two",
        &[
            crate::link::LinkInput {
                namespace: "a",
                file: &original,
            },
            crate::link::LinkInput {
                namespace: "b",
                file: &original,
            },
        ],
        false,
    )
    .unwrap();
    assert!(crate::import(&linked).is_ok());
    let last = linked.model.expressions.last().unwrap();
    assert!(matches!(last.node, RbcExprNode::StringConversion {
        format: RbcStringConversionFormat::Format { value }, ..
    } if value.0 == format.0 + original.model.expressions.len() as u32));
}

#[test]
fn triggered_clock_retains_condition_identity() {
    let mut original = clock_model(false, RbcClockAnchor::Absolute);
    original.model.conditions.push(RbcCondition {
        id: ConditionId(1),
        node: RbcConditionNode::Initial,
        provenance: source_provenance(),
    });
    original.model.clocks[0].node = RbcClockNode::Triggered {
        condition: ConditionId(1),
    };
    recompute_summary(&mut original.model);
    let again = round_trip(&original);
    assert!(matches!(
        again.model.clocks[0].node,
        RbcClockNode::Triggered {
            condition: ConditionId(1)
        }
    ));
    let linked = crate::link::link(
        "two",
        &[
            crate::link::LinkInput {
                namespace: "a",
                file: &original,
            },
            crate::link::LinkInput {
                namespace: "b",
                file: &original,
            },
        ],
        false,
    )
    .unwrap();
    assert!(matches!(
        linked.model.clocks[1].node,
        RbcClockNode::Triggered {
            condition: ConditionId(3)
        }
    ));
    assert!(crate::import(&linked).is_ok());
}

#[test]
fn a_terminal_owner_is_carried_not_discarded() {
    // `terminal()` used to be refused as an owner table bitcode could not
    // represent. It is carried now; what must still hold is that it is never
    // silently dropped: export keeps it and import rebuilds it.
    use rumoca_ir_dae as dae;
    let mut sources = rumoca_core::SourceMap::new();
    let source = sources.add("terminal.mo", "terminal()");
    let span = sources.try_span(source, 0, 10).unwrap();
    let at = dae::DaeProvenance::source(span).unwrap();
    let model = dae::Dae::construct(sources, |model| {
        model.temporal(|owner| owner.terminal(at))?;
        Ok(())
    })
    .unwrap();
    let file = crate::export(&model, None, "Terminal", &Default::default()).unwrap();
    assert_eq!(file.model.terminals.len(), 1, "the owner is exported");
}

//! Round-trip, codec, and adversarial tests.
//!
//! The adversarial cases matter most: an external pass is untrusted, so every
//! way a malformed artifact could reach reconstruction must be a clean
//! rejection rather than a panic or an invalid model.

mod clock_transport;
mod v0101_transport;

use crate::build::Builder;
use crate::codec::{Encoding, decode, encode};
use crate::schema::*;
use crate::validate::{ValidateOptions, ValidationError, recompute_summary, validate};

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn span() -> RbcSpan {
    RbcSpan {
        source: SourceId(0),
        start: 0,
        end: 4,
        line: 1,
        column: 1,
    }
}

fn source_provenance() -> RbcProvenance {
    RbcProvenance {
        origin: RbcOrigin::Source,
        span: span(),
    }
}

/// `der(x) = -k*x` in bitcode form: two variables, one residual.
/// `der(x) = -k*x`, the fixture most of this file is written against.
///
/// Built rather than spelled out. The struct-literal version named every field
/// of `RbcModel`, so every field added to the schema broke it — three times in
/// one week — and each repair was a mechanical edit that taught nobody
/// anything. The builder fills a new field with its default, so a schema
/// addition is a schema change and stops being a fixture change.
fn decay_model() -> RbcModel {
    let mut builder = Builder::new("Decay");
    // `x` before `k`, because tests here index the variable table directly and
    // the order is part of what the fixture is.
    let x = builder.state("x", 1.0);
    let k = builder.parameter("k", 1.0);
    let scaled = {
        let left = builder.parameter_ref(k);
        let right = builder.state_ref(x);
        builder.binary(RbcBinaryOp::Multiply, left, right)
    };
    let rhs = builder.negate_of(scaled);
    builder.derivative_equation(x, rhs);
    let mut model = builder.finish();
    // The fixture's provenance is `Source`, because several tests here assert
    // on what a *compiled* artifact looks like rather than a synthesised one.
    for variable in &mut model.variables {
        variable.declaration = source_provenance();
        variable.from_source = true;
    }
    for expression in &mut model.expressions {
        expression.provenance = source_provenance();
    }
    for equation in &mut model.equations {
        equation.provenance = source_provenance();
    }
    model.sources = vec![RbcSource {
        id: SourceId(0),
        name: "Decay.mo".into(),
        text: Some("model Decay end Decay;".into()),
    }];
    model
}

fn variable(id: u32, name: &str, role: RbcRole) -> RbcVariable {
    RbcVariable {
        id: VariableId(id),
        name: name.into(),
        role,
        causality: RbcCausality::Local,
        declared_causality: None,
        value_type: TypeId(0),
        scalar_count: 1,
        discrete_input: false,
        contract: None,
        declaration: source_provenance(),
        component: None,
        unit: None,
        physical_quantity: None,
        declaring_class: None,
        description: None,
        binding: None,
        start: None,
        min: None,
        max: None,
        nominal: None,
        fixed: None,
        fixed_elements: None,
        evaluable: false,
        held: false,
        state_select: RbcStateSelect::Default,
        tunable: false,
        from_source: true,
        connector: None,
    }
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

fn errors(model: &RbcModel) -> Vec<ValidationError> {
    validate(model, &ValidateOptions::default()).unwrap_err()
}

// ── Header ───────────────────────────────────────────────────────────────────

#[test]
fn header_rejects_foreign_magic() {
    let mut file = file(decay_model());
    file.magic = "NOT-RBC".into();
    assert!(file.check_header().is_err());
}

#[test]
fn header_rejects_another_version() {
    let mut file = file(decay_model());
    file.bitcode_version = RBC_VERSION + 1;
    assert!(file.check_header().is_err());
}

// ── Discrete value definitions (MLS Appendix B.1c) ───────────────────────────

/// A `discrete_value` variable must arrive with a definition.
///
/// Bitcode v1 originally had no field for B.1c definitions at all, so
/// `Boolean b; b = x < 0.5;` exported a variable nothing defined and
/// reconstruction refused it with "missing B.1c topology definition". The
/// artifact was internally consistent and passed validation, which is what
/// made it hard to see — so validation now states the invariant too.
#[test]
fn a_discrete_value_variable_carries_its_definition() {
    let mut model = decay_model();
    model.types.push(RbcType {
        id: TypeId(1),
        scalar: RbcScalar::Boolean,
        dimensions: Vec::new(),
        record: None,
    });
    let target = VariableId(model.variables.len() as u32);
    let mut flag = variable(target.0, "b", RbcRole::DiscreteValue);
    flag.value_type = TypeId(1);
    model.variables.push(flag);

    let value = ExprId(model.expressions.len() as u32);
    model.expressions.push(RbcExpr {
        id: value,
        value_type: TypeId(1),
        node: RbcExprNode::Literal {
            value: RbcLiteral::Boolean { value: true },
        },
        provenance: source_provenance(),
    });
    model.discrete_definitions.push(RbcDiscreteDefinition {
        observed: false,
        targets: vec![target],
        branches: vec![RbcDiscreteBranch {
            activation: RbcDiscreteActivation::Always,
            values: vec![value],
            provenance: source_provenance(),
        }],
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);

    assert_eq!(model.summary.discrete_definitions, 1);
    assert!(
        validate(&model, &ValidateOptions::default()).is_ok(),
        "a defined discrete variable must validate: {:?}",
        errors(&model)
    );

    // And it must survive both encodings with its activation intact.
    for encoding in [Encoding::Cbor, Encoding::Json] {
        let bytes = encode(&file(model.clone()), encoding).expect("encode");
        let (decoded, _) = decode(&bytes).expect("decode");
        let definition = &decoded.model.discrete_definitions[0];
        assert_eq!(definition.targets, vec![target]);
        assert_eq!(definition.branches[0].values, vec![value]);
        assert!(matches!(
            definition.branches[0].activation,
            RbcDiscreteActivation::Always
        ));
    }
}

/// A `when` branch carries its trigger and guard, which is what distinguishes
/// it from a plain equation on the same target.
#[test]
fn a_when_branch_keeps_its_trigger_and_guard() {
    let branch = RbcDiscreteBranch {
        activation: RbcDiscreteActivation::When {
            trigger: ConditionId(2),
            guard: ConditionId(5),
        },
        values: vec![ExprId(7)],
        provenance: source_provenance(),
    };
    let bytes = serde_json::to_vec(&branch).expect("encode");
    let back: RbcDiscreteBranch = serde_json::from_slice(&bytes).expect("decode");
    assert_eq!(back, branch);
}

/// The invariant that would have caught BUG-009 at the producer.
#[test]
fn a_discrete_value_variable_with_no_definition_is_rejected() {
    let mut model = decay_model();
    model.types.push(RbcType {
        id: TypeId(1),
        scalar: RbcScalar::Boolean,
        dimensions: Vec::new(),
        record: None,
    });
    let id = model.variables.len() as u32;
    let mut flag = variable(id, "b", RbcRole::DiscreteValue);
    flag.value_type = TypeId(1);
    model.variables.push(flag);
    recompute_summary(&mut model);

    assert!(
        errors(&model).iter().any(|error| matches!(
            error,
            ValidationError::UndefinedDiscreteValue { id: reported, .. } if *reported == id
        )),
        "an undefined discrete variable must be a validation error, got {:?}",
        errors(&model)
    );
}

/// A branch must give exactly one value per target.
#[test]
fn a_discrete_branch_must_match_its_target_count() {
    let mut model = decay_model();
    model.types.push(RbcType {
        id: TypeId(1),
        scalar: RbcScalar::Boolean,
        dimensions: Vec::new(),
        record: None,
    });
    let target = VariableId(model.variables.len() as u32);
    let mut flag = variable(target.0, "b", RbcRole::DiscreteValue);
    flag.value_type = TypeId(1);
    model.variables.push(flag);
    model.discrete_definitions.push(RbcDiscreteDefinition {
        observed: false,
        targets: vec![target],
        branches: vec![RbcDiscreteBranch {
            activation: RbcDiscreteActivation::Always,
            values: Vec::new(), // one target, no value
            provenance: source_provenance(),
        }],
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);

    assert!(
        errors(&model)
            .iter()
            .any(|error| matches!(error, ValidationError::DiscreteBranchArity { .. })),
        "arity mismatch must be reported, got {:?}",
        errors(&model)
    );
}

// ── Enumerations ─────────────────────────────────────────────────────────────

/// An enumeration value must not be carried as an `Integer`.
///
/// It was, until a real MSL model (`OpAmpCircuits.Der`, whose
/// `opAmp.homotopyType` is an enumeration parameter) exported cleanly and then
/// failed to import with `expression type mismatch: expected Enumeration,
/// found Integer`. Export and import disagreeing is the one thing an
/// interchange format cannot do, so both directions are pinned here.
#[test]
fn enumeration_literal_keeps_its_type_through_the_codec() {
    let mut model = decay_model();
    model.types.push(RbcType {
        id: TypeId(1),
        scalar: RbcScalar::Enumeration,
        dimensions: Vec::new(),
        record: None,
    });
    let id = ExprId(model.expressions.len() as u32);
    model.expressions.push(RbcExpr {
        id,
        value_type: TypeId(1),
        node: RbcExprNode::Literal {
            value: RbcLiteral::Enumeration { ordinal: 3 },
        },
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);

    for encoding in [Encoding::Cbor, Encoding::Json] {
        let bytes = encode(&file(model.clone()), encoding).expect("encode");
        let (decoded, _) = decode(&bytes).expect("decode");
        let node = &decoded.model.expressions[id.0 as usize].node;
        assert!(
            matches!(
                node,
                RbcExprNode::Literal {
                    value: RbcLiteral::Enumeration { ordinal: 3 }
                }
            ),
            "{encoding:?} degraded an enumeration literal to {node:?}",
        );
    }
}

/// MLS §4.9.5 ordinals are one-based, and `enumeration_literal` is the only
/// DAE constructor that proves it. A zero ordinal must survive the codec so
/// reconstruction is the thing that rejects it, with a span.
#[test]
fn enumeration_ordinal_is_carried_verbatim_for_reconstruction_to_judge() {
    let value = RbcLiteral::Enumeration { ordinal: 0 };
    let bytes = serde_json::to_vec(&value).expect("encode");
    let back: RbcLiteral = serde_json::from_slice(&bytes).expect("decode");
    assert_eq!(back, value);
}

// ── Codec ────────────────────────────────────────────────────────────────────

#[test]
fn both_encodings_carry_the_same_document() {
    let original = file(decay_model());
    for encoding in [Encoding::Cbor, Encoding::Json] {
        let bytes = encode(&original, encoding).expect("encode");
        let (decoded, detected) = decode(&bytes).expect("decode");
        assert_eq!(detected, encoding, "encoding must be detected from bytes");
        assert_eq!(decoded.model.name, original.model.name);
        assert_eq!(
            decoded.model.expressions.len(),
            original.model.expressions.len()
        );
    }
}

#[test]
fn re_encoding_a_decoded_document_is_byte_identical() {
    // The cheap determinism property: whatever a consumer decodes, it can
    // re-encode without drift. Milestone 2 requires deterministic export, and
    // this is the same obligation observed from the other side.
    let original = file(decay_model());
    for encoding in [Encoding::Cbor, Encoding::Json] {
        let first = encode(&original, encoding).expect("encode");
        let (decoded, _) = decode(&first).expect("decode");
        let second = encode(&decoded, encoding).expect("re-encode");
        assert_eq!(first, second, "{encoding:?} round trip must be stable");
    }
}

#[test]
fn garbage_is_rejected_not_guessed() {
    assert!(decode(b"not bitcode at all").is_err());
    assert!(decode(b"{\"magic\": \"RUMOCA-RBC\"").is_err());
}

#[test]
fn transcoding_preserves_fields_this_build_does_not_know() {
    // Forward compatibility: a newer producer may add fields. Changing how an
    // artifact is *stored* must not change what it *contains*, or `convert`
    // becomes a silent downgrade. The typed path deliberately drops them —
    // that is correct when rebuilding a model — so the two paths are asserted
    // against each other here.
    let original = file(decay_model());
    let json = encode(&original, Encoding::Json).expect("encode");
    let mut document: serde_json::Value = serde_json::from_slice(&json).expect("parse");
    document["model"]["variables"][0]["future_field"] = serde_json::json!("v2-data");
    document["model"]["future_collection"] = serde_json::json!([{"id": 0}]);
    let injected = serde_json::to_vec(&document).expect("re-encode");

    for encoding in [Encoding::Cbor, Encoding::Json] {
        let converted = crate::codec::transcode(&injected, encoding).expect("transcode");
        let back = crate::codec::dump_json(&converted).expect("dump");
        let seen: serde_json::Value = serde_json::from_str(&back).expect("parse");
        assert_eq!(
            seen["model"]["variables"][0]["future_field"],
            serde_json::json!("v2-data"),
            "{encoding:?} transcode dropped an unknown field"
        );
        assert!(
            seen["model"]["future_collection"].is_array(),
            "{encoding:?} transcode dropped an unknown collection"
        );
    }

    // The typed path is the contrast: it rebuilds only what it understands.
    let (typed, _) = decode(&injected).expect("decode");
    let typed_json = crate::codec::to_json(&typed).expect("render");
    assert!(
        !typed_json.contains("future_field"),
        "the typed view must not invent fields it cannot interpret"
    );
}

#[test]
fn transcoding_still_refuses_something_that_is_not_bitcode() {
    assert!(crate::codec::transcode(b"not bitcode", Encoding::Json).is_err());
}

// ── Validation: the artifact we produce is valid ─────────────────────────────

#[test]
fn a_well_formed_model_validates() {
    assert!(validate(&decay_model(), &ValidateOptions::default()).is_ok());
}

// ── Adversarial: every case from the project brief's mutation list ───────────

#[test]
fn rejects_reference_to_nonexistent_variable() {
    let mut model = decay_model();
    model.expressions[2].node = RbcExprNode::Coordinate {
        coordinate: RbcCoordinate::State {
            variable: VariableId(92831),
        },
    };
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::DanglingReference {
            target: "variable",
            id: 92831,
            ..
        }
    )));
}

#[test]
fn rejects_duplicate_id() {
    let mut model = decay_model();
    model.variables[1].id = VariableId(0);
    assert!(
        errors(&model)
            .iter()
            .any(|error| matches!(error, ValidationError::NonDenseId { .. }))
    );
}

#[test]
fn rejects_duplicate_variable_name() {
    let mut model = decay_model();
    model.variables[1].name = "x".into();
    assert!(
        errors(&model)
            .iter()
            .any(|error| matches!(error, ValidationError::DuplicateVariableName { .. }))
    );
}

#[test]
fn rejects_invalid_equation_reference() {
    let mut model = decay_model();
    model.equations[0].residual = ExprId(999);
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::DanglingReference {
            target: "expression",
            id: 999,
            ..
        }
    )));
}

#[test]
fn rejects_an_expression_cycle() {
    // Operands must be strictly earlier, which makes a cycle unrepresentable
    // rather than something to detect after the fact.
    let mut model = decay_model();
    model.expressions[3].node = RbcExprNode::Binary {
        op: RbcBinaryOp::Multiply,
        lhs: ExprId(3),
        rhs: ExprId(2),
    };
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::NonTopologicalOperand {
            expression: 3,
            operand: 3
        }
    )));
}

#[test]
fn rejects_forward_operand_reference() {
    let mut model = decay_model();
    model.expressions[0].node = RbcExprNode::Unary {
        op: RbcUnaryOp::Negate,
        operand: ExprId(5),
    };
    assert!(
        errors(&model)
            .iter()
            .any(|error| matches!(error, ValidationError::NonTopologicalOperand { .. }))
    );
}

#[test]
fn rejects_malformed_event() {
    let mut model = decay_model();
    model.events.push(RbcEventAction {
        id: EventId(0),
        trigger: ConditionId(7),
        guard: ConditionId(7),
        action: RbcAction::Reinitialize {
            state: VariableId(0),
            value: ExprId(0),
        },
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::DanglingReference {
            target: "condition",
            ..
        }
    )));
}

#[test]
fn rejects_invalid_connector_reference() {
    let mut model = decay_model();
    model.connections.push(RbcConnection {
        id: ConnectionId(0),
        left: VariableId(0),
        right: VariableId(404),
        quantity: RbcQuantityKind::Potential,
        left_connector: "a.p".into(),
        right_connector: "b.p".into(),
        equation: None,
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::DanglingReference {
            target: "variable",
            id: 404,
            ..
        }
    )));
}

#[test]
fn rejects_trace_point_naming_a_missing_variable() {
    let mut model = decay_model();
    model.trace_points.push(RbcTracePoint {
        id: TracePointId(0),
        variable: VariableId(77),
        label: "ghost".into(),
        connection: None,
        connection_set: None,
        quantity: None,
        unit: None,
        added_by: Some("test".into()),
    });
    recompute_summary(&mut model);
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::DanglingReference {
            target: "variable",
            id: 77,
            ..
        }
    )));
}

#[test]
fn rejects_a_summary_that_disagrees_with_the_contents() {
    // Catches a truncated or hand-edited artifact whose counts no longer match.
    let mut model = decay_model();
    model.summary.variables = 99;
    assert!(errors(&model).iter().any(|error| matches!(
        error,
        ValidationError::SummaryMismatch {
            field: "variables",
            declared: 99,
            ..
        }
    )));
}

#[test]
fn import_rejects_an_unsupported_node() {
    // Export may record an expression form the schema cannot carry. Import must
    // refuse it: a model it cannot faithfully rebuild is not a model.
    let mut model = decay_model();
    model.expressions[4].node = RbcExprNode::Unsupported {
        detail: "array comprehension".into(),
    };
    let strict = ValidateOptions {
        reject_unsupported: true,
    };
    let found = validate(&model, &strict).unwrap_err();
    assert!(found.iter().any(|error| matches!(
        error,
        ValidationError::UnsupportedNode { expression: 4, .. }
    )));
    // The same artifact passes the permissive gate an analysis pass would use.
    assert!(validate(&model, &ValidateOptions::default()).is_ok());
}

#[test]
fn validation_reports_every_problem_not_only_the_first() {
    // A pass author should fix one round of errors, not play whack-a-mole.
    let mut model = decay_model();
    model.equations[0].residual = ExprId(900);
    model.variables[1].value_type = TypeId(900);
    let found = errors(&model);
    assert!(found.len() >= 2, "expected several errors, got {found:?}");
}

// ── Import ───────────────────────────────────────────────────────────────────

#[test]
fn import_rebuilds_a_checked_dae() {
    let file = file(decay_model());
    let dae = crate::import(&file).expect("import a well-formed artifact");
    let (variables, equations) =
        dae.inspect(|view| (view.variable_count(), view.continuous_equation_count()));
    assert_eq!(variables, 2);
    assert_eq!(equations, 1);
}

#[test]
fn import_preserves_static_and_dynamic_time_event_owners() {
    let mut model = decay_model();
    model.time_events = vec![
        RbcTimeEvent {
            id: EventId(0),
            schedule: RbcSchedule::Static {
                numerator: 1,
                denominator: 10,
            },
            provenance: source_provenance(),
        },
        RbcTimeEvent {
            id: EventId(1),
            schedule: RbcSchedule::Dynamic {
                deadline: ExprId(0),
            },
            provenance: source_provenance(),
        },
    ];
    recompute_summary(&mut model);
    let dae = crate::import(&file(model)).expect("time schedules rebuild through checked owners");
    dae.inspect(|view| {
        assert_eq!(view.time_event_count(), 2);
        let first = view.time_event(view.time_event_id(0).unwrap()).unwrap();
        let instant = first.instant().unwrap();
        assert_eq!((instant.numerator(), instant.denominator()), (1, 10));
        assert!(
            view.time_event(view.time_event_id(1).unwrap())
                .unwrap()
                .deadline()
                .is_some()
        );
    });
}

#[test]
fn import_refuses_a_dangling_reference() {
    let mut model = decay_model();
    model.equations[0].residual = ExprId(999);
    let error = crate::import(&file(model)).expect_err("must refuse");
    assert!(
        matches!(error, crate::ImportError::Invalid(_)),
        "expected a validation failure, got {error}"
    );
}

#[test]
fn export_import_export_is_stable() {
    // The Milestone 3 obligation, stated as a property: reconstructing a DAE
    // and re-exporting it reproduces the artifact it came from.
    let original = file(decay_model());
    let dae = crate::import(&original).expect("import");
    let again =
        crate::export(&dae, None, &original.model.name, &Default::default()).expect("re-export");
    assert_eq!(original.model.variables.len(), again.model.variables.len());
    assert_eq!(
        original.model.expressions.len(),
        again.model.expressions.len()
    );
    for (before, after) in original.model.variables.iter().zip(&again.model.variables) {
        assert_eq!(before.name, after.name);
        assert_eq!(before.role, after.role);
    }
    for (before, after) in original
        .model
        .expressions
        .iter()
        .zip(&again.model.expressions)
    {
        assert_eq!(format!("{:?}", before.node), format!("{:?}", after.node));
    }
}

#[test]
fn unary_plus_survives_a_round_trip() {
    // MLS §3.4 unary plus reaches the DAE intact whenever constant folding does
    // not consume it, which `--no-fold-parameter-bindings` arranges for every
    // Real parameter. `Modelica.Electrical.Analog.Examples.InvertingAmp`
    // declares `parameter SI.Voltage Vps=+15` and
    // `...OpAmps.Comparator` did likewise: both exported, then failed their own
    // import with "unary operator not in bitcode v1", so a model that
    // simulated by default stopped simulating under the flag.
    let mut model = decay_model();
    let operand = ExprId(model.expressions.len() as u32 - 1);
    model.expressions.push(RbcExpr {
        id: ExprId(model.expressions.len() as u32),
        value_type: TypeId(0),
        node: RbcExprNode::Unary {
            op: RbcUnaryOp::Plus,
            operand,
        },
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);

    let original = file(model);
    let dae = crate::import(&original).expect("unary plus must import");
    let again =
        crate::export(&dae, None, &original.model.name, &Default::default()).expect("re-export");
    assert!(
        again.model.expressions.iter().any(|expression| matches!(
            expression.node,
            RbcExprNode::Unary {
                op: RbcUnaryOp::Plus,
                ..
            }
        )),
        "unary plus must survive the round trip rather than be dropped or refused"
    );
}

#[test]
fn textual_ir_round_trips_exactly() {
    // The property that makes it an IR rather than a listing: print, parse,
    // and the artifact is the one you started with.
    let original = file(decay_model());
    let text = crate::text::print_text_with(&original, crate::text::TextOptions { sources: true })
        .expect("print");
    let parsed = crate::text::parse_text(&text).expect("parse");

    assert_eq!(original.model.name, parsed.model.name);
    assert_eq!(original.model.variables.len(), parsed.model.variables.len());
    assert_eq!(
        original.model.expressions.len(),
        parsed.model.expressions.len()
    );
    assert_eq!(original.model.equations.len(), parsed.model.equations.len());
    // Serialize both: field-by-field equality is what "exactly" has to mean,
    // and comparing the encodings checks every field including ones added
    // after this test was written.
    let left = serde_json::to_value(&original.model).expect("encode original");
    let right = serde_json::to_value(&parsed.model).expect("encode parsed");
    assert_eq!(
        left, right,
        "textual round-trip must preserve the whole model"
    );
}

#[test]
fn textual_ir_omits_only_source_text_by_default() {
    // Source text is most of an artifact's bytes and none of its semantics, so
    // it is opt-in. That is the *only* thing the default drops, and this pins
    // it: anything else going missing is a silent loss, which the format is
    // supposed to make impossible.
    let original = file(decay_model());
    let text = crate::text::print_text(&original).expect("print");
    let mut parsed = crate::text::parse_text(&text).expect("parse");

    assert!(
        original.model.sources.iter().any(|s| s.text.is_some()),
        "the fixture must carry source text for this to test anything"
    );
    assert!(
        parsed.model.sources.iter().all(|s| s.text.is_none()),
        "the default must omit source text"
    );

    for (source, restored) in original
        .model
        .sources
        .iter()
        .zip(parsed.model.sources.iter_mut())
    {
        restored.text = source.text.clone();
    }
    assert_eq!(
        serde_json::to_value(&original.model).expect("encode original"),
        serde_json::to_value(&parsed.model).expect("encode parsed"),
        "with source text put back, nothing else differs"
    );
}

#[test]
fn textual_ir_reports_the_line_of_a_syntax_error() {
    let text = "rbc 2\nproducer \"x\"\nmodel \"M\"\n$0 type nonsense\n";
    let error = crate::text::parse_text(text).expect_err("must refuse");
    assert_eq!(error.line, 4, "the error must name the offending line");
    assert!(
        error.message.contains("nonsense"),
        "and quote what it saw: {error}"
    );
}

#[test]
fn textual_ir_refuses_an_unclosed_block() {
    let text = "rbc 2\nmodel \"M\"\ndisc 1 targets %0\n";
    let error = crate::text::parse_text(text).expect_err("must refuse");
    assert!(error.message.contains("never closed"), "got: {error}");
}

#[test]
fn connection_ids_are_dense_after_filtering() {
    // `export_connections` numbered by the pre-filter index, so an endpoint
    // that is not a DAE variable — a clocked signal removed during lowering —
    // dropped its entry and left a hole. `SubSample` exported `[0, 2]`, and
    // the artifact then failed its own validator, which requires
    // `position == id`. 28 of 35 Clocked models were unloadable for this.
    //
    // The property is structural, so it is asserted structurally rather than
    // by rebuilding that model: ids must be 0..n over whatever survives.
    let mut model = decay_model();
    model.connections = vec![
        RbcConnection {
            id: ConnectionId(0),
            left: VariableId(0),
            right: VariableId(1),
            quantity: RbcQuantityKind::Potential,
            left_connector: "a".into(),
            right_connector: "b".into(),
            equation: None,
            provenance: source_provenance(),
        },
        RbcConnection {
            id: ConnectionId(2), // the hole a pre-filter index leaves
            left: VariableId(0),
            right: VariableId(1),
            quantity: RbcQuantityKind::Potential,
            left_connector: "c".into(),
            right_connector: "d".into(),
            equation: None,
            provenance: source_provenance(),
        },
    ];
    recompute_summary(&mut model);

    let failures = errors(&model);
    assert!(
        failures
            .iter()
            .any(|e| e.to_string().contains("connections entry at position 1")),
        "a hole in the connection ids must be a validation error, got {failures:?}"
    );

    model.connections[1].id = ConnectionId(1);
    assert!(
        validate(&model, &ValidateOptions::default()).is_ok(),
        "dense ids must validate"
    );
}

#[test]
fn equation_families_are_exported_and_round_trip() {
    // TOOLBUG-014. Array and `for` equations live in the DAE as *families*,
    // and `export_equations` only walked the scalar list, so they were absent:
    // `Real x[3]` with a `for` equation exported three unknowns and zero
    // equations, and the artifact validated cleanly. Any consumer reasoning
    // about solvability read an incomplete system with no way to detect it.
    let mut model = decay_model();
    let body = ExprId(model.expressions.len() as u32 - 1);
    model.domains.push(loop_domain(0, 3));
    model.equation_families.push(RbcEquationFamily {
        id: FamilyId(0),
        domain: DomainId(0),
        bodies: vec![body],
        scalar_rows: 3,
        extents: vec![3],
        scalar_view: RbcScalarView::BinderSubstitution,
        reads: Vec::new(),
        reads_derivative: Vec::new(),
        reads_previous: Vec::new(),
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);

    // The summary must count the rows, not the families: a balance check needs
    // to know the family stands for three equations, not one.
    assert_eq!(model.summary.equation_families, 1);
    assert_eq!(model.summary.family_scalar_rows, 3);

    // Import rebuilds it. Earlier the schema carried no domain, so import had
    // to refuse outright: a DAE reconstructed without the domain would be
    // missing three equations and would still validate, which is the bug this
    // whole change exists to remove.
    let original = file(model);
    let dae = crate::import(&original).expect("a family with its domain imports");
    let again =
        crate::export(&dae, None, "test", &crate::ExportOptions::default()).expect("re-export");
    assert_eq!(
        again.model.summary.family_scalar_rows, 3,
        "the rebuilt DAE must still stand for three scalar equations"
    );
}

/// One `for i in 1:extent` axis.
fn loop_domain(id: u32, extent: i64) -> RbcDomain {
    RbcDomain {
        id: DomainId(id),
        binders: vec![RbcBinder {
            id: 0,
            display_name: "i".into(),
            lower: 1,
            upper: extent,
            step: 1,
        }],
        parent: None,
        extents: vec![extent as u32],
        scalar_count: extent as u32,
        provenance: source_provenance(),
    }
}

#[test]
fn the_textual_ir_carries_equation_families() {
    // The textual form is the other place a family could silently vanish.
    let mut model = decay_model();
    let body = ExprId(model.expressions.len() as u32 - 1);
    model.domains.push(loop_domain(0, 4));
    model.equation_families.push(RbcEquationFamily {
        id: FamilyId(0),
        domain: DomainId(0),
        bodies: vec![body],
        scalar_rows: 4,
        extents: vec![2, 2],
        scalar_view: RbcScalarView::BinderPrefixProjection { binder_count: 1 },
        reads: Vec::new(),
        reads_derivative: Vec::new(),
        reads_previous: Vec::new(),
        provenance: source_provenance(),
    });
    recompute_summary(&mut model);

    let original = file(model);
    let text = crate::text::print_text_with(&original, crate::text::TextOptions { sources: true })
        .expect("print");
    let parsed = crate::text::parse_text(&text).expect("parse");
    assert_eq!(
        serde_json::to_value(&original.model).expect("encode original"),
        serde_json::to_value(&parsed.model).expect("encode parsed"),
        "a family, its extents and its scalar view must all survive the text form"
    );
}

// ── Computational power ──────────────────────────────────────────────────────
//
// Rumoca Bitcode is deliberately *not* Turing complete, and every static
// analysis built on it depends on that: a witness search, an interval
// propagation and a maximum-flow matching all terminate without a step budget
// because the artifact they read cannot express unbounded iteration.
//
// That is a property of the schema, so it is checked against the schema. The
// classification below is an exhaustive `match` with no wildcard arm: adding a
// node kind does not silently inherit "bounded", it fails to compile until
// somebody classifies it. A doc comment claiming totality would not have that
// property, which is why this is a test and not a paragraph.

/// What a construct can cost to evaluate.
#[derive(Debug, PartialEq, Eq)]
enum Cost {
    /// Bounded by the size of the artifact: one forward pass over the arena.
    Bounded,
    /// Bounded by an index domain whose extents the artifact fixes.
    BoundedByDomain,
    /// Calls out of the artifact. The callee's cost is not ours to bound, and
    /// this is one of the three named holes in the totality claim.
    Opaque,
    /// Would introduce unbounded iteration. Nothing may be classified here.
    Unbounded,
}

fn cost_of_node(node: &RbcExprNode) -> Cost {
    match node {
        RbcExprNode::Literal { .. } => Cost::Bounded,
        RbcExprNode::StringConversion { .. } => Cost::Bounded,
        // One value read from a clock the artifact declares.
        RbcExprNode::ClockTransfer { .. } => Cost::Bounded,
        RbcExprNode::Coordinate { .. } => Cost::Bounded,
        RbcExprNode::Unary { .. } => Cost::Bounded,
        RbcExprNode::Binary { .. } => Cost::Bounded,
        RbcExprNode::Conditional { .. } => Cost::Bounded,
        RbcExprNode::Builtin { .. } => Cost::Bounded,
        RbcExprNode::Array { .. } => Cost::Bounded,
        RbcExprNode::Record { .. } => Cost::Bounded,
        RbcExprNode::Field { .. } => Cost::Bounded,
        RbcExprNode::Range { .. } => Cost::Bounded,
        // The only iteration in the language, and its trip count is the
        // domain's extent, which the artifact carries as a constant.
        RbcExprNode::Comprehension { .. } => Cost::BoundedByDomain,
        RbcExprNode::Index { .. } => Cost::Bounded,
        RbcExprNode::ArrayUpdate { .. } => Cost::Bounded,
        // A call names a function whose body this artifact does not carry.
        RbcExprNode::Call { .. } => Cost::Opaque,
        // Leaves. Each reads one value a function body already defined,
        // named by owner-local ordinal: no operand to walk and no
        // iteration. The cost of *producing* that value is the body's, and
        // `cost_of_body` classifies it there.
        RbcExprNode::FunctionValue { .. }
        | RbcExprNode::FunctionFoldParameter { .. }
        | RbcExprNode::FunctionFoldOutput { .. } => Cost::Bounded,
        RbcExprNode::Unsupported { .. } => Cost::Opaque,
    }
}

fn cost_of_body(body: &RbcFunctionBody) -> Cost {
    match body {
        // The body exists and is not here, so nothing in the artifact can
        // recurse: a call is a leaf as far as this IR is concerned.
        RbcFunctionBody::ElidedModelica => Cost::Opaque,
        RbcFunctionBody::External { .. } => Cost::Opaque,
        // A carried body is bounded *per body*: every statement form is
        // bounded, checked exhaustively below. What a carried body changes
        // is that a call is no longer a leaf -- the callee's body is now here
        // to follow -- so termination across calls rests on
        // `check_call_graph_acyclic`, enforced for any artifact that carries
        // one, rather than on bodies being absent.
        RbcFunctionBody::Modelica { statements } => {
            if statements.iter().all(statement_is_bounded) {
                Cost::Bounded
            } else {
                Cost::Opaque
            }
        }
    }
}

/// Exhaustive on purpose: a fifth statement form fails to compile here until
/// someone decides whether it can iterate without a bound.
fn statement_is_bounded(statement: &RbcFunctionStatement) -> bool {
    match statement {
        // Evaluate their expressions once each.
        RbcFunctionStatement::Assignment { .. }
        | RbcFunctionStatement::Assertion { .. }
        | RbcFunctionStatement::AssignmentGroup { .. } => true,
        // A fold over a compact domain: the trip count is the domain's,
        // fixed before evaluation exactly as for `Comprehension`.
        RbcFunctionStatement::For { statements, .. } => statements.iter().all(statement_is_bounded),
    }
}

#[test]
fn no_expression_node_can_iterate_without_a_bound() {
    // The enforcement is the exhaustive `match` in `cost_of_node`: a new node
    // kind fails to compile until it is classified, so this claim cannot rot
    // the way a doc comment would. What this test adds is that the
    // classification is actually applied to real nodes and that nothing in a
    // built artifact lands in the one category that would break totality.
    let model = decay_model();
    assert!(!model.expressions.is_empty(), "the fixture must have nodes");
    for expression in &model.expressions {
        assert_ne!(
            cost_of_node(&expression.node),
            Cost::Unbounded,
            "expression {} would make the IR Turing complete; every static \
             analysis here assumes an artifact is total",
            expression.id.0
        );
    }
    for function in &model.functions {
        assert_ne!(cost_of_body(&function.body), Cost::Unbounded);
    }
}

#[test]
fn the_expression_arena_cannot_hold_a_cycle() {
    // Not "does not", *cannot*: operands must be strictly earlier, so a cycle
    // is unrepresentable rather than something a checker has to find. This is
    // the structural reason evaluation is one forward pass.
    let mut model = decay_model();
    let last = model.expressions.len() - 1;
    model.expressions[last].node = RbcExprNode::Binary {
        op: RbcBinaryOp::Add,
        lhs: ExprId(last as u32), // itself
        rhs: ExprId(0),
    };
    let errors = validate(&model, &ValidateOptions::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, ValidationError::NonTopologicalOperand { .. })),
        "a self-referencing operand must be rejected, got {errors:?}"
    );

    let mut forward = decay_model();
    forward.expressions[0].node = RbcExprNode::Unary {
        op: RbcUnaryOp::Negate,
        operand: ExprId(forward.expressions.len() as u32 - 1),
    };
    assert!(
        validate(&forward, &ValidateOptions::default()).is_err(),
        "a forward operand reference must be rejected too"
    );
}

#[test]
fn a_carried_function_body_cannot_recurse_through_the_call_graph() {
    // Bodies are carried now, so recursion is no longer ruled out by their
    // absence. It is ruled out by `check_call_graph_acyclic`: a function
    // that calls itself is rejected before any consumer can evaluate it.
    let mut model = decay_model();
    let real = model.variables[0].value_type;
    model.functions.push(RbcFunction {
        folds: Vec::new(),
        values: Vec::new(),
        calls: vec![FunctionId(0)],
        id: FunctionId(0),
        name: "f".into(),
        parameters: vec![],
        results: vec![real],
        inline: RbcInline::Unstated,
        derivatives: Vec::new(),
        body: RbcFunctionBody::Modelica { statements: vec![] },
        declaration: source_provenance(),
    });
    recompute_summary(&mut model);
    let errors = errors(&model);
    assert!(
        errors.iter().any(|e| e.to_string().contains("acyclic")),
        "{errors:?}"
    );
}

// ── Building ─────────────────────────────────────────────────────────────────

#[test]
fn a_model_can_be_built_from_nothing_and_is_valid() {
    // The format calls itself an interchange format. Until `build` there was
    // no way for a Rust consumer to produce an artifact except to write out
    // every field of `RbcModel`, which meant in practice that only the Rumoca
    // compiler could write one.
    let mut builder = Builder::new("Built");
    let x = builder.state("x", 1.0);
    let k = builder.parameter("k", 2.0);
    let scaled = {
        let left = builder.parameter_ref(k);
        let right = builder.state_ref(x);
        builder.binary(RbcBinaryOp::Multiply, left, right)
    };
    let rhs = builder.negate_of(scaled);
    builder.derivative_equation(x, rhs);
    let model = builder.finish();

    assert!(
        validate(&model, &ValidateOptions::default()).is_ok(),
        "a built model must validate: {:?}",
        validate(&model, &ValidateOptions::default())
    );
    assert_eq!(model.variables.len(), 2);
    assert_eq!(model.equations.len(), 1);
    assert_eq!(
        model.summary.equations, 1,
        "finish() recomputes the summary the validator checks"
    );
}

#[test]
fn the_builder_derives_what_an_equation_reads() {
    // The schema carries `reads` so that no consumer has to re-derive it by
    // walking expressions, which means a producer that omits it leaves every
    // dependency analysis with a hole.
    let mut builder = Builder::new("Reads");
    let x = builder.state("x", 0.0);
    let k = builder.parameter("k", 1.0);
    let scaled = {
        let left = builder.parameter_ref(k);
        let right = builder.state_ref(x);
        builder.binary(RbcBinaryOp::Multiply, left, right)
    };
    builder.derivative_equation(x, scaled);
    let model = builder.finish();

    let equation = &model.equations[0];
    assert_eq!(equation.reads, vec![x, k], "both values are read");
    assert_eq!(
        equation.reads_derivative,
        vec![x],
        "and the derivative is recorded apart from the value"
    );
}

#[test]
#[should_panic(expected = "not strictly earlier")]
fn the_builder_refuses_a_forward_operand() {
    // The arena's topological order is what makes a cycle unrepresentable.
    // Validation catches a violation, but much later and by node id; this
    // catches it at the call that made the mistake.
    let mut builder = Builder::new("Forward");
    builder.expr(RbcExprNode::Unary {
        op: RbcUnaryOp::Negate,
        operand: ExprId(99),
    });
}

#[test]
fn everything_the_builder_adds_is_marked_generated() {
    // An entity a tool produced must be distinguishable from one a modeller
    // wrote. A pass that leaves `Source` provenance on its own additions is
    // indistinguishable from a compiler bug.
    let mut builder = Builder::new("Marked");
    let x = builder.state("x", 0.0);
    let zero = builder.real(0.0);
    builder.derivative_equation(x, zero);
    let model = builder.finish();

    for equation in &model.equations {
        assert!(
            matches!(equation.provenance.origin, RbcOrigin::Generated { .. }),
            "an equation the builder added claims to come from source"
        );
    }
    for expression in &model.expressions {
        assert!(matches!(
            expression.provenance.origin,
            RbcOrigin::Generated { .. }
        ));
    }
}

#[test]
fn operands_lists_every_child_of_every_node_kind() {
    // `operands` is how the builder checks the topological rule and how a
    // rewrite finds a path, so a node kind missing from it is walked as a leaf
    // and both silently do the wrong thing. The exhaustive `match` inside is
    // the guard; this pins the shapes that carry children.
    let mut builder = Builder::new("Operands");
    let a = builder.real(1.0);
    let b = builder.real(2.0);
    let sum = builder.binary(RbcBinaryOp::Add, a, b);
    let negated = builder.negate_of(sum);
    let model = builder.finish();

    let node = &model.expressions[sum.0 as usize].node;
    assert_eq!(crate::build::operands(node), vec![a, b]);
    let node = &model.expressions[negated.0 as usize].node;
    assert_eq!(crate::build::operands(node), vec![sum]);
    let node = &model.expressions[a.0 as usize].node;
    assert!(
        crate::build::operands(node).is_empty(),
        "a literal is a leaf"
    );
}

#[test]
fn a_subscript_is_an_operand() {
    // `x[i]` reads `i`. A walk that skipped subscripts computed read sets
    // and override dependencies that missed every variable used only as an
    // index.
    let mut builder = Builder::new("Subscripts");
    let base = builder.real(1.0);
    let index = builder.real(2.0);
    let node = RbcExprNode::Index {
        base,
        subscripts: vec![
            RbcSubscript::Index { expression: index },
            RbcSubscript::Whole,
        ],
    };
    assert_eq!(crate::build::operands(&node), vec![base, index]);
}

#[test]
fn a_call_projection_depends_on_its_head() {
    let head = ExprId(4);
    let projection = RbcExprNode::Call {
        owner: head,
        function: FunctionId(0),
        output: 1,
        arguments: vec![],
    };
    assert_eq!(crate::build::references(ExprId(5), &projection), vec![head]);
    let own_head = RbcExprNode::Call {
        owner: head,
        function: FunctionId(0),
        output: 0,
        arguments: vec![ExprId(1)],
    };
    assert_eq!(
        crate::build::references(head, &own_head),
        vec![ExprId(1)],
        "a head names itself as owner, which is not a dependency"
    );
}

#[test]
fn a_pass_can_continue_from_an_artifact_it_did_not_build() {
    // The instrumentation case: a pass receives a compiled model and adds to
    // it. `Builder::new` is for a producer starting from nothing;
    // `from_model` is for everything that arrives already built, which is the
    // common one and the reason the entry point exists.
    let original = decay_model();
    let variables_before = original.variables.len();
    let equations_before = original.equations.len();

    let mut builder = Builder::from_model(original);
    let observed = builder.state("integral_of_x", 0.0);
    let x = VariableId(0);
    let value = builder.state_ref(x);
    builder.derivative_equation(observed, value);
    builder.trace_point(observed, "integral of x", "test");
    let model = builder.finish();

    assert_eq!(model.variables.len(), variables_before + 1);
    assert_eq!(model.equations.len(), equations_before + 1);
    assert!(
        validate(&model, &ValidateOptions::default()).is_ok(),
        "the instrumented model must still validate: {:?}",
        validate(&model, &ValidateOptions::default())
    );
    assert_eq!(
        model.trace_points.len(),
        1,
        "and carry the observation the pass asked for"
    );
}

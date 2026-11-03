use super::*;
use crate::build::Builder;
use crate::codec::{Encoding, decode, encode};
use serde_json::json;

fn fixture() -> RbcFile {
    let mut b = Builder::new("Decay");
    let x = b.state("x", 2.0);
    let k = b.parameter("k", 3.0);
    let xr = b.state_ref(x);
    let kr = b.parameter_ref(k);
    let product = b.binary(RbcBinaryOp::Multiply, xr, kr);
    let rhs = b.negate_of(product);
    b.derivative_equation(x, rhs);
    b.trace_point(x, "x", "test");
    RbcFile {
        execution: None,
        magic: RBC_MAGIC.into(),
        bitcode_version: RBC_VERSION,
        producer: "test".into(),
        model: b.finish(),
    }
}

fn twice(file: &RbcFile) -> RbcFile {
    link(
        "Combined",
        &[
            LinkInput {
                namespace: "a",
                file,
            },
            LinkInput {
                namespace: "b",
                file,
            },
        ],
        false,
    )
    .unwrap()
}

#[test]
fn independent_instances_import_and_preserve_sources_and_literals() {
    let file = fixture();
    let before = encode(&file, Encoding::Json).unwrap();
    let linked = twice(&file);
    assert_eq!(
        linked
            .model
            .variables
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["a.x", "a.k", "b.x", "b.k"]
    );
    assert_eq!(linked.model.trace_points[1].label, "b.x");
    assert_eq!(linked.model.trace_points[1].variable, VariableId(2));
    assert_eq!(linked.model.sources[0].name, linked.model.sources[1].name);
    assert_eq!(
        linked.model.variables[2].declaration.span.source,
        SourceId(1)
    );
    assert_eq!(linked.model.summary.states, 2);
    assert_eq!(linked.model.summary.equations, 2);
    assert!(linked.execution.is_none());
    crate::import(&linked)
        .expect("linked artifact reconstructs through canonical DAE constructors");
    assert_eq!(before, encode(&file, Encoding::Json).unwrap());
    assert_eq!(
        encode(&linked, Encoding::Cbor).unwrap(),
        encode(&twice(&file), Encoding::Cbor).unwrap()
    );
    for encoding in [Encoding::Json, Encoding::Cbor] {
        let restored = decode(&encode(&linked, encoding).unwrap()).unwrap().0;
        assert_eq!(
            encode(&restored, Encoding::Json).unwrap(),
            encode(&linked, Encoding::Json).unwrap()
        );
    }
}

#[test]
fn invalid_headers_names_and_references_are_rejected() {
    let mut file = fixture();
    assert!(link("X", &[], false).is_err());
    for namespace in ["", "a.b", "0a", "a=b", "../a"] {
        assert!(
            link(
                "X",
                &[LinkInput {
                    namespace,
                    file: &file
                }],
                false
            )
            .is_err()
        );
    }
    let duplicate = [
        LinkInput {
            namespace: "a",
            file: &file,
        },
        LinkInput {
            namespace: "a",
            file: &file,
        },
    ];
    assert!(link("X", &duplicate, false).is_err());
    file.bitcode_version = 999;
    assert!(
        link(
            "X",
            &[LinkInput {
                namespace: "a",
                file: &file
            }],
            false
        )
        .is_err()
    );
    file.bitcode_version = RBC_VERSION;
    file.model.variables[0].start = Some(ExprId(u32::MAX));
    assert!(
        link(
            "X",
            &[LinkInput {
                namespace: "a",
                file: &file
            }],
            false
        )
        .is_err()
    );
}

#[test]
fn range_checks_reject_dangling_ids_and_overflow() {
    assert!(Range::new(u32::MAX as usize, 1, "test").is_err());
    let range = Range::new(100, 5, "test").unwrap();
    let mut id = 4;
    range.shift(&mut id).unwrap();
    assert_eq!(id, 104);
    assert!(range.shift(&mut 5).is_err());
}

#[test]
fn reject_non_dense_initial_ids_and_compact_row_count_overflow() {
    let mut file = fixture();
    let mut equation = file.model.equations[0].clone();
    equation.id = EquationId(1);
    file.model.initial_equations.push(equation);
    recompute_summary(&mut file.model);
    let error = link(
        "X",
        &[LinkInput {
            namespace: "x",
            file: &file,
        }],
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("initial_equations"));
    file.model.initial_equations.clear();
    let p = file.model.equations[0].provenance;
    file.model.domains.push(RbcDomain {
        id: DomainId(0),
        binders: vec![],
        parent: None,
        extents: vec![u32::MAX],
        scalar_count: u32::MAX,
        provenance: p,
    });
    file.model.equation_families.push(RbcEquationFamily {
        id: FamilyId(0),
        domain: DomainId(0),
        bodies: vec![ExprId(0)],
        scalar_rows: u32::MAX,
        extents: vec![u32::MAX],
        scalar_view: RbcScalarView::RowMajorProjection,
        reads: vec![],
        reads_derivative: vec![],
        reads_previous: vec![],
        provenance: p,
    });
    recompute_summary(&mut file.model);
    let error = link(
        "X",
        &[
            LinkInput {
                namespace: "x",
                file: &file,
            },
            LinkInput {
                namespace: "y",
                file: &file,
            },
        ],
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("row count overflow"));
}

// Exercise every expression/coordinate shape against deliberately different
// offsets, including fields whose numeric values must NOT be treated as IDs.
fn map_fixture() -> (RbcModel, RbcModel) {
    let base = fixture().model;
    let mut input = base.clone();
    let provenance = input.variables[0].declaration;
    input.domains.push(RbcDomain {
        id: DomainId(0),
        binders: vec![],
        parent: None,
        extents: vec![],
        scalar_count: 1,
        provenance,
    });
    input.functions.push(RbcFunction {
        folds: Vec::new(),
        values: Vec::new(),
        calls: Vec::new(),
        id: FunctionId(0),
        name: "f".into(),
        parameters: vec![],
        results: vec![TypeId(0)],
        inline: RbcInline::Unstated,
        derivatives: Vec::new(),
        body: RbcFunctionBody::External {
            language: "C".into(),
            symbol: "external_f".into(),
            purity: RbcPurity::Impure,
            arguments: Vec::new(),
            result: None,
            linkage: RbcExternalLinkage::default(),
        },
        declaration: provenance,
    });
    input.conditions.push(RbcCondition {
        id: ConditionId(0),
        node: RbcConditionNode::Always,
        provenance,
    });
    (input.clone(), input)
}

#[test]
// Every node variant is relocated and checked in one body, so a variant added
// to the schema and forgotten here is a visible gap rather than a silent one.
// SPEC_0021: Exception - an exhaustive expression-shape regression matrix.
#[allow(clippy::too_many_lines)]
fn all_expression_shapes_relocate_only_typed_references() {
    let (out, input) = map_fixture();
    let map = Map::new(&out, &input, "m").unwrap();
    let e = out.expressions.len();
    let cases = vec![
        (
            json!({"kind":"literal","value":{"kind":"integer","value":1}}),
            json!({"kind":"literal","value":{"kind":"integer","value":1}}),
        ),
        (
            json!({"kind":"unary","op":"negate","operand":0}),
            json!({"kind":"unary","op":"negate","operand":e}),
        ),
        (
            json!({"kind":"binary","op":"add","lhs":0,"rhs":1}),
            json!({"kind":"binary","op":"add","lhs":e,"rhs":e+1}),
        ),
        (
            json!({"kind":"conditional","branches":[{"condition":0,"value":1}],"fallback":0}),
            json!({"kind":"conditional","branches":[{"condition":e,"value":e+1}],"fallback":e}),
        ),
        (
            json!({"kind":"builtin","name":"sin","arguments":[0]}),
            json!({"kind":"builtin","name":"sin","arguments":[e]}),
        ),
        (
            json!({"kind":"array","elements":[0],"empty_type":null}),
            json!({"kind":"array","elements":[e],"empty_type":null}),
        ),
        (
            json!({"kind":"array","elements":[],"empty_type":0}),
            json!({"kind":"array","elements":[],"empty_type":1}),
        ),
        (
            json!({"kind":"record","ty":0,"fields":[0]}),
            json!({"kind":"record","ty":1,"fields":[e]}),
        ),
        (
            json!({"kind":"field","base":0,"field":17}),
            json!({"kind":"field","base":e,"field":17}),
        ),
        (
            json!({"kind":"range","start":0,"step":1,"stop":0}),
            json!({"kind":"range","start":e,"step":e+1,"stop":e}),
        ),
        (
            json!({"kind":"comprehension","domain":0,"body":0}),
            json!({"kind":"comprehension","domain":1,"body":e}),
        ),
        (
            json!({"kind":"index","base":0,"subscripts":[{"kind":"index","expression":1},{"kind":"whole"},{"kind":"slice","expression":0}]}),
            json!({"kind":"index","base":e,"subscripts":[{"kind":"index","expression":e+1},{"kind":"whole"},{"kind":"slice","expression":e}]}),
        ),
        (
            json!({"kind":"array_update","base":0,"value":1,"subscripts":[]}),
            json!({"kind":"array_update","base":e,"value":e+1,"subscripts":[]}),
        ),
        (
            json!({"kind":"call","owner":0,"function":0,"output":7,"arguments":[1]}),
            json!({"kind":"call","owner":e,"function":1,"output":7,"arguments":[e+1]}),
        ),
    ];
    for (original, expected) in cases {
        let mut node: RbcExprNode = serde_json::from_value(original.clone()).unwrap();
        node.shift(&map).unwrap();
        assert_eq!(serde_json::to_value(node).unwrap(), expected, "{original}");
    }
    for kind in [
        "parameter",
        "input",
        "state",
        "derivative",
        "algebraic",
        "discrete_real",
        "discrete_value",
        "pre_state",
        "pre_algebraic",
        "pre_discrete_real",
        "pre_discrete_value",
    ] {
        let mut node: RbcCoordinate =
            serde_json::from_value(json!({"kind":kind,"variable":0})).unwrap();
        node.shift(&map).unwrap();
        assert_eq!(
            serde_json::to_value(node).unwrap(),
            json!({"kind":kind,"variable":2})
        );
    }
    for (original, expected) in [
        (json!({"kind":"time"}), json!({"kind":"time"})),
        (
            json!({"kind":"binder","domain":0,"ordinal":9}),
            json!({"kind":"binder","domain":1,"ordinal":9}),
        ),
        (
            json!({"kind":"condition","condition":0}),
            json!({"kind":"condition","condition":1}),
        ),
        (
            json!({"kind":"function_parameter","function":0,"ordinal":9}),
            json!({"kind":"function_parameter","function":1,"ordinal":9}),
        ),
    ] {
        let mut node: RbcCoordinate = serde_json::from_value(original).unwrap();
        node.shift(&map).unwrap();
        assert_eq!(serde_json::to_value(node).unwrap(), expected);
    }
}

#[test]
fn event_condition_and_action_variants_relocate() {
    let (mut out, input) = map_fixture();
    out.relations.push(RbcRelation {
        id: RelationId(0),
        expression: ExprId(0),
        provenance: input.variables[0].declaration,
    });
    let mut input = input;
    input.relations = out.relations.clone();
    let map = Map::new(&out, &input, "m").unwrap();
    for kind in ["and", "or", "any_rise"] {
        let mut node: RbcConditionNode =
            serde_json::from_value(json!({"kind":kind,"lhs":0,"rhs":0})).unwrap();
        node.shift(&map).unwrap();
        assert_eq!(
            serde_json::to_value(node).unwrap(),
            json!({"kind":kind,"lhs":1,"rhs":1})
        );
    }
    let mut action = RbcAction::Reinitialize {
        state: VariableId(0),
        value: ExprId(1),
    };
    action.shift(&map).unwrap();
    assert_eq!(
        serde_json::to_value(action).unwrap(),
        json!({"kind":"reinitialize","state":2,"value":out.expressions.len()+1})
    );
    let mut opaque = RbcConditionNode::Unsupported {
        detail: "future".into(),
    };
    assert!(opaque.shift(&map).is_err());
}

#[test]
// The point is that every table's ids relocate together; splitting this would
// let one table's check pass while another's regressed.
// SPEC_0021: Exception - an exhaustive cross-table relocation fixture.
#[allow(clippy::too_many_lines)]
fn relocates_all_metadata_and_separate_equation_id_spaces() {
    let mut file = fixture();
    let m = &mut file.model;
    let p = m.variables[0].declaration;
    let e = m.equations[0].clone();
    m.initial_equations = vec![
        e.clone(),
        RbcEquation {
            id: EquationId(1),
            ..e
        },
    ];
    m.domains.push(RbcDomain {
        id: DomainId(0),
        parent: None,
        binders: vec![RbcBinder {
            id: 0,
            display_name: "i".into(),
            lower: 1,
            upper: 2,
            step: 1,
        }],
        extents: vec![2],
        scalar_count: 2,
        provenance: p,
    });
    let family = RbcEquationFamily {
        id: FamilyId(0),
        domain: DomainId(0),
        bodies: vec![ExprId(0)],
        scalar_rows: 2,
        extents: vec![2],
        scalar_view: RbcScalarView::BinderSubstitution,
        reads: vec![VariableId(0)],
        reads_derivative: vec![VariableId(0)],
        reads_previous: vec![VariableId(0)],
        provenance: p,
    };
    m.equation_families.push(family.clone());
    m.initial_equation_families = vec![
        family.clone(),
        RbcEquationFamily {
            id: FamilyId(1),
            ..family
        },
    ];
    m.functions.push(RbcFunction {
        folds: Vec::new(),
        values: Vec::new(),
        calls: Vec::new(),
        id: FunctionId(0),
        name: "f".into(),
        parameters: vec![RbcFunctionParameter {
            name: "p".into(),
            value_type: TypeId(0),
            declaration: None,
        }],
        results: vec![TypeId(0)],
        inline: RbcInline::Never,
        derivatives: Vec::new(),
        body: RbcFunctionBody::External {
            language: "C".into(),
            symbol: "f_external".into(),
            purity: RbcPurity::Impure,
            arguments: Vec::new(),
            result: None,
            linkage: RbcExternalLinkage::default(),
        },
        declaration: p,
    });
    m.types.push(RbcType {
        id: TypeId(1),
        scalar: RbcScalar::Record,
        dimensions: vec![],
        record: Some(RbcRecord {
            name: "Record".into(),
            fields: vec![RbcRecordField {
                name: "x".into(),
                value_type: TypeId(0),
            }],
        }),
    });
    m.variables[1].contract = Some(serde_json::from_value(json!({"variability":"parameter","binding_depends_on":[0],"declared_in":"Original.Class"})).unwrap());
    m.variables[1].min = Some(ExprId(0));
    m.variables[1].max = Some(ExprId(1));
    m.variables[1].nominal = Some(ExprId(0));
    m.components.push(RbcComponent {
        id: ComponentId(0),
        path: "part".into(),
        class_name: Some("Original.Class".into()),
    });
    m.variables[0].component = Some(ComponentId(0));
    m.conditions.push(RbcCondition {
        id: ConditionId(0),
        node: RbcConditionNode::Always,
        provenance: p,
    });
    m.relations.push(RbcRelation {
        id: RelationId(0),
        expression: ExprId(0),
        provenance: p,
    });
    m.roots.push(RbcRoot {
        id: RootId(0),
        relation: RelationId(0),
        activation: ConditionId(0),
        provenance: p,
    });
    m.events.push(RbcEventAction {
        id: EventId(0),
        trigger: ConditionId(0),
        guard: ConditionId(0),
        action: RbcAction::Assert {
            message: ExprId(0),
            level: Some(ExprId(1)),
        },
        provenance: p,
    });
    m.time_events = vec![
        RbcTimeEvent {
            id: EventId(0),
            schedule: RbcSchedule::Dynamic {
                deadline: ExprId(0),
            },
            provenance: p,
        },
        RbcTimeEvent {
            id: EventId(1),
            schedule: RbcSchedule::Static {
                numerator: 1,
                denominator: 2,
            },
            provenance: p,
        },
    ];
    m.discrete_real_equations.push(RbcDiscreteRealEquation {
        id: EquationId(0),
        residual: ExprId(0),
        activation: RbcDiscreteRealActivation::When {
            trigger: ConditionId(0),
            guard: ConditionId(0),
        },
        reads: vec![VariableId(0)],
        reads_derivative: vec![VariableId(0)],
        reads_previous: vec![VariableId(0)],
        provenance: p,
    });
    m.initial_discrete_values.push(RbcInitialDiscreteValue {
        target: VariableId(0),
        value: ExprId(0),
        provenance: p,
    });
    m.discrete_definitions.push(RbcDiscreteDefinition {
        observed: false,
        targets: vec![VariableId(0)],
        branches: vec![RbcDiscreteBranch {
            activation: RbcDiscreteActivation::When {
                trigger: ConditionId(0),
                guard: ConditionId(0),
            },
            values: vec![ExprId(0)],
            provenance: p,
        }],
        provenance: p,
    });
    m.connections.push(RbcConnection {
        id: ConnectionId(0),
        left: VariableId(0),
        right: VariableId(1),
        quantity: RbcQuantityKind::Potential,
        left_connector: "left".into(),
        right_connector: "right".into(),
        equation: Some(EquationId(0)),
        provenance: p,
    });
    m.connection_sets.push(RbcConnectionSet {
        id: ConnectionSetId(0),
        connectors: vec!["left".into(), "right".into()],
        potentials: vec![VariableId(0)],
        balances: vec![RbcFlowBalance {
            equation: Some(EquationId(0)),
            terms: vec![RbcFlowTerm {
                variable: VariableId(1),
                negated: true,
            }],
        }],
        potential_equations: vec![EquationId(0)],
        unconnected: false,
        provenance: p,
    });
    m.trace_points[0].connection = Some(ConnectionId(0));
    m.trace_points[0].connection_set = Some(ConnectionSetId(0));
    recompute_summary(m);
    let linked = twice(&file);
    let m = &linked.model;
    let offset = file.model.expressions.len() as u32;
    assert_eq!(m.initial_equations[2].id, EquationId(2));
    assert_eq!(m.initial_equation_families[2].id, FamilyId(2));
    assert_eq!(m.initial_equation_families[2].domain, DomainId(1));
    assert_eq!(m.equation_families[1].reads_previous, vec![VariableId(2)]);
    assert_eq!(m.domains[1].binders[0].id, 0);
    assert_eq!(m.functions[1].name, "b.f");
    assert_eq!(m.functions[1].parameters[0].value_type, TypeId(2));
    assert_eq!(m.functions[1].body, file.model.functions[0].body);
    assert_eq!(
        m.types[3].record.as_ref().unwrap().fields[0].value_type,
        TypeId(2)
    );
    assert_eq!(
        m.variables[3].contract.as_ref().unwrap().binding_depends_on,
        vec![VariableId(2)]
    );
    assert_eq!(
        m.variables[3]
            .contract
            .as_ref()
            .unwrap()
            .declared_in
            .as_deref(),
        Some("Original.Class")
    );
    assert_eq!(m.variables[3].max, Some(ExprId(offset + 1)));
    assert_eq!(m.variables[2].component, Some(ComponentId(1)));
    assert_eq!(m.roots[1].relation, RelationId(1));
    assert_eq!(m.events[1].trigger, ConditionId(1));
    assert_eq!(m.time_events[2].id, EventId(2));
    assert_eq!(
        serde_json::to_value(&m.time_events[2].schedule).unwrap(),
        json!({"kind":"dynamic","deadline":offset})
    );
    assert_eq!(m.discrete_real_equations[1].id, EquationId(1));
    assert_eq!(
        m.discrete_definitions[1].branches[0].values,
        vec![ExprId(offset)]
    );
    assert_eq!(m.initial_discrete_values[1].target, VariableId(2));
    assert_eq!(m.connections[1].equation, Some(EquationId(1)));
    assert_eq!(
        m.connection_sets[1].balances[0].terms[0].variable,
        VariableId(3)
    );
    assert!(m.connection_sets[1].balances[0].terms[0].negated);
    assert_eq!(m.trace_points[1].connection_set, Some(ConnectionSetId(1)));
}

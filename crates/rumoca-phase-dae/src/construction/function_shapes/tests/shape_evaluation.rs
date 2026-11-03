//! MLS §12.2 / §4.4.2 (SPEC_0022 FUNC-009 / DECL-018): dimensions
//! depend on proven input shapes even when array element values are unknown.

use super::*;

fn source_span() -> Span {
    let mut sources = SourceMap::new();
    let text = "min(size(A, 1), size(A, 2))";
    let source = sources.add("shape_evaluation.mo", text);
    Span::from_offsets(source, 0, text.len())
}

fn integer(value: i64, span: Span) -> Expression {
    Expression::Literal {
        value: Literal::Integer(value),
        span,
    }
}

fn reference(name: &str, span: Span) -> Expression {
    Expression::VarRef {
        name: Reference::new(name),
        subscripts: Vec::new(),
        span,
    }
}

fn builtin(function: BuiltinFunction, args: Vec<Expression>, span: Span) -> Expression {
    Expression::BuiltinCall {
        function,
        args,
        span,
    }
}

fn size(axis: i64, span: Span) -> Expression {
    builtin(
        BuiltinFunction::Size,
        vec![reference("A", span), integer(axis, span)],
        span,
    )
}

fn extrema(function: BuiltinFunction, span: Span) -> Expression {
    builtin(function, vec![size(1, span), size(2, span)], span)
}

#[test]
fn nested_min_max_read_rectangular_empty_and_large_shapes_without_values() {
    let span = source_span();
    let mut values = ShapeEnvironment::with_capacity(1);
    for (rows, columns) in [(2, 5), (5, 2), (0, 7), (7, 0), (u32::MAX, 1)] {
        values.insert(VarName::new("A"), vec![rows, columns]);
        assert_eq!(
            values.proven_extent(&extrema(BuiltinFunction::Min, span)),
            Some(i64::from(rows.min(columns)))
        );
        assert_eq!(
            values.proven_extent(&extrema(BuiltinFunction::Max, span)),
            Some(i64::from(rows.max(columns)))
        );
        assert!(
            values.values.get("A").is_none(),
            "shape does not prove elements"
        );
    }
}

#[test]
fn conditional_and_integer_arithmetic_share_shape_metadata() {
    let span = source_span();
    let mut values = ShapeEnvironment::with_capacity(1);
    values.insert(VarName::new("A"), vec![2, 9]);
    let expression = Expression::If {
        branches: vec![(
            Expression::Binary {
                op: OpBinary::Lt,
                lhs: Box::new(size(1, span)),
                rhs: Box::new(size(2, span)),
                span,
            },
            builtin(
                BuiltinFunction::Div,
                vec![size(2, span), integer(2, span)],
                span,
            ),
        )],
        else_branch: Box::new(integer(1, span)),
        span,
    };
    assert_eq!(values.proven_extent(&expression), Some(4));
    values.insert(VarName::new("A"), vec![9, 2]);
    assert_eq!(values.proven_extent(&expression), Some(1));
}

#[test]
fn scalar_shadowing_clears_outer_shape_for_every_binding_kind() {
    let span = source_span();
    let mut outer = ShapeEnvironment::with_capacity(1);
    outer.insert(VarName::new("A"), vec![4, 7]);
    for kind in 0..3 {
        let mut inner = outer.clone();
        match kind {
            0 => inner.insert(VarName::new("A"), Vec::new()),
            1 => inner.bind_scalar_value(VarName::new("A"), EvalValue::Integer(3)),
            _ => inner.bind_integer_bounds(VarName::new("A"), 1, 3),
        }
        assert_eq!(
            inner.proven_extent(&extrema(BuiltinFunction::Min, span)),
            None
        );
    }
    assert_eq!(
        outer.proven_extent(&extrema(BuiltinFunction::Min, span)),
        Some(4)
    );
}

#[test]
fn shape_registration_does_not_prove_runtime_values_or_invalid_dimensions() {
    let span = source_span();
    let mut values = ShapeEnvironment::with_capacity(2);
    values.bind_scalar_value(VarName::new("A"), EvalValue::Integer(99));
    values.insert(VarName::new("A"), vec![2, 5]);
    values.insert(VarName::new("n"), Vec::new());
    assert!(values.proven_value(&reference("A", span)).is_none());
    let runtime = builtin(
        BuiltinFunction::Min,
        vec![size(1, span), reference("n", span)],
        span,
    );
    assert_eq!(values.proven_extent(&runtime), None);
    let invalid = builtin(
        BuiltinFunction::Min,
        vec![size(3, span), integer(1, span)],
        span,
    );
    assert_eq!(values.proven_extent(&invalid), None);
    let real = builtin(
        BuiltinFunction::Min,
        vec![size(1, span), literal(1.0, span)],
        span,
    );
    assert_eq!(values.proven_extent(&real), None);
}

#[test]
fn calls_with_different_input_shapes_get_distinct_result_shapes() {
    let span = source_span();
    let mut function = rumoca_core::Function::new("f", span);
    function.add_input(
        real_param("A", vec![0, 0], span)
            .with_shape_expr(vec![Subscript::Colon { span }, Subscript::Colon { span }]),
    );
    function.add_output(
        real_param("y", vec![0], span).with_shape_expr(vec![Subscript::expr(
            Box::new(extrema(BuiltinFunction::Min, span)),
            span,
        )]),
    );
    let mut model = flat::Model::new();
    model.add_function(function);
    for dims in [[2, 5], [5, 2], [4, 7]] {
        model.add_equation(flat::Equation::new(
            Expression::FunctionCall {
                name: Reference::new("f"),
                args: vec![zeros(&dims, span)],
                is_constructor: false,
                span,
            },
            span,
            flat::EquationOrigin::ComponentEquation {
                component: String::new(),
            },
        ));
    }
    let analysis = FunctionShapeAnalysis::analyze(&model, &EvalContext::new())
        .expect("each call proves the output shape from its actual matrix shape");
    let mut results: Vec<_> = analysis
        .certificates()
        .iter()
        .map(|certificate| (certificate.key.inputs.clone(), certificate.results.clone()))
        .collect();
    results.sort();
    assert_eq!(
        results,
        vec![
            (vec![vec![2, 5]], vec![vec![2]]),
            (vec![vec![4, 7]], vec![vec![4]]),
            (vec![vec![5, 2]], vec![vec![2]]),
        ]
    );
}

//! Operator record contract tests - MLS §14

use rumoca_compile::compile::FailedPhase;
use rumoca_contracts::test_support::{
    expect_failure_in_phase_with_code, expect_resolve_failure_with_code, expect_success,
};

#[test]
fn oprec_operator_record_structure_allows_record_fields_and_operator_declarations() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '+'
                import Complex;
                function add
                    input Complex a;
                    input Complex b;
                    output Complex c;
                algorithm
                    c := Complex(a.re + b.re, a.im + b.im);
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

// =============================================================================
// OPREC-001: Encapsulated
// "Operator or operator function must be encapsulated"
// =============================================================================

#[test]
fn oprec_001_encapsulated_operator_ok() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '+'
                import Complex;
                function add
                    input Complex a;
                    input Complex b;
                    output Complex c;
                algorithm
                    c := Complex(a.re + b.re, a.im + b.im);
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn oprec_001_unencapsulated_operator_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            operator '+'
                import Complex;
                function add
                    input Complex a;
                    input Complex b;
                    output Complex c;
                algorithm
                    c := Complex(a.re + b.re, a.im + b.im);
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER071",
    );
}

// =============================================================================
// OPREC-002: Single output
// "All operator functions shall return exactly one output"
// =============================================================================

#[test]
fn oprec_002_single_output_ok() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '+'
                import Complex;
                function add
                    input Complex a;
                    input Complex b;
                    output Complex c;
                algorithm
                    c := Complex(a.re + b.re, a.im + b.im);
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn oprec_002_multiple_outputs_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '+'
                import Complex;
                function add
                    input Complex a;
                    input Complex b;
                    output Complex c;
                    output Complex d;
                algorithm
                    c := Complex(a.re + b.re, a.im + b.im);
                    d := c;
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER135",
    );
}

// =============================================================================
// OPREC-003: Record input
// "Must have at least one component of record class as input (except constructor)"
// =============================================================================

#[test]
fn oprec_003_record_input_ok() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '+'
                import Complex;
                function add
                    input Complex a;
                    input Complex b;
                    output Complex c;
                algorithm
                    c := Complex(a.re + b.re, a.im + b.im);
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn oprec_003_missing_record_input_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '+'
                import Complex;
                function add
                    input Real a;
                    input Real b;
                    output Complex c;
                algorithm
                    c := Complex(a + b, a - b);
                end add;
            end '+';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER136",
    );
}

// =============================================================================
// OPREC-004: Constructor output
// "Constructor shall return one component of the operator record class"
// =============================================================================

#[test]
fn oprec_004_constructor_output_ok() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator 'constructor'
                import Complex;
                function from_real
                    input Real x;
                    output Complex c;
                algorithm
                    c := Complex(x, x);
                end from_real;
            end 'constructor';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn oprec_004_constructor_output_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator 'constructor'
                import Complex;
                function from_real
                    input Real x;
                    output Real y;
                algorithm
                    y := x;
                end from_real;
            end 'constructor';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER137",
    );
}

// =============================================================================
// OPREC-008: Zero operator single
// "'0' operator can only contain one function with zero inputs"
// =============================================================================

#[test]
fn oprec_008_zero_operator_single_zero_input_ok() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '0'
                import Complex;
                function zero
                    output Complex c;
                algorithm
                    c := Complex(0, 0);
                end zero;
            end '0';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn oprec_008_zero_operator_multiple_functions_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '0'
                import Complex;
                function zero
                    output Complex c;
                algorithm
                    c := Complex(0, 0);
                end zero;

                function zero2
                    output Complex c;
                algorithm
                    c := Complex(0, 0);
                end zero2;
            end '0';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER072",
    );
}

#[test]
fn oprec_008_zero_operator_with_input_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator '0'
                import Complex;
                function zero
                    input Complex a;
                    output Complex c;
                algorithm
                    c := a;
                end zero;
            end '0';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER072",
    );
}

// =============================================================================
// OPREC-010: String operator output
// "operator A.'String' shall only contain functions declaring one output of String type"
// =============================================================================

#[test]
fn oprec_010_string_output_ok() {
    expect_success(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator 'String'
                import Complex;
                function to_string
                    input Complex a;
                    output String s;
                algorithm
                    s := "complex";
                end to_string;
            end 'String';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn oprec_010_non_string_output_rejected() {
    expect_resolve_failure_with_code(
        r#"
        encapsulated operator record Complex
            Real re;
            Real im;

            encapsulated operator 'String'
                import Complex;
                function to_string
                    input Complex a;
                    output Real y;
                algorithm
                    y := a.re;
                end to_string;
            end 'String';
        end Complex;

        model Test
            Complex c;
        equation
            c = Complex(1, 2);
        end Test;
    "#,
        "Test",
        "ER138",
    );
}

// =============================================================================
// OPREC-006: First two inputs no default values, rest must have defaults
// =============================================================================

#[test]
fn oprec_006_operator_input_default_rejected() {
    rumoca_contracts::test_support::expect_resolve_failure_with_code(
        r#"
        package P
            operator record OR
                Real re;
                encapsulated operator function '+'
                    import P.OR;
                    input OR a;
                    input OR b = OR(0);
                    output OR c = OR(a.re + b.re);
                end '+';
            end OR;
            model M
                OR x = OR(1);
                OR y = OR(2);
                OR z = x + y;
            end M;
        end P;
    "#,
        "P.M",
        "ER110",
    );
}

// =============================================================================
// OPREC-005: For potential call, shall not exist multiple matches
// =============================================================================

#[test]
fn oprec_005_identical_overload_signatures_rejected() {
    rumoca_contracts::test_support::expect_resolve_failure_with_code(
        r#"
        package P
            operator record OR
                Real re;
                encapsulated operator '*'
                    function mul1
                        import P.OR;
                        input OR a;
                        input OR b;
                        output OR c = OR(a.re * b.re);
                    end mul1;
                    function mul2
                        import P.OR;
                        input OR a;
                        input OR b;
                        output OR c = OR(a.re * b.re * 2);
                    end mul2;
                end '*';
            end OR;
            model M
                OR x = OR(1);
                OR y = OR(2);
                OR z = x * y;
            end M;
        end P;
    "#,
        "P.M",
        "ER122",
    );
}

// =============================================================================
// OPREC-007: Error if multiple functions match a binary operation
// =============================================================================

#[test]
fn oprec_007_ambiguous_binary_operator_match_rejected() {
    rumoca_contracts::test_support::expect_resolve_failure_with_code(
        r#"
        package P
            operator record OR
                Real re;
                encapsulated operator '+'
                    function add1
                        import P.OR;
                        input OR a;
                        input OR b;
                        output OR c = OR(a.re + b.re);
                    end add1;
                    function add2
                        import P.OR;
                        input OR a;
                        input OR b;
                        output OR c = OR(a.re - b.re);
                    end add2;
                end '+';
            end OR;
            model M
                OR x = OR(1);
                OR y = OR(2);
                OR z = x + y;
            end M;
        end P;
    "#,
        "P.M",
        "ER122",
    );
}

// =============================================================================
// OPREC-009: For pair of operator record classes C and D, at most one of
// C.'constructor'(d) and D.'constructor'(c) shall be legal
// =============================================================================

#[test]
fn oprec_009_cross_constructor_pair_rejected() {
    rumoca_contracts::test_support::expect_resolve_failure_with_code(
        r#"
        package P
            operator record C
                Real re;
                encapsulated operator 'constructor'
                    function fromD
                        import P.D;
                        import P.C;
                        input D d;
                        output C c = C(d.v);
                    end fromD;
                end 'constructor';
            end C;
            operator record D
                Real v;
                encapsulated operator 'constructor'
                    function fromC
                        import P.C;
                        import P.D;
                        input C c;
                        output D d = D(c.re);
                    end fromC;
                end 'constructor';
            end D;
            model M
                C x = C(1.0);
            end M;
        end P;
    "#,
        "P.M",
        "ER125",
    );
}

// =============================================================================
// OPREC-011: If inner dimension is zero for matrix*vector/matrix, uses '0'
// operator; error if '0' not defined. Operator-record arrays with zero inner
// dimensions are rejected (no '0' operator support yet), pinned here.
// =============================================================================

#[test]
fn oprec_011_zero_inner_dimension_product_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        package P
            operator record OR
                Real re;
            end OR;
            model M
                OR a[2, 0];
                OR b[0];
                OR c[2];
            equation
                c = a * b;
            end M;
        end P;
        "#,
        "P.M",
        FailedPhase::Typecheck,
        "ET011",
    );
}

#[test]
fn oprec_011_unused_zero_sized_array_is_legal() {
    expect_success(
        r#"
        package P
            operator record OR
                Real re;
            end OR;
            model Unused
                parameter Integer n = 0;
                OR values[n];
            end Unused;
            model M
                Real x;
            equation
                x = 1;
            end M;
        end P;
    "#,
        "P.M",
    );
}

// =============================================================================
// MLS §14.5: operator-record arithmetic in equations denotes calls of the
// operator functions; record equations count one equation per field.

const MINI_COMPLEX: &str = r#"
operator record Cx
    Real re;
    Real im;
    encapsulated operator 'constructor'
        import Cx;
        function fromReal
            input Real re;
            input Real im = 0;
            output Cx result(re = re, im = im);
        algorithm
            annotation(Inline = true);
        end fromReal;
    end 'constructor';
    encapsulated operator '-'
        import Cx;
        function negate
            input Cx c1;
            output Cx c2;
        algorithm
            c2 := Cx(-c1.re, -c1.im);
        end negate;
        function subtract
            input Cx c1;
            input Cx c2;
            output Cx c3;
        algorithm
            c3 := Cx(c1.re - c2.re, c1.im - c2.im);
        end subtract;
    end '-';
    encapsulated operator '*'
        import Cx;
        function multiply
            input Cx c1;
            input Cx c2;
            output Cx c3;
        algorithm
            c3 := Cx(c1.re*c2.re - c1.im*c2.im, c1.re*c2.im + c1.im*c2.re);
        end multiply;
    end '*';
    encapsulated operator function '+'
        import Cx;
        input Cx c1;
        input Cx c2;
        output Cx c3;
    algorithm
        c3 := Cx(c1.re + c2.re, c1.im + c2.im);
    end '+';
end Cx;
"#;

fn with_mini_complex(model: &str) -> String {
    format!("{MINI_COMPLEX}\n{model}")
}

#[test]
fn oprec_operator_expressions_lower_to_operator_function_calls() {
    let source = with_mini_complex(
        r#"
        model OperatorCalls
            Cx a;
            Cx b(re = 1, im = 1);
            Cx c;
            Cx d;
        equation
            a = -b;
            c = a + b*Cx(2, 3);
            d = 2*b - a;
        end OperatorCalls;
    "#,
    );
    rumoca_contracts::test_support::expect_balanced(&source, "OperatorCalls");
    let trace = rumoca_contracts::test_support::simulate_model(&source, "OperatorCalls", 0.1);
    for (name, expected) in [
        ("a.re", -1.0),
        ("a.im", -1.0),
        ("c.re", -2.0),
        ("c.im", 4.0),
        ("d.re", 3.0),
        ("d.im", 3.0),
    ] {
        assert!(
            (trace.final_value(name) - expected).abs() < 1e-9,
            "{name} = {}, expected {expected}",
            trace.final_value(name)
        );
    }
}

#[test]
fn oprec_if_equation_with_record_branches_counts_every_field() {
    let source = with_mini_complex(
        r#"
        model RecordBranches
            Cx vs;
            Cx vr(re = time, im = 1);
            Cx is;
            Cx ir;
            Boolean open = time > 0.5;
        equation
            if open then
                is = Cx(0);
                ir = Cx(0);
            else
                vs = vr;
                is = -ir;
            end if;
            vs.re = is.re;
            vs.im = is.im;
        end RecordBranches;
    "#,
    );
    rumoca_contracts::test_support::expect_balanced(&source, "RecordBranches");
    let trace = rumoca_contracts::test_support::simulate_model(&source, "RecordBranches", 1.0);
    assert!(trace.final_value("ir.re").abs() < 1e-9);
    assert!(trace.final_value("is.im").abs() < 1e-9);
    let early = trace.channel("ir.im")[1];
    assert!((early + 1.0).abs() < 1e-9, "ir.im before opening = {early}");
}

#[test]
fn oprec_conditional_record_operand_is_lowered_per_branch() {
    let source = with_mini_complex(
        r#"
        model ConditionalOperand
            parameter Boolean negateInput = false;
            parameter Cx k = Cx(2, 1);
            Cx u(re = 1, im = 1);
            Cx y;
        equation
            y = k*(if negateInput then -u else u);
        end ConditionalOperand;
    "#,
    );
    rumoca_contracts::test_support::expect_balanced(&source, "ConditionalOperand");
    let trace = rumoca_contracts::test_support::simulate_model(&source, "ConditionalOperand", 0.1);
    assert!((trace.final_value("y.re") - 1.0).abs() < 1e-9);
    assert!((trace.final_value("y.im") - 3.0).abs() < 1e-9);
}

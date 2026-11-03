//! FUNC (Function) contract tests - MLS §12
//!
//! Tests for the 40 function contracts defined in SPEC_0022.

// SPEC_0021 file-size exception: over the 2000-line action threshold. split plan:
// move the package/alias function-selection cases (`*_selected_*`,
// `*_package_*`) into a sibling `func_selection_contracts.rs`.

use rumoca_compile::compile::FailedPhase;
use rumoca_contracts::test_support::{
    expect_failure_in_phase_with_code, expect_parse_err_with_code, expect_parse_ok,
    expect_resolve_failure_with_code, expect_success,
};

// =============================================================================
// FUNC-001: Input/output only
// "Each public component must have prefix input or output"
// =============================================================================

#[test]
fn func_001_input_output_ok() {
    expect_success(
        r#"
        function F
            input Real x;
            output Real y;
        algorithm
            y := x * 2;
        end F;
        model Test
            Real z;
        equation
            z = F(2.0);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_001_public_no_prefix_fails() {
    expect_resolve_failure_with_code(
        r#"
        function F
            Real x;
            output Real y;
        algorithm
            y := x;
        end F;
    "#,
        "F",
        "ER013",
    );
}

// =============================================================================
// FUNC-002: Input read-only
// "Input formal parameters are read-only after being bound"
// =============================================================================

#[test]
fn func_002_input_readonly() {
    expect_resolve_failure_with_code(
        r#"
        function F
            input Real x;
            output Real y;
        algorithm
            x := 5;
            y := x;
        end F;
    "#,
        "F",
        "ER014",
    );
}

// =============================================================================
// FUNC-006: No equations
// "Function shall not have equations, shall not have initial algorithms"
// =============================================================================

#[test]
fn func_006_no_equations_in_function() {
    expect_parse_err_with_code(
        r#"
        function F
            input Real x;
            output Real y;
        equation
            y = x;
        end F;
    "#,
        "EP001",
    );
}

// =============================================================================
// FUNC-007: No when-statements
// "Function body shall not contain when-statements"
// =============================================================================

#[test]
fn func_007_no_when_in_function() {
    expect_resolve_failure_with_code(
        r#"
        function F
            input Real x;
            output Real y;
        algorithm
            when x > 0 then
                y := 1;
            end when;
        end F;
    "#,
        "F",
        "ER015",
    );
}

// =============================================================================
// FUNC-010: Forbidden operators
// "der, initial, terminal, sample, pre, edge, change, reinit, delay,
//  cardinality, inStream, actualStream"
// =============================================================================

#[test]
fn func_010_builtin_math_call_ok() {
    expect_success(
        r#"
        function F
            input Real x;
            output Real y;
        algorithm
            y := sin(x);
        end F;
        model Test
            Real z;
        equation
            z = F(2.0);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_010_initial_forbidden_in_function() {
    expect_resolve_failure_with_code(
        r#"
        function F
            output Boolean y;
        algorithm
            y := initial();
        end F;
    "#,
        "F",
        "ER056",
    );
}

#[test]
fn func_010_sample_forbidden_in_function() {
    expect_resolve_failure_with_code(
        r#"
        function F
            output Boolean y;
        algorithm
            y := sample(0, 1);
        end F;
    "#,
        "F",
        "ER056",
    );
}

// =============================================================================
// FUNC-011: No Clock components
// "Function may not contain components of type Clock"
// =============================================================================

#[test]
fn func_011_non_clock_component_ok() {
    expect_success(
        r#"
        function F
            output Real y;
        protected
            Boolean tick;
        algorithm
            tick := true;
            y := if tick then 1.0 else 0.0;
        end F;
        model Test
            Real z;
        equation
            z = F();
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_011_clock_component_fails() {
    expect_resolve_failure_with_code(
        r#"
        type MyClock = Clock;
        function F
            output Real y;
        protected
            MyClock clk;
        algorithm
            y := 0.0;
        end F;
    "#,
        "F",
        "ER061",
    );
}

// =============================================================================
// FUNC-012: No inner/outer
// "Function elements shall not have prefixes inner or outer"
// =============================================================================

#[test]
fn func_012_local_component_without_inner_outer_ok() {
    expect_success(
        r#"
        function F
            output Real y;
        protected
            Real local;
        algorithm
            local := 1.0;
            y := local;
        end F;
        model Test
            Real z;
        equation
            z = F();
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_012_inner_prefix_in_function_fails() {
    expect_resolve_failure_with_code(
        r#"
        function F
            output Real y;
        protected
            inner Real local;
        algorithm
            y := 0.0;
        end F;
    "#,
        "F",
        "ER060",
    );
}

#[test]
fn func_012_outer_prefix_in_function_fails() {
    expect_resolve_failure_with_code(
        r#"
        function F
            output Real y;
        protected
            outer Real local;
        algorithm
            y := 0.0;
        end F;
    "#,
        "F",
        "ER060",
    );
}

// =============================================================================
// FUNC-014: Single algorithm/external
// "Function can have at most one algorithm section or one external function interface"
// =============================================================================

#[test]
fn func_014_single_algorithm() {
    expect_success(
        r#"
        function F
            input Real x;
            output Real y;
        algorithm
            y := x * 2;
        end F;
        model Test
            Real z;
        equation
            z = F(2.0);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_014_extended_function_with_second_algorithm_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        function Base
            input Real x;
            output Real y;
        algorithm
            y := x;
        end Base;
        function Twice
            extends Base;
        algorithm
            y := 2 * x;
        end Twice;
        model Test
            Real z = Twice(time);
        end Test;
    "#,
        "Test",
        FailedPhase::Flatten,
        "EF035",
    );
}

// =============================================================================
// FUNC-017: Return in algorithm only
// "Return statement can only be used in an algorithm section of a function"
// =============================================================================

#[test]
fn func_017_return_in_function() {
    expect_success(
        r#"
        function F
            input Real x;
            output Real y;
        algorithm
            if x > 0 then
                y := x;
                return;
            end if;
            y := -x;
        end F;
        model Test
            Real z;
        equation
            z = F(2.0);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_017_return_outside_function_algorithm_fails() {
    expect_parse_err_with_code(
        r#"
        model Test
            Real x;
        equation
            return;
            x = 1.0;
        end Test;
    "#,
        "EP001",
    );
}

// =============================================================================
// FUNC-015: Component types
// "Function must not contain model, block, operator, or connector components"
// =============================================================================

#[test]
fn func_015_record_component_ok() {
    expect_success(
        r#"
        record R
            Real x;
        end R;
        function F
            input Real u;
            output Real y;
        protected
            R r;
        algorithm
            r.x := u;
            y := r.x;
        end F;
        model Test
            Real z;
        equation
            z = F(2.0);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_015_connector_component_fails() {
    expect_resolve_failure_with_code(
        r#"
        connector Pin
            Real v;
        end Pin;
        function F
            output Real y;
        protected
            Pin p;
        algorithm
            y := 0.0;
        end F;
    "#,
        "F",
        "ER062",
    );
}

// =============================================================================
// Function integration tests
// =============================================================================

#[test]
fn func_basic_call() {
    expect_success(
        r#"
        function Square
            input Real x;
            output Real y;
        algorithm
            y := x * x;
        end Square;
        model Test
            Real x;
            Real y;
        equation
            x = 3.0;
            y = Square(x);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_with_protected() {
    expect_parse_ok(
        r#"
        function F
            input Real x;
            output Real y;
        protected
            Real temp;
        algorithm
            temp := x * x;
            y := temp + 1;
        end F;
    "#,
    );
}

#[test]
fn func_multiple_outputs() {
    expect_parse_ok(
        r#"
        function SinCos
            input Real x;
            output Real s;
            output Real c;
        algorithm
            s := sin(x);
            c := cos(x);
        end SinCos;
    "#,
    );
}

#[test]
fn func_der_class_specifier_short_form_parses() {
    expect_parse_ok(
        r#"
        function f
            input Real x;
            output Real y;
        algorithm
            y := x;
        end f;

        function f_der = der(f, x);
    "#,
    );
}

#[test]
fn func_default_input() {
    expect_parse_ok(
        r#"
        function F
            input Real x;
            input Real scale = 1.0;
            output Real y;
        algorithm
            y := x * scale;
        end F;
    "#,
    );
}

// =============================================================================
// FUNC-019: Named arg slot error
// "Error if named argument slot is already filled" (MLS §12.4.1)
// =============================================================================

#[test]
fn func_019_named_slot_after_positional_accepted() {
    expect_success(
        r#"
        function F
            input Real a;
            input Real b = 1;
            output Real y;
        algorithm
            y := a + b;
        end F;
        model Test
            Real x(start = 0, fixed = true);
        equation
            der(x) = F(1, b = 2);
        end Test;
    "#,
        "Test",
    );
}

#[test]
fn func_019_named_arg_for_filled_slot_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        function F
            input Real a;
            input Real b = 1;
            output Real y;
        algorithm
            y := a + b;
        end F;
        model Test
            Real x(start = 0);
        equation
            der(x) = F(1, a = 2);
        end Test;
    "#,
        "Test",
        FailedPhase::Flatten,
        "EF016",
    );
}

// =============================================================================
// FUNC-020: Unfilled slots error
// "Error if any unfilled slots remain after argument processing" (MLS §12.4.1)
// =============================================================================

#[test]
fn func_020_unfilled_mandatory_input_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        function G
            input Real a;
            input Real b;
            output Real y;
        algorithm
            y := a + b;
        end G;
        model Test
            Real x(start = 0);
        equation
            der(x) = G(1);
        end Test;
    "#,
        "Test",
        FailedPhase::Flatten,
        "EF016",
    );
}

// =============================================================================
// FUNC-003: Output variables must be assigned inside the function
// =============================================================================

#[test]
fn func_003_unassigned_output_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        model M
            function F
                input Real u;
                output Real y;
            protected
                Real tmp;
            algorithm
                tmp := u;
            end F;
            Real z = F(1.0);
        end M;
    "#,
        "M",
        FailedPhase::Flatten,
        "EF013",
    );
}

// =============================================================================
// FUNC-013: For simulation, function shall not be partial
// =============================================================================

#[test]
fn func_013_partial_function_call_rejected() {
    // Partial functions are excluded from executable-function collection, so
    // the call is reported as unresolved when lowering to the DAE. A precise
    // flatten-time diagnostic is not possible because flatten legitimately
    // converts partial base functions that redeclares later make concrete
    // (the MSL Media pattern).
    //
    // `ED005` (`UnresolvedFunctionCall`) was retired when DAE construction
    // became valid-by-construction. SPEC_0008 requires retiring rather than
    // reusing a code, so the surviving DAE-boundary code for a call whose
    // callee has no resolved owner is `ED008` (`UnresolvedReference`).
    expect_failure_in_phase_with_code(
        r#"
        model M
            partial function F
                input Real u;
                output Real y;
            end F;
            Real z = F(1.0);
        end M;
    "#,
        "M",
        FailedPhase::ToDae,
        "ED008",
    );
}

// =============================================================================
// FUNC-016: Functions shall not be used in connections
// =============================================================================

#[test]
fn func_016_function_call_in_connect_rejected() {
    expect_parse_err_with_code(
        r#"
        model M
            connector C
                Real e;
                flow Real f;
            end C;
            function F
                output Real y;
            algorithm
                y := 1;
            end F;
            C c1;
        equation
            connect(F(), c1);
        end M;
    "#,
        "EP001",
    );
}

// =============================================================================
// FUNC-021: If function declared impure, any extending function shall be
// declared impure
// =============================================================================

#[test]
fn func_021_pure_function_extends_impure_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            impure function FBase
                input Real u;
                output Real y;
            algorithm
                y := u;
            end FBase;
            function FExt
                extends FBase;
            end FExt;
            Real z(start = 0);
        equation
            when time > 1 then
                z = FExt(1.0);
            end when;
        end M;
    "#,
        "M",
        "ER089",
    );
}

// =============================================================================
// FUNC-022: Impure only allowed from: impure function, when-equation/statement,
// pure(), initial equations/algorithms
// =============================================================================

#[test]
fn func_022_impure_call_in_continuous_equation_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            impure function F
                input Real u;
                output Real y;
            algorithm
                y := u;
            end F;
            Real z;
        equation
            z = F(time);
        end M;
    "#,
        "M",
        "ER088",
    );
}

// =============================================================================
// FUNC-034: Default values for inputs shall not depend on non-input variables
// in the function
// =============================================================================

#[test]
fn func_034_input_default_depends_on_output_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            function F
                input Real u = y;
                output Real y;
            algorithm
                y := u;
            end F;
            Real z = F();
        end M;
    "#,
        "M",
        "ER090",
    );
}

// =============================================================================
// FUNC-004: Error if impure function call is part of a system of equations
// =============================================================================

#[test]
fn func_004_impure_call_in_equation_system_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            impure function F
                input Real u;
                output Real y;
            algorithm
                y := u;
            end F;
            Real a;
            Real b;
        equation
            a + b = 1;
            b = F(a);
        end M;
    "#,
        "M",
        "ER088",
    );
}

// =============================================================================
// FUNC-008: Higher variability cannot assign to lower variability
// =============================================================================

#[test]
fn func_008_assign_to_parameter_in_algorithm_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            parameter Real p = 1;
            Real x = time;
        algorithm
            p := x;
        end M;
    "#,
        "M",
        "ER102",
    );
}

// =============================================================================
// FUNC-018: Relative ordering between input formal parameter declarations is
// significant
// =============================================================================

#[test]
fn func_018_named_arguments_reordered_accepted() {
    expect_success(
        r#"
        model M
            function F
                input Real a;
                input Real b;
                output Real y;
            algorithm
                y := a - b;
            end F;
            Real z = F(b = 1.0, a = 2.0);
        end M;
    "#,
        "M",
    );
}

// =============================================================================
// FUNC-023: Binding execution order must not have cycles
// =============================================================================

#[test]
fn func_023_cyclic_function_bindings_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            function F
                input Real u;
                output Real y;
            protected
                Real a = b + 1;
                Real b = a + 1;
            algorithm
                y := a + u;
            end F;
            Real z = F(1.0);
        end M;
    "#,
        "M",
        "ER007",
    );
}

// =============================================================================
// FUNC-024: Error to use or return an uninitialized variable
// =============================================================================

const FUNC_024_SOURCE: &str = r#"
    model M
        function F
            input Real u;
            output Real y;
        protected
            Real t;
        algorithm
            if u > 0 then
                t := 2 * u;
            end if;
            y := t + 1;
        end F;
        Real z = F(LIMIT - time);
    end M;
"#;

#[test]
fn func_024_value_assigned_on_the_executed_path_is_usable() {
    let trace = rumoca_contracts::test_support::simulate_model(
        &FUNC_024_SOURCE.replace("LIMIT", "2"),
        "M",
        1.0,
    );
    assert!((trace.final_value("z") - 3.0).abs() < 1e-9);
}

#[test]
fn func_024_use_of_a_value_the_executed_path_never_assigned_fails() {
    let error = rumoca_contracts::test_support::simulate_model_failure(
        &FUNC_024_SOURCE.replace("LIMIT", "0.5"),
        "M",
        1.0,
    );
    assert!(error.contains("`t` is used without a value"), "{error}");
}

#[test]
fn record_constructor_with_statically_present_conditional_field_accepted() {
    expect_success(
        r#"
        model M
            record R
                Real x;
                Real y if true;
            end R;
            R r = R(1.0, 2.0);
        end M;
    "#,
        "M",
    );
}

// =============================================================================
// FUNC-033: Function type parameter cannot be type-specifier of record or
// enumeration
// =============================================================================

#[test]
fn func_033_function_alias_of_record_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            record R
                Real x;
            end R;
            function Apply
                input F f;
                input Real u;
                output Real y;
                replaceable function F = R;
            algorithm
                y := u;
            end Apply;
            Real z = 1;
        end M;
    "#,
        "M",
        "ER091",
    );
}

// =============================================================================
// FUNC-030: Derivative output list shall not be empty
// =============================================================================

#[test]
fn func_030_derivative_function_without_outputs_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            function F
                input Real u;
                output Real y;
            algorithm
                y := u * u;
                annotation(derivative = dF);
            end F;
            function dF
                input Real u;
                input Real du;
            algorithm
                assert(true, "no outputs");
            end dF;
            Real z = F(time);
        end M;
    "#,
        "M",
        "ER120",
    );
}

// =============================================================================
// FUNC-031: zeroDerivative applies only if inputVar is independent of
// differentiation variables
// =============================================================================

#[test]
fn func_031_zero_derivative_names_non_input_rejected() {
    expect_resolve_failure_with_code(
        r#"
        model M
            function F
                input Real u;
                output Real y;
            algorithm
                y := u * u;
                annotation(derivative(zeroDerivative = w) = dF);
            end F;
            function dF
                input Real u;
                input Real du;
                output Real dy;
            algorithm
                dy := 2 * u * du;
            end dF;
            Real z = F(time);
        end M;
    "#,
        "M",
        "ER120",
    );
}

// =============================================================================
// FUNC-032: External function without explicit pure/impure declaration is
// deprecated
// =============================================================================

#[test]
fn func_032_external_function_without_purity_warns() {
    rumoca_contracts::test_support::expect_compile_warning(
        r#"
        model M
            function F
                input Real u;
                output Real y;
            external "C" y = my_func(u);
            end F;
            Real z(start = 0);
        equation
            when time > 1 then
                z = F(1.0);
            end when;
        end M;
    "#,
        "M",
        "WR001",
    );
}

/// MLS 3.7 §12.3 states two facts about the deprecated bare external form, and
/// the compiler must carry both from the declaration to the DAE: such a
/// function "shall be treated as impure", while writing no prefix "is
/// deprecated" — reported, not rejected. MLS 3.6 §12.3 (historical; 3.7
/// deprecates the bare form) put both in one sentence: "assumed to be impure,
/// but without any restriction on calling them".
///
/// So the body of `F` is impure — never pure by omission, which would license
/// the optimizations MLS forbids on impure calls — while the continuous-time
/// call to it is still accepted. `G` proves the written `pure` prefix is not
/// lost along the same path.
#[test]
fn func_032_external_function_without_purity_is_impure_but_unrestricted() {
    let result = expect_success(
        r#"
        model M
            function F
                input Real u;
                output Real y;
            external "C" y = my_func(u);
            end F;
            pure function G
                input Real u;
                output Real y;
            external "C" y = my_pure_func(u);
            end G;
            Real z = F(time) + G(time);
        end M;
    "#,
        "M",
    );

    let purities = result.dae.inspect(|view| {
        (0..view.function_count())
            .filter_map(|index| {
                let function = view.function(view.function_id(index)?)?;
                let external = function.external()?;
                Some((
                    function.name().as_str().to_string(),
                    external.purity().is_pure(),
                ))
            })
            .collect::<Vec<_>>()
    });

    let bare = purities
        .iter()
        .find(|(name, _)| name.ends_with('F'))
        .expect("the bare external function reaches the DAE");
    assert!(
        !bare.1,
        "an external function declaring no purity prefix has an impure body (MLS §12.3)"
    );
    let declared = purities
        .iter()
        .find(|(name, _)| name.ends_with('G'))
        .expect("the pure external function reaches the DAE");
    assert!(
        declared.1,
        "the written `pure` prefix is the only thing that makes an external body pure"
    );
}

/// MLS 3.7 §12.3 makes an external function without explicit purity "treated as
/// impure", and impurity is a fact about the body, so an initial algorithm that
/// determines a discrete value with such a call is rejected by the owner that
/// knows why: initialization applies its updates until they stop changing, and
/// an impure call never settles. The bare form must fail there — with the
/// initial-algorithm owner naming the call — rather than surviving to a later,
/// vaguer rejection.
#[test]
fn func_032_bare_external_cannot_determine_a_discrete_initial_value() {
    expect_failure_in_phase_with_code(
        r#"
        model BareDiscInit
            function f
                input Real u;
                output Real y;
            external "C" y = my_func(u);
            end f;
            discrete Real d;
            Real z;
        initial algorithm
            d := f(1.0);
        equation
            when time > 0.5 then
                d = pre(d) + 1;
            end when;
            z = d * time;
        end BareDiscInit;
    "#,
        "BareDiscInit",
        FailedPhase::ToDae,
        "ED013",
    );
}

/// MLS §12.3 lists `pure(impureFunction(…))` among the contexts an impure call
/// may occupy: "which allows calling impure functions in any pure context".
/// Rumoca does not accept it yet, and this test pins the exact deviation so it
/// is visible rather than assumed working.
///
/// The wrapper is recognized and erased during lowering, but suppressing the
/// callee's purity check needs a fact Flat can still see at DAE construction,
/// where the rule is re-proven by callee identity. Carrying it takes a call-site
/// marker on the Flat call node (144 construction sites) or a new builtin
/// variant threaded through every exhaustive builtin match — filed as task #57.
/// Until then the impure case is rejected by its earliest owner, Resolve, with
/// the call's own span.
#[test]
fn func_022_pure_wrapper_around_an_impure_call_is_not_yet_accepted() {
    expect_resolve_failure_with_code(
        r#"
        model PureWrap
            impure function f
                input Real u;
                output Real y;
            external "C" y = my_func(u);
            end f;
            Real z;
        equation
            z = pure(f(time));
        end PureWrap;
    "#,
        "PureWrap",
        "ER088",
    );
}

/// The wrapper is not a purity blanket: wrapping a pure call is legal and
/// changes nothing about it.
#[test]
fn func_022_pure_wrapper_around_a_pure_call_is_transparent() {
    let result = expect_success(
        r#"
        model PureOfPure
            pure function g
                input Real u;
                output Real y;
            external "C" y = my_pure(u);
            end g;
            Real z;
        equation
            z = pure(g(time));
        end PureOfPure;
    "#,
        "PureOfPure",
    );

    assert!(
        result.dae.inspect(|view| {
            view.function_id(0)
                .and_then(|id| view.function(id))
                .and_then(|function| function.external())
                .is_some_and(|external| external.purity().is_pure())
        }),
        "a wrapped pure external body is still pure"
    );
}

/// The wrapper is typed by what it wraps, whatever that is. MLS §12.3 spells
/// it around a call, but the grammar admits any expression, and one that
/// contains no call bypasses nothing and must still mean its own value — the
/// type checker gives `pure(1 + 2)` the type of `1 + 2` and lowering erases the
/// wrapper, so the model is an ordinary one.
#[test]
fn func_022_pure_wrapper_around_a_call_free_expression_is_transparent() {
    let result = expect_success(
        r#"
        model PureLiteral
            Real z;
        equation
            z = pure(1 + 2) * time;
        end PureLiteral;
    "#,
        "PureLiteral",
    );

    assert_eq!(
        result.dae.inspect(|view| view.function_count()),
        0,
        "the wrapper is erased, so nothing about it survives as a callable"
    );
}

/// MLS §12.3 spells the wrapper `pure(impureFunction(…))`: exactly one call.
/// The grammar admits other argument lists, and those bypass nothing, so they
/// are rejected with their own span instead of being lowered to a guess.
#[test]
fn func_022_pure_wrapper_without_exactly_one_argument_is_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        model PureArity
            pure function g
                input Real u;
                output Real y;
            external "C" y = my_pure(u);
            end g;
            Real z;
        equation
            z = pure(g(time), g(time));
        end PureArity;
    "#,
        "PureArity",
        FailedPhase::Typecheck,
        "ET008",
    );
}

// =============================================================================
// FUNC-027: Array arguments have to be the same size
// =============================================================================

#[test]
fn func_027_vectorized_args_size_mismatch_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        model M
            function F
                input Real u;
                input Real v;
                output Real y;
            algorithm
                y := u + v;
            end F;
            Real a[2] = {1, 2};
            Real b[3] = {1, 2, 3};
            Real z[2];
        equation
            z = F(a, b);
        end M;
    "#,
        "M",
        FailedPhase::Flatten,
        "EF016",
    );
}

// =============================================================================
// FUNC-036: ExternalObject lifecycle shape (MLS §12.9.7)
// "The owner uses the `class` restriction, directly extends ExternalObject and
//  owns exactly a non-replaceable constructor and destructor function"
// =============================================================================

#[test]
fn func_036_external_object_without_destructor_rejected() {
    expect_resolve_failure_with_code(
        r#"
        class Handle
            extends ExternalObject;
            function constructor
                output Handle object;
                external "C" object = create();
            end constructor;
        end Handle;

        model M
            Real x(start = 0, fixed = true);
        equation
            der(x) = 1;
        end M;
    "#,
        "M",
        "ER132",
    );
}

#[test]
fn func_036_external_object_replaceable_constructor_rejected() {
    expect_resolve_failure_with_code(
        r#"
        class Handle
            extends ExternalObject;
            replaceable function constructor
                output Handle object;
                external "C" object = create();
            end constructor;
            function destructor
                input Handle object;
                external "C" release(object);
            end destructor;
        end Handle;

        model M
            Real x(start = 0, fixed = true);
        equation
            der(x) = 1;
        end M;
    "#,
        "M",
        "ER132",
    );
}

// =============================================================================
// FUNC-037: ExternalObject lifecycle signatures (MLS §12.9.7)
// "constructor has one output of the owning type; destructor has one input of
//  that type and no outputs"
// =============================================================================

#[test]
fn func_037_external_object_destructor_with_output_rejected() {
    expect_resolve_failure_with_code(
        r#"
        class Handle
            extends ExternalObject;
            function constructor
                output Handle object;
                external "C" object = create();
            end constructor;
            function destructor
                input Handle object;
                output Integer status;
                external "C" status = release(object);
            end destructor;
        end Handle;

        model M
            Real x(start = 0, fixed = true);
        equation
            der(x) = 1;
        end M;
    "#,
        "M",
        "ER133",
    );
}

// =============================================================================
// FUNC-038: ExternalObject lifecycle calls (MLS §12.9.7)
// "constructor and destructor cannot be called explicitly"
//
// Registry status is Partial: only the clause tested below is enforced. The
// second clause ("each constructed object is constructed and destroyed exactly
// once") has no implementation, so this test is deliberately absent from
// `data/contract_cases.toml` and FUNC-038 is not in IMPLEMENTED_CONTRACT_IDS.
// =============================================================================

#[test]
fn func_038_explicit_destructor_call_rejected() {
    expect_resolve_failure_with_code(
        r#"
        class Handle
            extends ExternalObject;
            function constructor
                output Handle object;
                external "C" object = create();
            end constructor;
            function destructor
                input Handle object;
                external "C" release(object);
            end destructor;
        end Handle;

        model M
            Handle object;
        algorithm
            Handle.destructor(object);
        end M;
    "#,
        "M",
        "ER134",
    );
}

#[test]
fn func_redeclared_package_function_is_selected_through_alias() {
    // `Medium.f(T)` against `replaceable package Medium = PM`, with the
    // instance selecting `Air`, which redeclares `f` (TOOLBUG-077).
    let trace = rumoca_contracts::test_support::simulate_model(
        r#"
        package E
            partial package PM
                replaceable partial function f
                    input Real x;
                    output Real y;
                end f;
            end PM;
            package Air
                extends PM;
                redeclare function extends f
                algorithm
                    y := 2 * x;
                end f;
            end Air;
            model Src
                replaceable package Medium = PM;
                parameter Real T = 3;
                Real z = Medium.f(T);
            end Src;
            model Top
                Src s(redeclare package Medium = Air);
                Real t(start = 0, fixed = true);
            equation
                der(t) = 1;
            end Top;
        end E;
    "#,
        "E.Top",
        0.1,
    );
    assert_eq!(trace.final_value("s.z"), 6.0);
}

// =============================================================================
// Calls through an inherited replaceable package alias (TOOLBUG-130)
// =============================================================================

/// `replaceable package ref = R` declared in a base class and not redeclared:
/// `ref.f(x)` in the derived class used to reach function collection without a
/// member identity for `f` and panic.
#[test]
fn call_through_inherited_replaceable_package_alias() {
    let source = r#"
        package P
            package R
                function f
                    input Real x;
                    output Real y;
                algorithm
                    y := 2 * x;
                end f;
            end R;
            partial model Base
                replaceable package ref = P.R;
                Real v;
            equation
                v = ref.f(1.5);
            end Base;
            model M
                extends Base;
                Real w(start = 0, fixed = true);
            equation
                der(w) = ref.f(v);
            end M;
        end P;
    "#;
    let trace = rumoca_contracts::test_support::simulate_model(source, "P.M", 1.0);
    let value = trace.final_value("w");
    assert!(
        (value - 6.0).abs() < 1e-6,
        "w must integrate 2*2*1.5, got {value}"
    );
}

// =============================================================================
// Function selected through a per-instance package redeclaration (TOOLBUG-140)
// =============================================================================

const REDECLARED_MEDIUM_STATE_SOURCE: &str = r#"
    package P
        partial package PM
            replaceable record State
            end State;
            replaceable partial function temperature
                input State state;
                output Real T;
            end temperature;
        end PM;
        partial package PS
            extends PM;
            redeclare record extends State
                Real T;
            end State;
            redeclare function extends temperature
            algorithm
                T := state.T;
            end temperature;
        end PS;
        package W
            extends PS;
        end W;
        model Q
            replaceable package Medium = PM;
            Medium.State st(T = 2);
            Real w(start = 0, fixed = true);
        equation
            der(w) = Medium.temperature(st);
        end Q;
        model ByComponent
            Q q(redeclare package Medium = W);
        end ByComponent;
        model ByExtends
            extends Q(redeclare package Medium = W);
        end ByExtends;
    end P;
"#;

/// MLS §7.3: `Medium.temperature` in `Q` names the implementation selected by
/// the redeclaration (`PS.temperature`), whose formal `state` has the
/// redeclared `State` with member `T`. The class-tree exposure `Q.Medium`
/// still denotes the partial default, so converting the body in that scope
/// lost `state.T` (EF024) and the record argument's fields.
#[test]
fn function_selected_by_instance_package_redeclaration_keeps_record_members() {
    for (model, var) in [("P.ByComponent", "q.w"), ("P.ByExtends", "w")] {
        let trace = rumoca_contracts::test_support::simulate_model(
            REDECLARED_MEDIUM_STATE_SOURCE,
            model,
            1.0,
        );
        let value = trace.final_value(var);
        assert!(
            (value - 2.0).abs() < 1e-6,
            "{model}: {var} must integrate state.T = 2, got {value}"
        );
    }
}

// =============================================================================
// Function extending its interface and an implementation (TOOLBUG-141)
// =============================================================================

/// MLS §12.2: MSL's `Files.loadResource` extends the partial interface
/// `PartialLoadResource` and `ModelicaServices...loadResource`, which itself
/// extends that interface and supplies the algorithm. The interface-only base
/// contributes no body, so the implementation is the unique selection.
#[test]
fn function_extending_interface_and_implementation_selects_implementation() {
    let source = r#"
        package P
            partial function PI
                input Real x;
                output Real y;
            end PI;
            function Impl
                extends PI;
            algorithm
                y := 2 * x;
            end Impl;
            function F
                extends PI;
                extends Impl;
            end F;
            model M
                Real w(start = 0, fixed = true);
            equation
                der(w) = F(1.5);
            end M;
        end P;
    "#;
    let trace = rumoca_contracts::test_support::simulate_model(source, "P.M", 1.0);
    let value = trace.final_value("w");
    assert!(
        (value - 3.0).abs() < 1e-6,
        "w must integrate 2*1.5, got {value}"
    );
}

/// MLS §12.3 / OpenModelica: `ModelicaServices...loadResource` writes no purity
/// prefix but calls the impure `Files.fullPathName`. It is compiled as impure
/// with WR013 instead of being rejected as a pure body reaching an impure call.
#[test]
fn undeclared_purity_function_calling_impure_compiles_with_warning() {
    let source = r#"
        package P
            impure function g
                input Real x;
                output Real y;
                external "C" y = sin(x);
            end g;
            function f
                input Real x;
                output Real y;
            algorithm
                y := P.g(x);
            end f;
            model M
                parameter Real p = f(1);
            end M;
        end P;
    "#;
    rumoca_contracts::test_support::expect_compile_warning(source, "P.M", "WR013");
}

// =============================================================================
// Media function selection through replaceable packages (TOOLBUG-143..146)
// =============================================================================

/// MLS §12.2 / §7.3: a partial interface that declares its own formals and
/// only extends an empty icon is its own interface, not an alias of the icon;
/// and `replaceable package Medium = PM` puts both the alias and `PM` on the
/// override list without making the selection ambiguous. The call to the
/// still-partial function survives Flatten and is rejected at the DAE, which
/// owns executable semantics (OpenModelica checks such models but cannot
/// simulate them).
#[test]
fn partial_medium_interface_call_is_selected_through_its_alias() {
    let source = r#"
        package P
            partial function Icon
            end Icon;
            partial package PM
                replaceable partial function f
                    extends Icon;
                    input Real x;
                    output Real y;
                end f;
                replaceable partial function g
                    extends Icon;
                    input Real x;
                    output Real y;
                end g;
            end PM;
            model S
                replaceable package Medium = PM;
                Real y = Medium.f(Medium.g(time));
            end S;
        end P;
    "#;
    rumoca_contracts::test_support::expect_failure_in_phase_with_code(
        source,
        "P.S",
        FailedPhase::ToDae,
        "ED008",
    );
}

const INHERITED_PACKAGE_FUNCTION_SOURCE: &str = r#"
    package L
        partial package PM
            replaceable partial function f
                input Real x;
                output Real y;
            end f;
            replaceable function twice
                input Real x;
                output Real y;
            algorithm
                y := 2 * f(x);
            end twice;
        end PM;
        package W
            extends PM;
            redeclare function extends f
            algorithm
                y := 3 * x;
            end f;
        end W;
        model Q
            replaceable package Medium = PM;
            Real w(start = 0, fixed = true);
            Real v(start = 0, fixed = true);
        equation
            der(w) = Medium.twice(1);
            der(v) = Medium.twice(time);
        end Q;
        model ByComponent
            Q q(redeclare package Medium = W);
        end ByComponent;
        model ByExtends
            extends Q(redeclare package Medium = W);
        end ByExtends;
    end L;
"#;

/// MLS §5.3: the inherited, non-redeclared `PM.twice` calls `f` in the
/// package it is an element of, so with `Medium = W` it reaches `W.f`. A
/// component redeclaration used to keep the call exposed as `Q.Medium.twice`
/// (the partial default), leaving `PM.f` unresolved; and the call's `time`
/// argument was captured as the package member `q.Medium.time`.
#[test]
fn inherited_package_function_reaches_selected_package_members() {
    for (model, prefix) in [("L.ByComponent", "q."), ("L.ByExtends", "")] {
        let trace = rumoca_contracts::test_support::simulate_model(
            INHERITED_PACKAGE_FUNCTION_SOURCE,
            model,
            1.0,
        );
        let w = trace.final_value(&format!("{prefix}w"));
        let v = trace.final_value(&format!("{prefix}v"));
        assert!(
            (w - 6.0).abs() < 1e-6,
            "{model}: w must integrate 2*3*1, got {w}"
        );
        assert!(
            (v - 3.0).abs() < 1e-4,
            "{model}: v must integrate 6*t, got {v}"
        );
    }
}

/// MLS §4.5.1: MSL's `IF97_Utilities.phase_ph` calls the short alias
/// `hl_p = BaseIF97.Regions.hl_p`, and nothing calls the alias by its own name,
/// so no exposure under that name is collected; the call is matched to the
/// renamed declaration instead of failing with EF005.
#[test]
fn short_function_alias_called_only_by_a_sibling() {
    let source = r#"
        package P
            package B
                function f
                    input Real x;
                    output Real y;
                algorithm
                    y := 2 * x;
                end f;
            end B;
            function f = B.f;
            function g
                input Real x;
                output Real y;
            algorithm
                y := if x < 0 then 0 else f(x);
            end g;
            model M
                Real w(start = 0, fixed = true);
            equation
                der(w) = g(1.5);
            end M;
        end P;
    "#;
    let trace = rumoca_contracts::test_support::simulate_model(source, "P.M", 1.0);
    let value = trace.final_value("w");
    assert!(
        (value - 3.0).abs() < 1e-6,
        "w must integrate 2*1.5, got {value}"
    );
}

// =============================================================================
// Package forwarded under another name; port members typed through it
// (TOOLBUG-146, TOOLBUG-147)
// =============================================================================

const FORWARDED_MEDIUM_SOURCE: &str = r#"
    package L
      partial package PM
        type MassFlowRate = Real(unit = "kg/s");
        replaceable partial model BaseProperties
          Real x;
        end BaseProperties;
      end PM;
      package W
        extends PM;
        constant Real k = 2;
        redeclare model extends BaseProperties
        equation
          x = k * time;
        end BaseProperties;
      end W;
      connector Port
        replaceable package Medium = PM;
        flow Medium.MassFlowRate m_flow;
        Real p;
      end Port;
      block Pass
        input Real u;
        output Real y = u;
      end Pass;
      model Vol
        replaceable package Medium = PM;
        Medium.BaseProperties medium;
        Port port_a(redeclare package Medium = Medium);
        Pass pas(u = abs(port_a.m_flow));
      equation
        port_a.p = medium.x;
      end Vol;
      model Mach
        replaceable package MediumCon = PM;
        Vol vol(redeclare package Medium = MediumCon);
      end Mach;
      model T
        Mach chi(redeclare package MediumCon = W);
      end T;
    end L;
"#;

/// MLS §7.3: `Vol vol(redeclare package Medium = MediumCon)` names the
/// enclosing replaceable alias, so `vol.Medium` is the selection `chi` made
/// (`W`), not `MediumCon`'s partial default (EI012 on `Medium.BaseProperties`).
/// `pas(u = abs(port_a.m_flow))` reads a port member whose declared type
/// crosses the replaceable edge `Medium.MassFlowRate`; the member exists
/// (ET001 claimed it did not).
#[test]
fn package_forwarded_under_another_name_selects_the_enclosing_choice() {
    let trace = rumoca_contracts::test_support::simulate_model(FORWARDED_MEDIUM_SOURCE, "L.T", 1.0);
    let pressure = trace.final_value("chi.vol.port_a.p");
    assert!(
        (pressure - 2.0).abs() < 1e-6,
        "W.BaseProperties gives k*time = 2, got {pressure}"
    );
    // The unconnected port's flow is zero (MLS §9.2), so the block reads 0.
    let passed = trace.final_value("chi.vol.pas.y");
    assert!(
        passed.abs() < 1e-9,
        "pas.y must read the zero port flow, got {passed}"
    );
}

// =============================================================================
// Inherited package constant per extending package (TOOLBUG-148)
// =============================================================================

const INHERITED_PACKAGE_CONSTANT_SOURCE: &str = r#"
    package L
        partial package PM
            constant String names[:] = {"a"};
            constant Boolean red = false;
            final constant Integer nS = size(names, 1);
            final constant Integer n = if red then nS - 1 else nS;
        end PM;
        partial package PM2
            extends PM(red = true);
        end PM2;
        package W
            extends PM2(names = {"x", "y", "z"});
        end W;
        model C
            replaceable package Medium = PM;
            parameter Real s[Medium.n] = {i for i in 1:Medium.n};
            Real y = s[Medium.n] * time;
        end C;
        model ByAlias
            package Medium = W;
            parameter Real s[Medium.n] = {i for i in 1:Medium.n};
            Real y = s[Medium.n] * time;
        end ByAlias;
        model ByComponent
            C c(redeclare package Medium = W);
        end ByComponent;
    end L;
"#;

/// MLS §7.2: `W` modifies `names` two extends levels above the declaration of
/// `nS = size(names, 1)`, so `W.n = 2`. The declaration-keyed constant table
/// held one value per declaration (the last package extracted, here `PM2`'s
/// `size({"a"}, 1) - 1 = 0`), so `{i for i in 1:Medium.n}` lost its elements;
/// and the comprehension range was demanded before package constants existed
/// (EF004 "for-equation range end must be a constant integer or parameter").
#[test]
fn inherited_package_constant_takes_the_extending_package_value() {
    for (model, var) in [("L.ByAlias", "y"), ("L.ByComponent", "c.y")] {
        let trace = rumoca_contracts::test_support::simulate_model(
            INHERITED_PACKAGE_CONSTANT_SOURCE,
            model,
            1.0,
        );
        let value = trace.final_value(var);
        assert!(
            (value - 2.0).abs() < 1e-6,
            "{model}: {var} must be s[2]*1 = 2, got {value}"
        );
    }
}

// =============================================================================
// Flexible-size protected function locals (TOOLBUG-132)
// =============================================================================

/// MLS §12.4.4: `Real a[:] = {...}` takes its size from its declaration
/// equation (IBPSA/Buildings `R410A.pressureSatVap_T`).
#[test]
fn flexible_local_array_sized_by_declaration_equation() {
    let source = r#"
        package P
            function f
                input Real t;
                output Real y;
            protected
                final Real a[:] = {1, 2, 3};
            algorithm
                y := 0;
                for i in 1:size(a, 1) loop
                    y := y + a[i] * t;
                end for;
            end f;
            model M
                Real x(start = 0, fixed = true);
            equation
                der(x) = f(1.0);
            end M;
        end P;
    "#;
    let trace = rumoca_contracts::test_support::simulate_model(source, "P.M", 1.0);
    let value = trace.final_value("x");
    assert!(
        (value - 6.0).abs() < 1e-6,
        "x must integrate 6, got {value}"
    );
}

// =============================================================================
// FUNC-039: Component bindings read inputs (MLS §12.4.4)
// =============================================================================

#[test]
fn func_039_protected_binding_reads_record_input_fields() {
    let trace = rumoca_contracts::test_support::simulate_model(
        r#"
        package F1
          record Data
            Real diameter_a;
            Real diameter_b;
            Boolean zeta1_at_a = true;
            Real zeta1;
          end Data;
          function k
            input Real D;
            input Real zeta;
            output Real y;
          algorithm
            y := zeta/D^2;
          end k;
          function loss
            input Real m;
            input Data data;
            output Real dp;
          protected
            Real k1 = k(if data.zeta1_at_a then data.diameter_a else data.diameter_b, data.zeta1);
          algorithm
            dp := k1*m;
          end loss;
          model Top
            parameter Data data(diameter_a = 0.1, diameter_b = 0.2, zeta1 = 1);
            Real dp = loss(1 + time, data);
          end Top;
        end F1;
    "#,
        "F1.Top",
        1.0,
    );
    assert!((trace.final_value("dp") - 200.0).abs() < 1e-9);
}

#[test]
fn func_027_vectorized_call_evaluates_at_translation() {
    // MLS §12.4.6: a scalar function applied to an array argument maps over
    // its elements, also when a parameter binding evaluates it at translation.
    let trace = rumoca_contracts::test_support::simulate_model(
        r#"
        model M
            function toK
                input Real c;
                output Real k;
            algorithm
                k := c + 273.15;
            end toK;
            function f
                input Real T;
                output Real y;
            protected
                Real invTK[2] = 1 ./ toK({10, 20});
            algorithm
                y := invTK[1]*T + invTK[2];
            end f;
            parameter Real p = f(300);
            Real y = p*time;
        end M;
    "#,
        "M",
        1.0,
    );
    let expected = 300.0 / 283.15 + 1.0 / 293.15;
    assert!((trace.final_value("y") - expected).abs() < 1e-12);
}

// =============================================================================
// FUNC-024: Uninitialized error (MLS §12.4.4)
// =============================================================================

#[test]
fn func_024_uninitialized_record_result_field_rejected() {
    expect_failure_in_phase_with_code(
        r#"
        package P
            record Data
                Real d;
                Real e;
                Real c0 = 1;
            end Data;
            function make
                input Real d;
                output Data data;
            algorithm
                data.d := d;
            end make;
            model M
                Data r = make(time);
            end M;
        end P;
    "#,
        "P.M",
        FailedPhase::ToDae,
        "ED022",
    );
}

// =============================================================================
// FUNC-040: Function arguments (MLS §12.4.2.1)
// =============================================================================

#[test]
fn func_040_function_name_and_partial_application_arguments() {
    let trace = rumoca_contracts::test_support::simulate_model(
        r#"
        package P
            partial function Equation
                input Real u;
                output Real y;
            end Equation;
            function apply
                input Equation f;
                input Real x;
                output Real y;
            algorithm
                y := f(x);
            end apply;
            function square
                extends Equation;
            algorithm
                y := u*u;
            end square;
            function affine
                extends Equation;
                input Real a;
                input Real b;
            algorithm
                y := a*u + b;
            end affine;
            model M
                Real s = apply(square, 1 + time);
                Real a = apply(function affine(a = 2, b = time), 3);
            end M;
        end P;
    "#,
        "P.M",
        1.0,
    );
    assert!((trace.final_value("s") - 4.0).abs() < 1e-12);
    assert!((trace.final_value("a") - 7.0).abs() < 1e-12);
}

// =============================================================================
// FUNC-026: Vectorization non-replaceable (MLS §12.4.6, §6.3.1, §7.3)
// =============================================================================

const FUNC_026_SOURCE: &str = r#"
    package V
        partial package Base
            replaceable function prop
                input Real T;
                output Real y;
            algorithm
                y := T;
            end prop;
        end Base;
        package A
            extends Base;
            redeclare function prop
                input Real T;
                output Real y;
            algorithm
                y := 2*T + 1;
            end prop;
        end A;
        partial package Abstract
            replaceable partial function prop
                input Real T;
                output Real y;
            end prop;
        end Abstract;
        model Selected
            replaceable package Medium = A;
            Real y[2] = Medium.prop({1, 2}*time);
        end Selected;
        model Unselected
            replaceable package Medium = Abstract;
            Real y[2] = Medium.prop({1, 2}*time);
        end Unselected;
    end V;
"#;

#[test]
fn func_026_vectorized_call_through_selected_replaceable_package_accepted() {
    let trace = rumoca_contracts::test_support::simulate_model(FUNC_026_SOURCE, "V.Selected", 1.0);
    assert!((trace.final_value("y[1]") - 3.0).abs() < 1e-12);
    assert!((trace.final_value("y[2]") - 5.0).abs() < 1e-12);
}

#[test]
fn func_026_vectorized_call_of_unselected_callee_rejected() {
    expect_failure_in_phase_with_code(FUNC_026_SOURCE, "V.Unselected", FailedPhase::ToDae, "ED008");
}

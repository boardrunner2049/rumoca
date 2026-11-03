use rumoca_ir_dae as dae;

/// These checked builtins are linear in every value operand and retain their
/// index map when applied to a derivative of the same shape.
pub(super) fn is_linear_tensor_map(builtin: dae::PureBuiltin) -> bool {
    matches!(
        builtin,
        dae::PureBuiltin::Vector
            | dae::PureBuiltin::Transpose
            | dae::PureBuiltin::Diagonal
            | dae::PureBuiltin::Skew
            | dae::PureBuiltin::PromotedCat1
            | dae::PureBuiltin::PromotedCat2
    )
}

/// Both value preflight and reconstruction consume this profile. The DAE
/// constructor owns arity and shape; the caller proves each operand can be
/// reconstructed from exact state or invariant values.
pub(super) fn is_materializable_builtin(builtin: dae::PureBuiltin) -> bool {
    is_linear_tensor_map(builtin)
        || is_first_order_builtin(builtin)
        || matches!(
            builtin,
            dae::PureBuiltin::Zeros
                | dae::PureBuiltin::Ones
                | dae::PureBuiltin::Identity
                | dae::PureBuiltin::Cross
                | dae::PureBuiltin::OuterProduct
                | dae::PureBuiltin::Sin
                | dae::PureBuiltin::Cos
                | dae::PureBuiltin::Sqrt
                | dae::PureBuiltin::Exp
                | dae::PureBuiltin::Log
                | dae::PureBuiltin::Atan2
                | dae::PureBuiltin::LinearSolve
        )
}

/// Elementary functions (MLS §3.7.1) whose closed derivative the
/// differentiator carries at order one only: `tan`, `sinh`, `cosh`, `tanh`,
/// `asin`, `acos`, `atan`, `log10`, `abs` (`sign(u)*du`), and `sign`, constant
/// between the zero crossings of its argument.
pub(super) fn is_first_order_builtin(builtin: dae::PureBuiltin) -> bool {
    matches!(
        builtin,
        dae::PureBuiltin::Tan
            | dae::PureBuiltin::Sinh
            | dae::PureBuiltin::Cosh
            | dae::PureBuiltin::Tanh
            | dae::PureBuiltin::Asin
            | dae::PureBuiltin::Acos
            | dae::PureBuiltin::Atan
            | dae::PureBuiltin::Log10
            | dae::PureBuiltin::Abs
            | dae::PureBuiltin::Sign
    )
}

/// Arithmetic whose source tensor operation has an exact rule through order two.
pub(super) fn is_differentiable_binary(operator: dae::BinaryOperator) -> bool {
    matches!(
        operator,
        dae::BinaryOperator::Add
            | dae::BinaryOperator::Subtract
            | dae::BinaryOperator::Multiply
            | dae::BinaryOperator::Divide
            | dae::BinaryOperator::ElementwiseAdd
            | dae::BinaryOperator::ElementwiseSubtract
            | dae::BinaryOperator::ElementwiseMultiply
            | dae::BinaryOperator::ElementwiseDivide
    )
}

/// Scalar `u ^ v` whose exponent `v` is time-invariant has the closed
/// derivative `v * u^(v-1) * du` (and, at order two,
/// `v*(v-1)*u^(v-2)*du^2 + v*u^(v-1)*d2u`). Only the base is differentiated.
pub(super) fn is_differentiable_power<'dae>(
    view: dae::DaeView<'dae>,
    operator: dae::BinaryOperator,
    lhs: dae::ExprId<'dae>,
    rhs: dae::ExprId<'dae>,
) -> bool {
    operator == dae::BinaryOperator::Power
        && super::equalities::is_time_invariant(view, rhs)
        && are_scalars(view, lhs, rhs)
}

/// Any scalar `u ^ v` has the closed first derivative
/// `v*u^(v-1)*du + u^v*log(u)*dv`: the invariant-exponent term plus the
/// exponent's, so both operands are differentiated. The exponent term has no
/// second-order rule; an invariant exponent keeps the rule above.
pub(super) fn is_differentiable_varying_power<'dae>(
    view: dae::DaeView<'dae>,
    operator: dae::BinaryOperator,
    lhs: dae::ExprId<'dae>,
    rhs: dae::ExprId<'dae>,
    order: u8,
) -> bool {
    order == 1 && operator == dae::BinaryOperator::Power && are_scalars(view, lhs, rhs)
}

fn are_scalars<'dae>(
    view: dae::DaeView<'dae>,
    lhs: dae::ExprId<'dae>,
    rhs: dae::ExprId<'dae>,
) -> bool {
    [lhs, rhs].into_iter().all(|operand| {
        view.expression(operand)
            .is_some_and(|node| node.value_type().dimensions().is_empty())
    })
}

/// The structural preflight and reconstruction share this closed derivative
/// profile; the first-order elementary functions have no second-order rule.
pub(super) fn is_differentiable_builtin(builtin: dae::PureBuiltin, order: u8) -> bool {
    match order {
        1 => is_materializable_builtin(builtin),
        2 => is_materializable_builtin(builtin) && !is_first_order_builtin(builtin),
        _ => false,
    }
}

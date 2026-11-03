//! Closed first-derivative rules without a second-order counterpart: a power
//! whose exponent varies in time and the elementary functions of
//! `is_first_order_builtin`. The preflight admits both at order one only.

use super::super::builtin_profiles::is_differentiable_power;
use super::*;

impl<'source, 'borrow, 'storage, 'target> ExpressionRebuilder<'source, 'borrow, 'storage, 'target> {
    /// `d(u^v)`: the invariant-exponent rule when the preflight proved `v`
    /// time-invariant or `v` differentiates to zero, otherwise
    /// `v*u^(v-1)*du + u^v*log(u)*dv`.
    pub(super) fn differentiate_power(
        &mut self,
        base: dae::ExprId<'source>,
        exponent: dae::ExprId<'source>,
        order: u8,
        provenance: dae::DaeProvenance,
    ) -> Result<Derivative<'target>, dae::DaeConstructionError> {
        if !is_differentiable_power(self.source, dae::BinaryOperator::Power, base, exponent)
            && let Derivative::Expression(rate) =
                self.differentiate_order(exponent, 1, provenance)?
        {
            assert_eq!(
                order, 1,
                "the preflight admits a varying exponent at order one"
            );
            let along_base = self.differentiate_invariant_power(base, exponent, 1, provenance)?;
            let u = self.differentiation_value(base, provenance)?;
            let v = self.differentiation_value(exponent, provenance)?;
            let multiply = dae::BinaryOperator::Multiply;
            let power = self.first_order_term(dae::BinaryOperator::Power, u, v, provenance)?;
            let logarithm = self
                .target
                .at(provenance)
                .builtin(dae::PureBuiltin::Log, [u])?;
            let scale = self.first_order_term(multiply, power, logarithm, provenance)?;
            let along_exponent = self.first_order_term(multiply, scale, rate, provenance)?;
            return self.combine_sum(
                dae::BinaryOperator::Add,
                along_base,
                Derivative::Expression(along_exponent),
                provenance,
            );
        }
        self.differentiate_invariant_power(base, exponent, order, provenance)
    }

    /// `d sinh(u) = cosh(u) du`, `d cosh(u) = sinh(u) du`, `d |u| = sign(u) du`,
    /// `d tan(u) = du/cos(u)^2`, `d tanh(u) = du/cosh(u)^2`,
    /// `d asin(u) = du/sqrt(1 - u^2) = -d acos(u)`, `d atan(u) = du/(1 + u^2)`,
    /// `d log10(u) = du/(u ln 10)`, elementwise like the geometry rules; and
    /// `sign(u)` is constant between the zero crossings of `u`, so its
    /// derivative is zero there.
    pub(super) fn differentiate_first_order_builtin(
        &mut self,
        builtin: dae::PureBuiltin,
        arguments: dae::ExpressionOperands<'source>,
        order: u8,
        provenance: dae::DaeProvenance,
    ) -> Result<Derivative<'target>, dae::DaeConstructionError> {
        assert_eq!(order, 1, "the preflight admits these builtins at order one");
        if builtin == dae::PureBuiltin::Sign {
            return Ok(Derivative::Zero);
        }
        let argument = arguments.get(0).expect("checked unary builtin operand");
        let Derivative::Expression(rate) = self.differentiate_order(argument, 1, provenance)?
        else {
            return Ok(Derivative::Zero);
        };
        let u = self.differentiation_value(argument, provenance)?;
        let (operator, factor) = self.first_order_factor(builtin, u, provenance)?;
        let result = self.first_order_term(operator, rate, factor, provenance)?;
        if builtin == dae::PureBuiltin::Acos {
            return self
                .target
                .at(provenance)
                .unary(dae::UnaryOperator::Negate, result)
                .map(Derivative::Expression);
        }
        Ok(Derivative::Expression(result))
    }

    /// The function of `u` that multiplies or divides `du`.
    fn first_order_factor(
        &mut self,
        builtin: dae::PureBuiltin,
        u: dae::ExprId<'target>,
        provenance: dae::DaeProvenance,
    ) -> Result<(dae::BinaryOperator, dae::ExprId<'target>), dae::DaeConstructionError> {
        let multiply = dae::BinaryOperator::ElementwiseMultiply;
        let divide = dae::BinaryOperator::ElementwiseDivide;
        let apply = |this: &mut Self, builtin: dae::PureBuiltin, value: dae::ExprId<'target>| {
            this.target.at(provenance).builtin(builtin, [value])
        };
        let real = |this: &mut Self, value: f64| {
            this.target
                .at(provenance)
                .literal(dae::DaeLiteral::Real(value))
        };
        Ok(match builtin {
            dae::PureBuiltin::Sinh => (multiply, apply(self, dae::PureBuiltin::Cosh, u)?),
            dae::PureBuiltin::Cosh => (multiply, apply(self, dae::PureBuiltin::Sinh, u)?),
            dae::PureBuiltin::Abs => (multiply, apply(self, dae::PureBuiltin::Sign, u)?),
            dae::PureBuiltin::Tan | dae::PureBuiltin::Tanh => {
                let inner = if builtin == dae::PureBuiltin::Tan {
                    dae::PureBuiltin::Cos
                } else {
                    dae::PureBuiltin::Cosh
                };
                let inner = apply(self, inner, u)?;
                (
                    divide,
                    self.first_order_term(multiply, inner, inner, provenance)?,
                )
            }
            dae::PureBuiltin::Asin | dae::PureBuiltin::Acos => {
                let one = real(self, 1.0)?;
                let square = self.first_order_term(multiply, u, u, provenance)?;
                let subtract = dae::BinaryOperator::ElementwiseSubtract;
                let rest = self.first_order_term(subtract, one, square, provenance)?;
                (divide, apply(self, dae::PureBuiltin::Sqrt, rest)?)
            }
            dae::PureBuiltin::Atan => {
                let one = real(self, 1.0)?;
                let square = self.first_order_term(multiply, u, u, provenance)?;
                let add = dae::BinaryOperator::ElementwiseAdd;
                (divide, self.first_order_term(add, one, square, provenance)?)
            }
            dae::PureBuiltin::Log10 => {
                let ln10 = real(self, std::f64::consts::LN_10)?;
                (
                    divide,
                    self.first_order_term(multiply, u, ln10, provenance)?,
                )
            }
            _ => unreachable!("only a first-order elementary builtin has a factor here"),
        })
    }

    fn first_order_term(
        &mut self,
        operator: dae::BinaryOperator,
        lhs: dae::ExprId<'target>,
        rhs: dae::ExprId<'target>,
        provenance: dae::DaeProvenance,
    ) -> Result<dae::ExprId<'target>, dae::DaeConstructionError> {
        self.target.at(provenance).binary(operator, lhs, rhs)
    }
}

//! Exhaustive relocation of expressions and event predicates. Literal numbers,
//! field ordinals, binder ordinals and external symbols are never rebased.
use super::{Map, Result, Shift};
use crate::schema::*;

impl Shift for RbcExprNode {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::StringConversion { value, format } => {
                value.shift(m)?;
                match format {
                    RbcStringConversionFormat::Options {
                        minimum_length,
                        left_justified,
                        significant_digits,
                    } => {
                        minimum_length.shift(m)?;
                        left_justified.shift(m)?;
                        significant_digits.shift(m)?;
                    }
                    RbcStringConversionFormat::Format { value } => value.shift(m)?,
                }
            }
            Self::Literal { .. } => {}
            Self::ClockTransfer {
                source,
                source_clock,
                target_clock,
                ..
            } => {
                source.shift(m)?;
                source_clock.shift(m)?;
                target_clock.shift(m)?;
            }
            // `value`, `fold`, `carried` and `definition` are owner-local
            // ordinals *within* a function, so linking must not rebase them.
            // Only the owning `FunctionId` moves, and shifting it is what
            // relocates the whole reference.
            Self::FunctionValue { function, .. }
            | Self::FunctionFoldParameter { function, .. }
            | Self::FunctionFoldOutput { function, .. } => function.shift(m)?,
            Self::Unsupported { detail } => {
                return Err(super::LinkError(format!(
                    "unsupported expression: {detail}"
                )));
            }
            Self::Coordinate { coordinate } => coordinate.shift(m)?,
            Self::Unary { operand, .. } => operand.shift(m)?,
            Self::Binary { lhs, rhs, .. } => {
                lhs.shift(m)?;
                rhs.shift(m)?;
            }
            Self::Conditional { branches, fallback } => {
                for b in branches {
                    b.condition.shift(m)?;
                    b.value.shift(m)?;
                }
                fallback.shift(m)?;
            }
            Self::Builtin { arguments, .. } => arguments.shift(m)?,
            Self::Array {
                elements,
                empty_type,
            } => {
                elements.shift(m)?;
                empty_type.shift(m)?;
            }
            Self::Record { ty, fields } => {
                ty.shift(m)?;
                fields.shift(m)?;
            }
            Self::Field { base, .. } => base.shift(m)?,
            Self::Range { start, step, stop } => {
                start.shift(m)?;
                step.shift(m)?;
                stop.shift(m)?;
            }
            Self::Comprehension { domain, body } => {
                domain.shift(m)?;
                body.shift(m)?;
            }
            Self::Index { base, subscripts } => {
                base.shift(m)?;
                subscripts.shift(m)?;
            }
            Self::ArrayUpdate {
                base,
                value,
                subscripts,
            } => {
                base.shift(m)?;
                value.shift(m)?;
                subscripts.shift(m)?;
            }
            Self::Call {
                owner,
                function,
                arguments,
                ..
            } => {
                owner.shift(m)?;
                function.shift(m)?;
                arguments.shift(m)?;
            }
        }
        Ok(())
    }
}

impl Shift for RbcSubscript {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Whole => Ok(()),
            Self::Index { expression } | Self::Slice { expression } => expression.shift(m),
        }
    }
}

impl Shift for RbcCoordinate {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Parameter { variable }
            | Self::Input { variable }
            | Self::State { variable }
            | Self::Derivative { variable }
            | Self::Algebraic { variable }
            | Self::DiscreteReal { variable }
            | Self::DiscreteValue { variable }
            | Self::PreState { variable }
            | Self::PreAlgebraic { variable }
            | Self::PreDiscreteReal { variable }
            | Self::PreDiscreteValue { variable } => variable.shift(m),
            Self::Time => Ok(()),
            Self::Binder { domain, .. } => domain.shift(m),
            Self::Condition { condition } => condition.shift(m),
            Self::FunctionParameter { function, .. } => function.shift(m),
            Self::ClockInterval { clock } => clock.shift(m),
            Self::Delay { delay } => delay.shift(m),
            Self::Previous { previous } => previous.shift(m),
            Self::Terminal { terminal } => terminal.shift(m),
        }
    }
}

impl Shift for RbcConditionNode {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Initial | Self::Always => {}
            Self::ClockActivation { clock } => clock.shift(m)?,
            Self::Unsupported { detail } => {
                return Err(super::LinkError(format!("unsupported condition: {detail}")));
            }
            Self::Relation { relation } => relation.shift(m)?,
            Self::Discrete { expression } => expression.shift(m)?,
            Self::Not { operand } => operand.shift(m)?,
            Self::And { lhs, rhs } | Self::Or { lhs, rhs } | Self::AnyRise { lhs, rhs } => {
                lhs.shift(m)?;
                rhs.shift(m)?;
            }
        }
        Ok(())
    }
}

impl Shift for RbcAction {
    fn shift(&mut self, m: &Map<'_>) -> Result<()> {
        match self {
            Self::Reinitialize { state, value } => {
                state.shift(m)?;
                value.shift(m)?;
            }
            Self::Warning { message, condition } => {
                message.shift(m)?;
                condition.shift(m)?;
            }
            Self::Assert { message, level } => {
                message.shift(m)?;
                level.shift(m)?;
            }
            Self::Terminate { message } => message.shift(m)?,
        }
        Ok(())
    }
}

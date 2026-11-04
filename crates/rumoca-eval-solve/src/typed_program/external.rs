//! Runtime of a cataloged external function: the reference semantics of
//! [`rumoca_core::ModelicaExternalFunction::step`].

use super::*;

impl EvalFrame<'_, '_> {
    pub(super) fn eval_external(
        &mut self,
        function: rumoca_ir_solve::SolveExternalFunction,
        arguments: &[SolveRegisterId],
        destinations: &[SolveRegisterId],
        provenance: Span,
    ) -> Result<(), TypedProgramEvalError> {
        let ([state], [result, state_out]) = (arguments, destinations) else {
            return Err(invalid_error("external function interface", provenance));
        };
        let state = self
            .read(*state, provenance)?
            .elements
            .iter()
            .map(|element| match element {
                SolveValueKind::Integer(value) => Ok(*value),
                _ => Err(invalid_error("external Integer state", provenance)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (draw, next) = function.step(&state);
        let result_type = self.destination_type(*result, provenance)?.clone();
        let draw = match result_type.element_type() {
            rumoca_ir_solve::SolveScalarType::Real {
                format: rumoca_ir_solve::SolveRealFormat::Binary32,
            } => SolveValueKind::Real32((draw as f32).to_bits()),
            _ => SolveValueKind::Real64(draw.to_bits()),
        };
        let draw = TypedValue::checked(result_type, vec![draw], provenance)?;
        self.write(*result, draw, provenance)?;
        let state_type = self.destination_type(*state_out, provenance)?.clone();
        let next = TypedValue::checked(
            state_type,
            next.into_iter().map(SolveValueKind::Integer).collect(),
            provenance,
        )?;
        self.write(*state_out, next, provenance)
    }
}

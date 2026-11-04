//! The typed-program operation that invokes a cataloged MLS §12.9 external
//! C function. Any other external body stays refused at the construction
//! boundary with its name (`UnsupportedExternalFunction`).

use super::*;

// The catalog an `External` operation draws from is
// `rumoca_core::ModelicaExternalFunction`, which documents its interfaces and
// reference semantics.
use rumoca_core::ModelicaExternalFunction;

impl<'program> TypedProgramBuilder<'program> {
    /// Invoke one cataloged external function: `(result, stateOut) =
    /// f(stateIn)` with `stateIn`/`stateOut` Integer vectors of the
    /// generator's state length and `result` a Real scalar.
    pub fn external(
        &mut self,
        function: ModelicaExternalFunction,
        arguments: &[ProgramRegister<'program>],
        provenance: Span,
    ) -> Result<Vec<ProgramRegister<'program>>, SolveProgramConstructionError> {
        require_provenance(provenance)?;
        let [state] = arguments else {
            return Err(SolveProgramConstructionError::InvalidCallInterface { provenance });
        };
        let state_type = self.register_type(*state, provenance)?.clone();
        if !matches!(state_type.element_type(), SolveScalarType::Integer(_))
            || state_type.dimensions() != [function.state_len()]
        {
            return Err(SolveProgramConstructionError::InvalidCallInterface { provenance });
        }
        let result_type = SolveValueType::scalar(SolveScalarType::real(self.arithmetic));
        let result = self.issue_register(result_type, provenance)?;
        let state_out = self.issue_register(state_type, provenance)?;
        self.push(
            SolveOperation::External {
                function,
                arguments: Box::new([state.id]),
                destinations: Box::new([result.id, state_out.id]),
            },
            provenance,
        );
        Ok(vec![result, state_out])
    }
}

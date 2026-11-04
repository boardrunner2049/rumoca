//! MLS §8.3.5 `(a, b, ...) = f(...)` inside a `when` clause.
//!
//! The call is lowered once and every receiver reads its own result of that
//! one call, so the receivers take values of the same function evaluation at
//! the event (MLS §12.4.4 results of one call). Each receiver is then an
//! ordinary event assignment of the chain, with the same owner, guard and
//! target ownership rules as `a = expr`.

use super::*;

impl<'dae> WhenLowering<'_, '_, '_, 'dae> {
    pub(super) fn lower_function_call_outputs(
        &mut self,
        owners: WhenSemanticOwners,
        guard: EventGuard<'dae>,
        outputs: &[VarName],
        function: &Expression,
        span: Span,
    ) -> Result<(), dae::DaeConstructionError> {
        let Expression::FunctionCall {
            name,
            args,
            is_constructor: false,
            ..
        } = function
        else {
            return Err(dae::DaeConstructionError::InvalidExpressionForm { span });
        };
        let provenance = dae::DaeProvenance::source(span)?;
        let functions = self.request.functions;
        let symbols = LoweringSymbols {
            coordinates: self.request.coordinates,
            functions,
            shapes: functions.shapes.model_values(),
            function_body: None,
            values: None,
            owner_clock: guard.owner_clock,
        };
        let call = lower_call_operands(
            self.construction,
            symbols,
            &HashMap::new(),
            name,
            args,
            provenance,
        )?;
        let results = call.results(self.construction, 0..outputs.len(), provenance)?;
        for (target, value) in outputs.iter().zip(results) {
            let coordinate = self.request.coordinates[target];
            if let Coordinate::DiscreteReal(target) = coordinate {
                self.target_owners
                    .claim_discrete_real(target, owners.source, span)?;
            }
            lower_when_assignment(
                self.construction,
                self.discrete_values,
                owners.discrete_value,
                coordinate,
                guard,
                value,
                provenance,
            )?;
        }
        Ok(())
    }
}

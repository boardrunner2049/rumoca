//! A call followed through its straight-line Modelica body.

use super::*;

impl<'source, 'borrow, 'storage, 'target> ExpressionRebuilder<'source, 'borrow, 'storage, 'target> {
    /// The operations the elementary dispatch leaves to function bodies: a
    /// call the preflight followed through its body, and a read of an earlier
    /// definition of the same body.
    pub(super) fn differentiate_function_body_operation(
        &mut self,
        source_id: dae::ExprId<'source>,
        order: u8,
        provenance: dae::DaeProvenance,
    ) -> Result<Derivative<'target>, dae::DaeConstructionError> {
        let source = self
            .source
            .expression(source_id)
            .expect("differentiable expression identity resolves");
        match source.operation() {
            dae::ExpressionOperation::Call { .. } => {
                self.differentiate_call_body(source_id, order, provenance)
            }
            dae::ExpressionOperation::FunctionValue { definition, .. } => {
                self.differentiate_order(definition.rhs(), order, provenance)
            }
            _ => unreachable!("differentiability preflight rejects this operation"),
        }
    }

    /// The derivative of a call is the derivative of its selected result
    /// expression with the call's arguments substituted. Its value is still
    /// the call itself wherever the rebuild needs it, so the body's assertions
    /// stay with that call.
    fn differentiate_call_body(
        &mut self,
        call: dae::ExprId<'source>,
        order: u8,
        provenance: dae::DaeProvenance,
    ) -> Result<Derivative<'target>, dae::DaeConstructionError> {
        let (result, nested) = self
            .function_context
            .straight_line_call_result(self.source, call)
            .expect("differentiability preflight admitted this straight-line call body");
        let previous = std::mem::replace(&mut self.function_context, nested);
        let differentiated = self.differentiate_order(result, order, provenance);
        self.function_context = previous;
        differentiated
    }
}

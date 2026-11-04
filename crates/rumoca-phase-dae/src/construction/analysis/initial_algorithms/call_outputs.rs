//! Call statements that bind outputs, replayed into one value per receiver.
//!
//! `(origin, rotation) := frame(reference)` evaluates the call once and then
//! assigns each receiver the output in its position (MLS 3.7 §11.2.1.2). The
//! replay keeps one expression per written coordinate, so each receiver takes
//! the call — its arguments substituted with the values they have before the
//! statement — projected to its output ordinal. The first output is the call
//! itself, as in any expression. Any other ordinal is a [`call_output`]
//! projection, which only the DAE expression lowering reads: it lowers to the
//! checked call result of that ordinal, the same owner a multi-result
//! equation's receivers read.

use super::*;

/// Field spelling of a call-output projection; never a source identifier.
const CALL_OUTPUT_FIELD: &str = "__rumoca_call_output__.";

/// Output `ordinal` of `call`, whose declaration is `declaration`.
pub(in crate::construction) fn call_output(
    call: Expression,
    ordinal: usize,
    declaration: rumoca_core::DefId,
    span: Span,
) -> Expression {
    if ordinal == 0 {
        return call;
    }
    Expression::FieldAccess {
        base: Box::new(call),
        field: format!("{CALL_OUTPUT_FIELD}{ordinal}"),
        field_def_id: declaration,
        span,
    }
}

/// The call and the output ordinal a [`call_output`] projection reads.
pub(in crate::construction) fn as_call_output(
    expression: &Expression,
) -> Option<(&Expression, usize)> {
    let Expression::FieldAccess { base, field, .. } = expression else {
        return None;
    };
    let ordinal = field.strip_prefix(CALL_OUTPUT_FIELD)?.parse().ok()?;
    matches!(
        base.as_ref(),
        Expression::FunctionCall {
            is_constructor: false,
            ..
        }
    )
    .then_some((base.as_ref(), ordinal))
}

impl Replay<'_> {
    /// Replay `(a, b, ...) := f(args)` into one value per receiver.
    pub(super) fn call_outputs(
        &self,
        comp: &rumoca_core::Reference,
        args: &[Expression],
        outputs: &[Option<rumoca_core::ComponentReference>],
        span: &Span,
        values: &mut ReplayValues,
    ) -> Result<(), ToDaeError> {
        let function = output_function(self.flat, comp, *span)?;
        let call = substitute(
            &Expression::FunctionCall {
                name: comp.clone(),
                args: args.to_vec(),
                is_constructor: false,
                span: *span,
            },
            values,
        );
        let mut assigned = Vec::with_capacity(outputs.len());
        for (ordinal, receiver) in outputs.iter().enumerate() {
            let Some(receiver) = receiver else { continue };
            let target = receiver_target(self.flat, receiver, *span)?;
            let declaration = function
                .outputs
                .get(ordinal)
                .and_then(|output| output.def_id)
                .ok_or_else(|| {
                    unsupported(
                        format!(
                            "`{}` has no declared output {} for receiver `{target}`",
                            comp.as_str(),
                            ordinal + 1
                        ),
                        *span,
                    )
                })?;
            let value = ReplayedValue {
                expression: call_output(call.clone(), ordinal, declaration, *span),
                span: *span,
            };
            reject_oversized_replay(&target, &value)?;
            assigned.push((target, value));
        }
        // Every argument is read before any receiver is assigned.
        values.extend(assigned);
        Ok(())
    }
}

/// The collected, pure function a binding call statement names.
fn output_function<'flat>(
    flat: &'flat flat::Model,
    comp: &rumoca_core::Reference,
    span: Span,
) -> Result<&'flat rumoca_core::Function, ToDaeError> {
    let function = flat.functions.get(comp.var_name()).ok_or_else(|| {
        unsupported(
            format!(
                "a call statement to `{}` names a callee the Flat function table does not \
                 register",
                comp.as_str()
            ),
            span,
        )
    })?;
    if !function.body_is_pure() {
        return Err(unsupported(
            format!(
                "a call statement to the impure function `{}` binds outputs; the \
                 initialization system re-evaluates an algorithm-determined value until it \
                 stops changing, which an impure call never does",
                function.name
            ),
            span,
        ));
    }
    Ok(function)
}

/// Prove every receiver of a call statement names one declared coordinate.
pub(super) fn require_receiver_targets(
    flat: &flat::Model,
    outputs: &[Option<rumoca_core::ComponentReference>],
    span: Span,
) -> Result<(), ToDaeError> {
    outputs
        .iter()
        .flatten()
        .try_for_each(|receiver| receiver_target(flat, receiver, span).map(|_| ()))
}

/// The one declared coordinate a call statement's receiver names.
fn receiver_target(
    flat: &flat::Model,
    receiver: &rumoca_core::ComponentReference,
    span: Span,
) -> Result<VarName, ToDaeError> {
    assignment_target(flat, receiver).ok_or_else(|| {
        unsupported(
            format!(
                "call output receiver `{}` is not one whole declared coordinate, addressed by \
                 literal subscripts at most",
                rumoca_core::component_ref_to_base_reference(receiver).var_name()
            ),
            span,
        )
    })
}

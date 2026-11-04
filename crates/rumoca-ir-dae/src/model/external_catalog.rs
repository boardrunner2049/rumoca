//! The cataloged external C function a checked external body implements.
//!
//! [`rumoca_core::ModelicaExternalFunction`] names the foreign functions the
//! native evaluators and the packaged C targets execute. A DAE external body
//! is that catalog entry only when it calls the entry's C symbol with the
//! entry's exact interface; this is the one proof every consumer reads.

use rumoca_core::ModelicaExternalFunction;

use super::{
    CoordinateView, DaeView, ExpressionOperation, ExternalArgumentView, ExternalLanguage,
    FunctionValueRole, FunctionView,
};

/// A checked external body proven to be one catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogExternal {
    pub function: ModelicaExternalFunction,
    /// The ordinal of the declared result that receives the Real draw; the
    /// other result receives the next state.
    pub draw_result: usize,
}

impl CatalogExternal {
    /// The ordinal of the declared result that receives the next state.
    #[must_use]
    pub const fn state_result(self) -> usize {
        1 - self.draw_result
    }
}

/// The catalog entry `function`'s external body implements, if any.
///
/// Every entry is an MSL random generator, `f(stateIn, stateOut, result)`:
/// one Integer-vector parameter passed first, then the state output, then
/// the Real draw output, and no C return value.
#[must_use]
pub fn catalog_external<'dae>(
    view: DaeView<'dae>,
    function: FunctionView<'dae>,
) -> Option<CatalogExternal> {
    let external = function.external()?;
    if external.language() != ExternalLanguage::C || external.result().is_some() {
        return None;
    }
    let catalog = ModelicaExternalFunction::from_c_symbol(external.symbol().as_str())?;
    let [parameter] = function.parameters().collect::<Vec<_>>()[..] else {
        return None;
    };
    let [
        ExternalArgumentView::Input(state_in),
        ExternalArgumentView::Output(state_out),
        ExternalArgumentView::Output(draw),
    ] = external.arguments().collect::<Vec<_>>()[..]
    else {
        return None;
    };
    let reads_parameter = view.expression(state_in).is_some_and(|node| {
        matches!(
            node.operation(),
            ExpressionOperation::Coordinate(CoordinateView::FunctionParameter(read))
                if read == parameter.id()
        )
    });
    if !reads_parameter {
        return None;
    }
    let outputs = function
        .values()
        .filter(|value| value.role() == FunctionValueRole::Output)
        .map(|value| value.id())
        .collect::<Vec<_>>();
    let draw_result = match outputs[..] {
        [first, second] if first == draw && second == state_out => 0,
        [first, second] if first == state_out && second == draw => 1,
        _ => return None,
    };
    Some(CatalogExternal {
        function: catalog,
        draw_result,
    })
}

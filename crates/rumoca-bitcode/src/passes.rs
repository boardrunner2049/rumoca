//! In-compiler bitcode passes: `Flat -> DAE -> RBC -> [passes] -> DAE`.
//!
//! External passes are programs that read and write `.rbc`
//! ([writing-a-bitcode-pass.md](../../../docs/writing-a-bitcode-pass.md)).
//! These are the same idea run inside `rumoca compile`, so an optimization
//! that lives here is still applied by the compiler, and one that is *moved*
//! here out of lowering keeps working for everyone who never touches an
//! artifact.
//!
//! A pass rewrites an [`RbcModel`] in place. The pipeline then drops
//! expressions nothing references any more, recomputes the summary and
//! validates, so a pass may simply stop referring to a node and leave the
//! bookkeeping to the pipeline. The result is rebuilt into a checked DAE by
//! [`crate::import()`], so no pass can produce a model the compiler's own
//! constructors would reject.

mod asserts;
mod constants;
mod evaluate;
pub mod external;
mod family_branches;
mod functions;
mod inline_constants;
mod parameter_bindings;
pub mod pipeline;
mod pure_calls;

use crate::schema::*;

/// A rewrite over the equation IR.
pub trait Pass: Sync {
    /// Stable name used on the command line and in reports.
    fn name(&self) -> &'static str;
    /// One line on what it does.
    fn description(&self) -> &'static str;
    /// Rewrite the model; report how many rewrites were made.
    fn run(&self, model: &mut RbcModel) -> Result<usize, PassError>;
}

#[derive(Debug, thiserror::Error)]
#[error("bitcode pass: {0}")]
pub struct PassError(pub String);

impl From<crate::link::LinkError> for PassError {
    fn from(error: crate::link::LinkError) -> Self {
        Self(error.0)
    }
}

/// What one pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassReport {
    pub pass: String,
    /// Rewrites the pass made (for an external pass, 1 if it changed the
    /// artifact at all).
    pub rewrites: usize,
    /// Expressions the pipeline dropped after it because nothing referenced
    /// them any more.
    pub removed_expressions: usize,
}

impl PassReport {
    /// Whether the model is different after this step.
    pub fn changed(&self) -> bool {
        self.rewrites > 0 || self.removed_expressions > 0
    }
}

static DEAD_EXPRESSIONS: DeadExpressions = DeadExpressions;
static PRUNE_FUNCTIONS: functions::PruneFunctions = functions::PruneFunctions;
static FOLD_PARAMETER_BINDINGS: parameter_bindings::FoldParameterBindings =
    parameter_bindings::FoldParameterBindings;
static SELECT_FAMILY_BRANCHES: family_branches::SelectFamilyBranches =
    family_branches::SelectFamilyBranches;
static INLINE_CONSTANTS: inline_constants::InlineConstants = inline_constants::InlineConstants;
static FOLD_CONSTANTS: constants::FoldConstants = constants::FoldConstants;
static FOLD_PURE_CALLS: pure_calls::FoldPureCalls = pure_calls::FoldPureCalls;
static FOLD_ASSERTS: asserts::FoldAsserts = asserts::FoldAsserts;

/// Every pass this build knows, in the order `--pass default` runs them.
///
/// Constants are inlined before folding, so the arithmetic they feed folds
/// in the same run. Folding first, then pruning: a call folded to its value leaves its
/// callee unreachable, and dropping it is the point of asking for
/// optimization. (The frontend must prune *before* it folds, because there
/// the result is the compiler's default artifact and a declared function
/// vanishing from it because of how one call site was written is not
/// recoverable -- TOOLBUG-029. A pass runs only when asked.)
pub fn catalog() -> [&'static dyn Pass; 8] {
    [
        &SELECT_FAMILY_BRANCHES,
        &FOLD_PARAMETER_BINDINGS,
        &INLINE_CONSTANTS,
        &FOLD_CONSTANTS,
        &FOLD_PURE_CALLS,
        &FOLD_ASSERTS,
        &PRUNE_FUNCTIONS,
        &DEAD_EXPRESSIONS,
    ]
}

/// The pass with this name.
pub fn find(name: &str) -> Option<&'static dyn Pass> {
    catalog().into_iter().find(|pass| pass.name() == name)
}

/// Run a pass pipeline over an artifact ([`pipeline`] has the grammar and
/// the scheduling rules).
pub fn run(file: &mut RbcFile, names: &[String]) -> Result<Vec<PassReport>, PassError> {
    let steps = pipeline::parse(names)?;
    let mut scheduler = pipeline::Scheduler::default();
    scheduler.run(file, &steps)?;
    Ok(scheduler.reports)
}

/// Which expressions are reachable from outside the arena, operands closed.
pub(crate) fn live_expressions(model: &RbcModel) -> Result<Vec<bool>, PassError> {
    let mut live = crate::link::expression_roots(model)?;
    // Operands are strictly earlier, so one backward sweep closes the set.
    for index in (0..live.len()).rev() {
        if !live[index] {
            continue;
        }
        let id = ExprId(index as u32);
        for operand in crate::build::references(id, &model.expressions[index].node) {
            if let Some(slot) = live.get_mut(operand.0 as usize) {
                *slot = true;
            }
        }
    }
    Ok(live)
}

/// Old-to-new ids for a keep mask.
pub(crate) fn compaction(keep: &[bool]) -> Vec<Option<u32>> {
    let mut next = 0u32;
    keep.iter()
        .map(|&kept| {
            kept.then(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

/// Drop every expression nothing references; return how many.
pub fn remove_dead_expressions(model: &mut RbcModel) -> Result<usize, PassError> {
    let live = live_expressions(model)?;
    let removed = live.iter().filter(|kept| !**kept).count();
    if removed == 0 {
        return Ok(0);
    }
    let functions: Vec<Option<u32>> = (0..model.functions.len() as u32).map(Some).collect();
    crate::link::renumber(model, &compaction(&live), &functions)?;
    Ok(removed)
}

/// Dead expression removal as a pass of its own.
struct DeadExpressions;

impl Pass for DeadExpressions {
    fn name(&self) -> &'static str {
        "dead-expressions"
    }
    fn description(&self) -> &'static str {
        "drop expressions nothing references"
    }
    fn run(&self, model: &mut RbcModel) -> Result<usize, PassError> {
        remove_dead_expressions(model)
    }
}

#[cfg(test)]
mod tests;

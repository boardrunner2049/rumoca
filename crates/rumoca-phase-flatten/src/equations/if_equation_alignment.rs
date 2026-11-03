//! Row pairing for run-time if-equations (MLS §8.3.4).

use rumoca_ir_ast as ast;

use super::SimpleEquation;

/// The declaration identity of an unsubscripted component reference `v` that
/// forms the left-hand side of an explicit `v = expr` equation.
fn assigned_target_identity(equation: &SimpleEquation) -> Option<Vec<rumoca_core::DefId>> {
    let ast::Expression::ComponentReference(reference) = &equation.lhs else {
        return None;
    };
    reference
        .parts
        .iter()
        .map(|part| part.subs.is_none().then_some(part.def_id).flatten())
        .collect()
}

/// Pair the rows of a run-time if-equation by the variable each row assigns.
///
/// MLS §8.3.4 makes each branch a set of equations; their textual order carries
/// no meaning. When every row of every branch is an explicit `v = expr`
/// assignment of a distinct unsubscripted variable and all branches assign the
/// same variables, each branch is reordered to the first branch's target
/// order, so the conditional row built for one position assigns one variable in
/// every branch. Any other shape keeps its written order.
pub(super) fn align_branches_by_assigned_target(
    branches: &mut [(ast::Expression, Vec<SimpleEquation>)],
    else_equations: &mut Vec<SimpleEquation>,
) {
    let Some((_, first)) = branches.first() else {
        return;
    };
    let Some(order) = first
        .iter()
        .map(assigned_target_identity)
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let distinct = order.iter().collect::<std::collections::HashSet<_>>();
    if distinct.len() != order.len() {
        return;
    }
    let aligned = |equations: &[SimpleEquation]| -> Option<Vec<usize>> {
        let targets = equations
            .iter()
            .map(assigned_target_identity)
            .collect::<Option<Vec<_>>>()?;
        order
            .iter()
            .map(|target| targets.iter().position(|candidate| candidate == target))
            .collect::<Option<Vec<_>>>()
            .filter(|positions| {
                positions
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    == positions.len()
            })
    };
    let Some(permutations) = branches
        .iter()
        .map(|(_, equations)| aligned(equations))
        .chain(std::iter::once(aligned(else_equations)))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let reorder = |equations: &mut Vec<SimpleEquation>, positions: &[usize]| {
        let mut taken = std::mem::take(equations)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        *equations = positions
            .iter()
            .map(|&position| taken[position].take().expect("positions are distinct"))
            .collect();
    };
    let (else_positions, branch_positions) = permutations
        .split_last()
        .expect("the else permutation is present");
    for ((_, equations), positions) in branches.iter_mut().zip(branch_positions) {
        reorder(equations, positions);
    }
    reorder(else_equations, else_positions);
}

/// Row `eq_idx` of a run-time if-equation as one `v = if c1 then e1 elseif ...
/// else eN` equation, when every branch's row assigns the same unsubscripted
/// variable `v`.
///
/// This is the same equation as the conditional residual `if c1 then v - e1
/// ... else v - eN` (MLS §8.3.4 selects one branch's row at every instant), in
/// the explicit form that names `v` as the row's defined variable.
pub(super) fn common_target_equation(
    branches: &[(ast::Expression, Vec<SimpleEquation>)],
    else_equations: &[SimpleEquation],
    eq_idx: usize,
    span: rumoca_core::Span,
) -> Option<SimpleEquation> {
    let else_equation = else_equations.get(eq_idx)?;
    let target = assigned_target_identity(else_equation)?;
    let same_target = branches.iter().all(|(_, equations)| {
        equations
            .get(eq_idx)
            .and_then(assigned_target_identity)
            .is_some_and(|identity| identity == target)
    });
    if !same_target {
        return None;
    }
    Some(SimpleEquation {
        lhs: else_equation.lhs.clone(),
        rhs: ast::Expression::If {
            branches: branches
                .iter()
                .map(|(condition, equations)| (condition.clone(), equations[eq_idx].rhs.clone()))
                .collect(),
            else_branch: std::sync::Arc::new(else_equation.rhs.clone()),
            span,
        },
    })
}

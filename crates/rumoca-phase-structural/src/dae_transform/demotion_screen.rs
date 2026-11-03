//! A structural lower bound on the residue a direct-state demotion leaves.
//!
//! Demoting state `x` with exact definition `x = rhs` (no retained manifold)
//! rewrites only the continuous owners that reference `x`: each `der(x)`
//! becomes the derivative of `rhs`, and each value of `x` stays, now as an
//! algebraic unknown. The unknown catalog is the dense variable order, so
//! `x`'s columns keep their positions (its derivative scalars before, its
//! value scalars after) and the column count is unchanged; the equation rows
//! are unchanged too. Every other owner keeps its exact rows, which the
//! incremental incidence already relies on.
//!
//! The derivative of `rhs` reads a closure of columns that `DerivativeClosure`
//! collects by following the path the differentiator takes: the values of the
//! non-state coordinates read outside a linear position (a sum, a sign, an
//! array element, or a product or quotient with a parameter-only factor); a
//! state's derivative, or the coordinates its explicit derivative definition
//! reads, which a first derivative rebuilds as written; and for an algebraic
//! the state anchor the demotion selects for its equality class, else the
//! closure of its causal definition. A call with a supplied derivative reads
//! its arguments' values and the derivatives of its tangent inputs only; an
//! index the differentiator projects onto an element also follows that
//! element. A function body reads no model coordinate, so a record field
//! projected from a call reads the model only through the call's arguments.
//! Paths the closure does not follow (a
//! derivative read, a field projected from a coordinate, an auxiliary block,
//! or a record component definition) leave the candidate unbounded. So a
//! touched row after the demotion is contained in its prior row without `x`'s
//! columns, plus `x`'s columns, plus, when it read `der(x)`, the closure. A
//! maximum matching on these superset rows is at least the true one, so its
//! residue `E + U - 2M` is a lower bound on the rebuilt system's residue: when
//! the bound is not below the current residue, the demotion cannot reduce it.

use rumoca_ir_dae as dae;

use super::demotion_bounds::DemotionRowBounds;
use super::{DirectStateConstraint, StateDefinition};
use crate::incidence::ReusableIncidence;
use crate::incidence::rows::IncidenceRowsBuilder;

/// The residue bound of one round's demotion candidates.
pub(super) struct DemotionScreen<'a> {
    base: &'a ReusableIncidence,
    bounds: &'a DemotionRowBounds,
    residue: usize,
    /// A maximum matching of the base rows, built on the first bound.
    base_matching: std::cell::OnceCell<Vec<Option<usize>>>,
}

impl<'a> DemotionScreen<'a> {
    pub(super) const fn new(
        base: &'a ReusableIncidence,
        bounds: &'a DemotionRowBounds,
        residue: usize,
    ) -> Self {
        Self {
            base,
            bounds,
            residue,
            base_matching: std::cell::OnceCell::new(),
        }
    }

    /// The residue lower bound of demoting `candidate`, when the bound
    /// applies: an exact expression definition whose derivative closure is
    /// followed completely, and a state with unknown columns.
    pub(super) fn residue_bound(
        &self,
        view: dae::DaeView<'_>,
        facts: &super::constraints::DifferentiationFacts,
        candidate: &DirectStateConstraint,
    ) -> Option<usize> {
        let StateDefinition::Expression(rhs) = candidate.rhs else {
            return None;
        };
        let state_columns = self.bounds.columns(candidate.state)?;
        let rhs = view.expression_id(rhs as usize)?;
        let mut derivative_columns = DerivativeClosure {
            view,
            facts,
            bounds: self.bounds,
            demoted: candidate.state,
            columns: Vec::new(),
            algebraics: std::collections::BTreeSet::new(),
            pending: vec![rhs],
            rates: Vec::new(),
        }
        .collect()?;
        derivative_columns.sort_unstable();
        derivative_columns.dedup();
        let touched = self.bounds.owners(candidate.state);
        let n_eq = self.base.rows().len();
        // An array coordinate adds its whole column range to every
        // derivative row, so a wide closure over a wide state could make the
        // superset quadratic in the array extent. The bound is then skipped
        // (the candidate is simply rebuilt), keeping it linear in the base
        // incidence.
        let derivative_rows = touched
            .iter()
            .filter_map(|&owner| self.base.owner_rows(owner))
            .flatten()
            .filter(|&row| {
                self.base
                    .rows()
                    .row(row)
                    .iter()
                    .any(|column| state_columns.contains(column))
            })
            .count();
        if derivative_rows.saturating_mul(derivative_columns.len())
            > self.base.rows().entry_count().max(n_eq)
        {
            return None;
        }
        let mut rows = IncidenceRowsBuilder::with_row_capacity(n_eq);
        let mut owner = 0;
        let mut owner_end = self.base.owner_rows(0).map_or(n_eq, |range| range.end);
        for row in 0..n_eq {
            while row >= owner_end {
                owner += 1;
                owner_end = self.base.owner_rows(owner).map_or(n_eq, |range| range.end);
            }
            let columns = self.base.rows().row(row);
            if touched.binary_search(&owner).is_err() {
                rows.push_canonical_row(columns);
                continue;
            }
            let read_derivative = columns.iter().any(|column| state_columns.contains(column));
            rows.push_unsorted(
                columns
                    .iter()
                    .copied()
                    .filter(|column| !state_columns.contains(column))
                    .chain(state_columns.clone())
                    .chain(
                        read_derivative
                            .then(|| derivative_columns.iter().copied())
                            .into_iter()
                            .flatten(),
                    ),
            );
        }
        let rows = rows.finish();
        let n_var = self.bounds.unknown_count();
        // Every superset row contains its base row, so the base system's
        // maximum matching stays valid and only grows by augmentation.
        let seed = self.base_matching.get_or_init(|| {
            crate::matching::grow_maximum_matching(n_var, self.base.rows(), vec![None; n_eq]).0
        });
        let (_, matched) = crate::matching::grow_maximum_matching(n_var, &rows, seed.clone());
        Some((n_eq - matched) + (n_var - matched))
    }

    /// Whether demoting `candidate` provably cannot reduce the residue.
    pub(super) fn cannot_reduce(
        &self,
        view: dae::DaeView<'_>,
        facts: &super::constraints::DifferentiationFacts,
        candidate: &DirectStateConstraint,
    ) -> Option<usize> {
        self.residue_bound(view, facts, candidate)
            .filter(|bound| *bound >= self.residue)
    }
}

/// The unknown columns reading `node` as a value shows: none for a state,
/// whose value is known, and its own columns for any other coordinate. The
/// demoted state's value becomes unknown, but every row the closure reaches
/// already carries the demoted state's columns.
fn value_columns(
    bounds: &DemotionRowBounds,
    node: dae::ExpressionView<'_>,
) -> Option<std::ops::Range<usize>> {
    if matches!(
        node.operation(),
        dae::ExpressionOperation::Coordinate(dae::CoordinateView::State(_))
    ) {
        return None;
    }
    bounds.columns(node.variable_coordinate()?.index())
}

/// Whether differentiating `node` can read columns outside its subexpression:
/// a derivative (whose definition is followed), or a record field projected
/// from a coordinate (whose component definitions are followed).
fn reads_unprojected_columns<'dae>(
    view: dae::DaeView<'dae>,
    node: dae::ExpressionView<'dae>,
) -> bool {
    match node.operation() {
        dae::ExpressionOperation::Coordinate(dae::CoordinateView::Derivative(_)) => true,
        dae::ExpressionOperation::Field { mut base, .. } => loop {
            match view.expression(base).map(|base| base.operation()) {
                Some(dae::ExpressionOperation::Field { base: inner, .. }) => base = inner,
                Some(dae::ExpressionOperation::Coordinate(_)) | None => break true,
                Some(_) => break false,
            }
        },
        _ => false,
    }
}

/// The unknown columns the derivative of a demotion's definition can read,
/// following every path the differentiator can take, or `None` when a path
/// reads columns this closure does not follow.
///
/// Per coordinate, differentiating reads nothing for a parameter or time; for
/// a state, its derivative, or the coordinates its explicit derivative
/// definition reads; for an algebraic, its auxiliary block or record component
/// definition (not followed), else the demotion's state anchor for its
/// equality class, else the derivative of its causal definition. A value
/// appears only where the chain rule keeps it as a coefficient. A derivative
/// read or a record field projected from a coordinate is not followed.
struct DerivativeClosure<'a, 'dae> {
    view: dae::DaeView<'dae>,
    facts: &'a super::constraints::DifferentiationFacts,
    bounds: &'a DemotionRowBounds,
    demoted: u32,
    columns: Vec<usize>,
    /// Algebraics already followed.
    algebraics: std::collections::BTreeSet<u32>,
    /// Expressions whose derivative is still to be followed.
    pending: Vec<dae::ExprId<'dae>>,
    /// Explicit derivative definitions, read as values.
    rates: Vec<dae::ExprId<'dae>>,
}

impl<'dae> DerivativeClosure<'_, 'dae> {
    fn collect(mut self) -> Option<Vec<usize>> {
        while let Some(expression) = self.pending.pop() {
            self.differentiate(expression)?;
        }
        // A first derivative rebuilds an explicit derivative definition as
        // written, so it reads exactly its own coordinates' columns.
        let (bounds, columns) = (self.bounds, &mut self.columns);
        dae::ExpressionTraversal::new().visit_pruned(self.view, self.rates, |_, node| {
            columns.extend(value_columns(bounds, node).into_iter().flatten());
            true
        });
        Some(self.columns)
    }

    /// Follow the derivative of `expression`. A subexpression in a linear
    /// position (a sum, a difference, a sign, an array element, or a product
    /// or quotient with a parameter-only factor or divisor) contributes only
    /// its derivative; at any other operation the subexpression's values
    /// appear as the chain rule's coefficients and the operands the
    /// differentiator reaches are followed.
    fn differentiate(&mut self, expression: dae::ExprId<'dae>) -> Option<()> {
        let mut stack = vec![expression];
        while let Some(expression) = stack.pop() {
            let node = self.view.expression(expression)?;
            match node.operation() {
                dae::ExpressionOperation::Literal(_) => {}
                dae::ExpressionOperation::Coordinate(dae::CoordinateView::Derivative(_)) => {
                    return None;
                }
                dae::ExpressionOperation::Coordinate(dae::CoordinateView::State(state)) => {
                    self.state(state.index());
                }
                dae::ExpressionOperation::Coordinate(dae::CoordinateView::Algebraic(algebraic)) => {
                    self.algebraic(algebraic)?;
                }
                dae::ExpressionOperation::Coordinate(_) => {}
                dae::ExpressionOperation::Unary {
                    operator: dae::UnaryOperator::Plus | dae::UnaryOperator::Negate,
                    operand,
                } => stack.push(operand),
                dae::ExpressionOperation::Binary {
                    operator:
                        dae::BinaryOperator::Add
                        | dae::BinaryOperator::Subtract
                        | dae::BinaryOperator::ElementwiseAdd
                        | dae::BinaryOperator::ElementwiseSubtract,
                    lhs,
                    rhs,
                } => stack.extend([lhs, rhs]),
                dae::ExpressionOperation::Binary {
                    operator:
                        dae::BinaryOperator::Multiply | dae::BinaryOperator::ElementwiseMultiply,
                    lhs,
                    rhs,
                } if self.reads_parameters_only(lhs) => stack.push(rhs),
                dae::ExpressionOperation::Binary {
                    operator:
                        dae::BinaryOperator::Multiply
                        | dae::BinaryOperator::ElementwiseMultiply
                        | dae::BinaryOperator::Divide
                        | dae::BinaryOperator::ElementwiseDivide,
                    lhs,
                    rhs,
                } if self.reads_parameters_only(rhs) => stack.push(lhs),
                dae::ExpressionOperation::Array(elements) => stack.extend(elements.iter()),
                dae::ExpressionOperation::Field { .. } => {
                    self.differentiate_coefficients(expression)?;
                }
                operation => {
                    self.read_values(expression);
                    stack.extend(self.derivative_operands(expression, &operation));
                }
            }
        }
        Some(())
    }

    /// The operands whose derivatives the chain rule reads at a nonlinear
    /// `expression`. A call with a supplied derivative reads the derivatives
    /// of its tangent inputs only. An index the differentiator projects onto
    /// an element of an array or causal definition reads that element's
    /// derivative, taken here beside the operands'.
    fn derivative_operands(
        &self,
        expression: dae::ExprId<'dae>,
        operation: &dae::ExpressionOperation<'dae>,
    ) -> Vec<dae::ExprId<'dae>> {
        if matches!(operation, dae::ExpressionOperation::Call { .. })
            && let Some(selected) = super::function_derivatives::select_derivative(
                self.view,
                &rumoca_eval_dae::FunctionCallContext::default(),
                expression,
                1,
            )
        {
            return selected
                .arguments
                .iter()
                .filter(|argument| argument.order > 0)
                .map(|argument| argument.source)
                .collect();
        }
        let mut operands: Vec<_> =
            super::component_projection::projected_element(self.view, self.facts, expression)
                .into_iter()
                .collect();
        dae::ExpressionTraversal::new().visit_pruned(self.view, [expression], |id, _| {
            if id == expression {
                return true;
            }
            operands.push(id);
            false
        });
        operands
    }

    /// Read the value of every coordinate in `expression`.
    fn read_values(&mut self, expression: dae::ExprId<'dae>) {
        let (bounds, columns) = (self.bounds, &mut self.columns);
        dae::ExpressionTraversal::new().visit_pruned(self.view, [expression], |_, node| {
            columns.extend(value_columns(bounds, node).into_iter().flatten());
            true
        });
    }

    /// Whether `expression` reads no coordinate but parameters, so its
    /// derivative is zero and its value reads no unknown column.
    fn reads_parameters_only(&self, expression: dae::ExprId<'dae>) -> bool {
        let mut invariant = true;
        dae::ExpressionTraversal::new().visit_pruned(self.view, [expression], |_, node| {
            invariant &= match node.operation() {
                dae::ExpressionOperation::Coordinate(coordinate) => {
                    matches!(coordinate, dae::CoordinateView::Parameter(_))
                }
                dae::ExpressionOperation::Field { .. } | dae::ExpressionOperation::Record(_) => {
                    false
                }
                _ => true,
            };
            invariant
        });
        invariant
    }

    /// Follow the derivative of a subexpression whose coordinates' values may
    /// all appear in its derivative. A record field projected from a call or
    /// constructor differentiates the projected function output, which reads
    /// the model only through the call's arguments, all in this subexpression.
    fn differentiate_coefficients(&mut self, expression: dae::ExprId<'dae>) -> Option<()> {
        let mut nodes = Vec::new();
        dae::ExpressionTraversal::new().visit_pruned(self.view, [expression], |id, node| {
            nodes.push((id, node));
            true
        });
        for (id, node) in nodes {
            if reads_unprojected_columns(self.view, node) {
                return None;
            }
            self.columns
                .extend(value_columns(self.bounds, node).into_iter().flatten());
            match node.operation() {
                dae::ExpressionOperation::Coordinate(dae::CoordinateView::State(state)) => {
                    self.state(state.index());
                }
                dae::ExpressionOperation::Coordinate(dae::CoordinateView::Algebraic(algebraic)) => {
                    self.algebraic(algebraic)?;
                }
                dae::ExpressionOperation::Index { .. } => {
                    // A projected element's derivative is followed as well.
                    self.pending
                        .extend(super::component_projection::projected_element(
                            self.view, self.facts, id,
                        ));
                }
                _ => {}
            }
        }
        Some(())
    }

    /// A state's derivative: its column, or the values its definition reads.
    fn state(&mut self, state: u32) {
        match self.facts.derivative_definitions[state as usize] {
            Some(definition) => self
                .rates
                .extend(self.view.expression_id(definition.expression as usize)),
            None => self
                .columns
                .extend(self.bounds.columns(state).into_iter().flatten()),
        }
    }

    fn algebraic(&mut self, algebraic: dae::AlgebraicId<'dae>) -> Option<()> {
        let index = algebraic.index();
        if !self.algebraics.insert(index) {
            return Some(());
        }
        if self.facts.auxiliary_blocks[index as usize].is_some()
            || self.facts.component_definitions[index as usize].is_some()
        {
            return None;
        }
        // A demotion differentiates an algebraic through the anchor selected
        // for it: a state's derivative, zero for a time-invariant class, or
        // with no anchor the derivative of its causal definition.
        match self
            .facts
            .equalities
            .anchor_for_demotion(index, self.demoted)
        {
            Some((super::equalities::EqualityAnchor::State(state), _)) => self.state(state),
            Some((super::equalities::EqualityAnchor::Invariant { .. }, _)) => {}
            None => {
                let definition = self.facts.algebraic_definition(self.view, algebraic)?;
                self.pending.push(definition);
            }
        }
        Some(())
    }
}

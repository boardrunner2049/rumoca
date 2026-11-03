//! Source-owned row bounds for direct-state candidate search.
//!
//! Checked function bodies reject model coordinates, so calls can expose state
//! reads only through their arguments. Counting the entire containing owner
//! also covers tensor projections without enumerating their scalar coordinates.

use rumoca_ir_dae as dae;

use super::ManifoldConstraint;

pub(super) struct DemotionRowBounds {
    state_rows: Vec<usize>,
    owner_rows: Vec<usize>,
    /// Continuous owners that reference each variable in any coordinate form,
    /// in ascending owner order. A direct-state demotion rewrites exactly the
    /// owners that mention the demoted variable, so this is the set of rows the
    /// incremental incidence reprojects; every other row is reused unchanged.
    variable_owners: Vec<Vec<usize>>,
    /// The unknown columns of each state and algebraic variable, in the dense
    /// variable order the incidence catalog uses (a state's columns are its
    /// derivative scalars, an algebraic's its value scalars); `None` for a
    /// variable with no unknown columns.
    variable_columns: Vec<Option<std::ops::Range<usize>>>,
    /// The total unknown column count.
    unknown_count: usize,
}

impl DemotionRowBounds {
    pub(super) fn collect(view: dae::DaeView<'_>) -> Self {
        let mut bounds = Self {
            state_rows: vec![0; view.variable_count()],
            owner_rows: Vec::new(),
            variable_owners: vec![Vec::new(); view.variable_count()],
            variable_columns: vec![None; view.variable_count()],
            unknown_count: 0,
        };
        for (id, variable) in view.variables() {
            if !matches!(
                variable.identity(),
                dae::VariableIdentity::State(_) | dae::VariableIdentity::Algebraic(_)
            ) {
                continue;
            }
            let count = variable.value_type().scalar_count().unwrap_or(0);
            let start = bounds.unknown_count;
            bounds.unknown_count += count;
            bounds.variable_columns[id.index() as usize] = Some(start..bounds.unknown_count);
        }
        let mut traversal = dae::ExpressionTraversal::new();
        let mut seen = vec![0; view.variable_count()];
        let mut roots = Vec::new();
        for (owner_index, owner) in view.continuous_owners().enumerate() {
            roots.clear();
            let rows = match owner {
                dae::ContinuousOwnerView::Residual { equation, .. } => {
                    roots.push(equation.residual());
                    view.expression(equation.residual())
                        .expect("checked residual resolves")
                        .value_type()
                        .scalar_count()
                        .expect("checked continuous residual has finite shape")
                }
                dae::ContinuousOwnerView::Structured { family, .. } => {
                    roots.extend(family.bodies().iter());
                    family.scalar_rows() as usize
                }
            };
            bounds.owner_rows.push(rows);
            let stamp = bounds.owner_rows.len();
            let mut accumulator = CoordinateAccumulator {
                seen: &mut seen,
                counts: &mut bounds.state_rows,
                variable_owners: &mut bounds.variable_owners,
            };
            traversal.visit_pruned(view, roots.iter().copied(), |_, node| {
                accumulator.record(
                    node,
                    OwnerRows {
                        stamp,
                        owner_index,
                        rows,
                    },
                );
                true
            });
        }
        bounds
    }

    /// Mask of the continuous owners a demotion of `variable` rewrites.
    ///
    /// Every owner referencing the variable in any coordinate is `true`, so the
    /// incremental incidence reprojects exactly those and reuses the rest.
    pub(super) fn touched_owners(&self, variable: u32) -> Vec<bool> {
        let mut mask = vec![false; self.owner_rows.len()];
        let owners = self.variable_owners.get(variable as usize);
        for &owner in owners.into_iter().flatten() {
            if let Some(entry) = mask.get_mut(owner) {
                *entry = true;
            }
        }
        mask
    }

    /// The unknown columns of `variable`, if it has any.
    pub(super) fn columns(&self, variable: u32) -> Option<std::ops::Range<usize>> {
        self.variable_columns
            .get(variable as usize)
            .cloned()
            .flatten()
    }

    /// The total unknown column count.
    pub(super) const fn unknown_count(&self) -> usize {
        self.unknown_count
    }

    /// Whether replacing holonomic `owner`, promoting `lifted` if any, provably
    /// cannot sort a system with `n_eq` rows and `residue`.
    ///
    /// The replacement keeps its owner's shape and the promotion keeps its
    /// variable's columns, so only the replaced owner's rows and the rows of
    /// the promoted variable's readers change. Removing those `k` rows from a
    /// perfect matching of the rebuilt system leaves a matching of unchanged
    /// rows, at most the current matching `M`, so sorting needs `U <= M + k`.
    pub(super) fn holonomic_cannot_sort(
        &self,
        owner: usize,
        lifted: Option<u32>,
        n_eq: usize,
        residue: usize,
    ) -> bool {
        let readers = lifted.map_or(&[][..], |variable| self.owners(variable));
        let changed = readers
            .iter()
            .filter(|&&reader| reader != owner)
            .chain([&owner])
            .map(|&changed| self.owner_rows.get(changed).copied().unwrap_or(usize::MAX))
            .fold(0usize, usize::saturating_add);
        // U - M = (residue + U - E) / 2 for M matched pairs.
        (residue + self.unknown_count).saturating_sub(n_eq) > changed.saturating_mul(2)
    }

    /// The continuous owners that reference `variable`, ascending.
    pub(super) fn owners(&self, variable: u32) -> &[usize] {
        self.variable_owners
            .get(variable as usize)
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn cannot_sort(
        &self,
        state: u32,
        residue: usize,
        manifold: &[ManifoldConstraint],
    ) -> bool {
        let mut rows = self.state_rows[state as usize];
        for entry in manifold {
            if let Some(lifted) = entry.lifted
                && lifted.state == state
            {
                rows = rows.saturating_add(self.owner_rows[lifted.owner_ordinal]);
            }
        }
        // Removing changed rows from a new matching leaves a source matching.
        // E and U are unchanged, so E + U - 2*M can fall by at most twice rows.
        residue > rows.saturating_mul(2)
    }
}

/// The current owner's stamp, ordinal, and scalar-row count.
#[derive(Clone, Copy)]
struct OwnerRows {
    stamp: usize,
    owner_index: usize,
    rows: usize,
}

/// Per-variable accumulators shared across one collection pass.
struct CoordinateAccumulator<'a> {
    seen: &'a mut [usize],
    counts: &'a mut [usize],
    variable_owners: &'a mut [Vec<usize>],
}

impl CoordinateAccumulator<'_> {
    fn record(&mut self, node: dae::ExpressionView<'_>, owner: OwnerRows) {
        let Some(variable) = node.variable_coordinate() else {
            return;
        };
        let index = variable.index() as usize;
        if self.seen[index] != owner.stamp {
            self.seen[index] = owner.stamp;
            self.counts[index] = self.counts[index].saturating_add(owner.rows);
            self.variable_owners[index].push(owner.owner_index);
        }
    }
}

//! Graph-only sortability screen for holonomic candidates.
//!
//! A holonomic replacement keeps its owner's shape and a lifted promotion keeps
//! its variable's columns, so the rebuilt system has the pass model's rows and
//! unknowns, and only the replaced owner's rows and the rows of the promoted
//! variable's readers change (the invariant
//! [`DemotionRowBounds::holonomic_cannot_sort`](super::demotion_bounds::DemotionRowBounds)
//! also rests on). Every other row keeps its exact incident unknowns. A
//! perfect matching of the rebuilt system therefore restricts to a matching of
//! the pass model's unchanged rows that covers every one of them. When the
//! maximum matching of those rows leaves one unmatched, no replacement of the
//! changed rows can sort the system, whatever the rebuild produces for them.

use crate::incidence::ReusableIncidence;
use crate::incidence::rows::{IncidenceRows, IncidenceRowsBuilder};
use crate::matching::grow_maximum_matching;

/// The pass model's incidence and one maximum matching of it, shared by every
/// candidate the pass screens.
pub(super) struct SortabilityScreen<'a> {
    base: &'a ReusableIncidence,
    unknowns: usize,
    matching: Vec<Option<usize>>,
}

impl<'a> SortabilityScreen<'a> {
    pub(super) fn new(base: &'a ReusableIncidence, unknowns: usize) -> Self {
        let rows = base.rows().len();
        let (matching, _) = grow_maximum_matching(unknowns, base.rows(), vec![None; rows]);
        Self {
            base,
            unknowns,
            matching,
        }
    }

    /// Whether rewriting exactly the rows of `changed_owners` provably leaves
    /// the system without a perfect matching. An owner this incidence does not
    /// hold proves nothing, so the screen then answers `false`.
    pub(super) fn cannot_sort(&self, changed_owners: impl IntoIterator<Item = usize>) -> bool {
        let rows = self.base.rows();
        let mut changed = vec![false; rows.len()];
        for owner in changed_owners {
            let Some(range) = self.base.owner_rows(owner) else {
                return false;
            };
            for row in range {
                changed[row] = true;
            }
        }
        let unchanged = changed.iter().filter(|changed| !**changed).count();
        let (masked, seed) = self.unchanged_rows(rows, &changed);
        let (_, matched) = grow_maximum_matching(self.unknowns, &masked, seed);
        matched < unchanged
    }

    /// The pass incidence with every changed row emptied, and the pass matching
    /// restricted to the unchanged rows as the growth seed.
    fn unchanged_rows(
        &self,
        rows: &IncidenceRows,
        changed: &[bool],
    ) -> (IncidenceRows, Vec<Option<usize>>) {
        let mut builder = IncidenceRowsBuilder::with_row_capacity(rows.len());
        let mut seed = Vec::with_capacity(rows.len());
        for (row, &changed) in changed.iter().enumerate() {
            if changed {
                builder.push_canonical_row(&[]);
                seed.push(None);
            } else {
                builder.push_canonical_row(rows.row(row));
                seed.push(self.matching[row]);
            }
        }
        (builder.finish(), seed)
    }
}

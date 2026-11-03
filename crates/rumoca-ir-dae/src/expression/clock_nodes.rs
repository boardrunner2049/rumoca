use super::*;
use crate::clocks::ClockKind;

impl<'dae> ExpressionAt<'_, 'dae> {
    /// Transfer the last available value of `source` onto an exactly-derived target clock.
    pub fn clock_transfer(
        self,
        kind: ClockTransferKind,
        source: ExprId<'dae>,
        source_clock: ClockId<'dae>,
        target_clock: ClockId<'dae>,
    ) -> Result<ExprId<'dae>, DaeConstructionError> {
        let source_entry = self
            .storage
            .clocks
            .get(source_clock.index() as usize)
            .ok_or_else(|| unknown("clock", source_clock.index(), self.provenance))?;
        let target_entry = self
            .storage
            .clocks
            .get(target_clock.index() as usize)
            .ok_or_else(|| unknown("clock", target_clock.index(), self.provenance))?;
        require_transfer_relation(
            kind,
            (source_clock.index(), source_entry.kind),
            target_entry.kind,
            self.provenance,
        )?;
        require_source_clock(self.storage, source.index(), source_clock, self.provenance)?;
        let ty = self.storage.expr_type(source, self.provenance)?.clone();
        let variability = self.storage.expr_variability(source, self.provenance)?;
        let binder_domain = self.storage.expr_binder_domain(source, self.provenance)?;
        let ty = self.storage.intern_type(ty, self.provenance)?;
        self.insert(
            ExprNode::ClockTransfer {
                kind,
                source: source.index(),
                source_clock: source_clock.index(),
                target_clock: target_clock.index(),
            },
            ty,
            variability,
            binder_domain,
        )
    }
}

fn require_source_clock(
    storage: &Storage,
    root: u32,
    expected: ClockId<'_>,
    at: DaeProvenance,
) -> Result<(), DaeConstructionError> {
    let mut pending = vec![root];
    let mut found = false;
    while let Some(raw) = pending.pop() {
        let node = storage.expressions.nodes.get(raw as usize).ok_or_else(|| {
            DaeConstructionError::UnknownId {
                kind: "expression",
                index: raw,
                span: at.span(),
            }
        })?;
        match node {
            ExprNode::Coordinate(
                Coordinate::DiscreteReal(variable) | Coordinate::DiscreteValue(variable),
            ) => {
                let Some(&owner) = storage.clock_ownership_by_variable.get(variable) else {
                    return Err(DaeConstructionError::MissingClockOwnership {
                        variable: *variable,
                        clock: expected.index(),
                        span: at.span(),
                    });
                };
                if storage.clock_ownerships[owner as usize].clock != expected.index() {
                    return Err(DaeConstructionError::MissingClockOwnership {
                        variable: *variable,
                        clock: expected.index(),
                        span: at.span(),
                    });
                }
                found = true;
            }
            ExprNode::Coordinate(Coordinate::Previous(previous)) => {
                let entry = &storage.previous_values[*previous as usize];
                if entry.clock != expected.index() {
                    return Err(DaeConstructionError::MissingClockOwnership {
                        variable: entry.variable,
                        clock: expected.index(),
                        span: at.span(),
                    });
                }
                found = true;
            }
            ExprNode::ClockTransfer { target_clock, .. } => {
                if *target_clock != expected.index() {
                    return Err(DaeConstructionError::InvalidClockedOperand {
                        operator: "clocked value conversion",
                        span: at.span(),
                    });
                }
                found = true;
            }
            _ => node.for_each_child(&storage.expressions, |child| pending.push(child)),
        }
    }
    if found {
        Ok(())
    } else {
        Err(DaeConstructionError::InvalidClockedOperand {
            operator: "clocked value conversion",
            span: at.span(),
        })
    }
}

/// Prove that `target` is exactly `kind` applied to `source` (MLS §16.5.2).
///
/// A periodic transfer maps the source lattice onto the target's. An event
/// clock has no lattice, so its only transfer is `shiftSample(u, counter)`
/// with resolution 1, onto the clock that skips `counter` more ticks of the
/// same unshifted event clock.
fn require_transfer_relation(
    kind: ClockTransferKind,
    (source_index, source): (u32, ClockKind),
    target: ClockKind,
    at: DaeProvenance,
) -> Result<(), DaeConstructionError> {
    let invalid = || DaeConstructionError::InvalidClockedOperand {
        operator: "clocked value conversion",
        span: at.span(),
    };
    match (source, target) {
        (ClockKind::Periodic(source), ClockKind::Periodic(target)) => {
            let expected = kind.target_lattice(source.lattice()).map_err(|source| {
                DaeConstructionError::InvalidClockLattice {
                    source,
                    span: at.span(),
                }
            })?;
            (expected == target.lattice())
                .then_some(())
                .ok_or_else(invalid)
        }
        (
            ClockKind::Triggered(_) | ClockKind::Shifted { .. },
            ClockKind::Shifted {
                base,
                counter: target_counter,
                ..
            },
        ) => {
            let (source_base, source_counter) = match source {
                ClockKind::Shifted { base, counter, .. } => (base, counter),
                _ => (source_index, 0),
            };
            let ClockTransferKind::ShiftSample {
                counter,
                resolution: 1,
            } = kind
            else {
                return Err(invalid());
            };
            let shifted = i64::from(source_counter) + counter;
            (source_base == base && shifted == i64::from(target_counter))
                .then_some(())
                .ok_or_else(invalid)
        }
        _ => Err(invalid()),
    }
}

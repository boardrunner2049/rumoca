use std::marker::PhantomData;

use rumoca_core::{ClockLattice, PeriodicClockSchedule};

use crate::model::{Storage, check_provenance, checked_u32, unknown};
use crate::{
    ClockId, ClockOwnershipId, ConditionId, DaeConstructionError, DaeProvenance, DiscreteRealId,
    DiscreteValueId, ParameterId, PeriodicClockId, VariableId, VariableRole,
};

/// Exact MLS §16.5.2 relationship carried by one cross-clock value transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockTransferKind {
    SubSample { factor: i64 },
    SuperSample { factor: i64 },
    ShiftSample { counter: i64, resolution: i64 },
    BackSample { counter: i64, resolution: i64 },
}

impl ClockTransferKind {
    pub(crate) fn target_lattice(
        self,
        source: ClockLattice,
    ) -> Result<ClockLattice, rumoca_core::ClockLatticeErrorKind> {
        match self {
            Self::SubSample { factor } => source.sub_sample(factor),
            Self::SuperSample { factor } => source.super_sample(factor),
            Self::ShiftSample {
                counter,
                resolution,
            } => source.shift_sample(counter, resolution),
            Self::BackSample {
                counter,
                resolution,
            } => source.back_sample(counter, resolution),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClockKind {
    Periodic(PeriodicClockSchedule),
    /// MLS §3.7.5 `sample(start, interval)` whose operands are parameter
    /// expressions without a translation-time value: tick `k` is
    /// `start + k * interval`, both read from these parameter variables once
    /// initialization has fixed them.
    ParametricSample {
        start: u32,
        interval: u32,
    },
    Triggered(u32),
    /// MLS §16.5.2 `shiftSample(u, counter)` of an event clock: the clock
    /// `base` without its first `counter` ticks, ticking on `condition`.
    Shifted {
        base: u32,
        counter: u32,
        condition: u32,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockEntry {
    pub(crate) kind: ClockKind,
    /// Set on a periodic clock that is an MLS §3.7.5 `sample(start,
    /// interval)` time event rather than an MLS §16 clock.
    pub(crate) sample_event: Option<SampleEventEntry>,
    pub(crate) provenance: DaeProvenance,
}

impl ClockEntry {
    /// Whether this clock is an MLS §3.7.5 `sample(start, interval)` time
    /// event: its `when` bodies are event-style, not a synchronous partition.
    pub(crate) const fn is_sample_event(&self) -> bool {
        self.sample_event.is_some() || matches!(self.kind, ClockKind::ParametricSample { .. })
    }
}

/// The MLS §3.7.5 origin of a periodic clock (SPEC_0044 ME-EVENT-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SampleEventEntry {
    /// Both operands are fixed at translation.
    Fixed,
    /// An operand reads a tunable parameter: the lattice is the operands'
    /// translation-time value, and these generated Real parameters, bound to
    /// the operands, hold the start and interval initialization settles.
    Tunable { start: u32, interval: u32 },
}

/// The MLS §3.7.5 origin of a periodic clock, as its view reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleEvent<'dae> {
    /// Both operands are fixed at translation.
    Fixed,
    /// An operand reads a tunable parameter; the schedule an executor that
    /// sets parameters before initialization ticks on is these parameters'
    /// initialization values, the lattice their translation-time ones.
    Tunable {
        start: ParameterId<'dae>,
        interval: ParameterId<'dae>,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockOwnershipEntry {
    pub(crate) variable: u32,
    /// Which clocked coordinate role the owned variable was checked to have.
    ///
    /// Ownership is only ever minted after `own` rejects any other role, so
    /// the entry carries the outcome of that check rather than leaving a
    /// reader to re-derive it from the variable arena. The wire form omits it:
    /// replay re-issues the same checked operation and reproduces it.
    #[serde(skip)]
    pub(crate) kind: ClockedVariableKind,
    pub(crate) clock: u32,
    pub(crate) sampled: bool,
    pub(crate) provenance: DaeProvenance,
}

#[derive(Debug, Clone, Copy)]
pub enum ClockOperation<'dae> {
    Periodic(&'dae PeriodicClockSchedule),
    /// MLS §3.7.5 `sample(start, interval)` whose start and interval are the
    /// values these parameters take at initialization.
    ParametricSample {
        start: ParameterId<'dae>,
        interval: ParameterId<'dae>,
    },
    Triggered(ConditionId<'dae>),
    /// MLS §16.5.2 `shiftSample(u, counter)` of the event clock `base`
    /// (never itself shifted): it ticks at every tick of `base` after the
    /// first `counter`, which `condition` selects.
    Shifted {
        base: ClockId<'dae>,
        counter: u32,
        condition: ConditionId<'dae>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct ClockView<'dae> {
    pub(crate) operation: ClockOperation<'dae>,
    pub(crate) sample_event: Option<SampleEvent<'dae>>,
    pub(crate) provenance: DaeProvenance,
}

impl<'dae> ClockView<'dae> {
    pub const fn operation(self) -> ClockOperation<'dae> {
        self.operation
    }

    /// The MLS §3.7.5 origin of a [`ClockOperation::Periodic`] clock that is a
    /// `sample(start, interval)` time event; `None` for an MLS §16 clock.
    pub const fn sample_event(self) -> Option<SampleEvent<'dae>> {
        self.sample_event
    }

    pub const fn provenance(self) -> DaeProvenance {
        self.provenance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockedVariableKind {
    DiscreteReal,
    DiscreteValue,
}

#[derive(Debug, Clone, Copy)]
pub struct ClockOwnershipView<'dae> {
    pub(crate) variable: VariableId<'dae>,
    pub(crate) kind: ClockedVariableKind,
    pub(crate) clock: ClockId<'dae>,
    pub(crate) sampled: bool,
    pub(crate) provenance: DaeProvenance,
}

impl<'dae> ClockOwnershipView<'dae> {
    pub const fn variable(self) -> VariableId<'dae> {
        self.variable
    }

    pub const fn kind(self) -> ClockedVariableKind {
        self.kind
    }

    pub const fn clock(self) -> ClockId<'dae> {
        self.clock
    }

    /// Whether this owner is the coordinate defined by MLS §16.5.1
    /// `sample(u)`, whose source is read at the clock tick's left limit.
    pub const fn sampled(self) -> bool {
        self.sampled
    }

    pub const fn provenance(self) -> DaeProvenance {
        self.provenance
    }
}

pub struct Clocks<'storage, 'dae> {
    pub(crate) source_map: &'storage rumoca_core::SourceMap,
    pub(crate) storage: &'storage mut Storage,
    pub(crate) marker: PhantomData<&'dae mut &'dae ()>,
}

impl<'dae> Clocks<'_, 'dae> {
    pub fn periodic(
        &mut self,
        lattice: ClockLattice,
        provenance: DaeProvenance,
    ) -> Result<PeriodicClockId<'dae>, DaeConstructionError> {
        let schedule = PeriodicClockSchedule::absolute(lattice).map_err(|source| {
            DaeConstructionError::InvalidClockLattice {
                source,
                span: provenance.span(),
            }
        })?;
        self.scheduled(schedule, provenance)
    }

    pub fn scheduled(
        &mut self,
        schedule: PeriodicClockSchedule,
        provenance: DaeProvenance,
    ) -> Result<PeriodicClockId<'dae>, DaeConstructionError> {
        let schedule = match schedule.anchor() {
            rumoca_core::ClockPhaseAnchor::Absolute => {
                PeriodicClockSchedule::absolute(schedule.lattice())
            }
            rumoca_core::ClockPhaseAnchor::SimulationStart => {
                PeriodicClockSchedule::simulation_start_relative(schedule.lattice())
            }
        }
        .map_err(|source| DaeConstructionError::InvalidClockLattice {
            source,
            span: provenance.span(),
        })?;
        self.insert(ClockKind::Periodic(schedule), provenance)
            .map(|clock| PeriodicClockId::from_raw(clock.index()))
    }

    /// The MLS §3.7.5 `sample(start, interval)` time event on an exact
    /// schedule. `operands`, when an operand reads a tunable parameter, are
    /// the generated Real parameters bound to the start and interval; the
    /// schedule is then their translation-time value.
    pub fn sample_event(
        &mut self,
        schedule: PeriodicClockSchedule,
        operands: Option<(ParameterId<'dae>, ParameterId<'dae>)>,
        provenance: DaeProvenance,
    ) -> Result<PeriodicClockId<'dae>, DaeConstructionError> {
        let sample_event = match operands {
            None => SampleEventEntry::Fixed,
            Some((start, interval)) => {
                self.expect_parameters([start, interval], provenance)?;
                SampleEventEntry::Tunable {
                    start: start.index(),
                    interval: interval.index(),
                }
            }
        };
        let clock = self.scheduled(schedule, provenance)?;
        self.storage.clocks[clock.index() as usize].sample_event = Some(sample_event);
        Ok(clock)
    }

    fn expect_parameters(
        &self,
        parameters: [ParameterId<'dae>; 2],
        provenance: DaeProvenance,
    ) -> Result<(), DaeConstructionError> {
        for parameter in parameters {
            let entry = self.storage.variable(parameter.index(), provenance)?;
            if entry.role != VariableRole::Parameter {
                return Err(DaeConstructionError::InvalidVariableRole {
                    name: entry.name.clone(),
                    span: provenance.span(),
                });
            }
        }
        Ok(())
    }

    /// The MLS §3.7.5 `sample(start, interval)` time event whose start and
    /// interval are the initialization values of two Real parameters.
    pub fn parametric_sample(
        &mut self,
        start: ParameterId<'dae>,
        interval: ParameterId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockId<'dae>, DaeConstructionError> {
        check_provenance(self.source_map, provenance)?;
        self.expect_parameters([start, interval], provenance)?;
        self.insert(
            ClockKind::ParametricSample {
                start: start.index(),
                interval: interval.index(),
            },
            provenance,
        )
    }

    pub fn triggered(
        &mut self,
        condition: ConditionId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockId<'dae>, DaeConstructionError> {
        check_provenance(self.source_map, provenance)?;
        self.storage
            .conditions
            .get(condition.index() as usize)
            .ok_or_else(|| unknown("condition", condition.index(), provenance))?;
        self.insert(ClockKind::Triggered(condition.index()), provenance)
    }

    /// The MLS §16.5.2 `shiftSample(base, counter)` of the event clock `base`,
    /// whose ticks after the first `counter` of `base` are selected by
    /// `condition`. A shift of a shifted clock composes onto its base, so
    /// `base` here is always an unshifted event clock.
    pub fn shifted(
        &mut self,
        base: ClockId<'dae>,
        counter: u32,
        condition: ConditionId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockId<'dae>, DaeConstructionError> {
        check_provenance(self.source_map, provenance)?;
        let entry = self
            .storage
            .clocks
            .get(base.index() as usize)
            .ok_or_else(|| unknown("clock", base.index(), provenance))?;
        if !matches!(entry.kind, ClockKind::Triggered(_)) || counter == 0 {
            return Err(DaeConstructionError::InvalidClockedOperand {
                operator: "shiftSample of an event clock",
                span: provenance.span(),
            });
        }
        self.storage
            .conditions
            .get(condition.index() as usize)
            .ok_or_else(|| unknown("condition", condition.index(), provenance))?;
        self.insert(
            ClockKind::Shifted {
                base: base.index(),
                counter,
                condition: condition.index(),
            },
            provenance,
        )
    }

    pub fn own_discrete_real(
        &mut self,
        clock: ClockId<'dae>,
        variable: DiscreteRealId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockOwnershipId<'dae>, DaeConstructionError> {
        self.own(
            clock,
            variable.index(),
            ClockedVariableKind::DiscreteReal,
            false,
            provenance,
        )
    }

    pub fn own_discrete_value(
        &mut self,
        clock: ClockId<'dae>,
        variable: DiscreteValueId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockOwnershipId<'dae>, DaeConstructionError> {
        self.own(
            clock,
            variable.index(),
            ClockedVariableKind::DiscreteValue,
            false,
            provenance,
        )
    }

    /// Own a discrete Real coordinate defined by MLS §16.5.1 `sample(u)`.
    pub fn own_sampled_discrete_real(
        &mut self,
        clock: ClockId<'dae>,
        variable: DiscreteRealId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockOwnershipId<'dae>, DaeConstructionError> {
        self.own(
            clock,
            variable.index(),
            ClockedVariableKind::DiscreteReal,
            true,
            provenance,
        )
    }

    /// Own a discrete-valued coordinate defined by MLS §16.5.1 `sample(u)`.
    pub fn own_sampled_discrete_value(
        &mut self,
        clock: ClockId<'dae>,
        variable: DiscreteValueId<'dae>,
        provenance: DaeProvenance,
    ) -> Result<ClockOwnershipId<'dae>, DaeConstructionError> {
        self.own(
            clock,
            variable.index(),
            ClockedVariableKind::DiscreteValue,
            true,
            provenance,
        )
    }

    fn insert(
        &mut self,
        kind: ClockKind,
        provenance: DaeProvenance,
    ) -> Result<ClockId<'dae>, DaeConstructionError> {
        check_provenance(self.source_map, provenance)?;
        let raw = checked_u32(self.storage.clocks.len(), "clock arena", provenance)?;
        self.storage.clocks.push(ClockEntry {
            kind,
            sample_event: None,
            provenance,
        });
        Ok(ClockId::from_raw(raw))
    }

    fn own(
        &mut self,
        clock: ClockId<'dae>,
        variable: u32,
        kind: ClockedVariableKind,
        sampled: bool,
        provenance: DaeProvenance,
    ) -> Result<ClockOwnershipId<'dae>, DaeConstructionError> {
        check_provenance(self.source_map, provenance)?;
        self.storage
            .clocks
            .get(clock.index() as usize)
            .ok_or_else(|| unknown("clock", clock.index(), provenance))?;
        let expected_role = match kind {
            ClockedVariableKind::DiscreteReal => VariableRole::DiscreteReal,
            ClockedVariableKind::DiscreteValue => VariableRole::DiscreteValue,
        };
        let variable_entry = self.storage.variable(variable, provenance)?;
        if variable_entry.role != expected_role {
            return Err(DaeConstructionError::InvalidVariableRole {
                name: variable_entry.name.clone(),
                span: provenance.span(),
            });
        }
        if let Some(&index) = self.storage.clock_ownership_by_variable.get(&variable) {
            let entry = self
                .storage
                .clock_ownerships
                .get(index as usize)
                .expect("clock ownership index points into its dense arena");
            if entry.clock == clock.index() && entry.sampled == sampled {
                return Ok(ClockOwnershipId::from_raw(index));
            }
            return Err(DaeConstructionError::ConflictingClockOwnership {
                variable,
                established_clock: entry.clock,
                attempted_clock: clock.index(),
                established: entry.provenance,
                attempted: provenance,
            });
        }
        let raw = checked_u32(
            self.storage.clock_ownerships.len(),
            "clock ownership arena",
            provenance,
        )?;
        self.storage.clock_ownerships.push(ClockOwnershipEntry {
            variable,
            kind,
            clock: clock.index(),
            sampled,
            provenance,
        });
        let prior = self
            .storage
            .clock_ownership_by_variable
            .insert(variable, raw);
        debug_assert!(prior.is_none());
        Ok(ClockOwnershipId::from_raw(raw))
    }
}

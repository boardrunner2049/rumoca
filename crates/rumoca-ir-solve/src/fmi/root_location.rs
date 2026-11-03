//! How every FMI executor locates state events (SPEC_0044 ME-EVENT-004).
//!
//! The plan is constructed with the component, so the linked runtime, the
//! generated C, and every other executor read one set of location rules
//! instead of each deriving its own scan cadence, tolerance, refinement
//! budget, or tie-break.

/// Which crossing an accepted interval applies when several indicators change
/// domain in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum RootTieBreak {
    /// The crossing located at the least application coordinate; crossings
    /// located at the same coordinate apply together.
    LeastApplicationCoordinate,
}

/// The root-location rules of one FMI component.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct RootLocationPlan {
    /// Adjacent indicator samples are at most this fraction of the experiment
    /// width apart, independent of the output cadence (SPEC_0044 §6).
    scan_fraction: f64,
    /// The scan resolution of an experiment without a finite positive width.
    fallback_scan_resolution: f64,
    /// Bisection steps a bracket may take before location fails typed.
    refinement_iteration_cap: usize,
    /// Machine epsilons, scaled by the interval's time magnitude, that make
    /// up the roundoff of one accepted interval.
    roundoff_epsilons: f64,
    tie_break: RootTieBreak,
}

impl RootLocationPlan {
    /// The plan every component is constructed with.
    pub const STANDARD: Self = Self {
        scan_fraction: 1.0 / 8.0,
        fallback_scan_resolution: 1.0e-3,
        refinement_iteration_cap: 128,
        roundoff_epsilons: 100.0,
        tie_break: RootTieBreak::LeastApplicationCoordinate,
    };

    /// The adjacent-sample bound for an experiment of `experiment_width`.
    #[must_use]
    pub fn scan_resolution(&self, experiment_width: f64) -> f64 {
        let width = experiment_width.abs();
        if width.is_finite() && width > 0.0 {
            (width * self.scan_fraction).max(f64::MIN_POSITIVE)
        } else {
            self.fallback_scan_resolution
        }
    }

    /// The roundoff of an accepted interval starting at `current_time` and
    /// lasting `duration`.
    #[must_use]
    pub fn interval_roundoff(&self, current_time: f64, duration: f64) -> f64 {
        (self.roundoff_epsilons * f64::EPSILON * (current_time.abs() + duration.abs()))
            .max(f64::MIN_POSITIVE)
    }

    /// The location tolerance: the host's roundoff at the start of a scan
    /// interval, never wider than the scan resolution itself.
    #[must_use]
    pub fn location_tolerance(&self, roundoff: f64, scan_resolution: f64) -> f64 {
        roundoff.min(scan_resolution)
    }

    /// Whether a root located at `root_time`, in the accepted interval that
    /// starts at `interval_start` and lasts `duration`, coincides with the
    /// scheduled time event at `event_time`.
    ///
    /// A coincident root is not applied on its own: the time event's iteration
    /// handles it, so its relations see post-time-event values and the two
    /// changes form one event instant rather than two separated by roundoff.
    #[must_use]
    pub fn coincides_with_time_event(
        &self,
        root_time: f64,
        event_time: f64,
        interval_start: f64,
        duration: f64,
    ) -> bool {
        (event_time - root_time).abs() <= self.interval_roundoff(interval_start, duration)
    }

    #[must_use]
    pub const fn refinement_iteration_cap(&self) -> usize {
        self.refinement_iteration_cap
    }

    #[must_use]
    pub const fn tie_break(&self) -> RootTieBreak {
        self.tie_break
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scan follows the experiment width, not the output cadence, and an
    /// experiment without a finite width takes the fallback resolution.
    #[test]
    fn the_standard_plan_scales_with_the_experiment() {
        let plan = RootLocationPlan::STANDARD;
        assert_eq!(plan.scan_resolution(8.0), 1.0);
        assert_eq!(plan.scan_resolution(-8.0), 1.0);
        assert_eq!(plan.scan_resolution(0.0), 1.0e-3);
        assert_eq!(plan.scan_resolution(f64::INFINITY), 1.0e-3);
        assert_eq!(plan.location_tolerance(1.0e-12, 0.5), 1.0e-12);
        assert_eq!(plan.location_tolerance(1.0, 0.5), 0.5);
        assert_eq!(plan.refinement_iteration_cap(), 128);
        assert_eq!(plan.tie_break(), RootTieBreak::LeastApplicationCoordinate);
    }

    /// A root within the interval's roundoff of a scheduled time event is the
    /// same instant; one clearly before it is its own event.
    #[test]
    fn a_root_within_roundoff_of_a_time_event_coincides_with_it() {
        let plan = RootLocationPlan::STANDARD;
        assert!(plan.coincides_with_time_event(1.0 - 3.0e-15, 1.0, 0.99, 0.01));
        assert!(plan.coincides_with_time_event(1.0, 1.0, 0.99, 0.01));
        assert!(!plan.coincides_with_time_event(1.0 - 1.0e-9, 1.0, 0.99, 0.01));
    }
}

//! Defaults a direct `rumoca sim` or `rumoca sim bench` run takes from the model.

/// The `(t_start, t_end)` window of a direct run. It starts at the model's
/// `experiment(StartTime)` when finite, else 0. It ends at `--t-end` if given,
/// else at the model's `experiment(StopTime)` when finite and after the start,
/// else one time unit after the start. `experiment(Tolerance)` (see
/// [`direct_sim_rtol`]) and `experiment(Solver)` are honored the same way.
pub(crate) fn direct_sim_window(
    t_end: Option<f64>,
    start_time: Option<f64>,
    stop_time: Option<f64>,
) -> (f64, f64) {
    let t_start = start_time.filter(|start| start.is_finite()).unwrap_or(0.0);
    let t_end = t_end
        .or(stop_time.filter(|stop| stop.is_finite() && *stop > t_start))
        .unwrap_or(t_start + 1.0);
    (t_start, t_end)
}

/// The relative tolerance of a direct run: `--rtol` if given, else the
/// model's `experiment(Tolerance)` when it is a finite positive number, else
/// `None` (the backend default).
///
/// MLS 3.6 section 18.4 defines `Tolerance` as the default relative
/// integration tolerance, so it sets `rtol` only; the absolute tolerance stays
/// the backend default unless `--atol` names one.
pub(crate) fn direct_sim_rtol(rtol: Option<f64>, tolerance: Option<f64>) -> Option<f64> {
    rtol.or(tolerance.filter(|value| value.is_finite() && *value > 0.0))
}

/// The window of a direct `rumoca sim` run of `result`.
pub(super) fn sim_window(
    args: &super::SimCommandArgs,
    result: &crate::DaeCompilationResult,
) -> (f64, f64) {
    direct_sim_window(
        args.t_end,
        result.experiment_start_time,
        result.experiment_stop_time,
    )
}

#[cfg(test)]
mod tests {
    use super::{direct_sim_rtol, direct_sim_window};

    #[test]
    fn the_experiment_tolerance_is_the_relative_tolerance_unless_overridden() {
        assert_eq!(direct_sim_rtol(None, None), None);
        assert_eq!(direct_sim_rtol(None, Some(1e-10)), Some(1e-10));
        assert_eq!(direct_sim_rtol(Some(1e-4), Some(1e-10)), Some(1e-4));
        // A tolerance that is no tolerance leaves the backend default.
        assert_eq!(direct_sim_rtol(None, Some(0.0)), None);
        assert_eq!(direct_sim_rtol(None, Some(-1e-6)), None);
        assert_eq!(direct_sim_rtol(None, Some(f64::NAN)), None);
    }

    #[test]
    fn the_window_follows_the_experiment_unless_overridden() {
        assert_eq!(direct_sim_window(None, None, None), (0.0, 1.0));
        assert_eq!(direct_sim_window(None, None, Some(2.5)), (0.0, 2.5));
        assert_eq!(direct_sim_window(None, Some(1.0), Some(3.0)), (1.0, 3.0));
        assert_eq!(
            direct_sim_window(Some(4.0), Some(1.0), Some(3.0)),
            (1.0, 4.0)
        );
        // A stop at or before the start is not a window; one unit follows it.
        assert_eq!(direct_sim_window(None, Some(2.0), Some(2.0)), (2.0, 3.0));
        assert_eq!(
            direct_sim_window(None, Some(f64::NAN), Some(0.0)),
            (0.0, 1.0)
        );
    }
}

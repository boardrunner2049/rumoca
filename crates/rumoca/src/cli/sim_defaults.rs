//! Defaults a direct `rumoca sim` or `rumoca sim bench` run takes from the model.

/// The `(t_start, t_end)` window of a direct run. It starts at the model's
/// `experiment(StartTime)` when finite, else 0. It ends at `--t-end` if given,
/// else at the model's `experiment(StopTime)` when finite and after the start,
/// else one time unit after the start. `experiment(Tolerance)` (see
/// [`direct_sim_tolerances`]) and `experiment(Solver)` are honored the same way.
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

/// The `(atol, rtol)` of a direct run. Each is the flag if given, else the
/// model's `experiment(Tolerance)` when it is a finite positive number, else
/// `None` (the backend default).
///
/// MLS 3.6 section 18.4 names `Tolerance` the integration tolerance. Like the
/// worker, the WASM simulation API and the MSL profiling tools, a direct run
/// applies it to both the relative and the absolute tolerance, so every
/// in-tree runner integrates a model with the same tolerances.
pub(crate) fn direct_sim_tolerances(
    atol: Option<f64>,
    rtol: Option<f64>,
    tolerance: Option<f64>,
) -> (Option<f64>, Option<f64>) {
    let tolerance = tolerance.filter(|value| value.is_finite() && *value > 0.0);
    (atol.or(tolerance), rtol.or(tolerance))
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
    use super::{direct_sim_tolerances, direct_sim_window};

    #[test]
    fn the_experiment_tolerance_is_both_tolerances_unless_overridden() {
        let tol = direct_sim_tolerances;
        assert_eq!(tol(None, None, None), (None, None));
        assert_eq!(tol(None, None, Some(1e-10)), (Some(1e-10), Some(1e-10)));
        assert_eq!(
            tol(None, Some(1e-4), Some(1e-10)),
            (Some(1e-10), Some(1e-4))
        );
        assert_eq!(
            tol(Some(1e-3), None, Some(1e-10)),
            (Some(1e-3), Some(1e-10))
        );
        assert_eq!(tol(Some(1e-3), Some(1e-4), None), (Some(1e-3), Some(1e-4)));
        // A tolerance that is no tolerance leaves the backend default.
        assert_eq!(tol(None, None, Some(0.0)), (None, None));
        assert_eq!(tol(None, None, Some(-1e-6)), (None, None));
        assert_eq!(tol(None, None, Some(f64::NAN)), (None, None));
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

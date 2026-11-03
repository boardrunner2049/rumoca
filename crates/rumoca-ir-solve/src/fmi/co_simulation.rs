//! How a generated Co-Simulation component advances one communication step
//! (SPEC_0044 ME-LSW-001).
//!
//! The plan is constructed with the component, so every generated
//! Co-Simulation executor (the FMI 2 and 3 C profiles and fmi-ls-wasm, which
//! compiles the same C) integrates by the same rule instead of each template
//! choosing its own method, substep, and error control.
//!
//! # References
//!
//! The method is the embedded pair of J. R. Dormand and P. J. Prince, "A
//! family of embedded Runge-Kutta formulae", Journal of Computational and
//! Applied Mathematics 6(1):19-26, 1980, the tableau `rumoca-solver-rk45`
//! integrates Model Exchange with. The step-size controller is the elementary
//! one of E. Hairer, S. P. Norsett and G. Wanner, "Solving Ordinary
//! Differential Equations I: Nonstiff Problems", 2nd rev. ed., Springer 1993,
//! section II.4 (equations II.4.12 and II.4.13), with the same constants.
//!
//! An explicit method on a stiff mode is held at its stability boundary by
//! the controller, and the stiff component then carries a local error of the
//! order of the tolerance that never decays; an algebraic variable with a
//! large gain on that component (a node held by a small leakage conductance)
//! amplifies it. The plan therefore detects stiffness with the test of
//! Hairer, Norsett and Wanner, "Solving Ordinary Differential Equations II:
//! Stiff and Differential-Algebraic Problems", 2nd rev. ed., Springer 1996,
//! section IV.2 (the `h * |lambda|` estimate of their DOPRI5 code), and
//! substeps a stiff mode with the L-stable Rosenbrock 2(3) pair of
//! L. F. Shampine and M. W. Reichelt, "The MATLAB ODE Suite", SIAM Journal on
//! Scientific Computing 18(1):1-22, 1997, section 3 (ode23s).

use serde::Serialize;

/// The explicit integration method of a Co-Simulation substep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CoSimulationMethod {
    /// Dormand-Prince 5(4): the fifth-order solution advances, the embedded
    /// fourth-order solution estimates the local error, and the last stage is
    /// the next substep's first (first same as last).
    DormandPrince54,
}

/// How a communication step is divided into substeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CoSimulationSubstep {
    /// Adaptive substeps whose local error estimate stays within the
    /// component tolerance, the last one ending exactly at the communication
    /// point; a step the controller cannot complete within its budget is
    /// discarded and rolled back.
    ErrorControlled,
}

/// The linearly implicit method a stiff mode is substepped with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CoSimulationStiffMethod {
    /// Shampine-Reichelt Rosenbrock 2(3): L-stable, second order, with a
    /// third-order error estimate and the last stage the next substep's first.
    /// Its Jacobian is a forward difference of the state derivatives
    /// (including the time derivative), formed once per accepted point.
    ShampineReichelt23,
}

/// When an error-controlled substep rule leaves and re-enters its explicit
/// method.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CoSimulationStiffness {
    method: CoSimulationStiffMethod,
    /// `|h * lambda|` bound of the explicit method's stability region on the
    /// negative real axis: an accepted explicit substep whose estimate
    /// `h * |f(y1) - f(y_s)| / |y1 - y_s|` (the last two stages) exceeds it is
    /// stiff, and a stiff substep whose next size times the Jacobian's
    /// infinity norm (a bound on its spectral radius) stays within it may
    /// return to the explicit method.
    stability_bound: f64,
    /// Stiff explicit substeps, counted until `clear_after` non-stiff ones in
    /// a row reset the count, that switch the component to the stiff method.
    switch_after: u32,
    clear_after: u32,
    /// `-1 / (p + 1)` of the second-order stiff pair in the controller's
    /// `factor = safety * err^exponent`.
    exponent: f64,
}

/// The step-size controller of an error-controlled substep rule.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CoSimulationController {
    /// Relative and absolute tolerance of the local error when the importer
    /// defines none (the FMI setup tolerance replaces it otherwise); equal to
    /// the native simulation's default tolerance.
    default_tolerance: f64,
    /// `safety` of `factor = safety * err^exponent`.
    safety: f64,
    /// `-1 / (p - 1)` for the fifth-order pair.
    exponent: f64,
    /// Smallest and largest factor one substep may change the step size by.
    min_factor: f64,
    max_factor: f64,
    /// Attempted substeps one communication step may take before it is
    /// discarded.
    max_substeps: u64,
}

/// The Co-Simulation step rule of one FMI component.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CoSimulationStepPlan {
    method: CoSimulationMethod,
    substep: CoSimulationSubstep,
    controller: CoSimulationController,
    stiffness: CoSimulationStiffness,
}

impl CoSimulationStepPlan {
    /// The plan every component is constructed with. The accuracy of a
    /// communication step follows the component tolerance, not the importer's
    /// communication step size, so a stiff block the importer steps coarsely
    /// takes as many substeps as its fastest mode needs, and a mode the
    /// explicit method finds stiff is substepped by the L-stable stiff method
    /// until its step size no longer exceeds the explicit stability bound.
    pub const STANDARD: Self = Self {
        method: CoSimulationMethod::DormandPrince54,
        substep: CoSimulationSubstep::ErrorControlled,
        controller: CoSimulationController {
            default_tolerance: 1.0e-6,
            safety: 0.9,
            exponent: -0.2,
            min_factor: 0.2,
            max_factor: 5.0,
            max_substeps: 100_000,
        },
        stiffness: CoSimulationStiffness {
            method: CoSimulationStiffMethod::ShampineReichelt23,
            stability_bound: 3.25,
            switch_after: 15,
            clear_after: 6,
            exponent: -1.0 / 3.0,
        },
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Templates select their substep integrator and read its controller by
    /// these names, so the serialized plan is part of the rendering contract.
    #[test]
    fn the_standard_plan_serializes_the_names_templates_read() {
        let plan = serde_json::to_value(CoSimulationStepPlan::STANDARD).expect("serializes");
        assert_eq!(
            plan,
            serde_json::json!({
                "method": "DormandPrince54",
                "substep": "ErrorControlled",
                "controller": {
                    "default_tolerance": 1.0e-6,
                    "safety": 0.9,
                    "exponent": -0.2,
                    "min_factor": 0.2,
                    "max_factor": 5.0,
                    "max_substeps": 100_000,
                },
                "stiffness": {
                    "method": "ShampineReichelt23",
                    "stability_bound": 3.25,
                    "switch_after": 15,
                    "clear_after": 6,
                    "exponent": -1.0 / 3.0,
                },
            })
        );
    }
}

//! Parameter binding ownership shared by FMI metadata and C admission.

use super::{FmiCausality, FmiVariable};
use crate::{ScalarSlot, SolveProblem, SolvePureCallTable, SolveVariableStorageRole};

pub(super) fn classify_metadata(problem: &SolveProblem, variables: &mut [FmiVariable]) {
    let owned: std::collections::BTreeSet<_> = problem
        .initialization
        .update_targets()
        .iter()
        .chain(problem.initialization.projection_unknowns())
        .filter_map(|slot| match slot {
            ScalarSlot::P { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    classify_initialized_coordinates(problem, variables);
    for variable in variables {
        if variable.role() != Some(SolveVariableStorageRole::Parameter) {
            continue;
        }
        let Some(storage) = variable.backing.storage() else {
            continue;
        };
        if (storage.base()..storage.base() + storage.scalar_count()).any(|i| owned.contains(&i)) {
            variable.causality = FmiCausality::CalculatedParameter;
            let binding_owned = problem.initialization.update_targets().iter().any(|slot| {
                matches!(slot, ScalarSlot::P { index, .. }
                    if *index >= storage.base() && *index - storage.base() < storage.scalar_count())
            });
            if binding_owned {
                variable.initial = Some(super::FmiInitial::Calculated);
                variable.start = None;
            } else {
                variable.initial = Some(super::FmiInitial::Approx);
            }
        } else if variable.causality == FmiCausality::Parameter && variable.evaluable {
            // MLS §4.5, §18.3: a final or `Evaluate = true` parameter keeps
            // parameter variability, but its value is fixed at translation, so
            // the environment may not set it: FMI's non-settable parameter
            // form (FMI 3.0.2 §2.4.7.4).
            variable.causality = FmiCausality::CalculatedParameter;
            variable.initial = Some(super::FmiInitial::Calculated);
            variable.start = None;
        }
    }
}

/// FMI 2.0.5 §2.2.7 and FMI 3.0.2 §2.4.7: a non-parameter coordinate whose
/// start value the initialization may change is `approx` (a state or other
/// coordinate the initialization projection solves for keeps its start as the
/// guess), and one a discrete equation defines or an initialization update
/// assigns from a settable parameter is `calculated` (its start is not its
/// value).
fn classify_initialized_coordinates(problem: &SolveProblem, variables: &mut [FmiVariable]) {
    let projected: std::collections::BTreeSet<_> = problem
        .initialization
        .projection_unknowns()
        .iter()
        .filter_map(|slot| match slot {
            ScalarSlot::Y { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    let discrete = &problem.discrete;
    let defined: std::collections::BTreeSet<_> = discrete
        .update_targets
        .iter()
        .zip(&discrete.row_roles)
        .filter(|(_, role)| **role == crate::DiscreteRowRole::Equation)
        .filter_map(|(slot, _)| match slot {
            ScalarSlot::P { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    // A state whose start reads a settable parameter is assigned from it.
    let assigned: std::collections::BTreeSet<_> = problem
        .initialization
        .update_targets()
        .iter()
        .filter_map(|slot| match slot {
            ScalarSlot::Y { index, .. } => Some(*index),
            _ => None,
        })
        .collect();
    for variable in variables.iter_mut() {
        let Some(storage) = variable.backing.storage() else {
            continue;
        };
        if matches!(
            variable.role(),
            Some(SolveVariableStorageRole::Parameter | SolveVariableStorageRole::Constant) | None
        ) || variable.causality == FmiCausality::Input
        {
            continue;
        }
        let range = storage.base()..storage.base() + storage.scalar_count();
        let column = storage.column();
        let calculated = match column {
            super::FmiStorageColumn::P => range.clone().any(|i| defined.contains(&i)),
            super::FmiStorageColumn::Y => range.clone().any(|i| assigned.contains(&i)),
        };
        if calculated {
            variable.initial = Some(super::FmiInitial::Calculated);
            variable.start = None;
        } else if column == super::FmiStorageColumn::Y
            && variable.initial == Some(super::FmiInitial::Exact)
            && range.clone().any(|i| projected.contains(&i))
        {
            variable.initial = Some(super::FmiInitial::Approx);
        }
    }
}

/// The admitted initialization updates: the order the generated component
/// evaluates their programs in, the number of dependency levels of the
/// parameter bindings, and the programs that run while initializing only.
pub(super) struct UpdateOrder {
    pub(super) order: Vec<usize>,
    pub(super) levels: usize,
    pub(super) initial_only: Vec<usize>,
}

/// What one initialization update target is.
#[derive(Clone, Copy, PartialEq)]
enum UpdateTarget {
    /// A parameter binding's parameter.
    Parameter(usize),
    /// A state start assigned from settled parameters, which no binding reads.
    StateStart,
    /// A discrete coordinate or its `pre` slot an MLS §8.6 initial definition
    /// (`pre(m) = e`) assigns.
    Discrete(usize),
}

/// Admit the initialization updates and return the order the generated
/// component evaluates their programs in, with the number of dependency
/// levels of the parameter bindings (the repeated sweeps the runtime needs to
/// settle them).
///
/// The runtime applies the bindings as a Jacobi sweep repeated until no value
/// changes (`eval_and_apply_update_rows`), whatever their issued order. Each
/// program is a function of settled parameters and constants alone, so the
/// fixed point assigns every target the value its program computes from the
/// final values of its reads; evaluating the programs once in dependency
/// order computes exactly those values, bit for bit. A binding that reads
/// time, a continuous coordinate, an input, or anything in a cycle is refused.
///
/// An MLS §8.6 discrete initial definition follows the bindings and runs while
/// initializing only: it determines the value the coordinate and its `pre`
/// slot enter the initial event with, and the equation-section owner keeps
/// every later instant. The DAE proved it reads no discrete or `pre` value, so
/// the definitions are independent of one another; here each must read only
/// settled parameters, inputs, and states (an algebraic or `time` read is
/// refused, since the C initialization does not order it).
pub(super) fn validate(
    problem: &SolveProblem,
    calls: &SolvePureCallTable,
) -> Result<UpdateOrder, &'static str> {
    let init = &problem.initialization;
    validate_projection(problem)?;
    let targets = init
        .update_targets()
        .iter()
        .map(|slot| update_target(problem, slot))
        .collect::<Result<Vec<_>, _>>()?;
    let mut p = stable_parameters(problem);
    for target in &targets {
        if let UpdateTarget::Parameter(index) = target {
            p[*index] = false;
        }
    }
    let programs = init.update_rhs().programs();
    let mut outputs = Vec::with_capacity(programs.len());
    let mut discrete = Vec::new();
    let mut cursor = 0;
    for (program_index, program) in programs.iter().enumerate() {
        let count = crate::ScalarProgramBlock::program_output_count(program);
        let written = init
            .update_rhs()
            .output_indices()
            .get(cursor..cursor + count)
            .ok_or("missing parameter binding output index")?
            .iter()
            .map(|output| targets.get(*output).copied())
            .collect::<Option<Vec<_>>>()
            .ok_or("missing parameter update target")?;
        let assigns_discrete = written
            .iter()
            .any(|target| matches!(target, UpdateTarget::Discrete(_)));
        if assigns_discrete
            && !written
                .iter()
                .all(|target| matches!(target, UpdateTarget::Discrete(_)))
        {
            return Err("an initialization update mixes discrete and parameter targets");
        }
        if assigns_discrete {
            discrete.push(program_index);
        }
        outputs.push(
            written
                .into_iter()
                .filter_map(|target| match target {
                    UpdateTarget::Parameter(index) | UpdateTarget::Discrete(index) => Some(index),
                    UpdateTarget::StateStart => None,
                })
                .collect(),
        );
        cursor += count;
    }
    if cursor != targets.len() {
        return Err("missing parameter binding output");
    }
    let bindings = (0..programs.len())
        .filter(|index| !discrete.contains(index))
        .collect::<Vec<_>>();
    use super::static_assertions::scalar_dependencies::{OrderError, dependency_order};
    let refused = |error| match error {
        OrderError::Unsupported => "unsupported parameter binding dependency operation",
        OrderError::Unsettled => "parameter binding depends on unsettled or non-parameter values",
    };
    let y = vec![false; problem.layout.y_scalars()];
    let (mut order, levels) =
        dependency_order(calls, programs, &bindings, &outputs, &y, &mut p).map_err(refused)?;
    if !discrete.is_empty() {
        let (y, mut p) = initial_definition_reads(problem, p);
        let (definitions, _) = dependency_order(calls, programs, &discrete, &outputs, &y, &mut p)
            .map_err(|_| {
            "a discrete initial definition reads a value the C initialization does not \
                 settle before it (time or an algebraic)"
        })?;
        order.extend(definitions);
    }
    Ok(UpdateOrder {
        order,
        levels,
        initial_only: discrete,
    })
}

/// The kind of one initialization update target.
fn update_target(problem: &SolveProblem, slot: &ScalarSlot) -> Result<UpdateTarget, &'static str> {
    use SolveVariableStorageRole as Role;
    match *slot {
        ScalarSlot::P { index, .. } if owns(problem, Role::Parameter, slot) => {
            Ok(UpdateTarget::Parameter(index))
        }
        ScalarSlot::Y { .. } if owns(problem, Role::State, slot) => Ok(UpdateTarget::StateStart),
        ScalarSlot::P { index, .. }
            if owns(problem, Role::DiscreteReal, slot)
                || owns(problem, Role::DiscreteValue, slot)
                || problem
                    .solve_layout
                    .pre_param_bindings
                    .iter()
                    .any(|binding| binding.dest_p_index == index) =>
        {
            Ok(UpdateTarget::Discrete(index))
        }
        _ => Err("C initialization can only assign parameters, state starts, and discrete values"),
    }
}

/// The values settled when the discrete initial definitions run: every
/// parameter (bindings included, from `p`), and the inputs and states.
fn initial_definition_reads(problem: &SolveProblem, mut p: Vec<bool>) -> (Vec<bool>, Vec<bool>) {
    let mut y = vec![false; problem.layout.y_scalars()];
    for run in &problem.solve_layout.variable_storage_runs {
        let settled = match run.base {
            ScalarSlot::Y { index, .. } => matches!(
                run.role,
                SolveVariableStorageRole::State | SolveVariableStorageRole::ExternalInput
            )
            .then_some((&mut y, index)),
            ScalarSlot::P { index, .. } => {
                (run.role == SolveVariableStorageRole::ExternalInput).then_some((&mut p, index))
            }
            _ => None,
        };
        if let Some((values, base)) = settled
            && let Some(range) = values.get_mut(base..base + run.scalar_count)
        {
            range.fill(true);
        }
    }
    (y, p)
}

/// Whether `slot` lies in a storage run of `role`.
fn owns(problem: &SolveProblem, role: SolveVariableStorageRole, slot: &ScalarSlot) -> bool {
    problem
        .solve_layout
        .variable_storage_runs
        .iter()
        .any(|run| {
            run.role == role
                && match (run.base, slot) {
                    (ScalarSlot::P { index: base, .. }, ScalarSlot::P { index, .. })
                    | (ScalarSlot::Y { index: base, .. }, ScalarSlot::Y { index, .. }) => {
                        *index >= base && *index - base < run.scalar_count
                    }
                    _ => false,
                }
        })
}

pub(super) fn stable_parameters(problem: &SolveProblem) -> Vec<bool> {
    let mut p = vec![false; problem.layout.p_scalars()];
    for run in &problem.solve_layout.variable_storage_runs {
        if !matches!(
            run.role,
            SolveVariableStorageRole::Parameter | SolveVariableStorageRole::Constant
        ) {
            continue;
        }
        if let ScalarSlot::P { index, .. } = run.base {
            p[index..index + run.scalar_count].fill(true);
        }
    }
    p
}

/// The initialization projection the C kernel runs: the runtime's
/// `settle_initialization_system` without a homotopy sweep or delay history,
/// for residual rows evaluated on the settled
/// coordinates (the bindings and the algebraic refresh applied first, as the
/// runtime does whenever a row reads a reconstructed algebraic). Rows over
/// declaration seeds need single-row isolation and the forward-mode initial
/// Jacobian, which the C kernel does not emit.
fn validate_projection(problem: &SolveProblem) -> Result<(), &'static str> {
    use crate::{InitializationCoordinateKind, InitializationRowRole};
    let init = &problem.initialization;
    let rows = init
        .residual()
        .len()
        .map_err(|_| "the initialization residual has no checked row count")?;
    if rows == 0 {
        return if init.projection_plan().is_empty() {
            Ok(())
        } else {
            Err("C initialization projects blocks without residual rows")
        };
    }
    if problem
        .solve_layout
        .initial_homotopy_parameter_index
        .is_some()
    {
        return Err("C initialization cannot run the homotopy continuation");
    }
    let delays = &problem.events.delays;
    if !delays.source_rhs.is_empty() || !delays.value_parameter_indices.is_empty() {
        return Err("C initialization cannot synchronize delay histories");
    }
    let settled = init.row_roles().iter().any(|role| {
        matches!(
            role,
            InitializationRowRole::UnownedCoordinate(InitializationCoordinateKind::Algebraic)
                | InitializationRowRole::SolvedThroughAlgebraicRefresh
                | InitializationRowRole::SurplusAlgebraicCheck
        )
    });
    // Retained state-manifold rows read states and parameters only (Solve
    // lowering keeps algebraics out of retained constraints), so the settled
    // view gives them the values and tangent the stored coordinates do. The
    // same holds for any row set that neither reads nor solves an
    // algebraic/output coordinate: the refresh writes only those, so the
    // settled residual and its tangent are the ones over the seeds.
    if !settled && rows != init.manifold_row_count() && reads_or_solves_algebraics(problem) {
        return Err(
            "C initialization evaluates residual rows only on the settled algebraic refresh; rows over declaration seeds need row isolation and a forward initial Jacobian",
        );
    }
    if problem.solve_layout.solver_scalar_count() != problem.layout.y_scalars() {
        return Err("C initialization needs the solver coordinates to fill the Y storage");
    }
    Ok(())
}

/// Whether an initialization residual row reads, or a projection block
/// solves, a solver coordinate past the states: one the algebraic refresh
/// writes. A residual the checker cannot read per operation counts as one.
fn reads_or_solves_algebraics(problem: &SolveProblem) -> bool {
    let states = problem.solve_layout.state_scalar_count();
    let algebraics: std::collections::BTreeSet<usize> =
        (states..problem.layout.y_scalars()).collect();
    let init = &problem.initialization;
    let solves = init
        .projection_unknowns()
        .iter()
        .any(|slot| matches!(slot, ScalarSlot::Y { index, .. } if *index >= states));
    solves
        || init.residual().nodes.iter().any(|node| match node {
            crate::ComputeNode::ScalarPrograms(block) => block
                .programs()
                .iter()
                .flatten()
                .any(|op| op.reads_y_index_in(&algebraics)),
            _ => true,
        })
}

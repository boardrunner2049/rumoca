//! Checked adapter between the public scalar execution profile and Solve.
use rumoca_core::Span;
use rumoca_ir_solve as s;
use s::execution as public;
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, String>;

fn scalar_rows(
    block: &s::ComputeBlock,
    targets: &[Option<s::ScalarSlot>],
) -> Result<Vec<public::Row>> {
    let mut rows = Vec::new();
    for node in &block.nodes {
        let s::ComputeNode::ScalarPrograms(b) = node else {
            return Err("execution v1 requires scalar ComputeBlock nodes".into());
        };
        for (ops, output) in b.programs().iter().zip(b.output_indices()) {
            if s::ScalarProgramBlock::program_output_count(ops) != 1 {
                return Err("execution v1 requires single-output scalar rows".into());
            }
            let target = match targets.get(*output).copied().flatten() {
                Some(s::ScalarSlot::Y { index, .. }) => Some(index),
                None => None,
                _ => return Err("execution v1 supports Y projection unknowns only".into()),
            };
            rows.push(public::Row {
                output: *output,
                target,
                instructions: ops
                    .iter()
                    .map(public::ScalarOp::from_solve)
                    .collect::<Result<_>>()?,
            });
        }
    }
    Ok(rows)
}

/// Export a bounded profile, rejecting every unsupported numerical owner.
/// Derive the numerical program from a Solve model.
///
/// `requested` is `(trace point id, variable id, name)`. v1 keyed observations
/// by variable; v2 keys them by the trace point that asked for them, so the
/// host resolves a `snapshot.value` through the single registration site (D2).
pub fn export(
    model: &s::SolveModel,
    requested: &[(u32, u32, String)],
) -> Result<public::NumericalProgram> {
    model.validate().map_err(|e| e.to_string())?;
    let p = &model.problem;
    check_profile(model)?;
    let storage = export_storage(model)?;
    let mut observations = Vec::new();
    for (trace_point, variable_id, name) in requested {
        let instructions = observation(model, name)
            .map_err(|e| format!("trace point {trace_point} ({name}): {e}"))?;
        observations.push(public::Observation {
            trace_point: public::TracePointRef(*trace_point),
            variable_id: *variable_id,
            name: name.clone(),
            quantity: None,
            owner: None,
            member: None,
            instructions,
        });
    }
    let algebraic_blocks = p
        .continuous
        .algebraic_projection_plan
        .blocks
        .iter()
        .map(|b| {
            if b.tearing.is_some() {
                return Err("execution v1 does not support tearing".into());
            }
            Ok(public::Projection {
                rows: b.rows.clone(),
                unknowns: b.y_indices.clone(),
            })
        })
        .collect::<Result<_>>()?;
    if p.initialization
        .row_roles()
        .iter()
        .any(|r| *r != s::InitializationRowRole::Solved)
    {
        return Err("execution v1 requires explicitly solved initialization rows".into());
    }
    let initial_blocks = p
        .initialization
        .projection_plan()
        .blocks
        .iter()
        .map(|b| {
            let unknowns = b
                .unknowns
                .iter()
                .map(|slot| match slot {
                    s::ScalarSlot::Y { index, .. } => Ok(*index),
                    _ => Err("execution v1 requires Y initialization unknowns".into()),
                })
                .collect::<Result<_>>()?;
            Ok(public::Projection {
                rows: b.rows.clone(),
                unknowns,
            })
        })
        .collect::<Result<_>>()?;
    let public = public::NumericalProgram {
        source_name: "rbc:execution/solve-scalar-v1".into(),
        storage,
        residual: scalar_rows(
            &p.continuous.implicit_rhs,
            &p.continuous.implicit_row_targets,
        )?,
        derivatives: scalar_rows(&p.continuous.derivative_rhs, &[])?,
        initialization: scalar_rows(p.initialization.residual(), p.initialization.row_targets())?,
        algebraic_blocks,
        initial_blocks,
        observations,
    };
    reconstruct(&public)?;
    Ok(public)
}

fn observation(model: &s::SolveModel, name: &str) -> Result<Vec<public::ScalarOp>> {
    if let Some(index) = model.visible_names.iter().position(|n| n == name) {
        let row = model
            .visible_value_rows
            .program(index)
            .ok_or("missing observation row")?;
        return row.iter().map(public::ScalarOp::from_solve).collect();
    }
    // Consult the compiler-issued binding map, never an inferred ordinal.
    let slot = model
        .problem
        .layout
        .bindings()
        .iter()
        .find(|(n, _)| n.as_str() == name)
        .map(|(_, slot)| slot)
        .ok_or("no checked reconstruction mapping")?;
    let load = match slot {
        s::ScalarSlot::P { index, .. } => public::ScalarOp::LoadP {
            dst: 0,
            index: *index,
        },
        s::ScalarSlot::Y { index, .. } => public::ScalarOp::LoadY {
            dst: 0,
            index: *index,
        },
        s::ScalarSlot::Constant(value) => public::ScalarOp::Const {
            dst: 0,
            value: *value,
        },
        s::ScalarSlot::Time => public::ScalarOp::LoadTime { dst: 0 },
    };
    Ok(vec![load, public::ScalarOp::StoreOutput { src: 0 }])
}

fn block(rows: &[public::Row], y: usize, p: usize, span: Span) -> Result<s::ComputeBlock> {
    let programs = rows
        .iter()
        .map(|r| public::checked_scalar(&r.instructions, y, p))
        .collect::<Result<Vec<_>>>()?;
    let b = s::ScalarProgramBlock::with_output_indices(
        programs,
        vec![span; rows.len()],
        rows.iter().map(|r| r.output).collect(),
    )
    .map_err(|e| e.to_string())?;
    Ok(s::ComputeBlock::from_scalar_program_block(b))
}

fn targets(rows: &[public::Row], count: usize, y: usize) -> Result<Vec<Option<s::ScalarSlot>>> {
    let mut out = vec![None; count];
    for row in rows {
        if row.output >= count || row.target.is_some_and(|i| i >= y) {
            return Err("projection target/output outside storage".into());
        }
        out[row.output] = row.target.map(s::scalar_slot_y);
    }
    Ok(out)
}

fn check_projection(
    blocks: &[public::Projection],
    rows: &[public::Row],
    allowed: &BTreeSet<usize>,
) -> Result<()> {
    let mut seen_rows = BTreeSet::new();
    let mut seen_unknowns = BTreeSet::new();
    for b in blocks {
        if b.rows.len() != b.unknowns.len() || b.rows.is_empty() {
            return Err("projection blocks must be nonempty and square".into());
        }
        for row in &b.rows {
            if !rows.iter().any(|r| r.output == *row) || !seen_rows.insert(*row) {
                return Err("missing or duplicate projection row".into());
            }
        }
        for unknown in &b.unknowns {
            if !allowed.contains(unknown) || !seen_unknowns.insert(*unknown) {
                return Err("missing or duplicate projection unknown".into());
            }
        }
    }
    if seen_rows.len() != rows.len() || &seen_unknowns != allowed {
        return Err("projection does not cover all rows/unknowns".into());
    }
    Ok(())
}

/// Reconstruct through canonical scalar, refresh, AD and FMI constructors.
/// No equation IR is inspected and no DAE lowering occurs here.
pub fn reconstruct(public: &public::NumericalProgram) -> Result<s::fmi::FmiComponent> {
    if public.source_name.is_empty() {
        return Err("execution source owner is required".into());
    }
    let span = Span::from_offsets(
        rumoca_core::SourceId::from_source_name(&public.source_name),
        0,
        0,
    );
    let (mut model, inputs) = restore_storage(public, span)?;
    restore_programs(&mut model, public, span)?;
    let y = model.initial_y.len();
    let p = model.parameters.len();
    model.problem.continuous.refresh_owners =
        rumoca_eval_solve::refresh_plan::build_continuous_refresh_owners(&mut model.problem)
            .map_err(|e| e.to_string())?;
    model.artifacts = crate::lower_solve_artifacts(&model.problem).map_err(|e| e.to_string())?;
    // The host's visible inventory is complete storage; public observation
    // programs execute against these checked values, including signed aliases.
    model.visible_names = model.problem.solve_layout.solver_maps.names.clone();
    model.visible_value_rows = s::ScalarProgramBlock::with_program_spans(
        (0..y)
            .map(|i| {
                vec![
                    s::LinearOp::LoadY { dst: 0, index: i },
                    s::LinearOp::StoreOutput { src: 0 },
                ]
            })
            .collect(),
        vec![span; y],
    )
    .map_err(|e| e.to_string())?;
    let mut ids = BTreeSet::new();
    for observation in &public.observations {
        if !ids.insert(observation.variable_id) {
            return Err("duplicate semantic observation ID".into());
        }
        public::checked_scalar(&observation.instructions, y, p)?;
    }
    model.validate().map_err(|e| e.to_string())?;
    s::fmi::FmiComponent::construct(model, inputs).map_err(|e| e.to_string())
}

fn restore_storage(
    public: &public::NumericalProgram,
    span: Span,
) -> Result<(s::SolveModel, Vec<s::fmi::FmiVariableInput>)> {
    use s::fmi::FmiCausality;
    let y = public
        .storage
        .iter()
        .filter(|v| v.role != "parameter")
        .count();
    let p = public.storage.len() - y;
    let states = public.storage.iter().filter(|v| v.role == "state").count();
    let mut model = s::SolveModel {
        initial_y: vec![0.0; y],
        solver_nominals: vec![1.0; y],
        parameters: vec![0.0; p],
        ..Default::default()
    };
    let mut bindings = indexmap::IndexMap::new();
    let mut names = vec![String::new(); y];
    let mut seen = BTreeSet::new();
    let mut inputs = Vec::new();
    let layout = &mut model.problem.solve_layout;
    for v in &public.storage {
        if !v.start.is_finite()
            || !v.nominal.is_finite()
            || v.nominal <= 0.0
            || bindings.contains_key(&v.name)
        {
            return Err("invalid storage value/name/nominal".into());
        }
        let (role, slot, default_causality, variability) = restore_slot(
            v,
            &mut model.initial_y,
            &mut model.parameters,
            &mut model.solver_nominals,
            &mut names,
            states,
        )?;
        let causality = match v.causality.as_str() {
            "local" => FmiCausality::Local,
            "output" if v.role != "parameter" => FmiCausality::Output,
            "parameter" if v.role == "parameter" => default_causality,
            _ => return Err("unsupported execution storage causality".into()),
        };
        if !seen.insert((v.role == "parameter", v.index)) {
            return Err("overlapping storage".into());
        }
        bindings.insert(v.name.clone(), slot);
        layout
            .variable_storage_runs
            .push(s::SolveVariableStorageRun {
                base: slot,
                scalar_count: 1,
                role,
                value_kind: s::SolveVariableValueKind::Real,
            });
        layout
            .variable_declarations
            .push(s::SolveVariableDeclaration::new(
                role,
                s::SolveVariableValueKind::Real,
            ));
        inputs.push(fmi_input(v, role, causality, variability, span));
    }
    layout.state_scalar_count = states;
    layout.algebraic_scalar_count = y - states;
    layout.parameter_count = p;
    layout.compiled_parameter_len = p;
    layout.solver_maps.names = names.clone();
    for (i, name) in names.iter().enumerate() {
        layout.solver_maps.name_to_idx.insert(name.clone(), i);
        layout
            .solver_maps
            .base_to_indices
            .insert(name.clone(), vec![i]);
    }
    model.problem.layout = s::VarLayout::from_parts(bindings, y, p);
    Ok((model, inputs))
}

fn restore_programs(
    model: &mut s::SolveModel,
    public: &public::NumericalProgram,
    span: Span,
) -> Result<()> {
    let y = model.initial_y.len();
    let p = model.parameters.len();
    let states = model.problem.solve_layout.state_scalar_count;
    if public.derivatives.len() != states
        || public
            .derivatives
            .iter()
            .enumerate()
            .any(|(i, r)| r.output != i)
    {
        return Err("derivatives must cover every state in order".into());
    }
    check_projection(
        &public.algebraic_blocks,
        &public.residual,
        &(states..y).collect(),
    )?;
    let initial_unknowns: BTreeSet<_> = public
        .initialization
        .iter()
        .filter_map(|r| r.target)
        .collect();
    check_projection(
        &public.initial_blocks,
        &public.initialization,
        &initial_unknowns,
    )?;
    if public
        .residual
        .iter()
        .any(|r| r.output >= y || r.target.is_none())
        || public.initialization.iter().any(|r| r.target.is_none())
    {
        return Err("residual output/target outside the scalar equation profile".into());
    }
    let continuous = &mut model.problem.continuous;
    continuous.implicit_rhs = block(&public.residual, y, p, span)?;
    continuous.residual = continuous.implicit_rhs.clone();
    // Row ownership follows the checked block's output space, not Y storage.
    // A pure explicit ODE has states but zero implicit residual outputs.
    let count = continuous
        .implicit_rhs
        .output_count("execution.implicit_rhs")
        .map_err(|e| e.to_string())?;
    continuous.implicit_row_targets = targets(&public.residual, count, y)?;
    continuous.derivative_rhs = block(&public.derivatives, y, p, span)?;
    continuous.algebraic_projection_plan.blocks = public
        .algebraic_blocks
        .iter()
        .map(|b| s::AlgebraicProjectionBlock {
            rows: b.rows.clone(),
            y_indices: b.unknowns.clone(),
            tearing: None,
            alternate_charts: Vec::new(),
        })
        .collect();
    let row_targets = targets(&public.initialization, public.initialization.len(), y)?;
    let input = s::InitializationSystemInput {
        residual: block(&public.initialization, y, p, span)?,
        row_roles: vec![s::InitializationRowRole::Solved; public.initialization.len()],
        projection_plan: s::InitializationProjectionPlan {
            blocks: public
                .initial_blocks
                .iter()
                .map(|b| s::InitializationProjectionBlock {
                    rows: b.rows.clone(),
                    unknowns: b.unknowns.iter().copied().map(s::scalar_slot_y).collect(),
                    scales: vec![s::InitializationUnknownScale::Solver; b.unknowns.len()],
                })
                .collect(),
        },
        given_state_indices: (0..states)
            .filter(|index| !initial_unknowns.contains(index))
            .collect(),
        ..Default::default()
    };
    let initialization =
        s::InitializationSolveSystem::construct(input).map_err(|e| e.to_string())?;
    if initialization.row_targets() != row_targets {
        return Err("initialization row targets disagree with the projection owner".into());
    }
    model.problem.initialization = initialization;
    Ok(())
}

fn fmi_input(
    v: &public::Storage,
    role: s::SolveVariableStorageRole,
    causality: s::fmi::FmiCausality,
    variability: s::fmi::FmiVariability,
    span: Span,
) -> s::fmi::FmiVariableInput {
    s::fmi::FmiVariableInput {
        name: v.name.clone(),
        scalar_names: vec![v.name.clone()],
        role,
        value_kind: s::SolveVariableValueKind::Real,
        dimensions: vec![],
        start: vec![v.start],
        minimum: None,
        maximum: None,
        nominal: Some(vec![v.nominal]),
        unit: v.unit.clone(),
        description: None,
        causality,
        declared_causality: None,
        variability,
        tunable: false,
        evaluable: false,
        text_start: None,
        declaration: span,
    }
}

fn restore_slot(
    v: &public::Storage,
    initial_y: &mut [f64],
    parameters: &mut [f64],
    nominals: &mut [f64],
    names: &mut [String],
    states: usize,
) -> Result<(
    s::SolveVariableStorageRole,
    s::ScalarSlot,
    s::fmi::FmiCausality,
    s::fmi::FmiVariability,
)> {
    use s::fmi::{FmiCausality, FmiVariability};
    let y = initial_y.len();
    let p = parameters.len();
    let result = match v.role.as_str() {
        "parameter" if v.index < p => {
            parameters[v.index] = v.start;
            (
                s::SolveVariableStorageRole::Parameter,
                s::scalar_slot_p(v.index),
                FmiCausality::Parameter,
                FmiVariability::Fixed,
            )
        }
        "state" | "algebraic" if v.index < y => {
            if (v.role == "state") != (v.index < states) {
                return Err("states must own the initial Y slots".into());
            }
            names[v.index] = v.name.clone();
            initial_y[v.index] = v.start;
            nominals[v.index] = v.nominal;
            (
                if v.role == "state" {
                    s::SolveVariableStorageRole::State
                } else {
                    s::SolveVariableStorageRole::Algebraic
                },
                s::scalar_slot_y(v.index),
                FmiCausality::Local,
                FmiVariability::Continuous,
            )
        }
        _ => return Err("unsupported storage role or out-of-bounds index".into()),
    };
    Ok(result)
}

fn check_profile(model: &s::SolveModel) -> Result<()> {
    let p = &model.problem;
    let default = s::SolveProblem::default();
    for (name, actual, empty) in [
        (
            "discrete",
            serde_json::to_value(&p.discrete),
            serde_json::to_value(&default.discrete),
        ),
        (
            "events",
            serde_json::to_value(&p.events),
            serde_json::to_value(&default.events),
        ),
        (
            "clocks",
            serde_json::to_value(&p.clocks),
            serde_json::to_value(&default.clocks),
        ),
        (
            "pure calls",
            serde_json::to_value(&model.pure_calls),
            serde_json::to_value(s::SolvePureCallTable::default()),
        ),
    ] {
        if actual.map_err(|e| e.to_string())? != empty.map_err(|e| e.to_string())? {
            return Err(format!("execution v1 does not yet support {name}"));
        }
    }
    if !p.continuous.manifold_residual.nodes.is_empty()
        || !p.initialization.update_rhs().is_empty()
        || p.solve_layout.initial_homotopy_parameter_index.is_some()
    {
        return Err("execution v1 does not support manifold/homotopy/initial updates".into());
    }
    if !matches!(
        model.artifacts.continuous.mass_matrix,
        s::MassMatrix::Identity
    ) || serde_json::to_value(&p.continuous.residual).map_err(|e| e.to_string())?
        != serde_json::to_value(&p.continuous.implicit_rhs).map_err(|e| e.to_string())?
    {
        return Err("execution v1 requires identity mass and matching residual owners".into());
    }
    Ok(())
}

fn export_storage(model: &s::SolveModel) -> Result<Vec<public::Storage>> {
    let p = &model.problem;
    let mut storage = Vec::new();
    for run in &p.solve_layout.variable_storage_runs {
        if run.scalar_count != 1 || run.value_kind != s::SolveVariableValueKind::Real {
            return Err("execution v1 supports scalar real storage only".into());
        }
        let role = match run.role {
            s::SolveVariableStorageRole::Parameter => "parameter",
            s::SolveVariableStorageRole::State => "state",
            s::SolveVariableStorageRole::Algebraic => "algebraic",
            _ => {
                return Err(format!(
                    "execution v1 unsupported storage role {:?}",
                    run.role
                ));
            }
        };
        let name = p
            .layout
            .bindings()
            .iter()
            .find(|(_, slot)| **slot == run.base)
            .ok_or("storage run has no semantic binding")?
            .0
            .to_string();
        let (index, start, nominal) = match run.base {
            s::ScalarSlot::Y { index, .. } => {
                (index, model.initial_y[index], model.solver_nominals[index])
            }
            s::ScalarSlot::P { index, .. } => (index, model.parameters[index], 1.0),
            _ => return Err("unsupported storage coordinate".into()),
        };
        let unit = model
            .variable_meta
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| m.unit.clone());
        storage.push(public::Storage {
            variable_id: None,
            name,
            role: role.into(),
            causality: if role == "parameter" {
                "parameter"
            } else {
                "local"
            }
            .into(),
            index,
            start,
            nominal,
            unit,
        });
    }
    Ok(storage)
}

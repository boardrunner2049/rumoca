//! MLS §8.6 initial algorithms that determine `fixed = false` parameters from
//! coordinates the initialization system solves.
//!
//! `parameter Real p0(fixed = false); initial algorithm p0 := PMECH0;` (the
//! OpenIPSL turbine-governor idiom, `PMECH0` being a generator output) cannot
//! become a calculated-parameter binding: `PMECH0` has no value when the
//! parameter set runs. Its meaning is still exact and simple. An initial
//! algorithm made only of assignments to distinct targets, none of which reads
//! a target of the same section, assigns each target a closed expression of
//! values the section does not write, so it is the same initialization problem
//! as the initial equations `p0 = PMECH0` (MLS §8.6 solves both together with
//! the rest of the initialization system). Such a section is rewritten into
//! those initial equations before analysis, and the initialization projection
//! solves the deferred parameter as it does for a written initial equation.
//!
//! Only sections that need it are rewritten: at least one assignment must read
//! a coordinate that is neither a parameter nor a constant. Every other section
//! keeps the replayed calculated-parameter and discrete-value owners.

use super::*;
use std::borrow::Cow;

pub(super) fn normalize_initial_algorithms(flat: &flat::Model) -> Cow<'_, flat::Model> {
    let rewritten = flat
        .initial_algorithms
        .iter()
        .map(|algorithm| initial_equations_for(flat, algorithm))
        .collect::<Vec<_>>();
    if rewritten.iter().all(Option::is_none) {
        return Cow::Borrowed(flat);
    }
    let mut normalized = flat.clone();
    normalized.initial_algorithms.clear();
    for (algorithm, equations) in flat.initial_algorithms.iter().zip(rewritten) {
        match equations {
            Some(equations) => normalized.initial_equations.extend(equations),
            None => normalized.initial_algorithms.push(algorithm.clone()),
        }
    }
    Cow::Owned(normalized)
}

fn initial_equations_for(
    flat: &flat::Model,
    algorithm: &flat::Algorithm,
) -> Option<Vec<flat::Equation>> {
    let mut assignments = Vec::with_capacity(algorithm.statements.len());
    for statement in &algorithm.statements {
        let rumoca_core::Statement::Assignment { comp, value, span } = statement else {
            return None;
        };
        if comp.parts().is_empty()
            || !rumoca_core::component_ref_element_subscripts(comp).is_empty()
        {
            return None;
        }
        let target = rumoca_core::component_ref_to_base_reference(comp);
        let variable = flat.variables.get(target.var_name())?;
        if !matches!(variable.variability, Variability::Parameter(_))
            || variable.fixed != Some(false)
            || variable.binding.is_some()
            || !variable.dims.is_empty()
        {
            return None;
        }
        assignments.push((target, value, *span));
    }
    let targets = assignments
        .iter()
        .map(|(target, ..)| target.var_name().clone())
        .collect::<HashSet<_>>();
    if targets.len() != assignments.len() {
        return None;
    }
    let mut reads_unsettled = false;
    for (_, value, _) in &assignments {
        let mut references = Vec::new();
        value.collect_var_refs(&mut references);
        if references.iter().any(|name| targets.contains(name)) {
            return None;
        }
        reads_unsettled |= references.iter().any(|name| {
            flat.variables.get(name).is_some_and(|variable| {
                !matches!(
                    variable.variability,
                    Variability::Parameter(_) | Variability::Constant(_)
                )
            })
        });
    }
    if !reads_unsettled {
        return None;
    }
    let origin = flat::EquationOrigin::Algorithm {
        component: algorithm.origin.clone(),
    };
    Some(
        assignments
            .into_iter()
            .map(|(target, value, span)| {
                let lhs = Expression::VarRef {
                    name: target,
                    subscripts: Vec::new(),
                    span,
                };
                flat::Equation::new(
                    Expression::Binary {
                        op: OpBinary::Sub,
                        lhs: Box::new(lhs),
                        rhs: Box::new(value.clone()),
                        span,
                    },
                    span,
                    origin.clone(),
                )
            })
            .collect(),
    )
}

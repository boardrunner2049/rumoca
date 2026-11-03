//! Exact identity queries over the resolved class index: function
//! exposures, implementations, and package inheritance by `DefId`.

use super::*;

pub(super) fn exact_prefix_owner_def_id(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    prefix_def_id: rumoca_core::DefId,
) -> Option<rumoca_core::DefId> {
    if class_index.get(prefix_def_id).is_some() {
        return Some(prefix_def_id);
    }
    let declaration_owner = class_index.parent_def_id(prefix_def_id)?;
    let component = class_index
        .get(declaration_owner)?
        .components
        .values()
        .find(|component| component.def_id == Some(prefix_def_id))?;
    component.type_def_id.or(component.type_name.def_id)
}

pub(super) fn collect_function_exposures_for_implementation(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    owner: rumoca_core::DefId,
    implementation: rumoca_core::DefId,
    visited_owners: &mut FxHashSet<rumoca_core::DefId>,
    exposures: &mut FxHashSet<rumoca_core::DefId>,
) {
    if !visited_owners.insert(owner) {
        return;
    }
    let Some(class_def) = class_index.get(owner) else {
        return;
    };
    for nested in class_def
        .classes
        .values()
        .filter(|nested| nested.class_type == rumoca_core::ClassType::Function)
    {
        let Some(exposure) = nested.def_id else {
            continue;
        };
        let selected = if exposure == implementation {
            Some(exposure)
        } else {
            resolve_function_extends_target_def_id(class_index, exposure)
        };
        if selected == Some(implementation) {
            exposures.insert(exposure);
        }
    }
    for base in class_def
        .extends
        .iter()
        .filter_map(|extend| extend.base_def_id)
    {
        collect_function_exposures_for_implementation(
            class_index,
            base,
            implementation,
            visited_owners,
            exposures,
        );
    }
}

pub(super) fn resolve_function_extends_target_def_id(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    exposure: rumoca_core::DefId,
) -> Option<rumoca_core::DefId> {
    let mut visited = FxHashSet::default();
    if let FunctionBody::Unique(implementation) =
        unique_function_body(class_index, exposure, &mut visited)
        && implementation != exposure
    {
        return Some(implementation);
    }
    single_function_extends_chain_end(class_index, exposure)
}

/// The algorithm or external body a function denotes through its function
/// base classes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FunctionBody {
    /// Interface only: no base supplies a body.
    None,
    Unique(rumoca_core::DefId),
    Ambiguous,
}

/// Find the one body a function inherits (MLS §12.2: a function has at most
/// one algorithm section or external clause). A function such as MSL's
/// `Files.loadResource` extends both the partial interface and an
/// implementation that itself extends that interface; the interface-only base
/// contributes no body, so the implementation is the unique selection.
fn unique_function_body(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    current: rumoca_core::DefId,
    visited: &mut FxHashSet<rumoca_core::DefId>,
) -> FunctionBody {
    let Some(class_def) = class_index.get(current) else {
        return FunctionBody::Ambiguous;
    };
    if class_def.class_type != rumoca_core::ClassType::Function {
        return FunctionBody::Ambiguous;
    }
    if !class_def.algorithms.is_empty() || class_def.external.is_some() {
        return FunctionBody::Unique(current);
    }
    if !visited.insert(current) {
        return FunctionBody::Ambiguous;
    }
    // `visited` is the active extends path: a shared interface reached along
    // two branches (a diamond) is not a cycle.
    let mut body = FunctionBody::None;
    for base in class_def.extends.iter().filter_map(|ext| ext.base_def_id) {
        let is_function = class_index
            .get(base)
            .is_some_and(|base| base.class_type == rumoca_core::ClassType::Function);
        if !is_function {
            continue;
        }
        body = match (body, unique_function_body(class_index, base, visited)) {
            (FunctionBody::None, next) => next,
            (current, FunctionBody::None) => current,
            (FunctionBody::Unique(a), FunctionBody::Unique(b)) if a == b => body,
            _ => FunctionBody::Ambiguous,
        };
    }
    visited.remove(&current);
    body
}

/// The end of a single function-extends chain whose functions carry no body.
fn single_function_extends_chain_end(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    exposure: rumoca_core::DefId,
) -> Option<rumoca_core::DefId> {
    let mut current = exposure;
    let mut visited = FxHashSet::default();

    loop {
        if !visited.insert(current) {
            return None;
        }
        let class_def = class_index.get(current)?;
        if class_def.class_type != rumoca_core::ClassType::Function {
            return None;
        }
        if !class_def.algorithms.is_empty() || class_def.external.is_some() {
            return (current != exposure).then_some(current);
        }
        // A base that declares its own formals is the interface the alias
        // renames (`replaceable function position = positionBase`, where
        // `positionBase` only extends the empty `Modelica.Icons.Function`).
        if current != exposure && !class_def.components.is_empty() {
            return Some(current);
        }
        let mut candidates = class_def.extends.iter().filter_map(|ext| {
            let target = ext.base_def_id?;
            (class_index.get(target)?.class_type == rumoca_core::ClassType::Function)
                .then_some(target)
        });
        let Some(candidate) = candidates.next() else {
            // A partial function that declares its own inputs and outputs and
            // only inherits an icon or interface class is a definition, not
            // an alias: it has no implementation beyond itself, so unrelated
            // partial functions sharing a bodyless base do not collapse onto
            // that base.
            if current != exposure
                && class_index
                    .get(exposure)
                    .is_some_and(|declared| !declared.components.is_empty())
            {
                return Some(exposure);
            }
            return (current != exposure).then_some(current);
        };
        if candidates.any(|other| other != candidate) {
            return None;
        }
        current = candidate;
    }
}

pub(super) fn exact_package_chain_contains_def_id(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    package: rumoca_core::DefId,
    query: rumoca_core::DefId,
    visited: &mut FxHashSet<rumoca_core::DefId>,
) -> Result<bool, &'static str> {
    if package == query {
        return Ok(true);
    }
    if !visited.insert(package) {
        return Err("package inheritance contains a cycle");
    }
    let class_def = class_index
        .get(package)
        .ok_or("selected package DefId is absent from the resolved class index")?;
    for extend in &class_def.extends {
        let base = extend
            .base_def_id
            .or(extend.base_name.def_id)
            .ok_or("selected package base has no resolved DefId")?;
        if exact_package_chain_contains_def_id(class_index, base, query, visited)? {
            visited.remove(&package);
            return Ok(true);
        }
    }
    visited.remove(&package);
    Ok(false)
}

pub(super) fn exact_function_member_name(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    owner: rumoca_core::DefId,
    exposure: rumoca_core::DefId,
) -> Result<Option<String>, &'static str> {
    let class_def = class_index
        .get(owner)
        .ok_or("function exposure owner is absent from the resolved class index")?;
    let mut matches = class_def
        .classes
        .iter()
        .filter(|(_, nested)| nested.def_id == Some(exposure))
        .map(|(name, _)| name.clone());
    let name = matches.next();
    if matches.next().is_some() {
        return Err("function exposure owner contains duplicate exact DefId slots");
    }
    Ok(name)
}

pub(super) fn exact_package_function_exposure(
    class_index: &rumoca_ir_ast::ClassDefIndex<'_>,
    package: rumoca_core::DefId,
    member: &str,
    visited: &mut FxHashSet<rumoca_core::DefId>,
) -> Result<Option<rumoca_core::DefId>, &'static str> {
    if !visited.insert(package) {
        return Err("package inheritance contains a cycle");
    }
    let class_def = class_index
        .get(package)
        .ok_or("selected package DefId is absent from the resolved class index")?;
    if let Some(nested) = class_def.classes.get(member) {
        if nested.class_type != rumoca_core::ClassType::Function {
            return Err("selected package member is not a function");
        }
        let exposure = nested
            .def_id
            .map(Some)
            .ok_or("selected package function has no resolved DefId")?;
        visited.remove(&package);
        return Ok(exposure);
    }

    let mut selected = None;
    for extend in &class_def.extends {
        let base = extend
            .base_def_id
            .or(extend.base_name.def_id)
            .ok_or("selected package base has no resolved DefId")?;
        let Some(candidate) = exact_package_function_exposure(class_index, base, member, visited)?
        else {
            continue;
        };
        match selected {
            Some(existing) if existing != candidate => {
                return Err("selected package inherits multiple exact function exposures");
            }
            Some(_) => {}
            None => selected = Some(candidate),
        }
    }
    visited.remove(&package);
    Ok(selected)
}

/// A body-less function that only extends other functions renames the body
/// they supply. One that declares its own formals is its own interface, not
/// an alias: MSL's partial `PartialMedium.setState_phX` declares its inputs
/// and only extends the empty icon `Modelica.Icons.Function`.
pub(super) fn function_alias_requires_exact_selection(class_def: &rumoca_ir_ast::ClassDef) -> bool {
    class_def.class_type == rumoca_core::ClassType::Function
        && class_def.algorithms.is_empty()
        && class_def.external.is_none()
        && class_def.components.is_empty()
        && !class_def.extends.is_empty()
}

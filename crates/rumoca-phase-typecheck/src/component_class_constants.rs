//! Package constants seen from inside component classes.
//!
//! A dimension such as `Real S[PhaseSystem.n]` declared in a component's class
//! reads a constant of a class the component's class names: a local
//! (replaceable) package, possibly redeclared by an `extends` clause of the
//! component's class (MLS §7.3), or a sibling/enclosing package (`C.n`). The
//! dimension is evaluated in the component's instance scope, so those
//! constants are seeded under `{component}.{class name}.*`:
//!
//! 1. `extends Base(redeclare package P = Q)` in the component's class or its
//!    bases — the most-derived redeclaration wins;
//! 2. otherwise the class the reference's first segment resolved to.
//!
//! A redeclaration on the component itself (`E(redeclare package P = Q)`) is
//! seeded afterwards by the instance class-override collection and wins.

use super::*;

impl TypeChecker {
    /// Seed `{component}.{alias}.*` for every `extends(... redeclare ...)` in
    /// a component's class chain.
    pub(crate) fn collect_component_extends_redeclare_constants(
        tree: &ClassTree,
        overlay: &InstanceOverlay,
        ctx: &mut rumoca_eval_ast::eval::TypeCheckEvalContext,
    ) {
        for data in overlay.components.values() {
            if data.is_primitive {
                continue;
            }
            let Some(type_name) = data.type_def_id.and_then(|id| tree.def_map.get(&id)) else {
                continue;
            };
            let roots = Self::class_chain_redeclare_roots(tree, type_name);
            if roots.is_empty() {
                continue;
            }
            let component = data.qualified_name.to_component_path().to_flat_string();
            let own_override =
                |alias: &str| data.class_overrides.values().any(|o| o.alias == alias);
            for (alias, def_id) in roots.into_iter().filter(|(alias, _)| !own_override(alias)) {
                let prefix = format!("{component}.{alias}");
                Self::clear_alias_scope_values(ctx, &prefix);
                Self::extract_override_class_constants_at_prefix(tree, &prefix, def_id, ctx);
            }
        }
    }

    /// Redeclare roots of a class and its bases, most-derived first; one root
    /// per alias.
    fn class_chain_redeclare_roots(tree: &ClassTree, class_name: &str) -> Vec<(String, DefId)> {
        let mut roots: Vec<(String, DefId)> = Vec::new();
        let mut pending = vec![class_name.to_string()];
        let mut visited = std::collections::HashSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            let Some(class) = tree.get_class_by_qualified_name(&name) else {
                continue;
            };
            for root in Self::collect_redeclare_override_roots(tree, &name, class) {
                push_unique_alias(&mut roots, root);
            }
            let bases = class
                .extends
                .iter()
                .filter_map(|ext| ext.base_def_id.and_then(|id| tree.def_map.get(&id)));
            for base in bases {
                pending.insert(0, base.clone());
            }
        }
        roots
    }

    /// Seed the class a dimension reference's first segment resolved to
    /// (`C` in `C.n`, `PhaseSystem` in `PhaseSystem.n`) under the instance
    /// scope, unless that scope already holds values for it.
    pub(crate) fn seed_dimension_reference_classes(
        tree: &ClassTree,
        overlay: &InstanceOverlay,
        ctx: &mut rumoca_eval_ast::eval::TypeCheckEvalContext,
    ) {
        for data in overlay.components.values() {
            Self::seed_instance_dimension_classes(tree, data, ctx);
        }
    }

    fn seed_instance_dimension_classes(
        tree: &ClassTree,
        data: &rumoca_ir_ast::InstanceData,
        ctx: &mut rumoca_eval_ast::eval::TypeCheckEvalContext,
    ) {
        let path = data.qualified_name.to_component_path();
        let scope = path
            .parent()
            .map(|p| p.to_flat_string())
            .unwrap_or_default();
        let mut references = Vec::new();
        for subscript in &data.dims_expr {
            if let rumoca_ir_ast::Subscript::Expression(expr) = subscript {
                collect_class_rooted_references(tree, expr, &mut references);
            }
        }
        let classes = references
            .into_iter()
            .filter(|(_, def_id)| tree.get_class_by_def_id(*def_id).is_some());
        for (ident, def_id) in classes {
            let prefix = if scope.is_empty() {
                ident
            } else {
                format!("{scope}.{ident}")
            };
            if !scope_has_values(ctx, &prefix) {
                Self::extract_override_class_constants_at_prefix(tree, &prefix, def_id, ctx);
            }
        }
    }
}

fn push_unique_alias(roots: &mut Vec<(String, DefId)>, root: (String, DefId)) {
    if !roots.iter().any(|(seen, _)| *seen == root.0) {
        roots.push(root);
    }
}

fn scope_has_values(ctx: &rumoca_eval_ast::eval::TypeCheckEvalContext, prefix: &str) -> bool {
    let dotted = format!("{prefix}.");
    ctx.integers.keys().any(|k| k.starts_with(&dotted))
        || ctx.reals.keys().any(|k| k.starts_with(&dotted))
        || ctx.booleans.keys().any(|k| k.starts_with(&dotted))
}

/// `(first segment, its resolved declaration)` of every multi-segment
/// component reference in a dimension expression, followed by
/// `(class path, its declaration)` of the deepest class the reference names
/// before its member (`A.B.C` in `A.B.C.n`). Extraction from a class reaches
/// one nested level, so a package constant several packages deep
/// (`Modelica.Math.Random.Generators.Xorshift128plus.nState`) is only
/// seeded through its own enclosing class (MLS §5.3.2, §10.1).
fn collect_class_rooted_references(
    tree: &ClassTree,
    expr: &Expression,
    out: &mut Vec<(String, DefId)>,
) {
    match expr {
        Expression::ComponentReference(cr) => {
            if cr.parts.len() >= 2
                && let Some(first) = cr.parts.first()
                && let Some(def_id) = first.def_id
            {
                out.push((first.ident.text.to_string(), def_id));
                if let Some(deepest) = deepest_named_class(tree, cr, def_id) {
                    out.push(deepest);
                }
            }
        }
        Expression::Binary { lhs, rhs, .. } => {
            collect_class_rooted_references(tree, lhs, out);
            collect_class_rooted_references(tree, rhs, out);
        }
        Expression::Unary { rhs, .. } => collect_class_rooted_references(tree, rhs, out),
        Expression::Parenthesized { inner, .. } => {
            collect_class_rooted_references(tree, inner, out)
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                collect_class_rooted_references(tree, arg, out);
            }
        }
        Expression::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, value) in branches {
                collect_class_rooted_references(tree, cond, out);
                collect_class_rooted_references(tree, value, out);
            }
            collect_class_rooted_references(tree, else_branch, out);
        }
        _ => {}
    }
}

/// The deepest nested class (below the first segment's class) that the
/// non-final segments of `cr` name, as `(dotted path, declaration)`.
fn deepest_named_class(
    tree: &ClassTree,
    cr: &rumoca_ir_ast::ComponentReference,
    root: DefId,
) -> Option<(String, DefId)> {
    let mut class = tree.get_class_by_def_id(root)?;
    let mut path = cr.parts.first()?.ident.text.to_string();
    let mut deepest = None;
    for part in &cr.parts[1..cr.parts.len() - 1] {
        let name = part.ident.text.as_ref();
        let Some(nested) = class.classes.get(name) else {
            break;
        };
        let Some(def_id) = nested.def_id else {
            break;
        };
        path = format!("{path}.{name}");
        class = nested;
        deepest = Some((path.clone(), def_id));
    }
    deepest
}

//! Structural evaluation of an Integer reached through a component of the
//! current scope (`vehicle.geometry.nRotors`).

use super::*;

/// `vehicle.geometry.nRotors`: an Integer reached through a component of this
/// scope (MLS §5.3.2, a component's elements are looked up in its class).
///
/// Each step resolves the next part of the path against the component the
/// previous one named, in this order:
/// - a modifier this scope wrote on it for that part (`vehicle(geometry = g)`,
///   `vehicle(geometry(nRotors = 6))`) is evaluated here, where it was
///   written; a whole-component binding forwards the rest of the path to
///   whatever it is bound to;
/// - a declaration binding of the component (`Geometry geometry = QuadX()`)
///   does the same;
/// - otherwise the rest is looked up inside the component's class, after the
///   modifiers this scope wrote on its elements are settled here into values,
///   so no name in them resolves against the class.
///
/// Anything not settled this way is unknown: the caller keeps the dimension
/// unevaluated rather than read a default the modifiers replaced.
pub(in crate::eval_instantiate) fn eval_integer_component_member_ref(
    comp_ref: &ast::ComponentReference,
    env: IntegerEvalEnv<'_>,
    depth: usize,
) -> Option<i64> {
    let [root, rest @ ..] = comp_ref.parts.as_slice() else {
        return None;
    };
    if rest.is_empty()
        || depth > MAX_EXPR_EVAL_DEPTH
        || comp_ref.parts.iter().any(|part| part.subs.is_some())
    {
        return None;
    }
    // An outer modifier on a prefix of the path (`hexa(vehicle(geometry =
    // HexaX()))` seen from inside `hexa`) replaces whatever the component
    // declares there, so the rest of the path is read from it.
    for split in (1..comp_ref.parts.len()).rev() {
        let mut prefix = ast::QualifiedName::new();
        for part in &comp_ref.parts[..split] {
            prefix.push(part.ident.text.to_string(), Vec::new());
        }
        // Only a record constructor is read here: its value does not depend
        // on the scope it is read from. A reference (`geometry = geometry`)
        // names a component of the scope that wrote it, which the record
        // alias resolution below follows.
        if let Some(modification) = env.mod_env.get(&prefix)
            && matches!(modification.value, ast::Expression::FunctionCall { .. })
            && let Some(value) =
                eval_integer_bound_member(&modification.value, &comp_ref.parts[split..], env, depth)
        {
            return Some(value);
        }
    }
    let component = env.effective_components.get(root.ident.text.as_ref())?;
    if !component.shape_expr.is_empty() || !component.shape.is_empty() {
        return None;
    }
    let here = |expr: &ast::Expression| {
        try_eval_integer_expr_with_depth_and_locals(
            expr,
            env.mod_env,
            env.effective_components,
            env.tree,
            env.resolve_class_components,
            depth + 1,
            None,
        )
    };
    let member = rest[0].ident.text.as_ref();
    match component.modifications.get(member) {
        Some(ast::Expression::ClassModification { modifications, .. }) => {
            if let Some(value) = modifications.iter().find_map(|modification| {
                let ast::Expression::Modification { target, value, .. } = modification else {
                    return None;
                };
                same_path(&target.parts, &rest[1..]).then_some(value)
            }) {
                return here(value);
            }
        }
        Some(bound) => {
            return if rest.len() == 1 {
                here(bound)
            } else {
                eval_integer_bound_member(bound, &rest[1..], env, depth)
            };
        }
        None => {}
    }
    if let Some(bound) = component.binding.as_ref() {
        return eval_integer_bound_member(bound, rest, env, depth);
    }
    let class = record_class_for_component(env.tree, component)?;
    let mut members = (env.resolve_class_components)(env.tree, class);
    settle_member_modifiers(&component.modifications, &mut members, |expr| here(expr));
    // Outer modifiers reaching inside the component keep applying there.
    let mut inner = ast::ModificationEnvironment::new();
    for (target, modification) in &env.mod_env.active {
        if let [(first, subscripts), nested @ ..] = target.parts.as_slice()
            && !nested.is_empty()
            && subscripts.is_empty()
            && first.as_str() == root.ident.text.as_ref()
        {
            inner.add(
                ast::QualifiedName {
                    parts: nested.to_vec(),
                },
                modification.clone(),
            );
        }
    }
    try_eval_integer_expr_with_depth_and_locals(
        &ast::Expression::ComponentReference(ast::ComponentReference {
            parts: rest.to_vec(),
            ..comp_ref.clone()
        }),
        &inner,
        &members,
        env.tree,
        env.resolve_class_components,
        depth + 1,
        None,
    )
}

/// The rest of a path read from the value a component is bound to: another
/// component of this scope, or a record constructor call.
fn eval_integer_bound_member(
    bound: &ast::Expression,
    rest: &[ast::ComponentRefPart],
    env: IntegerEvalEnv<'_>,
    depth: usize,
) -> Option<i64> {
    let here = |expr: &ast::Expression| {
        try_eval_integer_expr_with_depth_and_locals(
            expr,
            env.mod_env,
            env.effective_components,
            env.tree,
            env.resolve_class_components,
            depth + 1,
            None,
        )
    };
    match bound {
        ast::Expression::Parenthesized { inner, .. } => {
            eval_integer_bound_member(inner, rest, env, depth)
        }
        ast::Expression::ComponentReference(reference) => {
            let mut path = reference.clone();
            path.parts.extend(rest.iter().cloned());
            here(&ast::Expression::ComponentReference(path))
        }
        ast::Expression::FunctionCall { comp, args, .. } => {
            let class = comp
                .parts
                .last()
                .and_then(|part| part.def_id)
                .and_then(|def_id| env.tree.get_class_by_def_id(def_id))
                .or_else(|| {
                    let name = comp.to_string();
                    find_class_in_tree(env.tree, &name)
                        .or_else(|| lookup_unique_class_suffix(env.tree, &name))
                })?;
            if class.class_type != rumoca_core::ClassType::Record {
                return None;
            }
            let mut members = (env.resolve_class_components)(env.tree, class);
            let positional = members.keys().cloned().collect::<Vec<_>>();
            let mut supplied = IndexMap::default();
            for (index, argument) in args.iter().enumerate() {
                match argument {
                    ast::Expression::NamedArgument { name, value, .. } => {
                        supplied.insert(name.text.to_string(), value.as_ref().clone());
                    }
                    value => {
                        supplied.insert(positional.get(index)?.clone(), value.clone());
                    }
                }
            }
            settle_member_modifiers(&supplied, &mut members, |expr| here(expr));
            let inner = ast::ModificationEnvironment::new();
            try_eval_integer_expr_with_depth_and_locals(
                &ast::Expression::ComponentReference(ast::ComponentReference {
                    local: false,
                    parts: rest.to_vec(),
                    span: comp.span,
                    qualified_display_name: None,
                }),
                &inner,
                &members,
                env.tree,
                env.resolve_class_components,
                depth + 1,
                None,
            )
        }
        _ => None,
    }
}

fn same_path(written: &[ast::ComponentRefPart], path: &[ast::ComponentRefPart]) -> bool {
    written.len() == path.len()
        && written
            .iter()
            .zip(path)
            .all(|(lhs, rhs)| lhs.subs.is_none() && lhs.ident.text == rhs.ident.text)
}

/// Replace each element a modifier reaches by the value the modifier settles
/// to where it was written. An element whose modifier does not settle to an
/// Integer loses its binding, so nothing downstream reads the default the
/// modifier replaced.
fn settle_member_modifiers(
    modifiers: &IndexMap<String, ast::Expression>,
    members: &mut IndexMap<String, ast::Component>,
    settle: impl Fn(&ast::Expression) -> Option<i64>,
) {
    for (name, modifier) in modifiers {
        let Some(member) = members.get_mut(name) else {
            continue;
        };
        if let ast::Expression::ClassModification { modifications, .. } = modifier {
            settle_nested_modifiers(modifications, member, &settle);
            continue;
        }
        member.binding = settle(modifier).map(|value| integer_literal(value, modifier.span()));
        member.modifications.clear();
    }
}

/// `member(field = value, ...)`: each one-level field modifier settled where
/// it was written, or dropped when it does not settle to an Integer.
fn settle_nested_modifiers(
    modifications: &[ast::Expression],
    member: &mut ast::Component,
    settle: &impl Fn(&ast::Expression) -> Option<i64>,
) {
    for modification in modifications {
        let ast::Expression::Modification { target, value, .. } = modification else {
            continue;
        };
        let [field] = target.parts.as_slice() else {
            continue;
        };
        let field = field.ident.text.to_string();
        match settle(value) {
            Some(settled) => {
                member
                    .modifications
                    .insert(field, integer_literal(settled, value.span()));
            }
            None => {
                member.modifications.shift_remove(&field);
            }
        }
    }
}

fn integer_literal(value: i64, span: rumoca_core::Span) -> ast::Expression {
    let magnitude = ast::Expression::Terminal {
        terminal_type: ast::TerminalType::UnsignedInteger,
        token: rumoca_core::Token {
            text: std::sync::Arc::from(value.unsigned_abs().to_string()),
            ..Default::default()
        },
        span,
    };
    if value >= 0 {
        return magnitude;
    }
    ast::Expression::Unary {
        op: rumoca_core::OpUnary::Minus,
        rhs: std::sync::Arc::new(magnitude),
        span,
    }
}

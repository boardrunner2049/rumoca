//! Declaration-side attributes and binding of one component, prepared before
//! the component itself is instantiated.

use super::*;

pub(super) struct ComponentBindingInfo {
    pub(super) attrs: ExtractedAttributes,
    pub(super) type_attribute_shapes: TypeAttributeShapes,
    pub(super) binding: Option<ast::Expression>,
    pub(super) binding_source: Option<ast::Expression>,
    pub(super) binding_source_scope: Option<ast::QualifiedName>,
    pub(super) binding_from_modification: bool,
    pub(super) binding_is_each: bool,
}

/// Instance path of the class declaring the component being instantiated.
pub(super) fn enclosing_instance_path(ctx: &InstantiateContext) -> ast::QualifiedName {
    let mut path = ctx.current_path();
    path.parts.pop();
    path
}

pub(super) fn prepare_component_binding_info(
    tree: &ast::ClassTree,
    comp: &ast::Component,
    ctx: &mut InstantiateContext,
    effective_components: &IndexMap<String, ast::Component>,
    type_overrides: &TypeOverrideMap,
    type_info: &TypeInfo<'_>,
    imports: &[(String, String)],
) -> InstantiateResult<ComponentBindingInfo> {
    let is_discrete_type = type_info.is_discrete;
    let eval_ctx = InstantiateEvalCtx {
        tree,
        mod_env: ctx.mod_env(),
        effective_components,
        resolve_class_components: resolve_effective_components_for_eval,
    };
    let ComponentAttrsAndBinding {
        mut attrs,
        mut binding,
        mut binding_source,
        binding_source_scope,
        binding_from_modification,
        binding_is_each,
    } = extract_component_attrs_and_binding(
        comp,
        ctx.mod_env(),
        &eval_ctx,
        imports,
        &enclosing_instance_path(ctx),
    )?;
    let type_attribute_shapes = if type_info.is_primitive {
        let merge = TypeAttributeMerge {
            ctx,
            comp,
            class_def: type_info.class_def,
            eval_ctx: &eval_ctx,
        };
        merge_type_hierarchy_attributes(&merge, &mut attrs)?
    } else {
        TypeAttributeShapes::default()
    };
    // Sibling component occurrences of this class are still being materialized,
    // so only class-alias selections can be proved for declaration-side
    // expressions here. A member that stays unproven keeps its absent identity
    // and is reported at the Flat boundary rather than guessed.
    let selected_component_types = SelectedComponentTypes::default();
    for expression in [
        &mut binding,
        &mut binding_source,
        &mut attrs.start,
        &mut attrs.min,
        &mut attrs.max,
        &mut attrs.nominal,
    ] {
        if let Some(value) = expression.take() {
            *expression = Some(resolve_dynamic_expression_targets(
                tree,
                type_overrides,
                &selected_component_types,
                value,
            )?);
        }
    }
    infer_local_attribute_source_scopes(ctx, comp, &mut attrs);
    let start_from_declaration_binding =
        !binding_from_modification && binding.is_some() && attrs.start == binding;
    if !binding_from_modification
        && declaration_binding_needs_structural_resolution(comp, is_discrete_type)
        && let Some(declaration_binding) = binding.as_ref()
    {
        let resolved_binding = mod_env::resolve_declaration_binding_expr(
            declaration_binding,
            ctx.mod_env(),
            effective_components,
            tree,
        )?;
        if resolved_binding != *declaration_binding {
            binding_source.get_or_insert_with(|| declaration_binding.clone());
        }
        if start_from_declaration_binding {
            attrs.start = Some(resolved_binding.clone());
        }
        binding = Some(resolved_binding);
    }
    Ok(ComponentBindingInfo {
        attrs,
        type_attribute_shapes,
        binding,
        binding_source,
        binding_source_scope,
        binding_from_modification,
        binding_is_each,
    })
}

fn declaration_binding_needs_structural_resolution(
    comp: &ast::Component,
    is_discrete_type: bool,
) -> bool {
    // A structural parameter (MLS §18.3) and a discrete-typed binding are
    // resolved here: the first decides array extents and branch selection,
    // the second the value a `when` settles on, and both change the shape of
    // the model rather than only a number inside it. Any other binding keeps
    // the dependency it was written with; replacing it by its value is the
    // `fold-parameter-bindings` bitcode pass (docs/design/minimal-frontend.md).
    comp.is_structural || is_discrete_type
}

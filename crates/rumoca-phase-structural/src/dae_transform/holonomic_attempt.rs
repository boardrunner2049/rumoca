//! One holonomic candidate attempt: its reconstruction, the postconditions
//! the rebuilt system must meet, and the outcome it proves.

use rumoca_ir_dae as dae;

use super::*;

/// What attempting one holonomic candidate against `model` found, mirroring
/// [`DirectAttempt`]: every recorded event is already emitted by the time
/// this returns.
pub(super) enum HolonomicAttempt {
    Sorted {
        dae: Box<dae::Dae>,
        manifold: Vec<ManifoldConstraint>,
        structural: PreparedStructuralAnalysis,
    },
    Accepted {
        constraint: HolonomicConstraint,
        residue: usize,
        step: Box<HolonomicStep>,
    },
    Blocked(DiscardedInitialValue),
    Rejected,
}

pub(super) fn refused_holonomic_outcome(next: usize, residue: usize) -> AttemptOutcome<'static> {
    debug_assert!(next >= residue);
    if next == residue {
        AttemptOutcome::Held { residue: next }
    } else {
        AttemptOutcome::Raised { residue: next }
    }
}

pub(super) fn residual_scalar_is_structurally_active<'dae>(
    view: dae::DaeView<'dae>,
    expression: dae::ExprId<'dae>,
    scalar: usize,
    domain_point: Option<(dae::DomainId<'dae>, &[i64])>,
    cache: &mut rumoca_eval_dae::ScalarCoordinateProjectionCache<'dae>,
) -> bool {
    let mut active = false;
    let projected = rumoca_eval_dae::for_each_scalar_coordinate_cached(
        view,
        expression,
        scalar,
        domain_point,
        cache,
        |coordinate, _| {
            active |= matches!(
                coordinate,
                dae::CoordinateView::Derivative(_) | dae::CoordinateView::Algebraic(_)
            );
        },
    );
    projected.is_ok() && active
}

/// Whether the exact replacement for one holonomic owner still contributes a
/// continuous unknown in at least one scalar equation it owns.
///
/// Differentiation can prove an expression admissible and nevertheless reduce
/// it to an exact shaped zero once causal definitions are substituted. Such a
/// residual is not a valid Pantelides replacement: retaining it would add an
/// equation row that can never match an unknown. Check the rebuilt owner, not
/// the source syntax, so this postcondition covers scalar residuals and compact
/// structured families through the same scalar projection used by incidence.
pub(super) fn holonomic_replacement_is_structurally_active(
    model: &dae::Dae,
    constraint: &HolonomicConstraint,
) -> bool {
    model.inspect(|view| {
        let Some(owner) = view.continuous_owners().nth(constraint.owner_ordinal) else {
            return false;
        };
        let mut cache = rumoca_eval_dae::ScalarCoordinateProjectionCache::default();
        match owner {
            dae::ContinuousOwnerView::Residual { equation, .. } => {
                let residual = equation.residual();
                let Some(scalar_count) = view
                    .expression(residual)
                    .and_then(|expression| expression.value_type().scalar_count())
                else {
                    return false;
                };
                (0..scalar_count).any(|scalar| {
                    constraint
                        .proof
                        .component
                        .as_ref()
                        .is_none_or(|component| component.scalar == scalar)
                        && residual_scalar_is_structurally_active(
                            view, residual, scalar, None, &mut cache,
                        )
                })
            }
            dae::ContinuousOwnerView::Structured { family, .. } => {
                structured_replacement_is_active(view, family, constraint, &mut cache)
            }
        }
    })
}

pub(super) fn structured_replacement_is_active<'dae>(
    view: dae::DaeView<'dae>,
    family: dae::StructuredFamilyView<'dae>,
    constraint: &HolonomicConstraint,
    cache: &mut rumoca_eval_dae::ScalarCoordinateProjectionCache<'dae>,
) -> bool {
    let Some(body_ordinal) = constraint.body_ordinal else {
        return false;
    };
    let Some(residual) = family.bodies().get(body_ordinal) else {
        return false;
    };
    let Some(domain) = view.domain(family.domain()) else {
        return false;
    };
    let structured = domain.structured();
    (0..domain.scalar_count() as usize).any(|point| {
        let Ok(Some(values)) = structured.index_tuple_at(point) else {
            return false;
        };
        let Some(scalar) = family.scalar_view().body_scalar(point, domain.extents()) else {
            return false;
        };
        if constraint
            .proof
            .component
            .as_ref()
            .is_some_and(|component| component.scalar != scalar)
        {
            return false;
        }
        residual_scalar_is_structurally_active(
            view,
            residual,
            scalar,
            Some((family.domain(), values.as_slice())),
            cache,
        )
    })
}

/// Try one certificate, observing its identity and outcome. Every decision
/// this makes is exactly the one the pre-observation code made at this same
/// branch point.
pub(super) fn observe_discarded_holonomic_initial(
    observer: &mut impl ReductionObserver,
    identity: Identity,
    discarded: &DiscardedInitialValue,
) {
    observer.observe(ReductionEvent::Attempt {
        lane: Lane::Holonomic,
        identity,
        outcome: AttemptOutcome::WouldDiscardInitial {
            variable: &discarded.variable,
            span: discarded.span,
        },
    });
}

pub(super) fn attempt_holonomic_candidate(
    source: &ReductionSource<'_>,
    residue: usize,
    prior_manifold: &[ManifoldConstraint],
    stated: &[u32],
    constraint: HolonomicConstraint,
    reuse: Option<&crate::incidence::ReusableIncidence>,
    observer: &mut impl ReductionObserver,
) -> Result<HolonomicAttempt, StructuralError> {
    let model = source.model();
    let identity = Identity::Holonomic(HolonomicIdentity::from(&constraint));
    let (rebuilt, manifold) =
        match rebuild_holonomic_constraint(source, &constraint, prior_manifold) {
            Ok(pair) => pair,
            Err(error) => {
                observer.observe(ReductionEvent::Attempt {
                    lane: Lane::Holonomic,
                    identity,
                    outcome: AttemptOutcome::NonSingularFailure { error: &error },
                });
                return Err(error);
            }
        };
    if !equation_activity::preserves_equations(source.model(), &rebuilt)
        || !holonomic_replacement_is_structurally_active(&rebuilt, &constraint)
    {
        observer.observe(ReductionEvent::Attempt {
            lane: Lane::Holonomic,
            identity,
            outcome: AttemptOutcome::WouldCreateVacuousResidual,
        });
        return Ok(HolonomicAttempt::Rejected);
    }
    let (next, retained_error, structural) =
        match holonomic_analysis(&rebuilt, &constraint, reuse, &source.demotion_rows) {
            Ok(structural) => (None, None, Some(structural)),
            Err(error) => match unmatched_residue(&error) {
                Some(next) if next <= residue => (Some(next), Some(error), None),
                Some(next) => {
                    observer.observe(ReductionEvent::Attempt {
                        lane: Lane::Holonomic,
                        identity,
                        outcome: refused_holonomic_outcome(next, residue),
                    });
                    return Ok(HolonomicAttempt::Rejected);
                }
                None => {
                    observer.observe(ReductionEvent::Attempt {
                        lane: Lane::Holonomic,
                        identity,
                        outcome: AttemptOutcome::NonSingularFailure { error: &error },
                    });
                    return Ok(HolonomicAttempt::Rejected);
                }
            },
        };
    let discarded = discarded_initial_after_attempt(
        model,
        &rebuilt,
        stated,
        Lane::Holonomic,
        identity,
        observer,
    )?;
    if let Some(discarded) = discarded {
        observe_discarded_holonomic_initial(observer, identity, &discarded);
        return Ok(HolonomicAttempt::Blocked(discarded));
    }
    let Some(next) = next else {
        observer.observe(ReductionEvent::Attempt {
            lane: Lane::Holonomic,
            identity,
            outcome: AttemptOutcome::Sorted,
        });
        observer.observe(ReductionEvent::Selected {
            lane: Lane::Holonomic,
            identity,
            residue_before: residue,
            residue_after: None,
        });
        return Ok(HolonomicAttempt::Sorted {
            dae: Box::new(rebuilt),
            manifold,
            structural: structural.expect("a sorted holonomic attempt retains its analysis"),
        });
    };
    observer.observe(ReductionEvent::Attempt {
        lane: Lane::Holonomic,
        identity,
        outcome: if next < residue {
            AttemptOutcome::Reduced { residue: next }
        } else {
            AttemptOutcome::Held { residue: next }
        },
    });
    Ok(HolonomicAttempt::Accepted {
        constraint,
        residue: next,
        step: Box::new(HolonomicStep::Reduced {
            dae: rebuilt,
            manifold,
            residue: next,
            error: retained_error.expect("reduced candidate retains its proving error"),
        }),
    })
}

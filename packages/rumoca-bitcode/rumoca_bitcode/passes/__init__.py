"""SDK bitcode passes: equation-IR rewrites that run outside the compiler.

Each pass takes a `Model` and rewrites its equation IR through a builder, so
provenance names the pass. `run(model, names)` runs a pipeline and validates
after every step, so a step that breaks the artifact is named as the one that
did. `PREPARE_EXECUTION` is the pipeline that turns a linked, composed model
into one `rumoca bitcode lower-execution` (execution v1: continuous, scalar
real storage, no events, no calls) accepts.

    python -m rumoca_bitcode.passes IN.rbc -o OUT.rbc [--pass a,b,...]

speaks the external-pass protocol (`COMMAND IN.rbc -o OUT.rbc`), so it also
runs as `rumoca compile --pass exec:'python -m rumoca_bitcode.passes'`.
"""
from __future__ import annotations

from typing import Callable

from ._graph import PassError
from .asserts import (check_monitors, fold_parameter_asserts, gc_event_tables,
                      monitor_runtime_asserts)
from .compose import bind_input, element, set_parameter
from .families import unroll_binder_families
from .fold import fold_constants
from .inline import inline_functions
from .observe import log_trace_points, observe
from .parameters import algebraic_outputs, freeze_derived_parameters, real_parameters
from .scalarize import compact_expressions, scalarize

__all__ = [
    "PassError", "PASSES", "PREPARE_EXECUTION", "run", "bind_input", "set_parameter",
    "element", "observe", "log_trace_points", "check_monitors", "compact_expressions",
]

#: name -> (function, one line). A function returns how many rewrites it made.
PASSES: dict[str, tuple[Callable, str]] = {
    "unroll-binder-families": (unroll_binder_families,
                               "expand binder-substituted equation families into scalar rows"),
    "inline-functions": (inline_functions, "inline straight-line Modelica function calls"),
    "fold-constants": (fold_constants, "fold literal arithmetic, branches and indexing"),
    "fold-parameter-asserts": (fold_parameter_asserts,
                               "decide parameter-only asserts; fail on a violated one"),
    "monitor-runtime-asserts": (monitor_runtime_asserts,
                                "turn run-time asserts into observed margin outputs"),
    "real-parameters": (real_parameters, "store numeric Integer parameters as Real"),
    "gc-event-tables": (gc_event_tables, "drop conditions/relations/roots nothing fires"),
    "scalarize": (scalarize, "split array variables and equations into scalars"),
    "freeze-derived-parameters": (freeze_derived_parameters,
                                  "pin computed parameters; freeze what they read"),
    "algebraic-outputs": (algebraic_outputs, "store output-role variables as algebraics"),
}

PREPARE_EXECUTION = [
    "unroll-binder-families",
    "inline-functions",
    "fold-constants",
    "fold-parameter-asserts",
    "monitor-runtime-asserts",
    "real-parameters",
    "gc-event-tables",
    "scalarize",
    "freeze-derived-parameters",
    "algebraic-outputs",
    "fold-constants",
]


def run(model, names=None, *, validate: bool = True, log: Callable[[str], None] | None = None):
    """Run passes in order; return `[(name, rewrites)]`."""
    reports = []
    for name in names or PREPARE_EXECUTION:
        if name not in PASSES:
            raise PassError(f"unknown pass {name!r}; known: {', '.join(PASSES)}")
        function = PASSES[name][0]
        builder = model.builder(f"rumoca_bitcode.passes.{name}")
        rewrites = function(model, builder)
        builder.finish()
        if validate:
            try:
                model.validate()
            except Exception as error:  # noqa: BLE001 - name the step, keep the cause
                raise PassError(f"artifact invalid after pass {name}: {error}") from error
        reports.append((name, rewrites))
        if log:
            log(f"pass {name}: {rewrites} rewrite(s)")
    return reports

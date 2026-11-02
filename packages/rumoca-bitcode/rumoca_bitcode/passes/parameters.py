"""real-parameters: store tunable Integer parameters as Real.

Execution v1 stores real scalars only. An Integer parameter used purely as a
number (`-spinSign * torque`) means the same as a Real one holding an integral
value, so it is retyped and every expression above it re-derives its type
(the arena is topological, so one forward sweep does it). A parameter that
an index, range or relation-free structural position reads is left alone and
reported: there an Integer is not interchangeable with a Real.
"""
from __future__ import annotations

from ._graph import Graph, PassError

_RETYPE = {"unary": ("negate", "plus"),
           "binary": ("add", "subtract", "multiply", "divide", "power",
                      "elementwise_add", "elementwise_subtract",
                      "elementwise_multiply", "elementwise_divide", "elementwise_power")}
_RETYPE_BUILTINS = {"abs", "min", "max", "sum", "noEvent", "smooth"}


def _structural_uses(g: Graph, vid: int) -> bool:
    """Whether `vid` is read as a subscript, range bound or domain size."""
    for entry in g.E:
        node = entry["node"]
        exprs = [s.get("expression") for s in node.get("subscripts") or ()]
        if node["kind"] == "range":
            exprs += [node.get("start"), node.get("stop"), node.get("step")]
        for e in exprs:
            if isinstance(e, int) and vid in g.reads(e):
                return True
    return False


def real_parameters(model, builder=None, report: list | None = None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.real-parameters")
    g = Graph(b)
    retyped = set()
    for v in g.raw["variables"]:
        if v.get("role") != "parameter" or g.scalar_of(v["value_type"]) != "integer":
            continue
        if _structural_uses(g, v["id"]):
            if report is not None:
                report.append(f"{v['name']}: read structurally, kept Integer")
            continue
        dims = g.dims_of(v["value_type"])
        v["value_type"] = g.type_like("real", dims)
        for key in ("binding", "start"):
            if isinstance(v.get(key), int):
                value = g.literal_value(v[key])
                if isinstance(value, int) and not isinstance(value, bool):
                    v[key] = g.literal(float(value), g.type_like("real", []))
        retyped.add(v["id"])
    if not retyped:
        if builder is None:
            b.finish()
        return 0
    from ._graph import children
    real_ids = set()
    for entry in g.E:
        node, kind = entry["node"], entry["node"]["kind"]
        if kind == "coordinate" and node["coordinate"].get("variable") in retyped \
                and node["coordinate"].get("kind") in ("parameter", "constant"):
            pass
        elif kind in _RETYPE and node.get("op") in _RETYPE[kind] \
                and any(c in real_ids for c in children(node)):
            pass
        elif kind == "builtin" and node["name"] in _RETYPE_BUILTINS \
                and any(c in real_ids for c in children(node)):
            pass
        else:
            continue
        t = entry["value_type"]
        if g.scalar_of(t) == "integer":
            entry["value_type"] = g.type_like("real", g.dims_of(t))
            real_ids.add(entry["id"])
    if builder is None:
        b.finish()
    return len(retyped)


def _parameter_reads(g: Graph, root: int, seen: set[int]) -> set[int]:
    """Parameters `root` depends on, through bindings, transitively."""
    found = set()
    for vid in g.reads(root):
        v = g.raw["variables"][vid]
        if v.get("role") not in ("parameter", "constant") or vid in seen:
            continue
        seen.add(vid)
        found.add(vid)
        source = v.get("binding", v.get("start"))
        if isinstance(source, int):
            found |= _parameter_reads(g, source, seen)
    return found


def freeze(g: Graph, variables: set[int]) -> list[str]:
    """Mark parameters non-tunable; return the names that changed."""
    changed = []
    for vid in sorted(variables):
        v = g.raw["variables"][vid]
        if v.get("tunable", True):
            v["tunable"] = False
            changed.append(v["name"])
    return changed


def freeze_derived_parameters(model, builder=None, report: list | None = None) -> int:
    """Pin derived (non-tunable, computed) parameters to their values.

    Execution v1 cannot run an initialization update, which is what a
    parameter computed from others needs. The value is evaluated here instead
    -- and every parameter it reads is made non-tunable, so a run cannot
    change an input and leave the pinned value stale. Change those with
    `set_parameter` and re-run the passes.
    """
    from ._graph import Evaluator, NotConstant
    b = builder or model.builder("rumoca_bitcode.passes.freeze-derived-parameters")
    g = Graph(b)
    ev = Evaluator(g)
    # Evaluate everything before pinning anything: a derived parameter read by
    # another must still lead the dependency walk back to its own inputs.
    plan, inputs = [], set()
    for v in g.raw["variables"]:
        if v.get("role") != "parameter" or v.get("tunable", True):
            continue
        source = v.get("binding", v.get("start"))
        if not isinstance(source, int) or g.node(source)["kind"] == "literal":
            continue
        if g.dims_of(v["value_type"]):
            raise PassError(f"{v['name']} is an array; run scalarize first")
        try:
            value = ev.variable(v["id"])
        except NotConstant as why:
            raise PassError(f"derived parameter {v['name']} is not computable: {why}") from why
        inputs |= _parameter_reads(g, source, set())
        plan.append((v, value))
    for v, value in plan:
        literal = g.literal(value, v["value_type"])
        v["binding"] = literal
        v["start"] = literal
        v.setdefault("contract", {})["effective_value"] = float(value)
    pinned = len(plan)
    changed = freeze(g, inputs)
    if report is not None:
        report.extend(f"{name}: frozen (a pinned derived parameter reads it)" for name in changed)
    if builder is None:
        b.finish()
    return pinned


def algebraic_outputs(model, builder=None) -> int:
    """Store `output`-role variables as algebraics with output causality.

    In this IR `output` is a role a top-level output declaration gets, but to
    the solver it is an algebraic unknown like any other; causality (which is
    kept) is what says it is published.
    """
    b = builder or model.builder("rumoca_bitcode.passes.algebraic-outputs")
    n = 0
    for v in b.raw["variables"]:
        if v.get("role") == "output":
            v["role"] = "algebraic"
            v["causality"] = "output"
            n += 1
    if builder is None:
        b.finish()
    return n

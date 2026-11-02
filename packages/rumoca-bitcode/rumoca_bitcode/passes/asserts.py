"""Assertions, decided before lowering.

fold-parameter-asserts
    An assert whose condition reads only parameters and constants is decided
    by the artifact's own parameter values. Satisfied: the event is removed.
    Violated: the pass fails with the assert's message, exactly as the run
    would at initialization. (A run that overrides one of those parameters
    must re-run this pass; the artifact records nothing that would hide it.)

monitor-runtime-asserts
    An assert on run-time values cannot be an event in execution v1. Rather
    than drop the check, each becomes an observable *margin* output,
    `__assert_<k> = lhs - rhs` (or `rhs - lhs`), with a trace point whose
    label carries the message and the bound (`>= 0` or `> 0`). The check
    moves from the solver to whoever reads the trace; `check_monitors` is
    that reader. Only single-relation conditions have a margin; anything
    else is refused.

gc-event-tables
    Drops conditions, relations and roots once nothing that could fire them
    is left (no events, discrete definitions, clocks or time events).
"""
from __future__ import annotations

from ._graph import Evaluator, Graph, NotConstant, PassError

MONITOR_PREFIX = "__assert_"
_FLIP = {"greater_equal": ("lhs", ">= 0"), "greater": ("lhs", "> 0"),
         "less_equal": ("rhs", ">= 0"), "less": ("rhs", "> 0")}


def _condition(ev: Evaluator, g: Graph, cid: int) -> bool:
    node = g.raw["conditions"][cid]["node"]
    kind = node["kind"]
    if kind == "always":
        return True
    if kind == "initial":
        return True
    if kind == "relation":
        return bool(ev.value(g.raw["relations"][node["relation"]]["expression"]))
    if kind == "discrete":
        return bool(ev.value(node["expression"]))
    if kind == "not":
        return not _condition(ev, g, node["operand"])
    if kind == "and":
        return _condition(ev, g, node["lhs"]) and _condition(ev, g, node["rhs"])
    if kind == "or":
        return _condition(ev, g, node["lhs"]) or _condition(ev, g, node["rhs"])
    raise NotConstant(kind)


def _message(g: Graph, event: dict) -> str:
    mid = event["action"].get("message")
    value = g.literal_value(mid) if isinstance(mid, int) else None
    return value if isinstance(value, str) else f"event {event['id']}"


def _renumber_events(g: Graph, kept: list) -> None:
    for index, event in enumerate(kept):
        event["id"] = index
    g.raw["events"] = kept


def fold_parameter_asserts(model, builder=None, report: list | None = None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.fold-parameter-asserts")
    g = Graph(b)
    ev = Evaluator(g)
    kept, removed, decided = [], 0, []
    for event in g.raw.get("events", []):
        if event["action"].get("kind") != "assert":
            kept.append(event)
            continue
        try:
            fires = _condition(ev, g, event["trigger"]) and _condition(ev, g, event["guard"])
        except NotConstant:
            kept.append(event)
            continue
        decided.append(event)
        if fires:
            level = event["action"].get("level")
            if level in (None, "error"):
                raise PassError(f"assert violated by the parameter values: {_message(g, event)}")
        removed += 1
        if report is not None:
            report.append(_message(g, event))
    # The decision used these parameters' values; a run must not change them.
    from .parameters import _parameter_reads, freeze
    read = set()
    for event in decided:
        for cid in (event["trigger"], event["guard"]):
            for root in _condition_roots(g, cid):
                read |= _parameter_reads(g, root, set())
    changed = freeze(g, read)
    if report is not None:
        report.extend(f"{name}: frozen (a decided assert reads it)" for name in changed)
    _renumber_events(g, kept)
    if builder is None:
        b.finish()
    return removed


def _condition_roots(g: Graph, cid: int) -> list[int]:
    node = g.raw["conditions"][cid]["node"]
    kind = node["kind"]
    if kind == "relation":
        return [g.raw["relations"][node["relation"]]["expression"]]
    if kind == "discrete":
        return [node["expression"]]
    if kind == "not":
        return _condition_roots(g, node["operand"])
    if kind in ("and", "or"):
        return _condition_roots(g, node["lhs"]) + _condition_roots(g, node["rhs"])
    return []


def _guard_relation(g: Graph, event: dict) -> int | None:
    """The relation `r` of a guard `not r` under an always/initial trigger."""
    trigger = g.raw["conditions"][event["trigger"]]["node"]
    guard = g.raw["conditions"][event["guard"]]["node"]
    if trigger["kind"] != "always" or guard["kind"] != "not":
        return None
    inner = g.raw["conditions"][guard["operand"]]["node"]
    if inner["kind"] != "relation":
        return None
    return g.raw["relations"][inner["relation"]]["expression"]


def _operands(g: Graph, lhs: int, rhs: int) -> str:
    names = sorted(g.raw["variables"][v]["name"] for v in g.reads(lhs) | g.reads(rhs))
    return ", ".join(names) or "constant"


def monitor_runtime_asserts(model, builder=None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.monitor-runtime-asserts")
    g = Graph(b)
    real = g.type_like("real", [])
    taken = {v["name"] for v in g.raw["variables"]}
    kept, monitors = [], []
    for event in g.raw.get("events", []):
        if event["action"].get("kind") != "assert":
            kept.append(event)
            continue
        relation = _guard_relation(g, event)
        node = g.node(relation) if relation is not None else None
        if node is None or node["kind"] != "binary" or node["op"] not in _FLIP \
                or g.dims_of(g.vtype(node["lhs"])) or g.dims_of(g.vtype(node["rhs"])):
            raise PassError("runtime assert is not a single scalar relation, "
                            f"so it has no margin to monitor: {_message(g, event)}")
        first, bound = _FLIP[node["op"]]
        lhs, rhs = (node["lhs"], node["rhs"]) if first == "lhs" else (node["rhs"], node["lhs"])
        margin = g.add({"kind": "binary", "op": "subtract", "lhs": lhs, "rhs": rhs}, real)
        k = 0
        while f"{MONITOR_PREFIX}{k}" in taken:
            k += 1
        name = f"{MONITOR_PREFIX}{k}"
        taken.add(name)
        var = b.add_variable(name, "algebraic", scalar_type="real", causality="output",
                             start=g.literal(0.0, real))
        g.raw["variables"][var]["description"] = f"margin of assert: {_message(g, event)}"
        b.add_equation(b.sub(b.ref(var), margin))
        b.add_trace_point(var, f"assert {bound}: {_message(g, event)} [{name}: {_operands(g, lhs, rhs)}]")
        monitors.append(name)
    _renumber_events(g, kept)
    if builder is None:
        b.finish()
    return len(monitors)


def gc_event_tables(model, builder=None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.gc-event-tables")
    raw = b.raw
    owners = ("events", "discrete_definitions", "discrete_real_equations", "clocks",
              "clock_ownerships", "time_events")
    if any(raw.get(table) for table in owners):
        if builder is None:
            b.finish()
        return 0
    dropped = sum(len(raw.get(t) or []) for t in ("conditions", "relations", "roots"))
    for table in ("conditions", "relations", "roots"):
        raw[table] = []
    if builder is None:
        b.finish()
    return dropped


def check_monitors(rows: list[dict], *, tolerance: float = 0.0) -> list[str]:
    """Violations in a trace: `rows` maps trace labels to values per sample."""
    found = []
    for row in rows:
        for label, value in row.items():
            if not label.startswith("assert "):
                continue
            strict = label.startswith("assert > 0")
            if value < -tolerance or (strict and value <= 0 and tolerance == 0):
                found.append(f"t={row.get('time')}: {label} (margin {value})")
    return found

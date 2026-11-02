"""inline-functions: replace calls to straight-line Modelica functions by
their result expression.

A lowered Modelica body is SSA: `assignment`, `assignment_group` (whose
`expressions` already hold each value's branch join) and `assertion`, plus
bounded `for` folds. Inlining is symbolic execution: bind the parameters to
the argument expressions, substitute each value's definition as it is made,
and read the requested output at the end. Each substituted expression is
constant-folded, so a call with literal arguments sheds its dead branches.

Refused, and left as a call (reported, never guessed):
- a body with a `for` fold, or an external / elided body;
- a value read before it is defined;
- an assertion that does not fold to `true` at that call site -- dropping it
  would delete a check, and a function cannot raise an event here.
"""
from __future__ import annotations

from ._graph import Graph
from .fold import fold


class _Refused(Exception):
    pass


def _inline_call(g: Graph, call_id: int, refusals: list) -> int | None:
    node = g.node(call_id)
    function = g.raw["functions"][node["function"]]
    body = function.get("body", {})
    if body.get("kind") != "modelica":
        refusals.append((function["name"], f"{body.get('kind')} body"))
        return None
    args = node["arguments"]
    if len(args) != len(function["parameters"]):
        refusals.append((function["name"], "argument count"))
        return None
    env: dict[int, int] = {}
    fid = function["id"]

    def leaf(_nid, n):
        if n["kind"] == "function_value" and n["function"] == fid:
            if n["value"] not in env:
                raise _Refused(f"value {n['value']} read before definition")
            return env[n["value"]]
        if n["kind"] == "coordinate":
            c = n["coordinate"]
            if c.get("kind") == "function_parameter" and c.get("function") == fid:
                return args[c["ordinal"]]
        return None

    def subst(expr: int) -> int:
        return fold(g, g.rebuild(expr, leaf=leaf))

    try:
        for statement in body["statements"]:
            kind = statement["kind"]
            if kind == "assignment":
                env[statement["value"]] = subst(statement["expression"])
            elif kind == "assignment_group":
                values = [subst(e) for e in statement["expressions"]]
                env.update(zip(statement["values"], values))
            elif kind == "assertion":
                if g.literal_value(subst(statement["condition"])) is not True:
                    raise _Refused("assertion not provable at this call site")
            else:
                raise _Refused(f"{kind} statement")
    except _Refused as why:
        refusals.append((function["name"], str(why)))
        return None
    outputs = [k for k, v in enumerate(function.get("values", []))
               if v.get("role") == "output"]
    ordinal = outputs[node.get("output", 0)]
    if ordinal not in env:
        refusals.append((function["name"], "output never assigned"))
        return None
    return env[ordinal]


def inline_functions(model, builder=None, report: list | None = None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.inline-functions")
    g = Graph(b)
    refusals: list = []
    count = [0]
    memo: dict = {}

    def post(nid: int) -> int:
        if g.node(nid)["kind"] != "call":
            return nid
        result = _inline_call(g, nid, refusals)
        if result is None:
            return nid
        count[0] += 1
        # The result may itself contain calls the body made.
        return g.rebuild(result, post=post, memo=memo)

    for table in ("equations", "initial_equations"):
        for index, row in enumerate(g.raw.get(table, [])):
            new = g.rebuild(row["residual"], post=post, memo=memo)
            if new != row["residual"]:
                b.rewrite_equation(index, new, initial=table == "initial_equations")
    for table in ("equation_families", "initial_equation_families"):
        for family in g.raw.get(table) or []:
            bodies = [g.rebuild(x, post=post, memo=memo) for x in family["bodies"]]
            if bodies != family["bodies"]:
                family["bodies"] = bodies
                family["reads"] = sorted(set().union(*(g.reads(x) for x in bodies)))
    if report is not None:
        report.extend(sorted(set(refusals)))
    if builder is None:
        b.finish()
    return count[0]

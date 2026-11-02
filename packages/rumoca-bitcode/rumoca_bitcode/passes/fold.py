"""fold-constants: evaluate what literals alone decide.

Only literals are folded, never parameters: a parameter is tunable and its
value belongs to the run, not to this artifact. What folds:

- arithmetic, comparison and logic over literal operands;
- `and`/`or` with a literal side, `not` of a literal;
- a conditional whose branch conditions are literal (dead arms dropped);
- `x[k]` with literal `k` of an array literal, an `array_update` or a
  constant comprehension -- this is what turns an inlined
  `q := zeros(4); q[1] := ...` into the plain `{e1, e2, e3, e4}`;
- `array_update` of an array literal at a literal index.
"""
from __future__ import annotations

from ._graph import Graph, _ARITH, _COMPARE


def _literal_index(g: Graph, sub: dict):
    if sub.get("kind") != "index":
        return None
    value = g.literal_value(sub["expression"])
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _comprehension_as_array(g: Graph, nid: int) -> int | None:
    """`{body for i in 1:n}` with a body that does not read `i`, as an array."""
    node = g.node(nid)
    domain = g.raw["domains"][node["domain"]]
    if len(domain["extents"]) != 1:
        return None
    body = node["body"]
    if g.uses(body, lambda n: n["kind"] == "coordinate"
              and n["coordinate"].get("kind") == "binder"):
        return None
    return g.add({"kind": "array", "elements": [body] * domain["extents"][0],
                  "empty_type": None}, g.vtype(nid))


def _as_array(g: Graph, nid: int) -> int | None:
    kind = g.node(nid)["kind"]
    if kind == "array":
        return nid
    if kind == "comprehension":
        return _comprehension_as_array(g, nid)
    return None


def fold_node(g: Graph, nid: int) -> int:
    """Simplify one node whose operands are already folded."""
    node = g.node(nid)
    kind = node["kind"]
    t = g.vtype(nid)
    if kind == "unary":
        x = g.literal_value(node["operand"])
        if x is None:
            return nid
        if node["op"] == "negate" and not isinstance(x, bool):
            return g.literal(-x, t)
        if node["op"] == "not" and isinstance(x, bool):
            return g.literal(not x, t)
        if node["op"] == "plus":
            return node["operand"]
        return nid
    if kind == "binary":
        op = node["op"]
        a, b = g.literal_value(node["lhs"]), g.literal_value(node["rhs"])
        if op in ("and", "or"):
            short = op == "or"
            for side, other in ((a, node["rhs"]), (b, node["lhs"])):
                if isinstance(side, bool):
                    return g.literal(short, t) if side == short else other
            return nid
        if a is None or b is None or isinstance(a, str) or isinstance(b, str):
            return nid
        try:
            if op in _COMPARE:
                return g.literal(_COMPARE[op](a, b), t)
            if op in _ARITH:
                if op in ("divide", "elementwise_divide") and b == 0:
                    return nid
                return g.literal(_ARITH[op](a, b), t)
        except (OverflowError, ValueError, ZeroDivisionError):
            return nid
        return nid
    if kind == "conditional":
        kept = []
        for branch in node["branches"]:
            c = g.literal_value(branch["condition"])
            if c is False:
                continue
            if c is True and not kept:
                return branch["value"]
            kept.append(branch)
            if c is True:
                break
        if not kept:
            return node["fallback"]
        if len(kept) == len(node["branches"]):
            return nid
        fallback = node["fallback"]
        if g.literal_value(kept[-1]["condition"]) is True:
            fallback = kept.pop()["value"]
        if not kept:
            return fallback
        return g.add({"kind": "conditional", "branches": kept, "fallback": fallback}, t)
    if kind == "index":
        return _fold_index(g, nid, node)
    if kind == "array_update":
        subs = node["subscripts"]
        if len(subs) != 1:
            return nid
        k = _literal_index(g, subs[0])
        base = _as_array(g, node["base"])
        if k is None or base is None:
            return nid
        elements = list(g.node(base)["elements"])
        if not 1 <= k <= len(elements):
            return nid
        elements[k - 1] = node["value"]
        return g.add({"kind": "array", "elements": elements, "empty_type": None}, t)
    return nid


def _fold_index(g: Graph, nid: int, node: dict) -> int:
    subs = node["subscripts"]
    k = _literal_index(g, subs[0]) if subs else None
    if k is None:
        return nid
    base_id = node["base"]
    base = g.node(base_id)
    picked = None
    if base["kind"] == "array" and 1 <= k <= len(base["elements"]):
        picked = base["elements"][k - 1]
    elif base["kind"] == "array_update" and len(base["subscripts"]) == 1:
        j = _literal_index(g, base["subscripts"][0])
        if j is None:
            return nid
        picked = base["value"] if j == k else None
        if picked is None:
            rest = g.add({"kind": "index", "base": base["base"], "subscripts": [subs[0]]},
                         g.type_like(g.scalar_of(g.vtype(base_id)),
                                     g.dims_of(g.vtype(base_id))[1:]))
            picked = fold_node(g, rest)
    elif base["kind"] == "comprehension":
        array = _comprehension_as_array(g, base_id)
        if array is not None:
            picked = g.node(array)["elements"][k - 1]
    if picked is None:
        return nid
    if len(subs) == 1:
        return picked
    rest = g.add({"kind": "index", "base": picked, "subscripts": subs[1:]}, g.vtype(nid))
    return fold_node(g, rest)


def fold(g: Graph, root: int, memo: dict | None = None) -> int:
    return g.rebuild(root, post=lambda nid: fold_node(g, nid), memo=memo)


def fold_constants(model, builder=None) -> int:
    """Fold every equation residual and family body; return rewrites."""
    from ._graph import Graph
    b = builder or model.builder("rumoca_bitcode.passes.fold-constants")
    g = Graph(b)
    memo: dict = {}
    rewrites = 0
    for table in ("equations", "initial_equations"):
        for index, row in enumerate(g.raw.get(table, [])):
            new = fold(g, row["residual"], memo)
            if new != row["residual"]:
                b.rewrite_equation(index, new, initial=table == "initial_equations")
                rewrites += 1
    for table in ("equation_families", "initial_equation_families"):
        for family in g.raw.get(table) or []:
            bodies = [fold(g, body, memo) for body in family["bodies"]]
            if bodies != family["bodies"]:
                family["bodies"] = bodies
                family["reads"] = sorted(set().union(*(g.reads(x) for x in bodies)))
                rewrites += 1
    if builder is None:
        b.finish()
    return rewrites

"""Expression-graph utilities shared by the SDK passes.

The arena is append-only and topologically ordered: a node's operands are
strictly earlier. Every rewrite here therefore *appends* the rebuilt node and
returns its id; the old node stays until nothing references it. Nothing is
edited in place except where a pass says so explicitly.

`children` / `with_children` know every operand position of every node kind,
including subscripts, which `builder.operands_of` deliberately does not walk.
"""
from __future__ import annotations

import math
import sys
from typing import Callable

sys.setrecursionlimit(max(sys.getrecursionlimit(), 20000))


class PassError(RuntimeError):
    """A pass refused to rewrite: the artifact would no longer mean the same."""


_FIELDS = {
    "string_conversion": ("value",),
    "unary": ("operand",),
    "binary": ("lhs", "rhs"),
    "field": ("base",),
    "range": ("start", "step", "stop"),
    "comprehension": ("body",),
    "index": ("base",),
    "array_update": ("base", "value"),
    "conditional": ("fallback",),
}
_LISTS = {
    "builtin": ("arguments",),
    "array": ("elements",),
    "record": ("fields",),
    "call": ("arguments",),
}


def children(node: dict) -> list[int]:
    """Every expression id `node` reads, subscripts included."""
    kind = node.get("kind", "")
    found: list[int] = []
    for name in _FIELDS.get(kind, ()):
        value = node.get(name)
        if isinstance(value, int):
            found.append(value)
    for name in _LISTS.get(kind, ()):
        found.extend(v for v in node.get(name) or () if isinstance(v, int))
    for branch in node.get("branches") or ():
        found.extend((branch["condition"], branch["value"]))
    for sub in node.get("subscripts") or ():
        if isinstance(sub.get("expression"), int):
            found.append(sub["expression"])
    if kind == "string_conversion":
        found.extend(v for k, v in node["format"].items()
                     if k != "kind" and isinstance(v, int))
    return found


def with_children(node: dict, remap: Callable[[int], int]) -> dict:
    """A copy of `node` with every operand passed through `remap`."""
    out = dict(node)
    kind = node.get("kind", "")
    for name in _FIELDS.get(kind, ()):
        if isinstance(out.get(name), int):
            out[name] = remap(out[name])
    for name in _LISTS.get(kind, ()):
        if isinstance(out.get(name), list):
            out[name] = [remap(v) if isinstance(v, int) else v for v in out[name]]
    if node.get("branches"):
        out["branches"] = [{"condition": remap(b["condition"]), "value": remap(b["value"])}
                           for b in node["branches"]]
    if node.get("subscripts"):
        out["subscripts"] = [dict(s, expression=remap(s["expression"]))
                             if isinstance(s.get("expression"), int) else dict(s)
                             for s in node["subscripts"]]
    if kind == "string_conversion":
        out["format"] = {k: remap(v) if k != "kind" and isinstance(v, int) else v
                         for k, v in node["format"].items()}
    return out


class Graph:
    """A builder plus the rewriting primitives every pass needs."""

    def __init__(self, builder):
        self.b = builder
        self.raw = builder.raw
        self.E = self.raw["expressions"]
        self.T = self.raw["types"]

    # ── types ────────────────────────────────────────────────────────────────

    def scalar_of(self, type_id: int) -> str:
        return self.T[type_id].get("scalar", "real")

    def dims_of(self, type_id: int) -> list[int]:
        return list(self.T[type_id].get("dimensions") or [])

    def type_like(self, scalar: str, dims: list[int]) -> int:
        """An existing type with this scalar and shape, else a new one."""
        for entry in self.T:
            if entry.get("scalar") == scalar and list(entry.get("dimensions") or []) == dims \
                    and "record" not in entry and "enumeration" not in entry:
                return entry["id"]
        return self.b.add_type(scalar, dims or None)

    # ── nodes ────────────────────────────────────────────────────────────────

    def node(self, nid: int) -> dict:
        return self.E[nid]["node"]

    def vtype(self, nid: int) -> int:
        return self.E[nid]["value_type"]

    def add(self, node: dict, value_type: int) -> int:
        return self.b.add_expression(node, value_type)

    def literal(self, value, value_type: int) -> int:
        scalar = self.scalar_of(value_type)
        if scalar == "boolean":
            payload = {"kind": "boolean", "value": bool(value)}
        elif scalar == "integer":
            payload = {"kind": "integer", "value": int(value)}
        elif scalar == "string":
            payload = {"kind": "string", "value": str(value)}
        else:
            payload = {"kind": "real", "value": float(value)}
        return self.add({"kind": "literal", "value": payload}, value_type)

    def literal_value(self, nid: int):
        """The Python value of a literal node, else None."""
        node = self.node(nid)
        if node["kind"] != "literal":
            return None
        return node["value"].get("value")

    # ── rewriting ────────────────────────────────────────────────────────────

    def rebuild(self, root: int, leaf: Callable[[int, dict], int | None] | None = None,
                post: Callable[[int], int] | None = None, memo: dict | None = None) -> int:
        """Rebuild the tree at `root`.

        `leaf(id, node)` may replace a node outright (its operands are then not
        visited). Otherwise operands are rebuilt first, the node is re-appended
        only if one changed, and `post(id)` may simplify the result.
        """
        memo = {} if memo is None else memo

        def walk(nid: int) -> int:
            if nid in memo:
                return memo[nid]
            node = self.node(nid)
            replaced = leaf(nid, node) if leaf else None
            if replaced is not None:
                memo[nid] = replaced
                return replaced
            ops = children(node)
            mapped = {op: walk(op) for op in ops}
            out = nid
            owner = node.get("owner") if node["kind"] == "call" else None
            # A call owns its result tuple; a projection of another output
            # names that owner, which must move with it.
            new_owner = None if owner in (None, nid) else walk(owner)
            if any(mapped[op] != op for op in ops) or (new_owner not in (None, owner)):
                rebuilt = with_children(node, lambda v: mapped.get(v, v))
                if owner is not None:
                    rebuilt["owner"] = len(self.E) if owner == nid else new_owner
                out = self.add(rebuilt, self.vtype(nid))
            if post is not None:
                out = post(out)
            memo[nid] = out
            return out

        return walk(root)

    def reads(self, root: int) -> set[int]:
        """Variable ids any coordinate under `root` names."""
        seen, found, stack = set(), set(), [root]
        while stack:
            nid = stack.pop()
            if nid in seen:
                continue
            seen.add(nid)
            node = self.node(nid)
            if node["kind"] == "coordinate" and "variable" in node["coordinate"]:
                found.add(node["coordinate"]["variable"])
            stack.extend(children(node))
        return found

    def uses(self, root: int, test: Callable[[dict], bool]) -> bool:
        seen, stack = set(), [root]
        while stack:
            nid = stack.pop()
            if nid in seen:
                continue
            seen.add(nid)
            node = self.node(nid)
            if test(node):
                return True
            stack.extend(children(node))
        return False


# ── evaluation ───────────────────────────────────────────────────────────────

class NotConstant(Exception):
    """The expression reads something only known at run time."""


_ARITH = {
    "add": lambda a, b: a + b, "subtract": lambda a, b: a - b,
    "multiply": lambda a, b: a * b, "divide": lambda a, b: a / b,
    "power": lambda a, b: a ** b,
    "elementwise_add": lambda a, b: a + b, "elementwise_subtract": lambda a, b: a - b,
    "elementwise_multiply": lambda a, b: a * b, "elementwise_divide": lambda a, b: a / b,
    "elementwise_power": lambda a, b: a ** b,
}
_COMPARE = {
    "equal": lambda a, b: a == b, "not_equal": lambda a, b: a != b,
    "less": lambda a, b: a < b, "less_equal": lambda a, b: a <= b,
    "greater": lambda a, b: a > b, "greater_equal": lambda a, b: a >= b,
}
_UNARY_MATH = {
    "sqrt": math.sqrt, "sin": math.sin, "cos": math.cos, "tan": math.tan,
    "asin": math.asin, "acos": math.acos, "atan": math.atan, "exp": math.exp,
    "log": math.log, "log10": math.log10, "sinh": math.sinh, "cosh": math.cosh,
    "tanh": math.tanh,
}


def _is_list(x) -> bool:
    return isinstance(x, list)


def _map2(f, a, b):
    if _is_list(a) and _is_list(b):
        if len(a) != len(b):
            raise NotConstant("shape mismatch")
        return [_map2(f, x, y) for x, y in zip(a, b)]
    if _is_list(a):
        return [_map2(f, x, b) for x in a]
    if _is_list(b):
        return [_map2(f, a, y) for y in b]
    return f(a, b)


def _map1(f, a):
    return [_map1(f, x) for x in a] if _is_list(a) else f(a)


def _flat(a):
    if _is_list(a):
        for x in a:
            yield from _flat(x)
    else:
        yield a


def _matmul(a, b):
    a_mat, b_mat = _is_list(a) and _is_list(a[0]), _is_list(b) and _is_list(b[0])
    if a_mat and b_mat:
        return [[sum(a[i][k] * b[k][j] for k in range(len(b))) for j in range(len(b[0]))]
                for i in range(len(a))]
    if a_mat:
        return [sum(r[k] * b[k] for k in range(len(b))) for r in a]
    if b_mat:
        return [sum(a[k] * b[k][j] for k in range(len(a))) for j in range(len(b[0]))]
    return sum(x * y for x, y in zip(a, b))


class Evaluator:
    """Evaluate expressions whose leaves are literals, parameters and constants.

    A parameter's value is its binding (else its start), evaluated in turn;
    a cycle or a run-time read raises `NotConstant`.
    """

    def __init__(self, graph: Graph, overrides: dict[int, object] | None = None):
        self.g = graph
        self.vars = graph.raw["variables"]
        self.cache: dict[int, object] = {}
        self.var_cache: dict[int, object] = dict(overrides or {})
        self.active: set[int] = set()
        self.binders: dict[tuple[int, int], int] = {}

    def variable(self, vid: int):
        if vid in self.var_cache:
            return self.var_cache[vid]
        v = self.vars[vid]
        if v.get("role") not in ("parameter", "constant"):
            raise NotConstant(v["name"])
        source = v.get("binding", v.get("start"))
        if source is None or vid in self.active:
            raise NotConstant(v["name"])
        self.active.add(vid)
        try:
            value = self.value(source)
        finally:
            self.active.discard(vid)
        self.var_cache[vid] = value
        return value

    def value(self, nid: int):
        if nid in self.cache:
            return self.cache[nid]
        out = self._value(nid)
        self.cache[nid] = out
        return out

    def _value(self, nid: int):
        node = self.g.node(nid)
        kind = node["kind"]
        if kind == "literal":
            return node["value"].get("value")
        if kind == "coordinate":
            c = node["coordinate"]
            if c["kind"] in ("parameter", "constant"):
                return self.variable(c["variable"])
            if c["kind"] == "binder" and (c["domain"], c["ordinal"]) in self.binders:
                return self.binders[(c["domain"], c["ordinal"])]
            raise NotConstant(c["kind"])
        if kind == "unary":
            x = self.value(node["operand"])
            op = node["op"]
            if op == "negate":
                return _map1(lambda v: -v, x)
            if op == "not":
                return _map1(lambda v: not v, x)
            return x
        if kind == "binary":
            op = node["op"]
            if op == "and":
                return bool(self.value(node["lhs"])) and bool(self.value(node["rhs"]))
            if op == "or":
                return bool(self.value(node["lhs"])) or bool(self.value(node["rhs"]))
            a, b = self.value(node["lhs"]), self.value(node["rhs"])
            if op == "multiply" and _is_list(a) and _is_list(b):
                return _matmul(a, b)
            if op in _COMPARE:
                return _COMPARE[op](a, b)
            if op in _ARITH:
                try:
                    return _map2(_ARITH[op], a, b)
                except (ZeroDivisionError, OverflowError, ValueError) as error:
                    raise NotConstant(str(error)) from error
            raise NotConstant(op)
        if kind == "array":
            return [self.value(e) for e in node["elements"]]
        if kind == "conditional":
            for branch in node["branches"]:
                if self.value(branch["condition"]):
                    return self.value(branch["value"])
            return self.value(node["fallback"])
        if kind == "index":
            base = self.value(node["base"])
            for sub in node["subscripts"]:
                if sub.get("kind") != "index":
                    raise NotConstant("slice")
                base = base[int(self.value(sub["expression"])) - 1]
            return base
        if kind == "builtin":
            return self._builtin(node)
        raise NotConstant(kind)

    def _builtin(self, node):
        name = node["name"]
        args = [self.value(a) for a in node["arguments"]]
        if name in ("noEvent", "smooth") and args:
            return args[-1]
        if name == "abs":
            return _map1(abs, args[0])
        if name in ("min", "max"):
            pick = min if name == "min" else max
            if len(args) == 1:
                return pick(_flat(args[0]))
            return pick(args[0], args[1])
        if name == "sum":
            return sum(_flat(args[0]))
        if name == "transpose":
            return [list(r) for r in zip(*args[0])]
        if name == "zeros":
            return _filled(0.0, [int(a) for a in args])
        if name == "ones":
            return _filled(1.0, [int(a) for a in args])
        if name == "identity":
            n = int(args[0])
            return [[1.0 if i == j else 0.0 for j in range(n)] for i in range(n)]
        if name == "atan2":
            return math.atan2(args[0], args[1])
        if name in _UNARY_MATH:
            try:
                return _map1(_UNARY_MATH[name], args[0])
            except ValueError as error:
                raise NotConstant(str(error)) from error
        raise NotConstant(name)


def _filled(value, dims):
    if not dims:
        return value
    return [_filled(value, dims[1:]) for _ in range(dims[0])]

"""scalarize: make every variable and every equation scalar.

Execution v1 stores scalar Reals only, so an array variable -- a state
`p_w[3]`, a matrix parameter `inertia[3,3]`, an output `R_wb[3,3]` -- cannot
be lowered as one. This pass splits each into scalar variables named the way
storage names them (`p_w[1]`, `R_wb[2,3]`), rewrites every read of the array
into an array literal of the scalar reads, and then *projects* each array
expression element by element:

- array literals, constant comprehensions, `array_update`, `index`;
- element-wise `+ - .* ./ .^`, scalar-array `* /`, unary minus;
- matrix-vector, vector-matrix and matrix-matrix products;
- `transpose`, `cross`, `diagonal`, `identity`, `zeros`, `ones`,
  element-wise builtins (`abs`, `sqrt`, `noEvent`, ...), conditionals.

Scalar reductions over arrays are expanded too: `a * b` for two vectors is
`a[1]*b[1] + ...`, and `sum`, `min`, `max`, `product` of an array become
chains. A row-major equation family becomes one scalar equation per row;
start, binding, min, max and nominal attributes are projected per element.

Anything the projector does not know raises: a silently wrong element is
worse than a refused artifact. Dead expressions are compacted afterwards, so
nothing in the arena still names a removed array variable.
"""
from __future__ import annotations

import itertools

from ._graph import Graph, PassError, children, with_children
from .fold import fold_node

_ELEMENTWISE = {"add", "subtract", "elementwise_add", "elementwise_subtract",
                "elementwise_multiply", "elementwise_divide", "elementwise_power"}
_ELEMENTWISE_BUILTINS = {"abs", "sign", "sqrt", "sin", "cos", "tan", "asin", "acos",
                         "atan", "exp", "log", "log10", "sinh", "cosh", "tanh",
                         "noEvent", "smooth", "floor", "ceil", "atan2"}
_REFUSE_TABLES = ("discrete_real_equations", "initial_discrete_values", "discrete_definitions",
                  "model_event_transactions", "previous_values", "terminals",
                  "structured_roots", "delays", "clocks", "clock_ownerships", "time_events")


def _label(index: tuple[int, ...]) -> str:
    return "[" + ",".join(map(str, index)) + "]"


def _cells(dims: list[int]):
    return list(itertools.product(*(range(1, d + 1) for d in dims)))


class Scalarizer:
    def __init__(self, builder):
        self.b = builder
        self.g = Graph(builder)
        self.memo: dict = {}
        self.proj_memo: dict = {}
        self.fallbacks = 0

    # ── types ────────────────────────────────────────────────────────────────

    def dims(self, nid: int) -> list[int]:
        return self.g.dims_of(self.g.vtype(nid))

    def scalar_type(self, nid: int) -> int:
        return self.g.type_like(self.g.scalar_of(self.g.vtype(nid)), [])

    def lit(self, value, like: int) -> int:
        return self.g.literal(value, self.scalar_type(like))

    def binary(self, op: str, a: int, b: int, t: int) -> int:
        return self.simplify(self.g.add({"kind": "binary", "op": op, "lhs": a, "rhs": b}, t))

    def total(self, terms: list[int], t: int) -> int:
        acc = terms[0]
        for term in terms[1:]:
            acc = self.binary("add", acc, term, t)
        return acc

    def simplify(self, nid: int) -> int:
        return fold_node(self.g, nid)

    # ── projection: element `idx` of array expression `nid` ──────────────────

    def project(self, nid: int, idx: tuple[int, ...]) -> int:
        if not idx:
            return self.reduce(nid)
        key = (nid, idx)
        if key not in self.proj_memo:
            self.proj_memo[key] = self._project(nid, idx)
        return self.proj_memo[key]

    def _project(self, nid: int, idx: tuple[int, ...]) -> int:
        g = self.g
        node = g.node(nid)
        kind = node["kind"]
        dims = self.dims(nid)
        if len(idx) > len(dims):
            raise PassError(f"projection {idx} deeper than shape {dims} (node {nid})")
        scalar_t = self.g.type_like(self.g.scalar_of(self.g.vtype(nid)), dims[len(idx):])
        if kind == "array":
            return self.project(node["elements"][idx[0] - 1], idx[1:])
        if kind == "comprehension":
            domain = g.raw["domains"][node["domain"]]
            if len(domain["binders"]) != 1:
                raise PassError("multi-binder comprehension")
            lower = domain["binders"][0]["lower"]
            step = domain["binders"][0].get("step", 1)
            value = lower + (idx[0] - 1) * step
            integer = g.type_like("integer", [])
            literal = g.literal(value, integer)
            did = node["domain"]

            def leaf(_n, n):
                c = n.get("coordinate") if n["kind"] == "coordinate" else None
                if c and c.get("kind") == "binder" and c.get("domain") == did:
                    return literal
                return None

            body = g.rebuild(node["body"], leaf=leaf)
            return self.project(body, idx[1:])
        if kind == "unary":
            inner = self.project(node["operand"], idx)
            return self.simplify(g.add(dict(node, operand=inner), scalar_t))
        if kind == "binary":
            return self._project_binary(node, idx, scalar_t)
        if kind == "conditional":
            branches = [{"condition": self.reduce(br["condition"]),
                         "value": self.project(br["value"], idx)} for br in node["branches"]]
            fallback = self.project(node["fallback"], idx)
            return self.simplify(g.add({"kind": "conditional", "branches": branches,
                                        "fallback": fallback}, scalar_t))
        if kind == "index":
            subs = node["subscripts"]
            head = []
            for sub in subs:
                if sub.get("kind") == "whole":
                    head.append(None)
                elif sub.get("kind") == "index":
                    k = g.literal_value(self.reduce(sub["expression"]))
                    if not isinstance(k, int) or isinstance(k, bool):
                        raise PassError("non-literal subscript on an array being scalarized")
                    head.append(k)
                else:
                    raise PassError("slice subscript")
            rest = list(idx)
            full = tuple(k if k is not None else rest.pop(0) for k in head) + tuple(rest)
            return self.project(node["base"], full)
        if kind == "array_update":
            k = g.literal_value(self.reduce(node["subscripts"][0]["expression"]))
            if len(node["subscripts"]) != 1 or not isinstance(k, int):
                raise PassError("array_update with a non-literal subscript")
            if idx[0] == k:
                return self.project(node["value"], idx[1:])
            return self.project(node["base"], idx)
        if kind == "builtin":
            return self._project_builtin(node, idx, scalar_t)
        if kind == "literal" and not dims:
            return nid
        raise PassError(f"cannot project a {kind} node (id {nid}) to element {_label(idx)}")

    def _project_binary(self, node: dict, idx, scalar_t) -> int:
        op = node["op"]
        a, b = node["lhs"], node["rhs"]
        da, db = self.dims(a), self.dims(b)
        if op in _ELEMENTWISE or (op in ("multiply", "divide") and (not da or not db)) \
                or (op == "power" and not db and not da):
            pa = self.project(a, idx) if da else self.reduce(a)
            pb = self.project(b, idx) if db else self.reduce(b)
            return self.binary(op, pa, pb, scalar_t)
        if op == "multiply":
            if len(da) == 2 and len(db) == 1:
                i, rest = idx[0], idx[1:]
                terms = [self.binary("multiply", self.project(a, (i, k)),
                                     self.project(b, (k,)), scalar_t) for k in range(1, db[0] + 1)]
                return self.project(self.total(terms, scalar_t), rest)
            if len(da) == 1 and len(db) == 2:
                j, rest = idx[0], idx[1:]
                terms = [self.binary("multiply", self.project(a, (k,)),
                                     self.project(b, (k, j)), scalar_t) for k in range(1, da[0] + 1)]
                return self.project(self.total(terms, scalar_t), rest)
            if len(da) == 2 and len(db) == 2:
                i, j = idx[0], idx[1]
                terms = [self.binary("multiply", self.project(a, (i, k)),
                                     self.project(b, (k, j)), scalar_t) for k in range(1, da[1] + 1)]
                return self.total(terms, scalar_t)
        raise PassError(f"cannot project {op} of shapes {da} and {db}")

    def _project_builtin(self, node: dict, idx, scalar_t) -> int:
        g = self.g
        name, args = node["name"], node["arguments"]
        if name in _ELEMENTWISE_BUILTINS:
            projected = [self.project(x, idx) if self.dims(x) else self.reduce(x) for x in args]
            return self.simplify(g.add({"kind": "builtin", "name": name,
                                        "arguments": projected}, scalar_t))
        if name in ("min", "max") and len(args) == 2:
            projected = [self.project(x, idx) if self.dims(x) else self.reduce(x) for x in args]
            return self.simplify(g.add({"kind": "builtin", "name": name,
                                        "arguments": projected}, scalar_t))
        if name == "transpose":
            return self.project(args[0], (idx[1], idx[0]) + tuple(idx[2:]))
        if name == "cross":
            i = idx[0]
            j, k = {1: (2, 3), 2: (3, 1), 3: (1, 2)}[i]
            a, b = args
            return self.binary("subtract",
                               self.binary("multiply", self.project(a, (j,)),
                                           self.project(b, (k,)), scalar_t),
                               self.binary("multiply", self.project(a, (k,)),
                                           self.project(b, (j,)), scalar_t), scalar_t)
        if name in ("zeros", "ones", "fill"):
            if name == "fill":
                return self.reduce(args[0])
            return g.literal(0.0 if name == "zeros" else 1.0, scalar_t)
        if name == "identity":
            return g.literal(1.0 if idx[0] == idx[1] else 0.0, scalar_t)
        if name == "diagonal":
            if idx[0] != idx[1]:
                return g.literal(0.0, scalar_t)
            return self.project(args[0], (idx[0],))
        raise PassError(f"cannot project builtin {name}")

    # ── reduction: a scalar node, with array operands expanded ───────────────

    def reduce(self, nid: int) -> int:
        """Rebuild scalar `nid` so no array value is left inside it."""
        if self.dims(nid):
            raise PassError(f"node {nid} is an array where a scalar is required")
        key = (nid, ())
        if key in self.proj_memo:
            return self.proj_memo[key]
        g = self.g
        node = g.node(nid)
        kind = node["kind"]
        t = g.vtype(nid)
        out = None
        if kind == "binary" and node["op"] == "multiply":
            da, db = self.dims(node["lhs"]), self.dims(node["rhs"])
            if len(da) == 1 and len(db) == 1:
                terms = [self.binary("multiply", self.project(node["lhs"], (k,)),
                                     self.project(node["rhs"], (k,)), t)
                         for k in range(1, da[0] + 1)]
                out = self.total(terms, t)
        elif kind == "builtin" and node["name"] in ("sum", "product", "min", "max") \
                and len(node["arguments"]) == 1 and self.dims(node["arguments"][0]):
            arg = node["arguments"][0]
            cells = [self.project(arg, c) for c in _cells(self.dims(arg))]
            name = node["name"]
            if name in ("sum", "product"):
                op = "add" if name == "sum" else "multiply"
                out = cells[0]
                for c in cells[1:]:
                    out = self.binary(op, out, c, t)
            else:
                out = cells[0]
                for c in cells[1:]:
                    out = self.simplify(g.add({"kind": "builtin", "name": name,
                                               "arguments": [out, c]}, t))
        elif kind == "index":
            subs = node["subscripts"]
            if all(s.get("kind") == "index" for s in subs):
                ks = [g.literal_value(self.reduce(s["expression"])) for s in subs]
                if all(isinstance(k, int) and not isinstance(k, bool) for k in ks):
                    out = self.project(node["base"], tuple(ks))
        if out is None:
            ops = children(node)
            if any(self.dims(op) for op in ops) and kind not in ("comprehension",):
                if kind == "call" or (kind == "builtin" and node["name"] in ("size", "ndims")):
                    out = nid
                else:
                    raise PassError(f"cannot scalarize {kind} "
                                    f"{node.get('op') or node.get('name') or ''} over arrays "
                                    f"(node {nid})")
            else:
                mapped = {op: self.reduce(op) for op in ops}
                out = nid
                if any(mapped[op] != op for op in ops):
                    out = g.add(with_children(node, lambda v: mapped.get(v, v)), t)
                out = self.simplify(out)
        self.proj_memo[key] = out
        return out


def scalarize(model, builder=None, report: list | None = None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.scalarize")
    s = Scalarizer(b)
    g = s.g
    raw = g.raw
    for table in _REFUSE_TABLES:
        if raw.get(table):
            raise PassError(f"scalarize does not handle {table} yet")
    if any(c.get("members") for c in raw.get("connectors", [])) and \
            any(g.dims_of(raw["variables"][m["variable"]]["value_type"])
                for c in raw.get("connectors", []) for m in c["members"]):
        raise PassError("scalarize does not handle array connector members")

    # 1. The new variable table, and old id -> new id (or nested ids).
    new_vars: list[dict] = []
    mapping: dict[int, object] = {}
    array_ids = set()
    for v in raw["variables"]:
        dims = g.dims_of(v["value_type"])
        if not dims:
            mapping[v["id"]] = len(new_vars)
            new_vars.append(dict(v, id=len(new_vars)))
            continue
        array_ids.add(v["id"])
        scalar_t = g.type_like(g.scalar_of(v["value_type"]), [])
        ids = {}
        for cell in _cells(dims):
            entry = {k: val for k, val in v.items()
                     if k not in ("start", "binding", "min", "max", "nominal")}
            entry.update(id=len(new_vars), name=v["name"] + _label(cell),
                         value_type=scalar_t, scalar_count=1)
            entry["contract"] = dict(v.get("contract") or {})
            entry["contract"].pop("effective_value", None)
            ids[cell] = len(new_vars)
            new_vars.append(entry)
        mapping[v["id"]] = ids
    if not array_ids and not raw.get("equation_families") \
            and not raw.get("initial_equation_families"):
        if builder is None:
            b.finish()
        return 0

    # 2. Reads of variables: scalars renumbered, arrays become array literals.
    def coordinate_leaf(nid, node):
        if node["kind"] != "coordinate" or "variable" not in node["coordinate"]:
            return None
        old = node["coordinate"]["variable"]
        target = mapping[old]
        if isinstance(target, int):
            if target == old:
                return None
            c = dict(node["coordinate"], variable=target)
            return g.add({"kind": "coordinate", "coordinate": c}, g.vtype(nid))
        dims = g.dims_of(g.vtype(nid))

        def build(prefix: tuple[int, ...]) -> int:
            level = len(prefix)
            if level == len(dims):
                c = dict(node["coordinate"], variable=target[prefix])
                return g.add({"kind": "coordinate", "coordinate": c},
                             g.type_like(g.scalar_of(g.vtype(nid)), []))
            elements = [build(prefix + (k,)) for k in range(1, dims[level] + 1)]
            return g.add({"kind": "array", "elements": elements, "empty_type": None},
                         g.type_like(g.scalar_of(g.vtype(nid)), dims[level:]))

        return build(())

    def scalar_root(expr: int) -> int:
        return s.reduce(g.rebuild(expr, leaf=coordinate_leaf, memo=s.memo))

    def array_root(expr: int, cell: tuple[int, ...]) -> int:
        rebuilt = g.rebuild(expr, leaf=coordinate_leaf, memo=s.memo)
        if not g.dims_of(g.vtype(rebuilt)):
            return s.reduce(rebuilt)
        return s.project(rebuilt, cell)

    # 3. Attributes, per element.
    for old in raw["variables"]:
        target = mapping[old["id"]]
        for key in ("start", "binding", "min", "max", "nominal"):
            if old.get(key) is None:
                continue
            if isinstance(target, int):
                new_vars[target][key] = scalar_root(old[key])
            else:
                for cell, vid in target.items():
                    new_vars[vid][key] = array_root(old[key], cell)
        contract = old.get("contract") or {}
        if contract.get("binding_depends_on"):
            deps = []
            for dep in contract["binding_depends_on"]:
                t = mapping[dep]
                deps.extend([t] if isinstance(t, int) else list(t.values()))
            for vid in ([target] if isinstance(target, int) else target.values()):
                new_vars[vid].setdefault("contract", {})["binding_depends_on"] = deps

    # 4. Equations: scalar rows stay rows; families become rows.
    for table, families, initial in (("equations", "equation_families", False),
                                     ("initial_equations", "initial_equation_families", True)):
        rows = []
        for row in raw.get(table, []):
            rows.append(scalar_root(row["residual"]))
        for family in raw.get(families) or []:
            view = family.get("scalar_view", {}).get("kind")
            if view != "row_major_projection":
                raise PassError(f"family {family['id']} has a {view} view; "
                                "run unroll-binder-families first")
            for body in family["bodies"]:
                rebuilt = g.rebuild(body, leaf=coordinate_leaf, memo=s.memo)
                for cell in _cells(g.dims_of(g.vtype(rebuilt))):
                    rows.append(s.project(rebuilt, cell))
        provenance = b._provenance()
        old_rows = raw.get(table, [])
        new_rows = []
        for index, residual in enumerate(rows):
            entry = dict(old_rows[index]) if index < len(old_rows) else {"provenance": provenance}
            entry["id"] = index
            entry["residual"] = residual
            reads, derived, previous = b.reads_of(residual)
            for key, value in (("reads", reads), ("reads_derivative", derived),
                               ("reads_previous", previous)):
                if value:
                    entry[key] = sorted(set(value))
                else:
                    entry.pop(key, None)
            new_rows.append(entry)
        raw[table] = new_rows
        raw[families] = []

    # 5. Relations, conditions, events, trace points.
    for relation in raw.get("relations", []):
        relation["expression"] = scalar_root(relation["expression"])
    for condition in raw.get("conditions", []):
        if condition["node"].get("kind") == "discrete":
            condition["node"]["expression"] = scalar_root(condition["node"]["expression"])
    for event in raw.get("events", []):
        action = event["action"]
        for key in ("message", "level", "value"):
            if isinstance(action.get(key), int):
                action[key] = g.rebuild(action[key], leaf=coordinate_leaf, memo=s.memo)
        if isinstance(action.get("variable"), int):
            if not isinstance(mapping[action["variable"]], int):
                raise PassError("reinit of an array variable")
            action["variable"] = mapping[action["variable"]]
    kept_points, dropped = [], []
    for point in raw.get("trace_points", []):
        target = mapping[point["variable"]]
        if isinstance(target, int):
            kept_points.append(dict(point, id=len(kept_points), variable=target))
        else:
            dropped.append(point["label"])
    raw["trace_points"] = kept_points
    for connector in raw.get("connectors", []):
        for member in connector["members"]:
            member["variable"] = mapping[member["variable"]]
    for cset in raw.get("connection_sets", []):
        cset["potentials"] = [mapping[i] for i in cset["potentials"]]
        for balance in cset.get("balances", []):
            for term in balance["terms"]:
                term["variable"] = mapping[term["variable"]]
    for connection in raw.get("connections", []):
        connection["left"] = mapping[connection["left"]]
        connection["right"] = mapping[connection["right"]]

    raw["variables"] = new_vars
    compact_expressions(raw)
    compact_domains(raw)
    if report is not None:
        report.extend(f"trace point {label} named an array; observe its elements"
                      for label in dropped)
    if builder is None:
        b.finish()
    return len(array_ids)


# ── dead-expression compaction ───────────────────────────────────────────────

def _function_roots(function: dict) -> list[int]:
    found = []

    def statements(body):
        for st in body:
            for key in ("expression", "condition", "message"):
                if isinstance(st.get(key), int):
                    found.append(st[key])
            found.extend(st.get("expressions") or [])
            cond = st.get("conditional")
            if cond:
                found.extend(cond["conditions"])
                for branch in cond["branches"]:
                    found.extend(branch)
                found.extend(cond["fallback"])
            statements(st.get("statements") or [])

    statements((function.get("body") or {}).get("statements") or [])
    for fold in function.get("folds") or []:
        for group in ("parameters", "initial", "update", "output"):
            found.extend(d["expression"] for d in fold.get(group) or [])
    return found


def _remap_function(function: dict, m) -> None:
    def statements(body):
        for st in body:
            for key in ("expression", "condition", "message"):
                if isinstance(st.get(key), int):
                    st[key] = m(st[key])
            if st.get("expressions"):
                st["expressions"] = [m(x) for x in st["expressions"]]
            cond = st.get("conditional")
            if cond:
                cond["conditions"] = [m(x) for x in cond["conditions"]]
                cond["branches"] = [[m(x) for x in br] for br in cond["branches"]]
                cond["fallback"] = [m(x) for x in cond["fallback"]]
            statements(st.get("statements") or [])

    statements((function.get("body") or {}).get("statements") or [])
    for fold in function.get("folds") or []:
        for group in ("parameters", "initial", "update", "output"):
            for d in fold.get(group) or []:
                d["expression"] = m(d["expression"])


def compact_expressions(raw: dict) -> int:
    """Drop expressions nothing reaches; renumber every reference."""
    for table in _REFUSE_TABLES:
        if raw.get(table):
            raise PassError(f"expression compaction does not handle {table} yet")
    E = raw["expressions"]
    roots: list[int] = []
    for v in raw["variables"]:
        roots += [v[k] for k in ("start", "binding", "min", "max", "nominal")
                  if isinstance(v.get(k), int)]
    for table in ("equations", "initial_equations"):
        roots += [row["residual"] for row in raw.get(table, [])]
    for table in ("equation_families", "initial_equation_families"):
        for family in raw.get(table) or []:
            roots += family["bodies"]
    roots += [r["expression"] for r in raw.get("relations", [])]
    roots += [c["node"]["expression"] for c in raw.get("conditions", [])
              if c["node"].get("kind") == "discrete"]
    for event in raw.get("events", []):
        roots += [event["action"][k] for k in ("message", "level", "value")
                  if isinstance(event["action"].get(k), int)]
    for function in raw.get("functions", []):
        roots += _function_roots(function)
    live = [False] * len(E)
    stack = list(roots)
    while stack:
        nid = stack.pop()
        if live[nid]:
            continue
        live[nid] = True
        node = E[nid]["node"]
        stack.extend(children(node))
        if node["kind"] == "call" and isinstance(node.get("owner"), int):
            stack.append(node["owner"])
    remap, kept = {}, []
    for entry in E:
        if live[entry["id"]]:
            remap[entry["id"]] = len(kept)
            kept.append(entry)
    m = remap.__getitem__
    for entry in kept:
        node = with_children(entry["node"], m)
        if node["kind"] == "call" and isinstance(node.get("owner"), int):
            node["owner"] = m(node["owner"])
        entry["node"] = node
        entry["id"] = remap[entry["id"]]
    raw["expressions"] = kept
    for v in raw["variables"]:
        for k in ("start", "binding", "min", "max", "nominal"):
            if isinstance(v.get(k), int):
                v[k] = m(v[k])
    for table in ("equations", "initial_equations"):
        for row in raw.get(table, []):
            row["residual"] = m(row["residual"])
    for table in ("equation_families", "initial_equation_families"):
        for family in raw.get(table) or []:
            family["bodies"] = [m(x) for x in family["bodies"]]
    for r in raw.get("relations", []):
        r["expression"] = m(r["expression"])
    for c in raw.get("conditions", []):
        if c["node"].get("kind") == "discrete":
            c["node"]["expression"] = m(c["node"]["expression"])
    for event in raw.get("events", []):
        for k in ("message", "level", "value"):
            if isinstance(event["action"].get(k), int):
                event["action"][k] = m(event["action"][k])
    for function in raw.get("functions", []):
        _remap_function(function, m)
    return len(E) - len(kept)


def compact_domains(raw: dict) -> int:
    """Drop domains nothing iterates any more; renumber every reference."""
    used = set()
    for table in ("equation_families", "initial_equation_families"):
        used.update(f["domain"] for f in raw.get(table) or [])
    for entry in raw["expressions"]:
        node = entry["node"]
        if node["kind"] == "comprehension":
            used.add(node["domain"])
        elif node["kind"] == "coordinate" and node["coordinate"].get("kind") == "binder":
            used.add(node["coordinate"]["domain"])
    for function in raw.get("functions", []):
        used.update(fold["domain"] for fold in function.get("folds") or [])
    domains = raw.get("domains") or []
    remap = {}
    kept = []
    for domain in domains:
        if domain["id"] in used:
            remap[domain["id"]] = len(kept)
            kept.append(dict(domain, id=len(kept)))
    if len(kept) == len(domains):
        return 0
    raw["domains"] = kept
    for table in ("equation_families", "initial_equation_families"):
        for family in raw.get(table) or []:
            family["domain"] = remap[family["domain"]]
    for entry in raw["expressions"]:
        node = entry["node"]
        if node["kind"] == "comprehension":
            node["domain"] = remap[node["domain"]]
        elif node["kind"] == "coordinate" and node["coordinate"].get("kind") == "binder":
            node["coordinate"]["domain"] = remap[node["coordinate"]["domain"]]
    for function in raw.get("functions", []):
        for fold in function.get("folds") or []:
            fold["domain"] = remap[fold["domain"]]
    return len(domains) - len(kept)

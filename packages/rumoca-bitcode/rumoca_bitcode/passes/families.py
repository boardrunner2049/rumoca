"""unroll-binder-families: expand `for i, j loop ... end for` into scalar rows.

Only families whose scalar view is `binder_substitution` -- each row is the
body with the binders replaced by their values -- are unrolled. That is what
lets a call `f(q, i, j)` inline into `f(q, 1, 2)` and its `if i == 1` arms
fold away. A `row_major_projection` family is an array equation whose rows are
the elements of one array residual; lowering already takes those as they are.
"""
from __future__ import annotations

import itertools

from ._graph import Graph
from .fold import fold


def unroll_binder_families(model, builder=None) -> int:
    b = builder or model.builder("rumoca_bitcode.passes.unroll-binder-families")
    g = Graph(b)
    integer = g.type_like("integer", [])
    rewrites = 0
    for table, initial in (("equation_families", False), ("initial_equation_families", True)):
        families = g.raw.get(table) or []
        kept = []
        for family in families:
            if family.get("scalar_view", {}).get("kind") != "binder_substitution":
                kept.append(family)
                continue
            domain = g.raw["domains"][family["domain"]]
            ranges = [range(bd["lower"], bd["upper"] + 1, bd.get("step", 1))
                      for bd in domain["binders"]]
            for values in itertools.product(*ranges):
                literals = {(family["domain"], k): g.literal(v, integer)
                            for k, v in enumerate(values)}

                def leaf(_nid, node, literals=literals):
                    c = node.get("coordinate") if node["kind"] == "coordinate" else None
                    if c and c.get("kind") == "binder":
                        return literals.get((c["domain"], c["ordinal"]))
                    return None

                for body in family["bodies"]:
                    residual = fold(g, g.rebuild(body, leaf=leaf))
                    b.add_equation(residual, initial=initial)
            rewrites += 1
        for index, family in enumerate(kept):
            family["id"] = index
        g.raw[table] = kept
    if builder is None:
        b.finish()
    return rewrites

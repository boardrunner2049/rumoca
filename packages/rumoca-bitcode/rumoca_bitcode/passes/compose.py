"""Composition operations: what a linker user needs after `Model.link`.

The linker namespaces modules and wires nothing. Wiring a signal-causal
component means *its inputs stop being inputs*: an equation the composer
writes now defines them. `bind_input` makes that change -- role `input` to
`algebraic`, every coordinate that names it relabelled to match, and a start
value (zeros of its shape) for the initial algebraic solve -- and leaves the
defining equation to the caller.

`set_parameter` writes a parameter's binding with the declared scalar type and
shape, so an Integer parameter gets an integer literal and a 3x3 matrix a
nested array. A value that does not fit is an error here rather than an
"expression shape mismatch" from the importer later.
"""
from __future__ import annotations

from ._graph import Graph, PassError


def _variable(g: Graph, name: str) -> dict:
    for v in g.raw["variables"]:
        if v["name"] == name:
            return v
    raise PassError(f"no variable named {name!r}")


def _shape_of(value) -> list[int]:
    if isinstance(value, (list, tuple)):
        inner = {tuple(_shape_of(x)) for x in value}
        if len(inner) > 1:
            raise PassError(f"ragged array value {value!r}")
        return [len(value), *(list(inner.pop()) if inner else [])]
    return []


def constant(g: Graph, value, scalar: str, dims: list[int]) -> int:
    """A literal (or nested array literal) of the given scalar and shape."""
    if not dims:
        if scalar == "integer" and (not float(value).is_integer()):
            raise PassError(f"{value!r} is not an Integer")
        return g.literal(value, g.type_like(scalar, []))
    elements = [constant(g, x, scalar, dims[1:]) for x in value]
    return g.add({"kind": "array", "elements": elements, "empty_type": None},
                 g.type_like(scalar, dims))


def set_parameter(builder, name: str, value) -> int:
    g = Graph(builder)
    v = _variable(g, name)
    if v.get("role") not in ("parameter", "constant"):
        raise PassError(f"{name} is a {v.get('role')}, not a parameter")
    scalar, dims = g.scalar_of(v["value_type"]), g.dims_of(v["value_type"])
    if _shape_of(value) != dims:
        raise PassError(f"{name} has shape {dims}, value has {_shape_of(value)}")
    if scalar not in ("real", "integer", "boolean"):
        raise PassError(f"{name}: {scalar} parameters are not supported")
    expr = constant(g, value, scalar, dims)
    v["binding"] = expr
    v["start"] = expr
    contract = v.setdefault("contract", {})
    if not dims and scalar != "boolean":
        contract["effective_value"] = float(value)
    else:
        contract.pop("effective_value", None)
    return v["id"]


def bind_input(builder, name: str) -> int:
    """Make input `name` an algebraic unknown; return its variable id."""
    g = Graph(builder)
    v = _variable(g, name)
    if v.get("role") != "input":
        raise PassError(f"{name} is a {v.get('role')}, not an input")
    vid = v["id"]
    v["role"], v["causality"] = "algebraic", "local"
    contract = v.setdefault("contract", {})
    contract["variability"] = "continuous"
    for entry in g.E:
        node = entry["node"]
        if node["kind"] == "coordinate":
            c = node["coordinate"]
            if c.get("kind") == "input" and c.get("variable") == vid:
                c["kind"] = "algebraic"
    if v.get("start") is None:
        dims = g.dims_of(v["value_type"])
        zeros = _zeros(dims)
        v["start"] = constant(g, zeros, "real", dims)
    return vid


def _zeros(dims):
    return 0.0 if not dims else [_zeros(dims[1:]) for _ in range(dims[0])]


def element(builder, name: str, *index: int) -> int:
    """`name[index...]` as a scalar expression."""
    g = Graph(builder)
    v = _variable(g, name)
    dims = g.dims_of(v["value_type"])
    if len(index) > len(dims):
        raise PassError(f"{name} has {len(dims)} dimensions")
    integer = g.type_like("integer", [])
    subs = [{"kind": "index", "expression": g.literal(k, integer)} for k in index]
    return g.add({"kind": "index", "base": builder.ref(v["id"]), "subscripts": subs},
                 g.type_like(g.scalar_of(v["value_type"]), dims[len(index):]))

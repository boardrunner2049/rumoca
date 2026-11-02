"""Observation for executable runs.

`observe(model, names)` (equation IR) registers one scalar trace point per
selected variable. A run resolves a trace point by its variable's storage
name, and storage is scalar, so arrays are observed after `scalarize` split
them. Existing scalar trace points (e.g. assert monitors) are logged too.

`log_trace_points(program, model)` (execution IR) is the logging pass: one
CSV sink, one row per publish -- time plus every scalar trace point the
model declares. It runs inside the native process; Python only authors it.
"""
from __future__ import annotations

import re

from ._graph import Graph, PassError

LOGGER_PASS = "rumoca_bitcode.passes.trace-csv"


def observe(model, names: list[str], builder=None) -> dict[str, int]:
    """Register one trace point per scalar variable each name selects.

    `x` selects the scalar `x`, or every element `x[...]` of a scalarized
    array; `x[2]` selects that element. An array that has not been
    scalarized is refused: a run can only observe scalar storage.
    """
    b = builder or model.builder("rumoca_bitcode.passes.observe")
    g = Graph(b)
    existing = {(tp["variable"], tp["label"]): tp["id"] for tp in g.raw.get("trace_points", [])}
    created = {}
    for name in names:
        chosen = [v for v in g.raw["variables"]
                  if v["name"] == name or re.fullmatch(re.escape(name) + r"\[[\d,]+\]", v["name"])]
        if not chosen:
            raise PassError(f"no variable named {name!r}")
        for v in chosen:
            if g.dims_of(v["value_type"]):
                raise PassError(f"{v['name']} is an array; run the scalarize pass first")
            key = (v["id"], v["name"])
            created[v["name"]] = existing.get(key) or b.add_trace_point(v["id"], v["name"])
    if builder is None:
        b.finish()
    return created


def log_trace_points(program, model, *, filename: str = "trace.csv") -> None:
    """Execution pass: write time and every scalar trace point to one CSV."""
    by_id = {v["id"]: v for v in model._document["model"]["variables"]}
    types = model._document["model"]["types"]
    points = [tp for tp in model._document["model"].get("trace_points", [])
              if not types[by_id[tp["variable"]]["value_type"]].get("dimensions")]
    if not points:
        raise PassError("no scalar trace points to log; call observe() first")
    with program.builder(LOGGER_PASS) as b:
        sink = b.declare_csv_sink(
            key="trace-csv", filename=filename,
            columns=[{"name": "time", "ty": "real"},
                     *[{"name": tp["label"], "ty": "real"} for tp in points]],
            metadata={"members": [{"trace_point": tp["id"]} for tp in points]},
        )
        with b.before_return("run_start") as ir:
            ir.emit("csv.open", sink=sink)
        with b.before_return("publish") as ir:
            snapshot = ir.argument("snapshot")
            time = ir.emit("snapshot.time", snapshot=snapshot)
            values = [ir.emit("snapshot.value", snapshot=snapshot, trace_point=tp["id"])
                      for tp in points]
            ir.emit("csv.write_row", sink=sink, values=[time, *values])
        with b.before_return("run_finish") as ir:
            ir.emit("csv.close", sink=sink)

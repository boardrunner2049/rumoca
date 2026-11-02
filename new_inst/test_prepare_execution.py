"""The SDK pass pipeline makes compiled models executable by execution v1.

Each case compiles Modelica to bitcode, runs `rumoca_bitcode.passes`, lowers
with the unchanged `bitcode lower-execution`, runs natively with `bitcode
run`, and compares against `rumoca sim` on the same source.

Run: PYTHONPATH=packages/rumoca-bitcode:new_inst RUMOCA=target/debug/rumoca
     python3 -m unittest discover -s new_inst -p test_prepare_execution.py -v
"""
from __future__ import annotations

import csv
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from rumoca_bitcode import Model, passes
from rumoca_bitcode.execution import lower
from rumoca_bitcode.passes import PassError, bind_input, element, set_parameter

RUMOCA = os.environ.get("RUMOCA", "rumoca")

# Every construct the pipeline rewrites: a call with Integer arguments,
# branches and an assert inside a for-equation; an array state; a matrix
# parameter product, transpose and cross; a numeric Integer parameter; a
# parameter assert and a run-time assert; derived final parameters; outputs.
KITCHEN_SINK = """
package Sink
  function element "Integer-indexed entry, with a guard and a branch"
    input Real q[3];
    input Integer i;
    input Integer j;
    output Real value;
  protected
    Real n;
  algorithm
    assert(i >= 1 and i <= 3 and j >= 1 and j <= 3, "index out of range");
    n := max(sqrt(q[1]*q[1] + q[2]*q[2] + q[3]*q[3]), 1e-9);
    if i == j then
      value := q[i] / n;
    elseif i < j then
      value := 2 * q[j];
    else
      value := -q[i];
    end if;
  end element;

  model Plant
    parameter Real A[3, 3] = {{-1, 0.2, 0}, {0, -2, 0.5}, {0.1, 0, -3}};
    parameter Real k = 0.5;
    parameter Integer sgn = -1;
    parameter Real x0[3] = {1, 2, 3};
    final parameter Real scale = sqrt(k * k + 1);
    final parameter Real B[3, 3] = transpose(A) / scale;
    Real x[3](start = x0, each fixed = true);
    output Real M[3, 3];
    output Real y;
    output Real w[3];
  initial equation
    assert(k > 0, "k must be positive");
  equation
    for i in 1:3 loop
      for j in 1:3 loop
        M[i, j] = element(x, i, j);
      end for;
    end for;
    der(x) = B * x + k * cross(x, {0.0, 0.0, 1.0}) + sgn * 0.01 * M * x;
    w = transpose(A) * x;
    y = x * x + sum(M);
    assert(y > -1e6, "y stays bounded");
  end Plant;
end Sink;
"""


def compile_bitcode(source: Path, model: str, out: Path) -> None:
    subprocess.run([RUMOCA, "compile", str(source), "-m", model, "--emit-bitcode", str(out)],
                   check=True, capture_output=True, text=True)


def simulate(source: Path, model: str, stop: float, out: Path) -> dict[str, float]:
    subprocess.run([RUMOCA, "sim", str(source), "-m", model, "--t-end", str(stop),
                    "--dt", str(stop), "--rtol", "1e-10", "--atol", "1e-12", "-o", str(out)],
                   check=True, capture_output=True, text=True)
    with out.open() as handle:
        rows = list(csv.DictReader(handle))
    return {k: float(v) for k, v in rows[-1].items()}


def run_native(model: Model, names: list[str], stop: float, root: Path) -> dict[str, float]:
    passes.run(model)
    passes.observe(model, names)
    program = lower(model)
    passes.log_trace_points(program, model)
    artifact = root / "executable.rbc"
    program.save(artifact)
    subprocess.run([RUMOCA, "bitcode", "run", str(artifact), "--execution", "require",
                    "--stop", str(stop), "--publish-interval", str(stop),
                    "--rtol", "1e-10", "--atol", "1e-12", "--trace-root", str(root / "traces")],
                   check=True, capture_output=True, text=True)
    with (root / "traces" / "trace.csv").open() as handle:
        rows = list(csv.DictReader(handle))
    return {k: float(v) for k, v in rows[-1].items()}


class PrepareExecution(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="rbc-prepare-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "src" / "Sink.mo"
        self.source.parent.mkdir()
        self.source.write_text(KITCHEN_SINK)
        self.rbc = self.root / "plant.rbc"
        compile_bitcode(self.source, "Sink.Plant", self.rbc)

    def test_unprepared_artifact_is_refused_by_lowering(self):
        with self.assertRaises(ValueError):
            lower(Model.load(self.rbc))

    def test_native_run_matches_rumoca_sim(self):
        reference = simulate(self.source, "Sink.Plant", 1.0, self.root / "ref.csv")
        names = ["x", "y", "w", "M"]
        native = run_native(Model.load(self.rbc), names, 1.0, self.root)
        compared = 0
        for label, value in native.items():
            if label == "time" or label.startswith("assert "):
                continue
            with self.subTest(label=label):
                self.assertAlmostEqual(value, reference[label],
                                       delta=1e-7 * max(1.0, abs(reference[label])))
                compared += 1
        self.assertEqual(compared, 3 + 1 + 3 + 9)
        monitors = [k for k in native if k.startswith("assert ")]
        self.assertEqual(len(monitors), 1, "the run-time assert must be observed")
        self.assertEqual(passes.check_monitors([native]), [])

    def test_pipeline_leaves_only_what_v1_lowers(self):
        model = Model.load(self.rbc)
        passes.run(model)
        raw = model._document["model"]
        self.assertFalse(raw["events"] or raw["relations"] or raw["roots"])
        self.assertFalse(raw["equation_families"])
        self.assertFalse(any(e["node"]["kind"] == "call" for e in raw["expressions"]))
        types = raw["types"]
        for v in raw["variables"]:
            self.assertIn(v["role"], ("parameter", "state", "algebraic"), v["name"])
            self.assertEqual(types[v["value_type"]].get("scalar"), "real", v["name"])
            self.assertFalse(types[v["value_type"]].get("dimensions"), v["name"])

    def test_parameters_a_decision_read_are_frozen(self):
        model = Model.load(self.rbc)
        passes.run(model)
        tunable = {v["name"]: v.get("tunable", True)
                   for v in model._document["model"]["variables"]}
        self.assertFalse(tunable["k"], "read by a decided assert and a pinned parameter")
        self.assertFalse(tunable["A[2,3]"], "read by pinned B")

    def test_violated_parameter_assert_fails_the_pass(self):
        model = Model.load(self.rbc)
        b = model.builder("test.override")
        set_parameter(b, "k", -1.0)
        b.finish()
        with self.assertRaisesRegex(PassError, "k must be positive"):
            passes.run(model)

    def test_set_parameter_checks_type_and_shape(self):
        model = Model.load(self.rbc)
        b = model.builder("test.override")
        with self.assertRaises(PassError):
            set_parameter(b, "A", [1.0, 2.0, 3.0])
        with self.assertRaises(PassError):
            set_parameter(b, "sgn", 0.5)
        set_parameter(b, "sgn", 1)


class ComposeLinkedParts(unittest.TestCase):
    """Two separately compiled parts, linked, wired through a bound input."""

    SOURCE = """
package Parts
  model Source
    output Real u(start = 1, fixed = true);
  equation
    der(u) = -u;
  end Source;
  model Gain
    parameter Integer n = 2;
    input Real v[2];
    output Real z;
  equation
    z = n * (v[1] + v[2]);
  end Gain;
  model Whole
    Source s;
    Gain g;
  equation
    g.v = {s.u, 2 * s.u};
  end Whole;
end Parts;
"""

    def test_bound_inputs_run_like_the_modelica_composition(self):
        with tempfile.TemporaryDirectory(prefix="rbc-compose-test-") as tmp:
            root = Path(tmp)
            source = root / "src" / "Parts.mo"
            source.parent.mkdir()
            source.write_text(self.SOURCE)
            for part in ("Source", "Gain"):
                compile_bitcode(source, f"Parts.{part}", root / f"{part}.rbc")
            model = Model.link({"s": root / "Source.rbc", "g": root / "Gain.rbc"},
                               name="Linked")
            self.assertEqual([v.name for v in model.inputs], ["g.v"])
            b = model.builder("test.wire")
            bind_input(b, "g.v")
            u = b.ref(b.variable("s.u"))
            b.add_equation(b.sub(element(b, "g.v", 1), u))
            b.add_equation(b.sub(element(b, "g.v", 2), b.mul(b.real(2.0), u)))
            b.finish()
            self.assertEqual(model.inputs, [])
            native = run_native(model, ["g.z", "s.u"], 1.0, root)
            reference = simulate(source, "Parts.Whole", 1.0, root / "ref.csv")
            self.assertAlmostEqual(native["g.z"], reference["g.z"], delta=1e-8)
            self.assertAlmostEqual(native["s.u"], reference["s.u"], delta=1e-8)


if __name__ == "__main__":
    unittest.main()

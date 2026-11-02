"""`python -m rumoca_bitcode.passes IN -o OUT [--pass a,b]` -- see the package."""
from __future__ import annotations

import argparse
import sys

from rumoca_bitcode import Model

from . import PASSES, PREPARE_EXECUTION, PassError, run


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="python -m rumoca_bitcode.passes",
                                     description=__doc__)
    parser.add_argument("input", nargs="?")
    parser.add_argument("-o", "--output", required=False)
    parser.add_argument("--pass", dest="passes", action="append", default=[],
                        help="comma-separated; default: " + ",".join(PREPARE_EXECUTION))
    parser.add_argument("--list", action="store_true", help="list passes and exit")
    args = parser.parse_args(argv)
    if args.list:
        for name, (_, line) in PASSES.items():
            print(f"{name:26s} {line}")
        return 0
    if not args.input or not args.output:
        parser.error("IN and -o OUT are required")
    names = [n.strip() for value in args.passes for n in value.split(",") if n.strip()]
    model = Model.load(args.input)
    try:
        run(model, names or None, log=lambda line: print(line, file=sys.stderr))
    except PassError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    model.save(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

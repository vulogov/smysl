#!/usr/bin/env python3
"""How the pure commands scale with store size.

The known-gaps note said `pack` and `salience` both "recompute over the whole store on
every call", blamed PageRank over the full adjacency, and admitted there was no
measurement. Measured, it turned out to be wrong in both directions: `salience` is linear
and fine, and `pack` is *quadratic* — through `improve()` in `smysl-pack/src/solve.rs`,
which scans the whole scope per candidate with a graph walk inside, `IMPROVEMENT_PASSES`
times over.

    python3 scripts/bench-scaling.py                # the default ladder
    python3 scripts/bench-scaling.py 500 1000        # specific sizes

**Needs a release build.** A debug binary is a different program for timing purposes, and
this script refuses rather than reporting numbers nobody should act on.

The generated stores are synthetic but not trivial: each claim grounds on its predecessor
and on one seven back, so the graph has real depth and fan-out rather than being a chain or
a star. Both shapes would understate the cost. The seed is fixed, so two runs on one machine
are comparable; absolute milliseconds are not comparable *between* machines, and the ratio
column is the part that means anything.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "smysl")
SIZES = [250, 500, 1000, 2000, 4000]
REPEATS = 3


def store(path: str, n: int) -> None:
    """A synthetic store with `n` claims over one piece of evidence."""
    out = [
        "@doc smysl/0.1 {\n  id: v/bench\n  intent: incident-brief\n  lang: en\n"
        "  roots: [c/u0]\n}\n",
        '@evidence e/base { status: measured, source: { kind: metric, ref: "m" } }\n'
        "~ A baseline reading for the synthetic store.\n",
    ]
    for i in range(n):
        # Two grounds where possible: a chain alone has no fan-out, and a star has no depth.
        g = ["e/base"] if i == 0 else [f"c/u{i-1}"] + ([f"c/u{i-7}"] if i > 7 else [])
        out.append(
            f'@claim c/u{i} {{ status: inferred, grounds: [{", ".join(g)}] }}\n'
            f"~ Synthetic claim number {i} in the benchmark store.\n"
        )
    # A rebuttal every ten units, so rule R has something to keep with a selection.
    for i in range(0, n, 10):
        if i + 3 < n:
            out.append(f"@rel c/u{i} --causes--> c/u{i+3}\n")
        if i + 5 < n:
            out.append(f"@rel c/u{i+5} --rebuts--> c/u{i}\n")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(out))


def timed(args: list[str]) -> float:
    """Median wall-clock milliseconds over `REPEATS` runs.

    Median rather than mean: one scheduling hiccup should not move the number, and the
    first run of any binary pays a page-in cost the rest do not.
    """
    runs = []
    for _ in range(REPEATS):
        t = time.perf_counter()
        subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        runs.append((time.perf_counter() - t) * 1000)
    runs.sort()
    return runs[len(runs) // 2]


def main() -> int:
    if not os.path.exists(BIN):
        print("no release binary: run `cargo build --release` first", file=sys.stderr)
        print("(a debug build is a different program for timing purposes)", file=sys.stderr)
        return 1

    sizes = [int(a) for a in sys.argv[1:]] or SIZES
    tmp = tempfile.mkdtemp(prefix="smysl-bench-")
    try:
        cmds = {
            "check": lambda p: [BIN, "check", p],
            "salience": lambda p: [BIN, "salience", p],
            "merge": lambda p: [BIN, "merge", p],
            # A budget large enough to admit everything, so the cost measured is the
            # selection machinery rather than the packing decision.
            "pack": lambda p: [BIN, "pack", "--budget", "100000", p],
        }
        names = list(cmds)
        print(f"{'units':>7}  " + "  ".join(f"{n:>10}" for n in names))
        print(f"{'':>7}  " + "  ".join(f"{'ms (x2/x)':>10}" for _ in names))
        prev: dict[str, float] = {}
        for n in sizes:
            p = os.path.join(tmp, f"bench{n}.smy")
            store(p, n)
            cells = []
            for name in names:
                ms = timed(cmds[name](p))
                # Ratio against the previous size: 2 is linear, 4 is quadratic. This is the
                # column worth reading, since absolute milliseconds do not travel between
                # machines.
                ratio = f" ({ms / prev[name]:.1f}x)" if name in prev and prev[name] > 0 else ""
                prev[name] = ms
                cells.append(f"{ms:6.0f}{ratio}".rjust(10))
            print(f"{n:>7}  " + "  ".join(cells))
        print()
        print("ratio 2.0 = linear, 4.0 = quadratic (each row doubles the store)")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

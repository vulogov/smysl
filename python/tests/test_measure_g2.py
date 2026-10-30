"""Dunning's G² from ``fixtures/measure/g2.tsv``, recomputed here from each table alone.

SMYSL-2.6 §5.1 asks for T7's G² to equal "an independent computation in the Python
implementation's test harness". The Rust measure crate (``crates/smysl-measure/tests/divergence.rs``)
writes each 2×2 table with its G², and this file computes G² again from the four cells by the
textbook definition — expectations from the margins, ``2 Σ O ln(O/E)``, ``0 · ln 0 = 0`` — with
Python's ``math.log``, which is the platform's and not the ``libm`` the Rust uses.

So the two values are compared to a relative 1e-12 rather than to the bit: the measure's own
determinism is checked by its goldens, and what this checks is that the formula is the formula.
The fixed-point payload number beside each, ``round_half_even(g · 2²⁰)``, is checked to the unit.
"""

from __future__ import annotations

import math
from pathlib import Path

ROWS = [
    line.split("\t")
    for line in (
        Path(__file__).resolve().parents[2] / "fixtures" / "measure" / "g2.tsv"
    )
    .read_text(encoding="utf-8")
    .splitlines()
    if line and not line.startswith("#")
]


def g2(o11: int, o12: int, o21: int, o22: int) -> float:
    n = o11 + o12 + o21 + o22
    rows = (o11 + o12, o21 + o22)
    cols = (o11 + o21, o12 + o22)
    total = 0.0
    for o, r, c in ((o11, 0, 0), (o12, 0, 1), (o21, 1, 0), (o22, 1, 1)):
        if o:
            expected = rows[r] * cols[c] / n
            total += o * math.log(o / expected)
    return 2.0 * total


def test_the_fixture_has_its_tables():
    assert len(ROWS) >= 8
    assert all(len(r) == 6 for r in ROWS)


def test_g2_is_the_formula():
    for row in ROWS:
        o11, o12, o21, o22 = (int(x) for x in row[:4])
        rust, q20 = float(row[4]), int(row[5])
        mine = g2(o11, o12, o21, o22)
        assert math.isclose(mine, rust, rel_tol=1e-12, abs_tol=1e-12), row
        assert abs(round(mine * 2**20) - q20) <= 1, row


def test_independence_is_zero_and_a_perfect_association_is_40_ln_2():
    assert g2(2, 4, 3, 6) == 0.0
    assert math.isclose(g2(10, 0, 0, 10), 40 * math.log(2), rel_tol=1e-15)

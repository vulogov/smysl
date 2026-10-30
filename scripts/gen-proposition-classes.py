#!/usr/bin/env python3
"""The Python reference for SMYSL-2.3 A-12.4: proposition classes.

# Why this exists

A-12.4 is normative, and the reason is arithmetic rather than taste: two implementations that
disagree about how many classes a store holds disagree about how many *propositions* it holds,
and that is the number every corpus measure in SMYSL-2.6 is divided by. So TX-P2 step 5's exit
is not "the algorithm has tests" — it is two implementations agreeing, and the only way to have
that is for each to check the other's work.

The split, which is the whole method here:

  - `crates/smysl-graph/tests/proposition_classes.rs` writes the **input**
    (`fixtures/proposition/store.cbor`) and never the expectation.
  - This script writes the **expectation** (`fixtures/proposition/classes.json`), from an
    independent reading of A-12.4 over `python/smysl`'s decoder.
  - Each side then compares against the other's file, and `python/tests/test_proposition.py`
    keeps this reading live rather than letting the committed JSON stand in for it.

A single generator producing both halves would be one implementation agreeing with itself, which
is the failure `scripts/verify-spec-tables.py`'s own header records: three readers "agreed"
because all three had read the same fixture.

# What it implements

> Let `E` be the set of live `x.text/same-as` relations: not withdrawn, and with both endpoints
> present. Edges are undirected for this purpose. Let `V` be the units with at least one edge in
> `E`, in ascending uid order. Repeat until every unit of `V` is assigned:
>   1. The smallest unassigned unit **opens** a class.
>   2. Every later unassigned unit, in ascending uid order, **joins** that class if it has an
>      edge in `E` to every current member.
> A class's id is its smallest member uid. Units with no edge in `E` form no class.

Ascending uid order is over the **32 bytes**, not over the base32 text. They agree here because
§2.1's alphabet is ordered — `abcdefghijklmnopqrstuvwxyz234567` is not ASCII-ascending, so the
two orders could differ, and this script sorts the bytes. A reference that sorted the text would
be a second, silently different procedure.

`component` (connected components, with a diameter) and `attested:n` (strict over edges carrying
at least n distinct attesting agents) are tool-level — not normative — and are generated here
too, because the Rust side implements them and two implementations of a tool-level policy
drifting apart is still a defect.

Run with `SMYSL_BLESS=1` to write the file; without it, it checks.
"""

from __future__ import annotations

import json
import os
import sys
from collections import deque
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))

import smysl  # noqa: E402

FIXTURES = ROOT / "fixtures" / "proposition"
SAME_AS = "x.text/same-as"

#: Record codes this script reads. Everything else in the store is preserved and ignored.
UNIT, ATTESTATION, RELATION, WITHDRAWAL = 1, 2, 3, 11


#: A uid's text form (§2.1). Not exported by the port, which names the four *library* prefixes
#: and leaves this one to the caller, so it is written down here rather than guessed at per use.
UID_PREFIX = "b3:"


def uid_of_unit(record) -> bytes:
    """A unit's canonical uid: BLAKE3 over the canonical CBOR of its body (§2.1).

    Re-encoded rather than hashed from the bytes as they arrived, which is the same decision
    `test_library.py` makes about the mid: hashing the input would check the hash, and hashing
    what this implementation's own encoder produces checks the encoder as well. A disagreement
    then says whether the two sides encoded differently or hashed differently.
    """
    return smysl.blake3(smysl.encode_one(record.body))


def relation_uid(body) -> bytes:
    """A relation's rid, which is what a withdrawal and an attestation name (§2.5).

    `relation_id` takes the kind's **name**, whether the record encodes it as a kernel integer
    or as text. Every edge this script looks at is `x.text/same-as`, so it is always the text.
    """
    return smysl.uid.relation_id(body.get(0), body.get(1), body.get(2))


def read(path: Path):
    """The store, as (units, same-as edges, withdrawn rids, attesting agents per rid)."""
    records = smysl.decode_store(path.read_bytes())
    units: set[bytes] = set()
    edges: list[tuple[bytes, bytes, bytes]] = []
    withdrawn: set[bytes] = set()
    agents: dict[bytes, set[str]] = {}

    for r in records:
        if r.code == UNIT:
            units.add(uid_of_unit(r))
        elif r.code == RELATION:
            kind = r.body.get(0)
            if kind != SAME_AS:
                continue
            frm, to = r.body.get(1), r.body.get(2)
            edges.append((relation_uid(r.body), frm, to))
        elif r.code == WITHDRAWAL:
            withdrawn.add(r.body.get(0))
        elif r.code == ATTESTATION:
            agents.setdefault(r.body.get(0), set()).add(r.body.get(1))
    return units, edges, withdrawn, agents


def adjacency(units, edges, withdrawn, agents, needed: int) -> dict[bytes, set[bytes]]:
    """`E`, undirected: live, both endpoints present, and attested enough."""
    out: dict[bytes, set[bytes]] = {}
    for rid, frm, to in edges:
        if frm == to:
            # A unit is the same proposition as itself, which is true and is not a class: A-12.4's
            # vertex set is "units with at least one edge", and a self-edge is not one.
            continue
        if frm not in units or to not in units or rid in withdrawn:
            continue
        if needed and len(agents.get(rid, set())) < needed:
            continue
        out.setdefault(frm, set()).add(to)
        out.setdefault(to, set()).add(frm)
    return out


def strict(edges: dict[bytes, set[bytes]]) -> list[list[bytes]]:
    """A-12.4: the smallest unassigned unit opens a class; later ones join if adjacent to all."""
    vertices = sorted(u for u, ns in edges.items() if ns)
    assigned: set[bytes] = set()
    out: list[list[bytes]] = []
    for seed in vertices:
        if seed in assigned:
            continue
        members = [seed]
        assigned.add(seed)
        for candidate in vertices:
            if candidate <= seed or candidate in assigned:
                continue
            if all(m in edges[candidate] for m in members):
                members.append(candidate)
                assigned.add(candidate)
        out.append(sorted(members))
    return out


def components(edges: dict[bytes, set[bytes]]) -> list[tuple[list[bytes], int]]:
    """Connected components, each with its diameter in edges."""
    seen: set[bytes] = set()
    out = []
    for start in sorted(edges):
        if start in seen or not edges[start]:
            continue
        members: set[bytes] = set()
        queue = deque([start])
        while queue:
            u = queue.popleft()
            if u in members:
                continue
            members.add(u)
            queue.extend(n for n in edges[u] if n not in members)
        seen |= members
        out.append((sorted(members), diameter(edges, members)))
    return out


def diameter(edges: dict[bytes, set[bytes]], members: set[bytes]) -> int:
    worst = 0
    for start in members:
        depth = {start: 0}
        queue = deque([start])
        while queue:
            u = queue.popleft()
            for n in edges[u]:
                if n not in depth:
                    depth[n] = depth[u] + 1
                    queue.append(n)
        worst = max(worst, max(depth.values(), default=0))
    return worst


def text(uid: bytes) -> str:
    return smysl.text_form(UID_PREFIX, uid)


def compute(path: Path) -> dict[str, list[list[str]]]:
    units, edges, withdrawn, agents = read(path)
    plain = adjacency(units, edges, withdrawn, agents, 0)
    attested_two = adjacency(units, edges, withdrawn, agents, 2)
    return {
        "strict": [[text(u) for u in c] for c in strict(plain)],
        # The diameter is not in the file. It is a number the Rust side reports and this one
        # computes, and the two are compared by `test_proposition.py`; what the *fixture* pins is
        # the partition, because that is what A-12.4 is normative about.
        "component": [[text(u) for u in c] for c, _ in components(plain)],
        "attested:2": [[text(u) for u in c] for c in strict(attested_two)],
    }


def main() -> int:
    store = FIXTURES / "store.cbor"
    if not store.is_file():
        print(
            f"{store} is missing; the Rust side writes it:\n"
            "  SMYSL_BLESS=1 cargo test -p smysl-graph --test proposition_classes",
            file=sys.stderr,
        )
        return 1
    want = json.dumps(compute(store), indent=2, sort_keys=True) + "\n"
    out = FIXTURES / "classes.json"
    if os.environ.get("SMYSL_BLESS"):
        out.write_text(want)
        print(f"wrote {out.relative_to(ROOT)}")
        return 0
    if not out.is_file():
        print(f"{out} is missing; SMYSL_BLESS=1 writes it", file=sys.stderr)
        return 1
    have = out.read_text()
    if have != want:
        print(
            "fixtures/proposition/classes.json is not what this reference computes;\n"
            "  SMYSL_BLESS=1 python3 scripts/gen-proposition-classes.py rewrites it, which "
            "changes what the Rust implementation is checked against",
            file=sys.stderr,
        )
        return 1
    classes = json.loads(have)
    print(
        "proposition-classes: "
        + ", ".join(f"{k} {len(v)}" for k, v in sorted(classes.items()))
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

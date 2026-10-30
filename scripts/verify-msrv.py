#!/usr/bin/env python3
"""Check that each crate's declared `rust-version` is the one its dependencies require.

# Why this exists

The workspace declared `rust-version = "1.79"` for eleven releases and nothing tested it. It
was false in every selection by the time anybody looked, and not marginally:

  - the **pure core** cannot be built at 1.79 at all, because `blake3` depends on
    `constant_time_eq` 0.4.2, whose manifest is edition 2024 — a 1.79 Cargo cannot *parse* it,
    let alone compile it. The format's hash function is the thing the claim fell over on.
  - the facade needed 1.86 through `ureq` → `url` → `idna` → `idna_adapter` → `icu_*` 2.2.
  - `--workspace --all-features`, which is what CI builds, needed 1.88 through
    `ratatui` → `instability` and `darling` 0.23.

An untested MSRV is not a promise, it is a decoration. RFC SMYSL-2.4 OQ-40 and 2.8 OQ-66 asked
whether to raise it to 1.90 for `redb` 4.x and whether a 1.79 toolchain could even resolve a
lockfile holding an edition-2024 dependency. The second question answered the first: `redb` was
never the reason, and there is no 1.79 build to preserve.

# What this gate does, and what it does not

It reads `cargo metadata` and, for every workspace member, takes the **maximum declared
`rust-version` over its transitive normal and build dependencies** — what a consumer of that
crate compiles — and compares it against the crate's own declaration. Both directions fail:

  - declaring *below* what the dependencies require is the false claim this gate was written
    for; and
  - declaring *above* it is a floor nobody has a reason for, which rots into the first case
    the moment somebody trusts it.

It needs no second toolchain and no network, so it runs in every CI job's few seconds rather
than in a matrix. What it cannot see is **our own** source using a language feature newer than
any dependency requires. One CI job compiling at the declared base covers that, and `EXCEEDS`
below is where such a crate would be recorded, with the feature that forced it.

Dev-dependencies are deliberately excluded: they are what *we* compile to run the tests, not
what a dependent compiles, and Cargo checks them only when building test targets. `fuzz/` is
excluded from the workspace and so is invisible here; it declares no `rust-version` and needs
nightly regardless, so there is no claim of its to check.

**Platform-gated dependencies are excluded too**, and that one was found by disagreeing with a
compiler. This gate first reported `smysl-embed` as needing 1.87 for `wasip2`, while
`cargo +1.85 check -p smysl-embed --all-features` passed: `wasip2` sits behind
`cfg(target_arch = "wasm32", target_os = "wasi")` and is never built here, so Cargo's own
`rust-version` check never considered it. Cargo's behaviour is the contract, so a dependency
counts only when some dependency edge to it is unconditional. The cost is stated rather than
hidden: a floor declared here is the floor for the targets this project builds, and a
cross-compile to a platform with a higher-MSRV gated dependency can need more. The excluded
edges are printed so the number is never a surprise.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# A crate whose own source needs a newer toolchain than any dependency requires, with the
# reason. Empty today, and an entry here is a claim somebody has to justify: the gate reports
# the dependency-required floor beside it so the gap stays visible.
EXCEEDS: dict[str, tuple[str, str]] = {}

failures: list[str] = []


def ver(s: str) -> tuple[int, ...]:
    """`1.86` and `1.86.0` are the same floor; Cargo accepts both spellings."""
    parts = tuple(int(x) for x in s.split("."))
    return parts + (0,) * (3 - len(parts))


def metadata() -> dict:
    # `--all-features` so the floor is the maximum over every feature combination, which is
    # what a package's single `rust-version` has to be. `--locked` so a stale `Cargo.lock`
    # fails here rather than being quietly updated — the floors are a property of the resolved
    # tree, and resolving a different one would answer a different question.
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"],
        cwd=ROOT, capture_output=True, text=True, check=False,
    )
    if out.returncode != 0:
        print(f"verify-msrv: cargo metadata failed\n{out.stderr}", file=sys.stderr)
        sys.exit(1)
    return json.loads(out.stdout)


def main() -> int:
    m = metadata()
    pkgs = {p["id"]: p for p in m["packages"]}
    nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
    members = set(m["workspace_members"])
    gated: set[tuple[str, str, str]] = set()

    base_m = re.search(r'^rust-version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)
    if not base_m:
        print("verify-msrv: the workspace table declares no `rust-version` to inherit")
        return 1
    base = base_m.group(1)

    def required(root: str) -> tuple[str, str]:
        """The highest `rust-version` among `root`'s transitive normal/build deps, and who.

        Workspace members are walked *through*, not treated as leaves: a crate that depends on
        `smysl-core` inherits whatever `smysl-core`'s dependencies require.
        """
        best, who = "0.0", "nothing"
        seen, stack = {root}, [root]
        while stack:
            cur = stack.pop()
            for d in nodes[cur]["deps"]:
                kinds = [k for k in d["dep_kinds"]
                         if k["kind"] is None or k["kind"] == "build"]
                if not kinds:
                    continue  # a dev-dependency only; see the module docstring
                if all(k["target"] is not None for k in kinds):
                    # Platform-gated, so Cargo does not weigh it here either. Recorded, not
                    # counted — the docstring says why, and why it is a stated cost.
                    rv = pkgs[d["pkg"]].get("rust_version")
                    if rv:
                        gated.add((pkgs[d["pkg"]]["name"], rv, kinds[0]["target"]))
                    continue
                if d["pkg"] in seen:
                    continue
                seen.add(d["pkg"])
                stack.append(d["pkg"])
                if d["pkg"] in members:
                    continue  # walked through, but its own declaration is not a constraint
                rv = pkgs[d["pkg"]].get("rust_version")
                if rv and ver(rv) > ver(best):
                    p = pkgs[d["pkg"]]
                    best, who = rv, f"{p['name']} {p['version']}"
        return best, who

    rows = []
    for mid in sorted(members, key=lambda i: pkgs[i]["name"]):
        p = pkgs[mid]
        name = p["name"]
        declared = p.get("rust_version")
        req, who = required(mid)
        rows.append((name, declared, req, who))

        if declared is None:
            failures.append(
                f"{name} declares no `rust-version`; its dependencies require {req} ({who})"
            )
            continue
        # The floor a crate may honestly declare. Never below what its dependencies need; and
        # the workspace base is always allowed, because every crate here is built alongside the
        # others and a crate needing *less* than the base buys nobody anything — `xtask` has no
        # non-dev dependencies at all and would otherwise be asked to declare 0.0.
        want = max([req, base], key=ver)
        cause = who if ver(req) >= ver(base) else "the workspace base"
        if name in EXCEEDS:
            entry, why = EXCEEDS[name]
            if ver(entry) <= ver(want):
                failures.append(
                    f"{name} is in EXCEEDS at {entry} for {why}, but {want} is already "
                    f"required — delete the entry rather than keep a stale reason"
                )
            else:
                # Attribute the requirement to the entry, not to the dependency it overtook.
                # Naming `constant_time_eq` for a floor that came from EXCEEDS sends the next
                # reader to the wrong file.
                want, cause = entry, f"EXCEEDS: {why}"
        if ver(declared) < ver(want):
            failures.append(
                f"{name} declares {declared} but needs {want} ({cause}) — the claim is "
                f"false, which is the case this gate exists for"
            )
        elif ver(declared) > ver(want):
            failures.append(
                f"{name} declares {declared} but needs only {want} — a floor with no reason "
                f"rots into a false one; lower it, or record in EXCEEDS the language feature "
                f"that forces it"
            )

    width = max(len(r[0]) for r in rows)
    for name, declared, req, who in rows:
        print(f"  {name:<{width}}  declares {declared or '—':<7} deps need {req:<7} {who}")

    # The highest floor in the workspace is what `--workspace` builds at, and therefore what CI
    # needs. Printed rather than asserted: it is a consequence, not a decision.
    top = max(rows, key=lambda r: ver(r[2]))
    print(f"verify-msrv: {len(rows)} crates, workspace floor {top[2]} (set by {top[0]})")

    # Only the gated edges that would *raise* a floor are worth printing. The rest sit below
    # the base and change nothing, and seventeen lines of them buries the one that matters.
    above = sorted((n, rv, tgt) for n, rv, tgt in gated if ver(rv) > ver(base))
    if above:
        print("  platform-gated and therefore excluded, but above the base — a cross-compile")
        print("  to one of these targets needs more than any floor above (see the docstring):")
        for n, rv, tgt in above:
            print(f"    {n} needs {rv} under {tgt}")
    print(f"verify-msrv: {len(gated) - len(above)} further gated dependenc(ies) sit at or "
          f"below the base and bound nothing")

    # The base is what the MSRV CI job pins. If it stops being some crate's actual floor, that
    # job compiles a toolchain nothing claims and proves nothing about any crate.
    at_base = [r[0] for r in rows if r[1] == base]
    if not at_base:
        failures.append(
            f"the workspace base {base} is no crate's floor, so the MSRV job that pins it "
            f"proves nothing about any crate"
        )
    else:
        print(f"verify-msrv: base {base} is the floor of {len(at_base)} crate(s), "
              f"the ones the MSRV job compiles")

    for f in failures:
        print(f"  ✗ {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Tie the format's constants to the document that is supposed to define them.

Every other check in this repository asks whether the implementations agree with each other.
This one asks whether what they agree *on* is written down, and it exists because in 1.2.0 the
answer turned out to be no in four places at once.

# What went wrong, and why nothing saw it

`nodejs/` reached C-Produce by decoding `fixtures/wire/uid/cases.json` to learn four things
§2.2 and §2.1 did not say: the status integers, the `source` sub-map's key layout, the
`kind` enum, and the base32 alphabet. `python/` and `go/` had reached C-Produce through the
same four gaps two cycles earlier. Nothing disagreed, because all three were reading the same
fixture — so three independent implementations "agreed", and the agreement was evidence of
nothing except that they had all consulted the same artifact.

The suite's stated method is that a place a reader had to guess gets a `SPEC:` mark. Two
readers guessed four times and left no mark, and the READINESS gate reported the fixtures
as agreement between independent readings for two releases.

# What this gate does about it

The tables in `SMYSL_FORMAT_SPEC.md` are **parsed**, not quoted. Each implementation's copy is
compared against what the document actually says, in both directions, so that:

  - a constant in an implementation with no counterpart in the spec fails, which is the case
    that was missed four times; and
  - a table in the spec that no implementation carries fails, which is the case where the
    document drifts ahead of the code.

The in-language tests keep their hand-typed tables — they are fast and they belong next to the
code — but this is what ties those copies to the document. Before this existed, all three
implementations read the specification file only to assert it contained the string
"Deterministic CBOR", and the claim that they checked the §2.2 and §3.1 tables "against the
document" described something that did not happen.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = ROOT / "Documentation" / "SMYSL_FORMAT_SPEC.md"

failures: list[str] = []
checks = 0


def fail(msg: str) -> None:
    failures.append(msg)


def check(label: str, got, want, note: str = "") -> None:
    """One comparison, counted. The count is asserted at the end for the usual reason."""
    global checks
    checks += 1
    if got != want:
        extra = f"\n    {note}" if note else ""
        missing = sorted(set(want) - set(got)) if isinstance(want, dict) else None
        spurious = sorted(set(got) - set(want)) if isinstance(want, dict) else None
        detail = ""
        if missing:
            detail += f"\n    in the spec, not in the implementation: {missing}"
        if spurious:
            detail += f"\n    in the implementation, not in the spec: {spurious}"
        fail(f"{label}:\n    spec  {want}\n    code  {got}{detail}{extra}")


# ---------------------------------------------------------------------------
# Reading the specification
# ---------------------------------------------------------------------------

def spec_text() -> str:
    if not SPEC.exists():
        print(f"{SPEC} is missing, and it is the subject of every check here", file=sys.stderr)
        raise SystemExit(2)
    return SPEC.read_text(encoding="utf-8")


def table_after(text: str, anchor: str, key_col: int = 0, val_col: int = 1) -> dict[int, str]:
    """The first markdown table following `anchor`, as {int: str}.

    Rows whose first cell is not an integer are skipped, which is what drops the header and the
    `|---|` separator without having to recognise them. A table that yields nothing is an error
    rather than an empty dict: a silently empty table would make every comparison against it
    pass, which is the exact shape of vacuity this repository keeps finding.
    """
    at = text.find(anchor)
    if at < 0:
        fail(f"the spec has no anchor {anchor!r}; this gate cannot find its table")
        return {}
    out: dict[int, str] = {}
    for line in text[at:].splitlines():
        s = line.strip()
        if not s.startswith("|"):
            if out:  # the table ended
                break
            continue
        cells = [c.strip().strip("`") for c in s.strip("|").split("|")]
        if len(cells) <= max(key_col, val_col):
            continue
        try:
            k = int(cells[key_col])
        except ValueError:
            continue
        out[k] = cells[val_col]
    if not out:
        fail(f"the table after {anchor!r} parsed to nothing; the gate would pass vacuously")
    return out


def sections_that_exist(text: str) -> set[str]:
    return set(re.findall(r"^#{2,3} (\d+(?:\.\d+)?)", text, re.M))


# ---------------------------------------------------------------------------
# Reading the implementations
# ---------------------------------------------------------------------------

def read(rel: str) -> str:
    p = ROOT / rel
    if not p.exists():
        fail(f"{rel} is missing; this gate is checking a file that is not there")
        return ""
    return p.read_text(encoding="utf-8")


def block(text: str, start: str, end: str) -> str:
    """The source between `start` and the first `end` after it.

    Scoping matters more than it looks. `records.py` holds RECORD_NAMES and UNIT_KEYS one after
    the other and their rows are spelled identically, so a regex run over the whole file merges
    the two tables into one and compares the union against each — which passes for the keys they
    happen to share and fails confusingly for the rest. The first run of this gate did exactly
    that, and reported five mismatches that were entirely its own.
    """
    at = text.find(start)
    if at < 0:
        fail(f"no {start!r} to extract; this gate is looking for something that moved")
        return ""
    rest = text[at + len(start):]
    to = rest.find(end)
    if to < 0:
        fail(f"{start!r} is never closed by {end!r}")
        return ""
    return rest[:to]


def pairs(text: str, pattern: str, key: int = 1, val: int = 2, lower: bool = True) -> dict[int, str]:
    """Every match of `pattern` as {int: name}, in source order."""
    out: dict[int, str] = {}
    for m in re.finditer(pattern, text, re.M):
        name = m.group(val)
        out[int(m.group(key))] = name.lower() if lower else name
    return out


def named_ints(text: str, names: list[str], pattern: str) -> dict[int, str]:
    """{int: name} for a fixed list of symbol names, each matched by `pattern % name`."""
    out: dict[int, str] = {}
    for name in names:
        m = re.search(pattern % re.escape(name), text, re.M)
        if m:
            out[int(m.group(1))] = name.lower()
    return out


# The spelling differences between the document's prose names and the identifiers three
# languages actually use. Kept explicit: the integers are the format and are compared exactly,
# the names are local spelling and are compared through this map. An alias added here is a
# decision someone made on purpose, which a fuzzy string match would not be.
ALIASES = {
    "unit core": "unit",
    "pack info": "pack_info",
    "schema declaration": "schema_decl",
    "label binding": "label_binding",
}


def canon(name: str) -> str:
    n = name.strip().lower()
    return ALIASES.get(n, n).replace(" ", "_")


def canon_map(d: dict[int, str]) -> dict[int, str]:
    return {k: canon(v) for k, v in d.items()}


# ---------------------------------------------------------------------------

def main() -> int:
    text = spec_text()

    # -- The four tables of §2.2 and §3.1 ------------------------------------
    unit_keys = canon_map(table_after(text, "| key | field | type | presence |", 0, 1))
    status = canon_map(table_after(text, "| value | status |", 0, 1))
    source_keys = canon_map(
        table_after(text[text.find("`source` (key 7) is a map"):], "| key | field |", 0, 1)
    )
    source_kind = canon_map(table_after(text, "| value | kind |", 0, 1))
    record_codes = canon_map(table_after(text, "| code | record |", 0, 1))

    # The unit-core table carries a `≥9` row for unknown keys, which is a rule rather than a
    # key. Rows below are integers only, so it never parses — asserted so that the gate is not
    # quietly relying on a parser accident.
    check("§2.2 unit core keys are 0..8", sorted(unit_keys), list(range(9)))
    check("§2.2 has six statuses", sorted(status), list(range(6)))
    check("§2.2 source sub-map is 0..2", sorted(source_keys), [0, 1, 2])
    check("§2.2 has five source kinds", sorted(source_kind), list(range(5)))
    check("§3.1 record codes are 1..10", sorted(record_codes), list(range(1, 11)))

    # -- python/ -------------------------------------------------------------
    py_records = read("python/smysl/records.py")
    py_uid = read("python/smysl/uid.py")
    py_cbor = read("python/smysl/cbor.py")
    PY_ROW = r"^\s+(\d+): \"(\w+)\","
    check(
        "python: §2.2 unit core keys",
        canon_map(pairs(block(py_records, "UNIT_KEYS = {", "\n}"), PY_ROW)),
        unit_keys,
    )
    check(
        "python: §3.1 record codes",
        canon_map(pairs(block(py_records, "RECORD_NAMES = {", "\n}"), PY_ROW)),
        record_codes,
    )
    check(
        "python: §2.2 source sub-map keys",
        named_ints(py_uid, ["SOURCE_KIND", "SOURCE_REFERENCE", "SOURCE_CAPTURED"],
                   r"^%s = (\d+)$"),
        {0: "source_kind", 1: "source_reference", 2: "source_captured"},
        "python names the source sub-map's keys SOURCE_*; the spec calls them kind/reference/captured",
    )
    check("python: §3 constraint 9 nesting bound",
          int(re.search(r"^MAX_NESTING = (\d+)", py_cbor, re.M).group(1)), 128)
    check("python: §2.1 digest width",
          int(re.search(r"^UID_LEN = (\d+)", py_uid, re.M).group(1)), 32)

    # -- go/ -----------------------------------------------------------------
    go_records = read("go/records.go")
    go_uid = read("go/uid.go")
    go_cbor = read("go/cbor.go")
    check("go: §2.2 unit core keys",
          named_ints(go_uid,
                     ["keySchema", "keyGist", "keyBody", "keyDetail", "keyDeps", "keyGrounds",
                      "keyStatus", "keySource", "keyPayload"],
                     r"^\t%s\s+= (\d+)$"),
          {k: "key" + v for k, v in unit_keys.items()})
    # Unanchored: gofmt packs several entries onto one line, so a `^\t`-anchored row regex
    # sees only the first of each and reports the rest as missing from the implementation.
    GO_ROW = r"(\d+): \"(\w+)\""
    check("go: §3.1 record codes",
          canon_map(pairs(block(go_records, "RecordNames = map[uint64]string{", "\n}"), GO_ROW)),
          record_codes)
    check("go: §2.2 unit core key names",
          canon_map(pairs(block(go_records, "UnitKeys = map[uint64]string{", "\n}"), GO_ROW)),
          unit_keys)
    check("go: §2.2 statuses",
          {k: v.replace("status", "") for k, v in
           named_ints(go_uid, ["StatusUnfounded", "StatusSpeculative", "StatusInferred",
                               "StatusDerived", "StatusCited", "StatusMeasured"],
                      r"^\t%s\s+Status = (\d+)$").items()},
          status)
    check("go: §2.2 source sub-map keys",
          named_ints(go_uid, ["keySourceKind", "keySourceReference", "keySourceCaptured"],
                     r"^\t%s\s+= (\d+)$"),
          {k: "keysource" + v for k, v in source_keys.items()})
    check("go: §3 constraint 9 nesting bound",
          int(re.search(r"^const MaxNesting = (\d+)", go_cbor, re.M).group(1)), 128)
    check("go: §2.1 digest width",
          int(re.search(r"^const UidLen = (\d+)", go_uid, re.M).group(1)), 32)

    # -- nodejs/ -------------------------------------------------------------
    js_records = read("nodejs/src/records.js")
    js_uid = read("nodejs/src/uid.js")
    js_cbor = read("nodejs/src/cbor.js")
    JS_ROW = r"^\s+\[(\d+), \"(\w+)\"\],"
    JS_ENUM = r"^  (\w+): (\d),$"
    check("nodejs: §2.2 unit core keys",
          canon_map(pairs(block(js_records, "UNIT_KEYS = new Map([", "\n]);"), JS_ROW)),
          unit_keys)
    check("nodejs: §3.1 record codes",
          canon_map(pairs(block(js_records, "RECORD_NAMES = new Map([", "\n]);"), JS_ROW)),
          record_codes)
    check("nodejs: §2.2 statuses",
          {int(m.group(2)): m.group(1)
           for m in re.finditer(JS_ENUM, block(js_uid, "STATUS = Object.freeze({", "});"), re.M)},
          status)
    check("nodejs: §2.2 source kinds",
          {int(m.group(2)): m.group(1)
           for m in re.finditer(JS_ENUM,
                                block(js_uid, "SOURCE_KIND = Object.freeze({", "});"), re.M)},
          source_kind)
    check("nodejs: §3 constraint 9 nesting bound",
          int(re.search(r"MAX_NESTING = (\d+)", js_cbor).group(1)), 128)
    check("nodejs: §2.1 digest width",
          int(re.search(r"^const UID_BYTES = (\d+);", js_uid, re.M).group(1)), 32)

    # -- §2.1's base32, which the spec did not name until 1.2 ----------------
    spec_alphabet = re.search(r"`(abcdefghijklmnopqrstuvwxyz234567)`", text)
    check("§2.1 names a base32 alphabet", bool(spec_alphabet), True)
    if spec_alphabet:
        check("nodejs: §2.1 base32 alphabet",
              re.search(r'^const ALPHABET = "(\w+)";', js_uid, re.M).group(1),
              spec_alphabet.group(1))
        rust_alphabet = re.search(r'const ALPHABET: &\[u8; 32\] = b"(\w+)"',
                                  read("crates/smysl-core/src/ids.rs"))
        check("rust: §2.1 base32 alphabet",
              rust_alphabet.group(1) if rust_alphabet else None, spec_alphabet.group(1))

    # -- The masthead, against the version the reference actually writes -----
    #
    # This one caught a real drift the moment it was written. The masthead read `smysl/0.1`
    # while §8.6 of the same document said `smysl/1.0` arrived in 0.15 and is what new
    # documents declare — so the normative header named the version the writer had stopped
    # defaulting to two releases earlier, and no gate read the masthead at all.
    lib = read("crates/smysl-core/src/lib.rs")
    default = re.search(r'FORMAT_VERSION_DEFAULT: &str = "([^"]+)"', lib)
    masthead = re.search(r"^\*\*Format version:\*\* `([^`]+)`", text, re.M)
    check("the spec's masthead names the version the writer emits",
          masthead.group(1) if masthead else None,
          default.group(1) if default else None,
          "§8.5: the writer emits the version a document arrived as; a new one gets the default")

    supported = re.search(r'FORMAT_VERSIONS_SUPPORTED: &\[&str\] = &\[([^\]]+)\]', lib)
    if supported and masthead:
        listed = re.findall(r'"([^"]+)"', supported.group(1))
        check("every supported format version is mentioned in the spec",
              [v for v in listed if v in text], listed)

    # -- Every §-citation resolves ------------------------------------------
    #
    # `go/uid.go` cited §1.1 twice for the source sub-map, and the spec has no §1.1 — §1 has no
    # subsections at all. A citation to a section that does not exist is worse than none: it
    # reads as a reference to a clause that settles the question.
    exists = sections_that_exist(text)
    check("the spec has the sections this gate assumes", {"2.1", "2.2", "3.1", "7"} <= exists, True)
    for rel in ["python/smysl/uid.py", "python/smysl/cbor.py", "python/smysl/records.py",
                "go/uid.go", "go/cbor.go", "go/records.go",
                "nodejs/src/uid.js", "nodejs/src/cbor.js", "nodejs/src/records.js"]:
        body = read(rel)
        cited = set(re.findall(r"§(\d+(?:\.\d+)?)", body))
        bogus = sorted(c for c in cited if c not in exists)
        check(f"{rel}: every §-citation resolves", bogus, [],
              "the format spec has sections " + " ".join(sorted(exists, key=_num)))

    # -- The gate cannot have quietly stopped checking -----------------------
    #
    # Every assertion above is `check`, and a `check` that never runs is indistinguishable from
    # one that passed. The floor is the count at the time of writing, less a little slack.
    if checks < 30:
        fail(f"only {checks} comparisons ran; this gate has lost most of its coverage")

    print(f"spec-tables: {checks} comparisons, {len(failures)} MISMATCHED")
    for f in failures:
        print(f"  ✗ {f}")
    return 1 if failures else 0


def _num(s: str) -> tuple:
    return tuple(int(p) for p in s.split("."))


if __name__ == "__main__":
    raise SystemExit(main())

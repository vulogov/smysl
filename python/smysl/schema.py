"""FC-9: schema declaration keys 4 and 5, read by a second implementation (SMYSL-2.5 §5.8).

TX-P12 step 1 gives record 8 a field table (key 4) and lens steps (key 5), SMYSL-2.3 A-7 and
Appendix B. Lens *application* is not a conformance obligation (D-8); decoding the structure,
writing key 5 back byte for byte (L9) and the lens-chain hash are. The checks mirror
``crates/smysl-core/src/schema/decode.rs``: a key that does not match its structure is opaque
and reported ``SMY-W430``; an op whose tag, or one of whose keys, this reader does not know is
kept as written and makes its step unusable, ``SMY-W469``.
"""

from __future__ import annotations

from .blake3 import blake3
from .cbor import CborError, decode_one, encode_one

#: The keys each known op may carry. A key outside its set makes the op unknown.
OP_KEYS = {
    "rename-type": {0, 1, 2},
    "rename-relation": {0, 1, 2},
    "rename-key": {0, 1, 2},
    "map-value": {0, 1, 2},
    "split-value": {0, 1, 2, 3, 4},
    "require": {0, 1, 2},
    "default": {0, 1, 2, 3},
    "retire": {0, 1},
}

CHAIN_DOMAIN = b"smysl/lens-chain/1\x00"


class Mismatch(ValueError):
    pass


def _uint(v) -> int:
    if not isinstance(v, int) or isinstance(v, bool) or v < 0 or v > 0xFFFFFFFF:
        raise Mismatch("not a revision")
    return v


def _text(v) -> str:
    if not isinstance(v, str):
        raise Mismatch("not text")
    return v


def _texts(v) -> list:
    if not isinstance(v, list):
        raise Mismatch("not an array")
    return [_text(x) for x in v]


def _text_map(v) -> dict:
    if not isinstance(v, dict) or not all(isinstance(k, str) for k in v):
        raise Mismatch("not a text-keyed map")
    return v


def _cond(v) -> None:
    if len(_text_map(v)) != 1:
        raise Mismatch("a condition has one key")


def _shape(raw: bytes) -> int:
    try:
        value, used = decode_one(raw)
    except CborError as e:
        raise Mismatch(str(e)) from e
    if used != len(raw) or not isinstance(value, dict):
        raise Mismatch("a shape is one map")
    if any(not isinstance(k, int) for k in value) or 0 not in value:
        raise Mismatch("a shape is {0: fields}")
    fields = _text_map(value[0])
    for f in fields.values():
        if not isinstance(f, dict) or any(not isinstance(k, int) for k in f):
            raise Mismatch("a field is an integer-keyed map")
        if 0 not in f:
            raise Mismatch("a field has a type")
        _text(f[0])
        for k, check in ((1, _texts), (2, _uint), (3, _text), (4, _uint), (5, _cond), (6, _texts)):
            if k in f:
                check(f[k])
    return len(fields)


def _op(op) -> tuple[str, bool]:
    """The op's tag, and whether it was read in full (a known op). Raises on a mismatch."""
    if not isinstance(op, dict) or any(not isinstance(k, int) for k in op) or 0 not in op:
        raise Mismatch("an op is a map with a tag")
    tag = _text(op[0])
    allowed = OP_KEYS.get(tag)
    if allowed is None or any(k not in allowed for k in op):
        return tag, False
    need = {
        "rename-type": {1: _text, 2: _text},
        "rename-relation": {1: _text, 2: _text},
        "rename-key": {1: _text, 2: _text},
        "map-value": {1: _text, 2: lambda m: [_text(x) for x in _text_map(m).values()]},
        "split-value": {1: _text, 2: _text, 3: _text, 4: _text_map},
        "require": {1: _text},
        "default": {1: _text, 2: lambda v: v},
        "retire": {1: _text},
    }[tag]
    for k, check in need.items():
        if k not in op:
            raise Mismatch(f"{tag} needs key {k}")
        check(op[k])
    when = {"require": 2, "default": 3}.get(tag)
    if when is not None and when in op:
        _cond(op[when])
    return tag, True


def _steps(lenses) -> list[dict]:
    if not isinstance(lenses, list):
        raise Mismatch("lenses are an array")
    out = []
    for s in lenses:
        if not isinstance(s, dict) or set(s) != {0, 1, 2}:
            raise Mismatch("a step is {0: from, 1: to, 2: ops}")
        lo, hi = _uint(s[0]), _uint(s[1])
        if hi <= lo or not isinstance(s[2], list):
            raise Mismatch("a step goes up and has an op array")
        ops = [_op(o) for o in s[2]]
        out.append(
            {
                "from": lo,
                "to": hi,
                "ops": [t for t, _ in ops],
                "usable": all(k for _, k in ops),
                "bytes": encode_one(s),
            }
        )
    return out


def decode(body: dict) -> dict:
    """One schema declaration's FC-9 reading: fields, steps and the codes decoding reports."""
    codes: list[str] = []
    fields = None
    if 4 in body:
        try:
            if not isinstance(body[4], bytes):
                raise Mismatch("payload_shape is a byte string")
            fields = _shape(body[4])
        except Mismatch:
            fields = "opaque"
            codes.append("SMY-W430")
    steps: list[dict] = []
    opaque = False
    if 5 in body:
        try:
            steps = _steps(body[5])
        except Mismatch:
            steps, opaque = [], True
            codes.append("SMY-W430")
    codes.extend("SMY-W469" for s in steps if not s["usable"])
    return {"fields": fields, "lenses_opaque": opaque, "steps": steps, "codes": codes}


def _head(major: int, n: int) -> bytes:
    if n < 24:
        return bytes([major << 5 | n])
    for extra, size in ((24, 1), (25, 2), (26, 4), (27, 8)):
        if n < 1 << (8 * size):
            return bytes([major << 5 | extra]) + n.to_bytes(size, "big")
    raise ValueError("too long")


def encode_lenses(steps: list[dict]) -> bytes:
    """Key 5's bytes: an array of each step's bytes, as received (L9)."""
    return _head(4, len(steps)) + b"".join(s["bytes"] for s in steps)


def chain_hash(schema_id: str, lo: int, hi: int, steps: list[dict]) -> bytes:
    """Appendix B: BLAKE3 over the domain and ``[id, from, to, [step…]]``."""
    body = (
        _head(4, 4)
        + encode_one(schema_id)
        + encode_one(lo)
        + encode_one(hi)
        + encode_lenses(steps)
    )
    return blake3(CHAIN_DOMAIN + body)

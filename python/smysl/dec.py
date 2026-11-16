"""``dec``, the canonical decimal of RFC SMYSL-3 §5 (C-Tel-Read).

A value is ``m × 10**e``. On the wire it is a CBOR array of two shortest-form integers, and
one value has exactly one encoding: the mantissa carries no trailing zero, zero is ``[0, 0]``,
the mantissa's magnitude is at most ``2**64 - 1`` (the sign rides on the major type, so the
one CBOR integer outside the range, ``-2**64``, is refused) and the exponent is in
``[-400, 400]`` (§5.2). A decoder refuses every other encoding rather than normalising it,
because two encodings of one value would be two uids. That refusal is ``SMY-E602``.

Written from the RFC's text, as the rest of this package is written from the format spec.
Where the text left a choice, a ``SPEC:`` comment says what was chosen.
"""

from __future__ import annotations

from .cbor import CborError, Decoder

M_MAX = (1 << 64) - 1
E_MIN, E_MAX = -400, 400


class DecError(ValueError):
    """Not a ``dec``. ``kind`` is ``syntax``, ``not-a-value``, ``out-of-range`` or ``E602``."""

    def __init__(self, kind: str, why: str) -> None:
        super().__init__(f"{kind}: {why}")
        self.kind = kind


def normalise(m: int, e: int) -> tuple[int, int]:
    """Trailing zeros into the exponent, zero to ``(0, 0)``, then the range check (§5.2)."""
    if m == 0:
        return (0, 0)
    while m % 10 == 0:
        m //= 10
        e += 1
    if abs(m) > M_MAX or not E_MIN <= e <= E_MAX:
        raise DecError("out-of-range", f"{m}e{e}")
    return (m, e)


def parse_token(token: str) -> tuple[int, int]:
    """A decimal token read exactly as written, never through binary64 (§5.3).

    SPEC: §5.3 names the sources (a JSON number, a Prometheus sample value, a Zabbix value
    string) but not one grammar. Read here as: an optional sign, digits with at most one point
    and at least one digit, and an optional exponent. That admits every JSON number and the
    ``.5`` and ``5.`` a Prometheus or Go reader accepts. ``NaN`` and the infinities are refused
    as not values (§5.2 rule 5) rather than as syntax.
    """
    low = token.lstrip("+-").lower()
    if low in ("nan", "inf", "infinity"):
        raise DecError("not-a-value", token)
    i, n = 0, len(token)
    negative = False
    if i < n and token[i] in "+-":
        negative = token[i] == "-"
        i += 1
    int_part, frac_part, seen_point = "", "", False
    while i < n and (token[i].isdigit() and token[i].isascii() or token[i] == "."):
        if token[i] == ".":
            if seen_point:
                raise DecError("syntax", token)
            seen_point = True
        elif seen_point:
            frac_part += token[i]
        else:
            int_part += token[i]
        i += 1
    if not (int_part or frac_part):
        raise DecError("syntax", token)
    exp = 0
    if i < n:
        if token[i] not in "eE":
            raise DecError("syntax", token)
        rest = token[i + 1 :]
        digits = rest[1:] if rest[:1] in ("+", "-") else rest
        if not digits or not (digits.isascii() and digits.isdigit()):
            raise DecError("syntax", token)
        exp = int(digits) * (-1 if rest[:1] == "-" else 1)
    digits = (int_part + frac_part).lstrip("0")
    trimmed = digits.rstrip("0")
    if not trimmed:
        return (0, 0)
    if len(trimmed) > 20:
        raise DecError("out-of-range", token)
    m = int(trimmed)
    e = exp - len(frac_part) + (len(digits) - len(trimmed))
    return normalise(-m if negative else m, e)


def from_f64(x: float) -> tuple[int, int]:
    """A binary64 as the decimal ECMA-262's ``Number::toString`` selects (§5.3).

    ``repr`` gives the shortest string that round-trips and, among those, the closest, which is
    the same choice; ``fixtures/tel/dec/cases.json`` carries what a JavaScript engine gives, and
    the tests hold this to it.
    """
    if x != x or x in (float("inf"), float("-inf")):
        raise DecError("not-a-value", repr(x))
    if x == 0.0:
        return (0, 0)
    return parse_token(repr(x))


def spell(m: int, e: int) -> str:
    """The one text spelling (§5.3): plain decimal, no exponent, no ``+``."""
    digits = str(abs(m))
    if e >= 0:
        out = "0" if m == 0 else digits + "0" * e
    elif len(digits) > -e:
        out = digits[: len(digits) + e] + "." + digits[len(digits) + e :]
    else:
        out = "0." + "0" * (-e - len(digits)) + digits
    return ("-" if m < 0 else "") + out


def _head(major: int, arg: int) -> bytes:
    if arg < 24:
        return bytes([(major << 5) | arg])
    for ai, width in ((24, 1), (25, 2), (26, 4), (27, 8)):
        if arg < 1 << (8 * width):
            return bytes([(major << 5) | ai]) + arg.to_bytes(width, "big")
    raise DecError("out-of-range", str(arg))


def _int(v: int) -> bytes:
    return _head(0, v) if v >= 0 else _head(1, -1 - v)


def to_cbor(m: int, e: int) -> bytes:
    """``[m, e]`` as two shortest-form integers."""
    return b"\x82" + _int(m) + _int(e)


def from_cbor(data: bytes) -> tuple[int, int]:
    """A ``dec`` from the whole of ``data``, refusing anything not canonical (``SMY-E602``)."""
    d = Decoder(data)
    try:
        major, n, _ = d._head()
        if major != 4 or n != 2:
            raise DecError("E602", "not an array of two integers")
        parts = []
        for _ in range(2):
            major, arg, _ = d._head()
            if major == 0:
                parts.append(arg)
            elif major == 1:
                parts.append(-1 - arg)
            else:
                raise DecError("E602", "not an array of two integers")
    except CborError as err:
        raise DecError("E602", str(err)) from err
    if d.i != len(data):
        raise DecError("E602", "bytes after the dec")
    m, e = parts
    if abs(m) > M_MAX:
        raise DecError("E602", "the mantissa is -2**64")
    if not E_MIN <= e <= E_MAX:
        raise DecError("E602", "the exponent is outside [-400, 400]")
    if m == 0 and e != 0:
        raise DecError("E602", "zero is [0, 0]")
    if m != 0 and m % 10 == 0:
        raise DecError("E602", "the mantissa has a trailing zero")
    return (m, e)

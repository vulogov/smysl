"""Deterministic CBOR, per §3 of ``SMYSL_FORMAT_SPEC.md``.

This is a *strict* reader. Every rule in §3 is a rejection, not a normalisation: a decoder
that quietly accepted a non-shortest integer would let two byte strings mean one record, and
a uid would stop naming exactly one thing. That is the whole reason the constraints exist, so
tolerating a violation here would defeat the point of implementing them.

Written from the specification rather than from the reference implementation, which is the
only way it can serve as evidence that the specification is sufficient. Where the spec was
unclear, the ambiguity is recorded in a comment beginning ``SPEC:`` rather than resolved by
reading the Rust — those comments are the deliverable as much as the code is.
"""

from __future__ import annotations

import struct
from typing import Any

MAX_NESTING = 128  # §3, constraint 8


class CborError(ValueError):
    """Input that the format forbids. Corresponds to `SMY-E080`/`SMY-E004`."""


class Decoder:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.i = 0

    # -- primitives ----------------------------------------------------------

    def _byte(self) -> int:
        if self.i >= len(self.data):
            raise CborError("input ended inside a value")
        b = self.data[self.i]
        self.i += 1
        return b

    def _take(self, n: int) -> bytes:
        if n < 0 or self.i + n > len(self.data):
            raise CborError("length runs past the end of the input")
        out = self.data[self.i : self.i + n]
        self.i += n
        return out

    def _head(self) -> tuple[int, int, int]:
        """Return ``(major, argument, extra)``, enforcing shortest form (§3, constraint 2).

        SPEC: constraint 2 says "no value encoded in more bytes than it needs" without saying
        it applies to *integers and lengths* rather than to everything a head can carry. It
        cannot apply to major type 7, where the trailing bytes are a float's payload and
        0x3F800000 is 1.0 rather than an over-long encoding of 1 065 353 216. Enforcing it
        there rejected every fixture, which is how the ambiguity was found. Worth a sentence
        in the spec.
        """
        b = self._byte()
        major, extra = b >> 5, b & 0x1F
        floaty = major == 7

        if extra < 24:
            return major, extra, extra
        if extra == 24:
            v = self._byte()
            if not floaty and v < 24:
                raise CborError(f"{v} encoded in two bytes; shortest form is one")
            return major, v, extra
        if extra == 25:
            (v,) = struct.unpack(">H", self._take(2))
            if not floaty and v <= 0xFF:
                raise CborError(f"{v} encoded in three bytes; a shorter form exists")
            return major, v, extra
        if extra == 26:
            (v,) = struct.unpack(">I", self._take(4))
            if not floaty and v <= 0xFFFF:
                raise CborError(f"{v} encoded in five bytes; a shorter form exists")
            return major, v, extra
        if extra == 27:
            (v,) = struct.unpack(">Q", self._take(8))
            if not floaty and v <= 0xFFFFFFFF:
                raise CborError(f"{v} encoded in nine bytes; a shorter form exists")
            return major, v, extra
        if extra == 31:
            # §3, constraint 3. Indefinite length is how the same value gets two encodings.
            raise CborError("indefinite-length item; definite lengths only")
        raise CborError(f"reserved additional-information value {extra}")

    # -- values --------------------------------------------------------------

    def value(self, depth: int = 0) -> Any:
        if depth > MAX_NESTING:
            raise CborError(f"nesting deeper than {MAX_NESTING}")
        major, arg, extra = self._head()

        if major == 0:
            return arg
        if major == 1:
            return -1 - arg
        if major == 2:
            return self._take(arg)
        if major == 3:
            raw = self._take(arg)
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError as e:
                raise CborError(f"text is not valid UTF-8: {e}") from e
            # §3, constraint 6. Checked rather than applied: normalising here would accept
            # two encodings of one string, which is the thing being forbidden.
            import unicodedata

            if unicodedata.normalize("NFC", text) != text:
                raise CborError("text is not NFC-normalised")
            return text
        if major == 4:
            return [self.value(depth + 1) for _ in range(arg)]
        if major == 5:
            return self._map(arg, depth)
        if major == 7:
            return self._simple(arg, extra)
        # SPEC: §3 lists no use for major type 6 (tags) and does not say whether a decoder
        # must reject one. Rejecting, on the grounds that constraint 1 makes the kernel's
        # shape exhaustive and an unknown tag could carry a second encoding of a value.
        raise CborError(f"major type {major} is not part of this format")

    def _map(self, n: int, depth: int) -> dict[Any, Any]:
        out: dict[Any, Any] = {}
        prev_key_bytes: bytes | None = None
        for _ in range(n):
            start = self.i
            key = self.value(depth + 1)
            key_bytes = self.data[start : self.i]
            # §3, constraint 4. Compared as encoded bytes, which orders integers and text
            # consistently without needing to know which a given map uses.
            if prev_key_bytes is not None and key_bytes <= prev_key_bytes:
                raise CborError("map keys are not in ascending order, or are duplicated")
            prev_key_bytes = key_bytes
            out[key] = self.value(depth + 1)
        return out

    def _simple(self, arg: int, extra: int) -> Any:
        if extra == 20:
            return False
        if extra == 21:
            return True
        if extra == 22:
            # §3, constraint 5. An absent optional must be omitted, so a `null` on the wire
            # is a violation rather than a value.
            raise CborError("null is forbidden; omit the key instead")
        if extra == 26:
            (v,) = struct.unpack(">f", struct.pack(">I", arg))
            # §3, constraint 7. `round(v * 1024) / 1024` must be a fixed point.
            if v == v and abs(v) != float("inf"):
                scaled = v * 1024.0
                if scaled != int(scaled):
                    raise CborError(f"float {v} is not a multiple of 1/1024")
            else:
                raise CborError("float is not finite")
            return v
        if extra == 27:
            raise CborError("binary64 float; the format uses binary32")
        raise CborError(f"simple value {extra} is not part of this format")


class Encoder:
    """Emit exactly what §3 requires, so a decoded record re-encodes to its own bytes."""

    def __init__(self) -> None:
        self.out = bytearray()

    def _head(self, major: int, arg: int) -> None:
        if arg < 24:
            self.out.append((major << 5) | arg)
        elif arg <= 0xFF:
            self.out.append((major << 5) | 24)
            self.out.append(arg)
        elif arg <= 0xFFFF:
            self.out.append((major << 5) | 25)
            self.out += struct.pack(">H", arg)
        elif arg <= 0xFFFFFFFF:
            self.out.append((major << 5) | 26)
            self.out += struct.pack(">I", arg)
        else:
            self.out.append((major << 5) | 27)
            self.out += struct.pack(">Q", arg)

    def value(self, v: Any) -> None:
        if v is True:
            self.out.append(0xF5)
        elif v is False:
            self.out.append(0xF4)
        elif isinstance(v, int):
            if v >= 0:
                self._head(0, v)
            else:
                self._head(1, -1 - v)
        elif isinstance(v, float):
            self.out.append(0xFA)
            self.out += struct.pack(">f", v)
        elif isinstance(v, bytes):
            self._head(2, len(v))
            self.out += v
        elif isinstance(v, str):
            raw = v.encode("utf-8")
            self._head(3, len(raw))
            self.out += raw
        elif isinstance(v, list):
            self._head(4, len(v))
            for item in v:
                self.value(item)
        elif isinstance(v, dict):
            self._head(5, len(v))
            # Sorted by encoded key bytes, which is what constraint 4 specifies and is not
            # the same as sorting by the key's Python value once text keys appear.
            for key in sorted(v, key=encode_one):
                self.value(key)
                self.value(v[key])
        else:
            raise CborError(f"cannot encode {type(v).__name__}")


def encode_one(v: Any) -> bytes:
    e = Encoder()
    e.value(v)
    return bytes(e.out)


def decode_one(data: bytes) -> tuple[Any, int]:
    """Decode one value, returning it and how many bytes it used."""
    d = Decoder(data)
    v = d.value()
    return v, d.i

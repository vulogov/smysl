"""Write cases.json: window summaries (RFC SMYSL-3 §9.3) computed with exact rationals.

    python3 -I fixtures/tel/window/gen_cases.py > fixtures/tel/window/cases.json

An independent oracle for `smysl_tel::evidence::summarize`: Python's `Fraction` is exact, and
`float(Fraction)` rounds a rational to the nearest binary64, ties to even, which is §9.3's "rounded
once". `repr(float)` is the shortest round-tripping spelling, the decimal §5.3 asks for.
Seeded, so the file is reproducible."""
import json, random
from fractions import Fraction

M64 = 2**64 - 1

def dec_of_fraction(f):
    """A Fraction that is a finite decimal, as (m, e) normalised, or None if out of range."""
    if f == 0:
        return (0, 0)
    num, den = f.numerator, f.denominator
    # den is 2^a 5^b: scale to a power of ten.
    a = b = 0
    d = den
    while d % 2 == 0:
        d //= 2; a += 1
    while d % 5 == 0:
        d //= 5; b += 1
    assert d == 1
    k = max(a, b)
    num = num * (10**k // den)
    e = -k
    while num % 10 == 0:
        num //= 10; e += 1
    if abs(num) > M64 or not -400 <= e <= 400:
        return None
    return (num, e)

def spell(m, e):
    neg = m < 0
    s = str(abs(m))
    if e >= 0:
        s = s + "0" * e
    else:
        s = s.rjust(-e + 1, "0")
        s = s[:e] + "." + s[e:]
    return ("-" if neg else "") + s

def to_float(f):
    """Nearest binary64, ties to even; None past the largest finite one."""
    try:
        x = float(f)
    except OverflowError:
        return None
    return None if x in (float("inf"), float("-inf")) else x

def float_spell(x):
    if x is None:
        return None
    if x == 0:
        return "0"
    return spell(*dec_of_fraction(Fraction(repr(x))))

def token(rng):
    kind = rng.random()
    if kind < 0.3:
        m, e = rng.randint(-10**6, 10**6), 0
    elif kind < 0.6:
        m, e = rng.randint(-10**9, 10**9), -rng.randint(1, 6)
    elif kind < 0.8:
        m, e = rng.randint(-M64, M64), rng.randint(-30, 30)
    else:
        m, e = rng.choice([1, -1, 3, 7]) * rng.randint(1, 999), rng.choice([-400, -300, 300, 400, -20, 20])
    if m == 0:
        return "0"
    while m % 10 == 0:
        m //= 10; e += 1
    if not -400 <= e <= 400:
        e = max(-400, min(400, e))
    return f"{m}e{e}"

rng = random.Random(9)
cases = []
for n in range(400):
    k = rng.choice([1, 2, 3, 5, 8, 13, 40])
    values = [token(rng) for _ in range(k)]
    fr = [Fraction(int(v.split("e")[0])) * Fraction(10) ** int(v.split("e")[1]) for v in values]
    total = sum(fr)
    integer = all(int(v.split("e")[1]) >= 0 for v in values)
    case = {"values": values}
    if integer:
        d = dec_of_fraction(total)
        case["sum"] = spell(*d) if d else None
    else:
        case["sum"] = float_spell(to_float(total))
    case["mean"] = float_spell(to_float(total / len(fr)))
    cases.append(case)
print(json.dumps({"about": "RFC SMYSL-3 §9.3 summaries by exact rationals; written by gen_cases.py", "cases": cases}, indent=0))

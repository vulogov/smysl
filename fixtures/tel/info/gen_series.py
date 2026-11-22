#!/usr/bin/env python3
"""Write series.tsv for RFC SMYSL-5's info cases (TI-P1, GE-I1).

Six hours at one minute from 1768550400000, eight series, deterministic (an LCG, no library
randomness), so that any implementation can regenerate the same bytes:

- info_noise      gauge      noise in [0, 10), sampled 7 s after each minute; a 20-minute gap
                             and a stale marker at minute 200 + 30 s, resumed at minute 205;
- info_osc        gauge      a 12-minute oscillation, on the minute;
- info_state      gauge      a sticky state, 0, 1 or 2;
- info_requests   counter    a cumulative count with a reset at minute 180;
- info_hist       histogram  a histogram's count, which no statistic reads;
- info_driver     gauge      an AR(1), x = floor(0.8x) + e, e in [-1, 1), in thousandths (TI-P2);
- info_follower   gauge      info_driver three minutes late, plus noise in [-0.2, 0.2], so
                             mi/1 of it against the driver peaks at lag -3 (TI-P2);
- info_shift      gauge      noise in [0, 4) for two hours, then in [0, 8): jsd/1 against the
                             first two hours sees the change (TI-P2).

Columns: key, class, ts_ms, value, flags (1 stale, 2 reset).
"""
import math

FROM = 1_768_550_400_000
MIN = 60_000
N = 360


class Lcg:
    def __init__(self, seed):
        self.x = seed

    def next(self):
        self.x = (6364136223846793005 * self.x + 1442695040888963407) % 2**64
        return self.x >> 33


def milli(m):
    """An integer count of thousandths as a decimal token: -1234 is -1.234."""
    sign = "-" if m < 0 else ""
    return f"{sign}{abs(m) // 1000}.{abs(m) % 1000:03d}"


def main():
    rows = []
    r = Lcg(5)
    for i in range(N):
        if 120 <= i < 140 or 201 <= i < 205:
            continue
        if i == 200:
            rows.append(("info_noise", "gauge", FROM + i * MIN + 30_000, "0", 1))
            continue
        v = r.next() % 1000
        rows.append(("info_noise", "gauge", FROM + i * MIN + 7_000, f"{v // 100}.{v % 100:02d}", 0))
    for i in range(N):
        v = round(5 + 4 * math.sin(2 * math.pi * i / 12), 1)
        rows.append(("info_osc", "gauge", FROM + i * MIN, f"{v:.1f}", 0))
    s, state = Lcg(11), 0
    for i in range(N):
        if s.next() % 10 == 0:
            state = (state + 1 + s.next() % 2) % 3
        rows.append(("info_state", "gauge", FROM + i * MIN, str(state), 0))
    c, total = Lcg(17), 1000
    for i in range(N):
        if i == 180:
            total = c.next() % 20
            rows.append(("info_requests", "counter", FROM + i * MIN, str(total), 2))
            continue
        total += c.next() % 20
        rows.append(("info_requests", "counter", FROM + i * MIN, str(total), 0))
    h, count = Lcg(23), 0
    for i in range(N):
        count += h.next() % 5
        rows.append(("info_hist", "histogram", FROM + i * MIN, str(count), 0))
    d, x, driver = Lcg(29), 0, []
    for i in range(N):
        x = (8 * x) // 10 + (d.next() % 2000 - 1000)
        driver.append(x)
        rows.append(("info_driver", "gauge", FROM + i * MIN, milli(x), 0))
    fo = Lcg(31)
    for i in range(N):
        e = fo.next() % 401 - 200
        y = (driver[i - 3] if i >= 3 else 0) + e
        rows.append(("info_follower", "gauge", FROM + i * MIN, milli(y), 0))
    sh = Lcg(37)
    for i in range(N):
        wide = 4000 if i < 120 else 8000
        rows.append(("info_shift", "gauge", FROM + i * MIN, milli(sh.next() % wide), 0))
    with open("series.tsv", "w") as f:
        f.write("# key\tclass\tts_ms\tvalue\tflags   (written by gen_series.py)\n")
        for row in rows:
            f.write("\t".join(str(x) for x in row) + "\n")


if __name__ == "__main__":
    main()

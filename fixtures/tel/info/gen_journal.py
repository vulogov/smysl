#!/usr/bin/env python3
"""Write journal.json for RFC SMYSL-5's tent/1 cases (TI-P3, GE-I1).

`journalctl -o json` lines for one stream, `cart.service` on host `h1`: six hours from
1768550400000, five events a minute at fixed offsets, deterministic (an LCG, no library
randomness). Four message shapes, each with variable fields Drain masks or merges:

- connection from 10.0.A.B closed after N ms
- GET /api/items/N 200 in N ms
- cache miss key=HEX
- order N placed by user N

From minute 240 one event in five is `upstream db-N timed out after N ms`, so the last two
hours have a fifth template and a different mix: tent/1 over them is higher.
"""

FROM = 1_768_550_400_000
MIN = 60_000
N = 360


class Lcg:
    def __init__(self, seed):
        self.x = seed

    def next(self):
        self.x = (6364136223846793005 * self.x + 1442695040888963407) % 2**64
        return self.x >> 33


def main():
    r = Lcg(43)
    lines = []
    for i in range(N):
        for j in range(5):
            ts_us = (FROM + i * MIN + j * 11_000 + 137) * 1000
            pick = r.next() % 10
            if i >= 240 and j == 4:
                msg = f"upstream db-{r.next() % 4} timed out after {3000 + r.next() % 500} ms"
                prio = "3"
            elif pick < 4:
                msg = f"connection from 10.0.{r.next() % 4}.{r.next() % 250} closed after {r.next() % 90} ms"
                prio = "6"
            elif pick < 7:
                msg = f"GET /api/items/{r.next() % 1000} 200 in {r.next() % 40} ms"
                prio = "6"
            elif pick < 9:
                msg = f"cache miss key={r.next():08x}"
                prio = "7"
            else:
                msg = f"order {r.next() % 100000} placed by user {r.next() % 5000}"
                prio = "6"
            lines.append(
                '{"__REALTIME_TIMESTAMP": "%d", "_HOSTNAME": "h1", "_SYSTEMD_UNIT": "cart.service", '
                '"PRIORITY": "%s", "MESSAGE": "%s"}' % (ts_us, prio, msg)
            )
    with open("journal.json", "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()

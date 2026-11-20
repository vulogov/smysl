# ge_m9

GE-M9 (RFC SMYSL-3 §14), TL-P8's gate: does retention keep the round trip? The RFC's design is
six months of GE-M1-scale data, evicted under rule TL, exported, imported, and merged with a
peer that kept everything. A P1 failure over records, or a substrate resurrection that changes
a unit, blocks TL-P8. GE-M1's cluster is not here, so the data is simulated, and the report says
so.

```
make ge-m9                            # 20 gauges, 182 days, about 90 s and 630 MB, removed afterwards
cargo run --release --features tel --example ge_m9 -- --series 3 --days 10 --keep 4 --dir /tmp/m9
```

| step | checked |
|---|---|
| A, before | P1 and P3 over its records; every window recomputes clean |
| A evicts all but `--keep` days | records unchanged; exactly the evicted days' windows are `SMY-W605`; no `SMY-E604` |
| A exports `--with-substrate`, C imports it | the export verifies; every evicted day is listed; C's records are A's, with A's gaps |
| C merges peer B's export | records unchanged; every window recomputes to the unit it is; C holds B's samples |
| A merges B's export | the same, and no eviction record is left |
| exchange is a set | a second import adds nothing; the other order gives the same records and samples |
| control | a peer with one sample more on an evicted day is caught as one `SMY-E604`, and changes no record |

The records are compared by the rsd of an export of everything, and by their units, relations
and attestations. The samples are compared by the digest of every series' whole window. Exit 0
when every check holds, 1 otherwise.

Arguments: `--dir`, `--series` (20), `--days` (182), `--interval` seconds (60), `--keep` days
(30), `--seed` (9).

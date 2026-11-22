# oko_ledger

A stand-in for OKO's proof of concept (RFC SMYSL-3 §24, TL-P6's exit criterion). OKO is a
separate project and its code is not in this repository, so the criterion itself, that OKO
writes a rotation ledger, is still owed. This runs smysl's side of it end to end, at a rotation's
size, through the same library calls OKO will use.

```
make oko-ledger                       # 1,500 investigations, about 6 minutes, removed afterwards
cargo run --release --features tel --example oko_ledger -- --investigations 200 --dir /tmp/l
```

| OKO decision (§24) | what this does |
|---|---|
| one ledger store per on-call rotation | one library, `<dir>/<rotation>`, and `rotation` on every investigation |
| investigations as views | an `x.tel/investigation` and a View rooted at it (D-T8) |
| humans by handle, model agents by provider and model hash | attestations by `human:<handle>` and `model:ollama/<hash>` |
| full redacted command output kept | probe and action output redacted, then stored as a part |
| confirmation on unsafe commands | `confirmed_by` on a model's action |
| Zabbix demo source | `fixtures/tel/zabbix/trends.json` through `zabbix/1`, as cited windows |

Before the incidents it commits two statistics (RFC SMYSL-5, TI-P4) over GE-I1's driver and
follower (`fixtures/tel/info/`): an `ent/1`, and an `mi/1` lag scan with 50 surrogates. The
first investigation consults them and quotes them: once at their stored six places, which
verifies, and once rounded to two, a planted wrong quote that rule TQ must catch with the rest.

Every ledger operation is committed alone and read back at once, as OKO appends. The run
reports commit latency (and the staging half of it) and read latency over its first and last
tenth, because AC-8 (incremental
derived state) is on OKO's critical path. It ends by reopening the library and running `check`.
Every quote the run fabricated on purpose (`--fabricate N`: one hypothesis in N) must be a
`SMY-W606`, and nothing else may be raised. Exit 0 when that holds.

Arguments: `--dir`, `--rotation` (`sre-eu`), `--investigations` (1500), `--fabricate` (5),
`--seed` (24), and `--verify`, which checks each operation in full as well and asserts that the
kept check (AC-8, RFC SMYSL-3 §17.1) reported the same.

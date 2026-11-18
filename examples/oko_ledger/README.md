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

Every ledger operation is committed alone and read back at once, as OKO appends. The run
reports commit latency (and the staging half of it) and read latency over its first and last
tenth, because AC-8 (incremental
derived state) is on OKO's critical path. It ends by reopening the library and running `check`.
Every quote the run fabricated on purpose (`--fabricate N`: one hypothesis in N) must be a
`SMY-W606`, and nothing else may be raised. Exit 0 when that holds.

Arguments: `--dir`, `--rotation` (`sre-eu`), `--investigations` (1500), `--fabricate` (5),
`--seed` (24).

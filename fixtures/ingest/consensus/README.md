# Consensus extraction fixtures (SMYSL-2.4 §3.4, TX-P5 step 3a)

A window, two scripted extractions of it, and the `identical-span` agreement those two
extractions have. `crates/smysl-ingest/tests/consensus.rs` drives the real ingest path over
them with a scripted provider, so the figure under test is the one the boundary computes and
not one the test restates.

- `window.txt` — the passage both passes are given. One chunk at any provider context this
  repository configures, so the call count is exactly the pass count.
- `agree/pass-1.json`, `agree/pass-2.json` — what the model said, twice. **Identical**, which
  is what temperature 0 measures as: S0's MS-8 found arm L bit-identical across all 145 runs
  at T=0. This is the shape the exit clause names.
- `drift/pass-1.json`, `drift/pass-2.json` — the same window where the second pass found one
  place the first did not, and worded a second differently. Not a model this repository
  observed; a scripted disagreement, so the measurement can be shown to discriminate rather
  than to return 1.0 whatever it is handed.
- `*/agreement.txt` — the figures `smysl_ingest::consensus::Agreement` must report, one
  `key value` per line. `rate` is `agreed / comparable` to three places.

Every unit here carries the window's span, because the fixture runs under a caller-supplied
source (`SourcePolicy::Override`): `identical-span` is a relation between *places*, and a unit
with no span cannot establish it in either direction. Per-unit spans arrive with span
attachment (§6 TX-P5 step 3); until then `spanless` is what a fixture without a source
measures, and `agreement.txt` would state it.

One consequence worth stating, because the test asserts it: in `drift/` the second pass gives
`e/pool` to a unit with a different gist, hence a different uid, so the label binding the first
pass made stands and the second pass's unit is staged **without a name** (`SMY-W054`). That is
the pre-existing cross-chunk rule applied to passes, and it is the right outcome — a name that
moved between two readings of one window names nothing reliably — but it means a disagreeing
pass loses its labels rather than its units.

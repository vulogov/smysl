# The kernel schema, as one file both sides read

`unit.json` is what `smysl_ingest::schema::unit_schema()` emits — Appendix C of the manual,
the schema every structured provider call is built from.

It is a file rather than a constant in each crate because it had been both, and they had
drifted. `smysl-provider` cannot depend on `smysl-ingest` (ingest depends on the provider, so
that is the cycle), and it kept its own copy to test the strict-mode translation against. By
0.14 that copy had 2 of the 13 kernel types, 2 of the 5 statuses, 1 of the 3 conditionals, and
a different `label` pattern — while `openai.rs` documented its tests as running "against the
full Appendix C schema rather than a miniature of it". It was the miniature.

Two tests keep the file honest, one on each side of the boundary:

  * `smysl-ingest` asserts `schema::unit_schema()` still equals this file, so the fixture
    cannot go stale as the schema evolves;
  * `smysl-provider` translates this file for strict mode and asserts every object in the
    result is strict-legal, so the translation is exercised against the real schema rather
    than a reduction of it.

Regenerate with the first of those two after a deliberate schema change — it prints the diff.

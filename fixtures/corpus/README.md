# Corpus F1–F8 (§27.2)

Each fixture ships as surface text (`.smy`) and an expected diagnostic set (`.expected`).
The two are what `tests/conformance_fixtures.rs` discovers; a `.smy` without its sibling
fails the suite rather than being skipped.

Canonical CBOR is *derived*, not shipped. `every_corpus_fixture_round_trips` encodes each
fixture and decodes it back in memory, which tests the same property a committed `.cbor`
would without a second copy to keep in step. (An earlier version of this file promised
`.cbor` files; none were ever written, and the round-trip test is why they were not
missed.)

| ID | Fixture | Exercises | Lands |
|---|---|---|---|
| F1 | Incident report, `default` granularity | the baseline path; rules M, R, V1 | **landed** |
| F2 | Research trace, `fine` granularity | deep grounds chains; rule M cascades; corroboration | **landed** |
| F3 | Narrative text, `coarse` granularity | the design's most likely falsifier (GE-2) | **landed** |
| F4 | Q&A session | `answers` relations; the `qa` thread schema | **landed** |
| F5 | Dataset analysis with tables | `data`, `artifact-ref`, extension payloads | **landed** |
| F6 | Adversarial store | laundering attempts across a chain; rule M | **landed** |
| F7 | Mixed-granularity merge | D-5; merging the F1 and F2 stores | SM-P15 |
| F8 | Multi-agent contention | concurrent supersession, rebuttal forks, label collisions | SM-P15 |

F2, F4 and F5 were due in SM-P5 and SM-P11 and did not land with them; they arrived in
SM-P15, when the evaluation harness needed F1–F5 and the gap became visible. Each carries
a targeted test beside the generic ones, because a fixture that merely parses proves only
that it parses: F2's asserts a grounds chain three hops deep and a `backs` edge, F4's
derives a `qa` thread and requires every role filled, F5's requires the extension payloads
to survive — the one property that would vanish silently, since a fixture whose unknown
keys were dropped still parses, checks and round-trips.

F6 is the only fixture that is *supposed* to fail. Its `.expected` set is `SMY-E030`,
and a run that reports nothing means rule M has stopped binding.

Rule T cannot be exercised from surface text: attestations have no surface syntax
(Appendix A's `record = unit | relation | thread`), so provenance can only be authored
programmatically. Rule T's cases live in `crates/smysl-check/src/passes/trust.rs` until
`ingest` lands in SM-P14.

F3 and F6 carry more weight than their size suggests. F3 is where GE-2 is decided — a
claim-graph substrate carrying narrative without damaging it is an assertion, not a
result. F6 is where rules M and T are attacked on purpose rather than merely satisfied.

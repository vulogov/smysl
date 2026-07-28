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
| F7 | Mixed-granularity merge | D-5; merging the F1 and F2 stores | **landed** |
| F8 | Multi-agent contention | concurrent supersession, rebuttal forks, label collisions | **landed** |

F2, F4, F5, F7 and F8 were due in SM-P5, SM-P6 and SM-P11 and did not land with them; they
arrived in SM-P15, when the evaluation harness needed F1–F5 and the gap became visible.
Each carries a targeted test beside the generic ones, because a fixture that merely parses
proves only that it parses: F2's asserts a grounds chain three hops deep and a `backs`
edge, F4's derives a `qa` thread and requires every role filled, F5's requires the
extension payloads to survive — the one property that would vanish silently, since a
fixture whose unknown keys were dropped still parses, checks and round-trips.

**F7 and F8 are about merging, so they are not shaped like the others.**

F7 is a single document because a fixture is one file, but the property is about two. Its
own test asserts the shape — two body bands side by side, `SMY-W041` and no error — while
a second test asserts D-5 on the real thing, by merging the actual F1 and F2 stores and
requiring the result to still `check`. A merged store that failed would make merge
unusable across teams, which is the whole of D-5.

F8 ships as **two files**, `F8a-agent-alpha` and `F8b-agent-beta`, because a label
collision cannot be written down in one document: labels are document-local, so two agents
binding `c/cause` to different conclusions only collide once their stores meet. Each half
is clean on its own. Merging them raises all three detections of §5.4 at once — the label
collision, the supersession fork alpha left by superseding one claim twice, and the live
rebuttal beta's own thread presents. The test asserts the merge *report* rather than the
store, because §5.4 detections are reported and not recorded: a detection written into the
log would be a stale finding the moment a third store supplied the edge that ordered it.

F6 is the only fixture that is *supposed* to fail. Its `.expected` set is `SMY-E030`,
and a run that reports nothing means rule M has stopped binding.

Rule T cannot be exercised from surface text: attestations have no surface syntax
(Appendix A's `record = unit | relation | thread`), so provenance can only be authored
programmatically. Rule T's cases live in `crates/smysl-check/src/passes/trust.rs` until
`ingest` lands in SM-P14.

F3 and F6 carry more weight than their size suggests. F3 is where GE-2 is decided — a
claim-graph substrate carrying narrative without damaging it is an assertion, not a
result. F6 is where rules M and T are attacked on purpose rather than merely satisfied.

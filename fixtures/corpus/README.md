# Corpus F1–F8 (§27.2)

Each fixture ships as surface text (`.smy`), canonical CBOR (`.cbor`), and an expected
diagnostic set (`.expected`).

| ID | Fixture | Exercises | Lands |
|---|---|---|---|
| F1 | Incident report, `default` granularity | the baseline path; rules M, R, V1 | **landed** |
| F2 | Research trace, `fine` granularity | deep grounds chains; rule M cascades; corroboration | SM-P5 |
| F3 | Narrative text, `coarse` granularity | the design's most likely falsifier (GE-2) | **landed** |
| F4 | Q&A session | `answers` relations; the `qa` thread schema | SM-P11 |
| F5 | Dataset analysis with tables | `data`, `artifact-ref`, extension payloads | SM-P5 |
| F6 | Adversarial store | laundering attempts across a chain; rule M | **landed** |
| F7 | Mixed-granularity merge | D-5; merging the F1 and F2 stores | SM-P6 |
| F8 | Multi-agent contention | concurrent supersession, rebuttal forks, label collisions | SM-P6 |

F6 is the only fixture that is *supposed* to fail. Its `.expected` set is `SMY-E030`,
and a run that reports nothing means rule M has stopped binding.

Rule T cannot be exercised from surface text: attestations have no surface syntax
(Appendix A's `record = unit | relation | thread`), so provenance can only be authored
programmatically. Rule T's cases live in `crates/smysl-check/src/passes/trust.rs` until
`ingest` lands in SM-P14.

F3 and F6 carry more weight than their size suggests. F3 is where GE-2 is decided — a
claim-graph substrate carrying narrative without damaging it is an assertion, not a
result. F6 is where rules M and T are attacked on purpose rather than merely satisfied.

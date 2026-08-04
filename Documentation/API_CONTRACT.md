# What the facade promises

**Status:** decided for the three names that were open; the buckets themselves stand as
written. Anything not settled here is still a proposal.

`tests/public-api.txt` records 239 names and `cargo-semver-checks` stops them moving by
accident. Neither says which of them anyone *meant*. Gate 3 in `READINESS.md` calls that
mechanised-but-not-decided, and this file is the decision put in front of whoever makes it.

The question is not "is this name public" — it is. The question is what a consumer is entitled
to rely on, because a name in the second or third bucket below is one we would rather be free
to move, and today nothing distinguishes it from the first.

---

## The shape of what is exported

237 re-exports plus one module, under `--all-features`. A default build exposes fewer: the
ingest, provider, render and retrieval names sit behind features, so a consumer's actual
surface depends on what they turned on.

| kind | count | examples |
|---|---|---|
| functions | ~55 | `pack`, `merge`, `check`, `parse_surface`, `canonical_uid` |
| identifiers | 8 | `Uid`, `AgentId`, `SchemaId`, `ThreadId`, `ViewId`, `NodeId` |
| inputs | 11 | `PackRequest`, `CheckOptions`, `MergeOptions`, `IngestOptions` |
| outputs | 10 | `Report`, `MergeReport`, `ParseOutcome`, `FidelityReport` |
| errors | 11 | `CodecError`, `ProviderError`, `IntegrityError`, `AnyError` |
| enumerations | 15 | `Status`, `RelKind`, `SourceKind`, `TraceKind`, `PackMode` |
| everything else | ~124 | the kernel types, graph types, thread and render types |

---

## Bucket 1 — contract

Names a consumer cannot avoid, and which we should treat as fixed until a major bump.

**The kernel vocabulary.** `Uid`, `UnitCore`, `Unit`, `Status`, `RelKind`, `Relation`,
`SourceRef`, `SourceKind`, `SchemaId`, `AgentId`, `Attestation`, `View`, `Thread`, `Record`.
These *are* the format. §2 of the specification describes them, three other implementations
model them, and changing one is a format question rather than an API question.

**The twelve operations.** `parse_surface`, `write_surface`, `to_cbor`, `from_cbor`,
`canonical_uid`, `check`, `pack`, `merge`, `salience`, `derive_thread`, `trace`, `compact`.
Guarantee A5 already says making any of these non-reproducible is breaking regardless of
signature, which is a stronger promise than semver makes — so their signatures had better be
stable too.

**The errors those return.** `CodecError`, `IntegrityError`, `ShapeError`, `MergeError`,
`PackError`, `ParseError`. A caller matches on these; they are part of the calling convention.

**The inputs and outputs of the twelve.** `PackRequest`, `CheckOptions`, `MergeOptions`,
`SalienceRequest`, `DeriveOptions` and their `*Report` counterparts. All but one are already
`#[non_exhaustive]`, which is the right shape: a struct a caller builds and we keep adding to.

## Bucket 2 — incidental

Public because something in bucket 1 needs them, not because anyone chose to publish them.
Worth keeping public and worth saying we may move.

- **Provider machinery** — `ProviderConfig`, `Request`, `Completion`, `Usage`,
  `StructuredMode`, `Capabilities`, `ProviderId`. This is a plugin seam, not a format
  concern, and `Hybrid` changing shape twice inside 0.7 was all in here. An embedder wiring a
  provider needs them; nobody parsing a document does.
- **Render and retrieval** — `Bm25`, `Hybrid`, `Semantic`, `EmbedModel`, `Query`, `Hit`,
  `Retriever`, `BuildOptions`, `RenderError`. Behind features, and the retrieval trait is one
  cycle old.
- **The renamed re-exports** — `EmbedModel` (from `Model`) and `retrieve_tokenize` (from
  `tokenize`). A rename at the facade is a sign the original name was too generic to publish;
  it is not a sign anyone designed the pair.

## Bucket 3 — the three that were open, now decided

**`NodeId` is blessed as contract.** It stays a bare `u32` alias, because every traversal
returns `Vec<NodeId>` and an opaque wrapper buys safety a caller unwraps again immediately.
The cost is now stated where someone will meet it rather than discovered: a `NodeId` is an
index and not an identity, stable for one store at one moment, renumbered by any insertion
because the ordering is by uid and a uid can land anywhere in it. Hold one across a traversal;
never persist, send, or compare one between stores. The `Uid` is what survives all three.

**The bare `Error` is dropped**, and the type is kept as `AnyError`. It is not a leak — it is
the unified error, wrapping the other ten and carrying `exit_code()` — so removing it would
have taken real capability away from an embedder. What was wrong was the *name*: inside
`smysl-core`, `Error` is idiomatic; through a facade that flattens eleven error types into one
namespace, `Error` beside `CodecError` and `ParseError` reads as a twelfth sibling rather than
as the enum that wraps the eleven.

**`unit_core_bytes` and `hash_bytes` are kept, as contract.** The pair is what a second
implementation needs to derive a uid: `python/` uses exactly this decomposition, hashing the
canonical bytes separately from producing them so a disagreement localises to one half. What
that commits to is the algorithm, and changing the hash moves every uid in existence — which
makes it a format break under §8.2 rather than an API decision at all.

---

## Two things to fix whichever way the buckets fall

**`SalienceRequest` was the only one of eleven input types without `#[non_exhaustive]`.**
Fixed. Adding the marker is itself a break, which is why it is in `SEMVER_BREAKING` — a break
whose whole purpose is to stop the next addition being one.

**The surface is feature-dependent and the golden file is not.** `tests/public-api.txt` records
`--all-features`, so a consumer on default features has a different and smaller API than the
one under version control. Nothing checks the default surface at all.

---

## What is done, and what is not

Done: the three bucket-3 decisions, and `SalienceRequest`. All four are breaks, all four are
recorded in `SEMVER_BREAKING`, and the golden file moved by exactly one line — `Error` became
`AnyError`, which is the whole visible surface of the change.

Bucket 2 is now said in `src/lib.rs`, where a consumer reads it rather than only here. Being
told "this is a seam, not a promise" is the entire value of that bucket; a classification
nobody outside this file can see is a classification that has not been made.

Also outstanding: `tests/public-api.txt` records the `--all-features` surface, so a consumer on
default features has a different and smaller API than the one under version control, and
nothing checks the default one at all.

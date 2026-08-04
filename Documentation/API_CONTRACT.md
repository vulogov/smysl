# What the facade promises — a proposal

**Status:** a proposal, not a decision. Nothing here is enforced until someone accepts it.

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
| errors | 11 | `CodecError`, `ProviderError`, `IntegrityError`, `Error` |
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

## Bucket 3 — should not be public

**`NodeId`.** `pub type NodeId = u32`, documented as "position in the ascending-uid ordering".
It is an index into a store's internal layout, it changes when the store changes, and as a
bare alias it carries no type safety. Every traversal returns `Vec<NodeId>`, so removing it
means changing those signatures — which is exactly why it should be decided now rather than
after somebody stores one.

**The bare `Error`.** Eleven error types are exported and one of them is called `Error`. In a
facade that is a name nobody can use without aliasing it.

**`unit_core_bytes` and `hash_bytes`.** The first is the hash input of §2.1 and the second is
BLAKE3 over arbitrary bytes. Both are genuinely useful to an implementer and genuinely
internal to identity; if they stay, they are contract, and if they are convenience they should
not be at the top level.

---

## Two things to fix whichever way the buckets fall

**`SalienceRequest` is the only one of eleven input types without `#[non_exhaustive]`.** All
its fields are `pub`, so adding one is a breaking change and the other ten are protected. This
is an oversight rather than a decision — ten to one is not a design.

**The surface is feature-dependent and the golden file is not.** `tests/public-api.txt` records
`--all-features`, so a consumer on default features has a different and smaller API than the
one under version control. Nothing checks the default surface at all.

---

## What accepting this would change

Bucket 1 stays as it is, and `SEMVER_BREAKING` becomes the only way to move it.

Bucket 2 gets said out loud in the crate documentation — that these are a seam rather than a
promise — so a consumer who builds on them has been told.

Bucket 3 is three decisions, each small and each easier now than after a release: hide
`NodeId` behind an opaque type or accept it as contract, rename or drop the bare `Error`, and
decide whether the two hash functions are API or convenience.

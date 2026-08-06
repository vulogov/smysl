# What the facade promises

**Status:** decided for the three names that were open; the buckets themselves stand as
written. Anything not settled here is still a proposal.

`tests/public-api.txt` records 243 names and `cargo-semver-checks` stops them moving by
accident. Neither says which of them anyone *meant*. Gate 3 in `READINESS.md` calls that
mechanised-but-not-decided, and this file is the decision put in front of whoever makes it.

## Which artefact is the contract

Three, with different jobs, because each has a blind spot another covers. This was written
down in §1.2 S5 after measuring what each gate actually sees — and the measurement contradicted
what had been assumed.

| gate | sees | blind to |
|---|---|---|
| `tests/public-api.txt` (243 names) | every name the facade exports | anything *behind* a name: methods, signatures, variants |
| `make semver` | every item in each library crate, methods included, under the real semver rules | the facade — a `pub use` from another crate is a line it cannot expand |
| `tests/public-api-counts.txt` (11 lines) | a crate's surface changing size | an addition and a removal that cancel |

**The assumption that had to be corrected** is that `make semver` was authoritative and the
golden file merely an index. It is the other way round for the facade. `cargo-semver-checks`
run against `smysl` reports *"no semver update required"* for the 0.12 rename of `Error` to
`AnyError` — although `v0.11.0` exported `smysl::Error` and nothing exports it now. It has the
same `pub use` blind spot as `cargo public-api`. **The golden file caught that rename; the
semver gate did not.**

So: for the facade's names, `tests/public-api.txt` is the gate. For everything behind a name,
`make semver` is. For a public item added to a library crate by accident — nobody's break, so
nobody's failure — `tests/public-api-counts.txt` is, and nothing else was watching that at all.

Two further things S5 changed, both about `make semver` not running:

- **It no longer skips.** A crate in `SEMVER_BREAKING` used to be `continue`d with a one-line
  SKIP, so a crate with one deliberate break had *nothing* watching it and a second,
  unintended break rode along invisibly. Now those crates are run and their failures printed,
  ungated, to be compared against the reasons recorded beside the list.
- **That immediately found a wrong entry.** `smysl-core` had been listed for the `AnyError`
  rename and reported no failures at all, because it never broke: the type there is still
  `Error`, and the rename is a `pub use ... as` in the facade. The entry now names `smysl`.

**What none of the three covers** is a baseline that has gone stale: `make semver` compares
against the last *published* version, so a cut-but-unpublished release means every break since
accumulates against an older baseline. That is recorded beside `BASELINE` in the `Makefile`
rather than fixed, because publication is the only thing that fixes it.

---

**One more correction from the 1.0 measurement**, in `ROAD_TO_1.0.md` §0.2:
**The seam is where the choice is.** Splitting the surface by whether an item hangs off a
   facade-exported type puts ~92% in "methods on things we deliberately export". The
   discretionary remainder concentrates in `smysl-provider` (26%) and `smysl-ingest` (36%) —
   which is to say, in bucket 2. The instinct below about *which* surface is unsettled was
   right; what follows from it is narrowing, not exemption.

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

## The `#[non_exhaustive]` rule (§1.1, settled in 0.13)

**191 distinct public types. 152 carry the attribute; 39 do not, and none of those is an
oversight.** The rule, rather than the list:

1. **A type with no public fields is closed by encapsulation.** The attribute adds nothing — a
   caller already cannot write a struct literal or match it exhaustively. 24 opaque structs and
   9 newtypes fall here, and they need no annotation and no note.
2. **Anything that carries the format is `#[non_exhaustive]`.** §8 says the crate and format
   versions are independent axes, and gives the precedent: *"record type 10 was added in 0.2
   without a format bump"*. An exhaustive `UnitCore` or `Relation` would turn the next such
   addition into a crate major — coupling the two axes the specification separates. This is the
   argument, not a general preference for the attribute.

   The clearest case is already scheduled: §0.1's migration to `smysl/1.0` has to add a field
   to `ParseOutcome` to carry the version a document declared. Exhaustive, that is a 2.0.
3. **Reports, options and errors are `#[non_exhaustive]`.** They exist to grow, and callers
   read them rather than build them. Where a caller does build one, it gets a constructor or is
   built from `Default` and adjusted — `Detected::new` and `Optimality::new` were added for
   exactly that, and `Constraints`, `SalienceWeights` and `ParseOutcome` are now built from
   their defaults at their four call sites.
4. **Six types are closed on purpose, and say so where they are declared.** `Hlc`, `Date`,
   `Span`, `Spanned`, `HValue`, `Severity`. Each is a shape that is complete rather than
   unfinished: a hybrid logical clock has no fourth component, a byte range is two offsets, and
   `HValue`'s variants are the JSON data model's rather than this crate's. Callers match on
   `HValue` and `Severity` exhaustively on purpose.

The test of whether the rule was applied honestly is that it produced both answers. A rule that
only ever says "add the attribute" is a preference wearing a rule's clothes.

---

## Three rules the 1.0 review left behind

Not fixes — standing constraints, recorded because each was learned by nearly getting it wrong.

**A public trait's method set is frozen harder than a struct's fields.** `#[non_exhaustive]`
lets a struct grow; there is no equivalent for a trait. `Retriever` is unsealed and anyone may
implement it, so every method added after 1.0 must carry a default or it is a 2.0 — as
`is_empty` already does and `search` and `len` cannot. The same holds for `Provider`.

**Do not put a transport's vocabulary on an abstraction that has none.** `Provider` names no
HTTP: not `http`, not `status`, not `u16`. Three of its eight implementors speak no HTTP at
all. §1.2 S3 came close to moving `status_error(&self, u16, &str)` onto it for the sake of
enforcing a shared shape, which would have frozen HTTP into a general trait and forced three
non-HTTP implementors to fake a method. The shape went onto a separate `StatusMapping` instead.

**Enforce shared shapes where construction happens, not where it is declared.** Writing
`StatusMapping` down did not oblige anyone to implement it; a sixth mapper could still skip it.
`map::build` boxing through `fn boxed<P: Provider + StatusMapping>` is what makes it a rule,
because that is the one path every mapper takes to reach a caller.

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

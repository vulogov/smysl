#import "design.typ": *

// ═══════════════════════════════════════════════════════════════════════
// Appendix A — Full Command and Flag Reference
// ═══════════════════════════════════════════════════════════════════════

#appendix(letter: "A", title: "Full Command and Flag Reference")

Every flag below is transcribed from `fn cli()` in `src/main.rs` — the single place the
command surface is declared, and therefore the only place this table can drift out of
sync with the binary. Each subcommand heading repeats three facts from the command table
in `src/main.rs` (`§23`): its one-line description, its *purity* (`pure` — a bit-reproducible
function of its inputs; `mixed` — pure except for one option; `model-dependent` — the only
kind that can reach off the machine), and the delivery phase that wired it up. All
twenty wired subcommands are covered; `ui` (SM-P15) is a stub in this build and prints
"not wired in this build" rather than accepting flags, so it has no flag table.

Three of them — `import`, `relink` and `compact` — were absent from this appendix until
0.4.0. They were wired in SM-P15 and the appendix was never extended to match, which is
exactly the drift the paragraph above claims this table cannot have. It could, and it did.

#section("Global flags")

These apply to every subcommand, in any position on the command line.

#dtable(
  (auto, 1fr),
  (
    ([Flag], [Meaning]),
    ([`-C, --config FILE`], [Configuration file.]),
    ([`-s, --store PATH`], [Store path; `-` reads stdin (rule P).]),
    ([`-o, --output PATH`], [Output path; defaults to stdout.]),
    ([`--format surface|cbor`], [Output form; defaults to `cbor` on a non-TTY stdout (rule P).]),
    ([`--strict`], [Treat warnings as errors.]),
    ([`--offline`], [Hard-fail rather than send anything off the machine.]),
    ([`--no-color`], [Disable colour.]),
    ([`--noprogress`], [Disable progress bars, whatever the terminal is.]),
    ([`--json`], [Machine-readable output.]),
    ([`-q, --quiet`], [Suppress non-error output.]),
    ([`-v, --verbose`], [Increase verbosity; repeatable (`-vv`, `-vvv`, …).]),
    ([`--seed-check`], [Assert this invocation is bit-reproducible (rule D).]),
  ),
)

#section("fmt")

*Canonicalise surface text and verify the round-trip.* Pure · SM-P2.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--check`], [—], [Exit 3 if reformatting would change bytes.]),
    ([`--write`], [—], [Rewrite files in place.]),
    ([`FILE …`], [positional, 0 or more], [Files to format; `-` or none reads stdin (rule P).]),
  ),
)

#section("check")

*Run the check pipeline over a store.* Pure · SM-P4.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--conformance`], [`CLASS`], [Assert the store is consumable at a conformance class.]),
    ([`--as`], [`SCHEMA` (repeatable)], [Report fidelity for a consumer implementing these schemas.]),
    ([`--granularity`], [—], [Report the granularity distribution of the store.]),
    ([`--pass`], [`NAME` (repeatable)], [Run only these passes.]),
    ([`FILE …`], [positional, 0 or more], [Stores to check; `-` or none reads stdin (rule P).]),
  ),
)

#section("pack")

*Budget-bounded, closure-complete selection.* Pure · SM-P9.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--budget`], [`N` (required)], [Token budget, counted with the recorded estimator.]),
    ([`--focus`], [`UID` (repeatable)], [Units that must reach L1; packing fails if they cannot.]),
    ([`--lod`], [`auto|L0|L1|L2`], [Cap every unit at this level.]),
    ([`--explain`], [—], [Say which constraint put each unit in.]),
    ([`--tokenizer`], [`ID`], [Cost model; recorded in the packinfo either way (D-2).]),
    ([`--mode`], [`greedy|exact`], [`exact` proves optimality by branch and bound; needs the `exact-pack` feature.]),
    ([`PATH`], [positional], [Store to pack.]),
  ),
)

#section("merge")

*Join-semilattice union; materialise contentions.* Pure · SM-P6.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--policy`], [`latest|all|contend`], [Supersession policy.]),
    ([`--retraction`], [`strict|advisory|ignore`], [Retraction policy.]),
    ([`--staged`], [—], [Commit `.smysl/staged.smy` into the store.]),
    ([`--fail-on-contention`], [—], [Exit 5 when the merged store carries a contention.]),
    ([`--max-contentions-per-agent`], [`N`], [Warn when one merge raises more than N contentions.]),
    ([`STORE …`], [positional, 1 or more, required], [Stores to merge; `-` reads stdin (rule P).]),
  ),
)

#section("diff")

*Partition uids across stores or hops.* Pure · SM-P7.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--hop`], [`A..B`], [Partition units across a hop range instead of comparing stores.]),
    ([`--by-agent`], [—], [Attribute each change to the agents responsible.]),
    ([`--recipe`], [—], [Flag whether the prompt changed or the content did (D-8).]),
    ([`STORE …`], [positional, 1 or more, required], [One store for `--hop`, two to compare.]),
  ),
)

#section("trace")

*Walk provenance or evidential support.* Pure · SM-P7.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`UID`], [positional, required], [The unit to trace from.]),
    ([`--depth`], [`N`], [How far back to walk.]),
    ([`--parents`], [—], [Causal: attestation parents and supersession.]),
    ([`--grounds`], [—], [Evidential: grounds and deps (the default).]),
    ([`--both`], [—], [Both walks at once.]),
    ([`--agents`], [—], [Name the agents behind each step.]),
    ([`PATH`], [positional], [Store to trace within.]),
  ),
)

#section("view")

*Define or print a view.* Pure · SM-P7.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--roots`], [`UID` (repeatable)], [Roots of the view.]),
    ([`--id`], [`NAME`], [View identifier.]),
    ([`--threads`], [`ID` (repeatable)], [Threads to associate with the view.]),
    ([`--requires`], [`SCHEMA` (repeatable)], [Schemas the view requires a consumer to implement.]),
    ([`PATH`], [positional], [Store to read.]),
  ),
)

#section("bundle")

*Emit the reachable closure of a view.* Pure · SM-P7.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--view`], [`ID`], [Which view to bundle; the first if omitted.]),
    ([`--include-retracted`], [—], [Keep units that have been retracted.]),
    ([`PATH`], [positional], [Store to bundle from.]),
  ),
)

#section("thread")

*Derive, refine, list, show, or import threads.* Mixed (pure except `--refine`, which is
not yet wired) · SM-P11.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--derive`], [`analysis|narrative|brief|qa|plan`, 0 or 1 args], [Derive a thread from the graph; the schema may follow here or in `--schema`.]),
    ([`--schema`], [`analysis|narrative|brief|qa|plan`], [Schema to derive under (§23 spells it this way).]),
    ([`--only`], [—], [Emit the thread record alone rather than the store it belongs to.]),
    ([`--list`], [—], [List the threads the store already holds.]),
    ([`--show`], [`ID`], [Print one thread, step by step.]),
    ([`--id`], [`T`], [Thread id for the derived thread.]),
    ([`--as`], [`AGENT`], [Owner of the derived thread.]),
    ([`--scope`], [`UID` (repeatable)], [Derive over these units only.]),
    ([`--arity`], [`ROLE=N` (repeatable)], [Override how many units a role may hold.]),
    ([`--explain`], [—], [Say which role each unit took and what repair added.]),
    ([`PATH`], [positional], [Store to derive from.]),
  ),
)

#section("salience")

*Report derived salience with per-term breakdown.* Pure · SM-P8.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--top`], [`N`], [Show only the N highest-scoring units.]),
    ([`--explain`], [`UID`], [Break one unit's score into its three terms.]),
    ([`--weights`], [`C,R,T`], [Override the centrality, corroboration and role weights.]),
    ([`--seed`], [`UID` (repeatable)], [Personalise against these units; the view roots by default.]),
    ([`PATH`], [positional], [Store to score.]),
  ),
)

#section("retract")

*Retract a unit; report the blast radius first.* Pure · SM-P6.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`UID`], [positional, required], [The unit to retract; the display form is resolved as a prefix.]),
    ([`--dry-run`], [—], [Report the blast radius without applying anything.]),
    ([`--as`], [`AGENT` (repeatable)], [The agent(s) issuing the retraction.]),
    ([`--reason`], [`TEXT`], [Why.]),
    ([`--authority`], [`A`], [`origin | any | quorum:N`.]),
    ([`PATH`], [positional], [Store to retract from.]),
  ),
)

#section("render")

*Thread plus profile to artifact.* Pure · SM-P12.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--thread`], [`ID`], [Thread to render; the store's only thread by default.]),
    ([`--profile`], [`NAME`], [Built-in profile name, or a path to a profile file.]),
    ([`--target`], [`markdown|md|typst|html|slides|json|text`], [Output format.]),
    ([`--lod`], [`L0|L1|L2`], [Cap every block at this level, whatever the profile says.]),
    ([`--contentions`], [`show|suppress`], [Override the profile's rule V2 setting.]),
    ([`--as`], [`AGENT`], [Whose thread, when several agents hold one under that id.]),
    ([`--profiles`], [—], [List the built-in profiles and exit.]),
    ([`PATH`], [positional], [Store to render from.]),
  ),
)

#section("import")

*Tabular readings to measured units, without a model.* Pure · SM-P15.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--key`], [`COL` (repeatable)], [Columns naming the reading rather than carrying its value.]),
    ([`--kind`], [`file|metric|tool|url|doc`], [Source kind recorded on each unit; `file` by default.]),
    ([`PATH`], [positional, required], [Delimiter-separated file to import; `-` reads stdin (rule P).]),
  ),
)

#section("relink")

*Re-point references onto superseded units.* Pure · SM-P15.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--dry-run`], [—], [Report what would be re-pointed without emitting anything.]),
    ([`PATH`], [positional], [Store to relink; `-` reads stdin (rule P).]),
  ),
)

#section("compact")

*Drop superseded units nothing needs; never in place.* Pure · SM-P15.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--dry-run`], [—], [Report what would be dropped without emitting anything.]),
    ([`PATH`], [positional], [Store to compact; `-` reads stdin (rule P).]),
  ),
)

#section("ingest")

*Prose or data to staged units.* Model-dependent · SM-P14.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`FILE`], [positional], [Document to ingest; `-` reads stdin.]),
    ([`--rung`], [`computed|document|web|model`], [Trust rung of the source; caps what units may claim (rule T).]),
    ([`--granularity`], [`P`], [Granularity profile the units are produced under.]),
    ([`--path`], [`auto|surface|json-ast`], [Override the path D-9 would choose.]),
    ([`--repair`], [`N`], [Repair attempts before a span degrades to opaque prose.]),
    ([`--yes`], [—], [Commit the staged batch instead of exiting 10.]),
    ([`--dry-run`], [—], [Report what would be sent and to whom; make no call.]),
  ),
)

#section("attest")

*Semantic checks that require a model.* Model-dependent · SM-P14.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--what`], [`gist-coverage|warrant-plausibility|granularity`], [Which semantic question to ask.]),
    ([`--sample`], [`N` (or `all`)], [How many units to ask about; `all` for the whole store.]),
    ([`PATH`], [positional], [Store to attest.]),
  ),
)

#section("providers")

*List providers, capabilities, and what would egress.* Pure · SM-P13.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--probe`], [—], [Contact each provider and report what it actually is.]),
    ([`--models`], [—], [List each provider's installed models.]),
    ([`--tasks`], [—], [Report which tasks would send content off the machine.]),
  ),
)

#section("usage")

*Token and cost ledger.* Pure · SM-P13.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--by`], [`provider|task|run|model`], [How to group the ledger.]),
    ([`--since`], [`MS`], [Only calls at or after this epoch-millisecond timestamp.]),
    ([`--reset`], [—], [Discard the ledger.]),
  ),
)

#section("reindex")

*Rebuild the derived index from the log alone.* Pure · SM-P3.

#dtable(
  (auto, auto, 1fr),
  (
    ([Flag], [Value], [Meaning]),
    ([`--verify`], [—], [Compare the rebuilt index against the sidecar instead of writing it.]),
    ([`PATH`], [positional], [Store to reindex.]),
  ),
)

// ═══════════════════════════════════════════════════════════════════════
// Appendix B — Full Diagnostic Code Reference
// ═══════════════════════════════════════════════════════════════════════

#appendix(letter: "B", title: "Full Diagnostic Code Reference")

Every diagnostic `smysl` can emit has a stable code, declared once in the `registry!`
macro invocation in `crates/smysl-core/src/diag.rs` and never reused. The registry is
single-sourced: wire string, severity, group, and one-line meaning all come from that one
place, and a workspace test (`registry_matches_appendix_d_size`) asserts the count below
stays at 49. Codes are grouped exactly as the source groups them; group membership is
reporting structure only; it carries no weight on the wire.

#dtable(
  (auto, auto, 1fr),
  (
    ([Group], [Count], [Covers]),
    ([Parse], [7], [Surface and CBOR parsing; deterministic-encoding and float-quantisation rules.]),
    ([Identity], [7], [Dangling references, dependency cycles, hash and uid integrity, index staleness.]),
    ([Lod], [7], [Granularity and level-of-detail shape rules — gist, body, detail, closure.]),
    ([Epistemics], [6], [Rule M and rule T — status versus grounds, versus the authoring rung.]),
    ([Merge], [6], [Retraction, supersession, and the contentions merge materialises.]),
    ([PackRender], [5], [Budget feasibility and render-time rule V1/V2 enforcement.]),
    ([Extension], [4], [Rule X — unknown schemas and relation kinds at the extension boundary.]),
    ([Provider], [7], [The model boundary — reachability, offline, context limits, ingest repair.]),
  ),
)

#subsection("Parse — surface and codec")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E001`], [error], [Surface parse error.]),
    ([`SMY-E002`], [error], [Unsupported kernel major version.]),
    ([`SMY-E003`], [error], [Unsupported format version.]),
    ([`SMY-E004`], [error], [Malformed CBOR envelope — or, since 0.3, nesting deeper than 128. The reader walks containers recursively, so an unbounded depth let deeply nested input overflow the stack and *abort the process*, which is worse than an error because it cannot be caught. 128 is far above anything real: the deepest shape the kernel defines is three levels.]),
    ([`SMY-W014`], [warning], [Unknown envelope type code — preserved verbatim, skipped semantically. Reported by `check` since 0.2; before that the code existed and nothing emitted it, so an unknown record was preserved in silence.]),
    ([`SMY-E080`], [error], [Non-deterministic encoding (key order, indefinite length, non-shortest int, null optional, non-NFC text).]),
    ([`SMY-E081`], [error], [Float not binary32 or not quantised to 1/1024.]),
  ),
)

#subsection("Identity — hash and integrity")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E060`], [error], [Dangling reference.]),
    ([`SMY-E061`], [error], [Cycle in deps.]),
    ([`SMY-W062`], [warning], [Cycle in causes or sequences.]),
    ([`SMY-E070`], [error], [Hash mismatch — recomputed uid differs from stored uid.]),
    ([`SMY-E071`], [error], [Truncated uid in a canonical record.]),
    ([`SMY-E072`], [error], [Ambiguous uid prefix.]),
    ([`SMY-W110`], [warning], [Stale or corrupt index — rebuilding.]),
  ),
)

#subsection("Lod — granularity and shape")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E020`], [error], [L1 closure violation — body references a uid absent from deps or grounds.]),
    ([`SMY-E021`], [error], [Missing gist.]),
    ([`SMY-E022`], [error], [Gist exceeds `l0_max`.]),
    ([`SMY-E023`], [error], [`detail` without `body`.]),
    ([`SMY-W024`], [warning], [Gist appears to depend on body (heuristic; confirm via `attest`).]),
    ([`SMY-E040`], [error], [Multi-assertion body under single-assertion admission.]),
    ([`SMY-W041`], [warning], [Body outside `l1_range`.]),
  ),
)

#subsection("Epistemics — rules M and T")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E030`], [error], [Rule M violation — status exceeds weakest ground.]),
    ([`SMY-E031`], [error], [`derived`/`inferred` with empty grounds.]),
    ([`SMY-E032`], [error], [`measured`/`cited` without source.]),
    ([`SMY-E033`], [error], [Rule T violation — status exceeds the ceiling for the attestation's rung.]),
    ([`SMY-E034`], [error], [`unfounded` authored.]),
    ([`SMY-W035`], [warning], [`measured` with `op: Authored` rather than `Imported`.]),
    ([`SMY-W036`], [warning], [Rule M applied at ingest — status lowered to what its grounds support.]),
  ),
)

#subsection("Merge — retraction, supersession, contention")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E050`], [error], [Orphaned grounds — all grounds retracted under strict.]),
    ([`SMY-E051`], [error], [Retraction authority not satisfied.]),
    ([`SMY-W052`], [warning], [Retracted unit retained under advisory.]),
    ([`SMY-W053`], [warning], [Concurrent supersession materialised as a contention.]),
    ([`SMY-W054`], [warning], [Label and uid do not correspond one to one — two labels for one unit, or one label for two.]),
    ([`SMY-W055`], [warning], [Agent contention rate exceeds `--max-contentions-per-agent`.]),
  ),
)

#subsection("PackRender — budget and render-time rules")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E200`], [error], [Pack infeasible — C3/C4/C5 unsatisfiable; reports minimum feasible budget.]),
    ([`SMY-E201`], [error], [Focus unit absent from store.]),
    ([`SMY-W202`], [warning], [Greedy mode above `exact_threshold`; optimality gap reported.]),
    ([`SMY-E210`], [error], [Rule V1 — profile lacks a rendering for some status.]),
    ([`SMY-W211`], [warning], [Rule V2 — contentions suppressed; recorded in output metadata.]),
  ),
)

#subsection("Extension — rule X")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-W010`], [warning], [Unknown schema — degraded fidelity (rule X). Two cases: a type this *build* does not know, reported always; and a type the named `--as` consumer does not implement, reported only when one is named. The first covers a kernel type added by a later version, which decodes and round-trips rather than failing.]),
    ([`SMY-E011`], [error], [Rule X violation — unrecognised payload dropped on re-emission.]),
    ([`SMY-E012`], [error], [Extension schema attempts to weaken a kernel rule.]),
    ([`SMY-W013`], [warning], [Unknown relation kind treated as `elaborates` for closure.]),
  ),
)

#subsection("Provider — the model boundary and ingest")

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Sev.], [Meaning]),
    ([`SMY-E300`], [error], [Provider unreachable, no fallback configured.]),
    ([`SMY-E301`], [error], [Offline violation.]),
    ([`SMY-E302`], [error], [Context window exceeded after chunking.]),
    ([`SMY-W303`], [warning], [Structured output unsupported; fell back to the surface path.]),
    ([`SMY-W304`], [warning], [Span unrepairable; degraded to opaque prose (rule I).]),
    ([`SMY-W305`], [warning], [Token count estimated rather than provider-reported.]),
    ([`SMY-W306`], [warning], [Usage threshold exceeded — informational only, never blocks.]),
    ([`SMY-E307`], [error], [Attributed quote does not occur in the source text.]),
    ([`SMY-W308`], [warning], [Attributed quote occurs only loosely — elided or reworded.]),
  ),
)

Two of these have no emission site and are listed for completeness only:
`SMY-W305`'s information already reaches you through the usage totals line, and
`SMY-W306` describes a threshold feature that does not exist. They are on the
list to be emitted or deleted rather than left as codes you could wait for
forever.

// ═══════════════════════════════════════════════════════════════════════
// Appendix C — Exit Codes
// ═══════════════════════════════════════════════════════════════════════

#appendix(letter: "C", title: "Exit Codes")

Twelve codes, `0` through `11`, declared in `ExitCode` in
`crates/smysl-core/src/error.rs`. They are part of the contract: stable across minor
versions, so a pipeline can branch on exactly what happened rather than parsing stderr.
A workspace test (`exit_codes_are_contiguous_and_unique`) asserts the set never grows a
gap or a duplicate.

#dtable(
  (auto, auto, 1fr, 1fr),
  (
    ([Code], [Name], [Means], [Seen from]),
    ([`0`], [`Success`], [Nothing to report.], [Any command that completed cleanly.]),
    ([`1`], [`Failure`], [A generic failure — see stderr.], [I/O errors, an absent store, a missing thread.]),
    ([`2`], [`Usage`], [The command line itself was wrong.], [A bad flag value, a missing required argument.]),
    ([`3`], [`CheckErrors`], [`check` (or `fmt --check`) found something at or above the failure threshold.], [`check`, `fmt --check`, and any command whose input fails to parse.]),
    ([`4`], [`PackInfeasible`], [No selection satisfies the budget and focus constraints (`SMY-E200`).], [`pack --budget` too small for the mandatory floor.]),
    ([`5`], [`Contentions`], [`merge --fail-on-contention` found disagreement.], [`merge` with that flag set, when the join raises a contention.]),
    ([`6`], [`Provider`], [A model provider failed.], [`providers --probe`, `ingest`, `attest` against an unreachable or erroring provider.]),
    ([`7`], [`Offline`], [`--offline` blocked a call that would have left the machine.], [Any model-dependent command run with `--offline` against a hosted provider.]),
    ([`8`], [`UnsupportedVersion`], [The document's format or kernel major isn't supported by this build.], [Parsing a store from a future or otherwise incompatible format/kernel major.]),
    ([`9`], [`HashVerification`], [A recomputed hash didn't match a stored uid.], [`fmt`'s round-trip check, `reindex --verify`, integrity failures.]),
    ([`10`], [`Staged`], [Output is staged, awaiting `merge --staged` to confirm it.], [`ingest`, unless `--yes` is given.]),
    ([`11`], [`StagedWithCorrections`], [The same, and rule M lowered at least one unit — the model claimed more than its grounds support and was corrected. Not a failure. New in 0.2; a script testing `= 10` should test `>= 10`.], [`ingest`, including under `--yes`.]),
  ),
)

// ═══════════════════════════════════════════════════════════════════════
// Appendix D — Glossary
// ═══════════════════════════════════════════════════════════════════════

#appendix(letter: "D", title: "Glossary")

A reference restatement of every term boxed elsewhere in this manual — tight, not a first
introduction. Where a chapter's own wording differs slightly, trust the chapter for nuance
and this page for a quick reminder of what the word means.

#term("Store")[
  Whatever `smysl` is operating on: a `.smy` surface file, a CBOR log (what `merge -o`
  writes), or `-` for stdin. Almost every command takes one, either as a trailing
  positional argument or via the global `-s`/`--store` flag.
]

#term("Purity")[
  A command's classification as *pure* (a bit-reproducible function of its inputs — same
  bytes in, same bytes out, on any machine, forever), *mixed* (pure except for one option),
  or *model-dependent* (the only kind that can reach off the machine). Only `ingest` and
  `attest` are ever model-dependent; `thread` is mixed only because of an unwired
  `--refine`.
]

#term("Canonical form")[
  The one, unique byte-for-byte spelling of a set of records — quoting decided by content
  rather than by how you typed it, granularity always expanded to its full field set.
  Hashes are computed over CBOR, never over surface text, so reformatting never changes a
  unit's identity, only the bytes a person reads.
]

#term("Trust rung")[
  The rung a unit was *produced* at — `computed`, `document`, `web`, or `model` — which
  caps the highest status it may ever claim (rule T). `ingest --rung document` is the
  default: a model reading a document you supplied may propose at most `cited`, never
  `measured`, however confidently it phrases something.
]

#term("Staging")[
  The holding area a model's output lands in before it becomes part of your store.
  `ingest` writes candidate units to `.smysl/staged.smy` and exits `10` rather than `0`;
  nothing from a model reaches a real store without a deliberate `merge --staged` (or
  `ingest --yes`, accepting the gate ahead of time). The staged file is ordinary surface
  text, by design — the thing you are asked to approve is the thing you can read.
]

#term("Recipe")[
  A hash of the full conditions of one model call — provider, model, prompt, everything
  that could change the answer. `usage --by model` groups the ledger by it, so one logical
  ingest aggregates across vendors instead of looking like unrelated calls.
]

#term("Ledger")[
  A local, append-only record of what was called, how many tokens it cost, and under what
  recipe — never the content itself. `usage` reads it back; `usage --reset` discards it.
]

#term("Label binding")[
  The record that remembers a label names a particular unit. You write the label as part of
  the unit; the binding is how it reaches the wire, and it is a separate record because a
  label is not identity — a label inside hashed content would make renaming one produce a
  different unit. Two stores binding the same label to different uids are a
  `label-collision` contention on merge. New in 0.2: before it, labels survived a parse and
  not a store round trip.
]

#term("Comment")[
  A line beginning `#` or `//` at column 0, outside any record. Skipped by the parser and
  not carried by any record, so canonical form cannot reproduce one — `fmt` warns before it
  drops them. A comment is a comment wherever it appears, including inside a body, which is
  why a body cannot open a line with either marker.
]

#term("Contention")[
  What `merge` materialises instead of silently picking a winner: a record naming the unit
  in dispute, every position taken on it, and why it was detected — `live-rebuttal` (a
  `rebuts` edge landed both sides in the same graph), `supersession-fork` (two units both
  claim to supersede the same target), or `label-collision` (the same label resolves to two
  different uids across the merged sources). `merge --fail-on-contention` turns any of
  these into exit `5`.
]

#term("Blast radius")[
  Everything a retraction would reach — every unit that grounds on the target, recursively
  — reported *before* anything is applied. `retract --dry-run` only ever reports it;
  dropping that flag applies the retraction and reports how many units now read as
  `unfounded`. A small blast radius is itself informative: it says the retracted line of
  reasoning was a dead end, not a foundation anything else was built on.
]

#term("Salience")[
  A score in `[0, 1]` built from three named, individually-weighted terms — *centrality*
  (how much else depends on this unit), *corroboration* (how many independent agents
  attested it), and *role* (where it sits in a thread) — never a single opaque number.
  `salience --explain` breaks any one score back into those three terms.
]

#term("View")[
  A name plus a root set — never a container. Everything reachable from the roots belongs
  to it at zero copying cost; `bundle` is what turns that reachable closure into a portable,
  self-contained store.
]

#term("Conformance class")[
  One of five named tiers `check --conformance` certifies a store against —
  `C-Read` through `C-Full` — each one a safety statement about what the store may
  correctly be used for (reading, consuming, producing into, merging), rather than a
  statement about whether any individual claim is true.
]

#term("Fidelity")[
  What a consumer implementing certain schemas can do with a store: `Full` (everything
  `requires` names is implemented), `Degraded` (an extension is missing — read what you
  can), or `Refuse` (the kernel major itself isn't supported — the one case that is not
  allowed to degrade silently).
]

#term("Profile")[
  A named bundle of *rendering* choices — register, audience, verbosity, how deep to go by
  role, how to display status — entirely separate from the document itself. `render
  --profiles` lists the built-ins; a profile that would flatten epistemic status entirely
  (rule V1) is refused before a single byte is emitted.
]

#term("Model boundary")[
  The line around the only three operations that may ever consult a model: `ingest`,
  `attest`, and `thread --refine`. Everything on the far side of it — selecting, merging,
  ordering, ranking, rendering — is a pure function of its inputs, verified in CI rather
  than merely intended.
]

#term("Facade crate")[
  The `smysl` crate itself: a `[lib]` re-exporting the public surface of every other crate
  in the workspace, plus a `[[bin]]` that is a thin shell over it. Nothing the CLI does is
  unreachable from the library — every `cmd_*` function in `src/main.rs` calls straight
  into a facade re-export, never around it.
]

#term("Grounds")[
  The list naming exactly which units a claim depends on. A claim's status can never
  outrank the weakest thing in its `grounds` (rule M) — retract a ground and the tool can
  tell you exactly what falls with it.
]

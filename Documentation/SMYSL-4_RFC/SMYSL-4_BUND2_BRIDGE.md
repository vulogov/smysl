# RFC SMYSL-4 — Bund2 programmable queries over a library, through a subprocess

**Author:** Vladimir Ulogov
**Status:** amended draft, for acceptance. The draft this replaces was checked against both code
bases before any of it was built; Appendix A lists what that check found and what changed.
**Target version:** not fixed. Phases are named (BQ-P1 to BQ-P4), not numbered by release.
**Evidence baseline:**
- smysl `dev/1.18.0` at `fdd92ed` ("Open 1.18.0");
- Bund2 `development/0.56.0` at `63fa39d`, read and not built.

Every code pointer below describes those trees. Re-check them against the trees in hand before
starting a phase.
**External runtime:** the `bund2` CLI of `vulogov/Bund2` 0.56.0 or later, built with
`--features smysl`.
**Upstream constraint:** no modification to Bund2. Not a crate split, not a VM change, not a
Cargo change.
**Supersedes:** *RFC SMYSL-4: Bund2 Advanced Query & Logic Integration (Path 2 — Zero-Modification
Subprocess Bridge)*, final draft (2026-11-20).
**Companions:**
- Bund2 RFC-0014 (`docs/rfc/RFC-0014-smysl-corpora.md`), the 28 `smysl.*` words;
- Bund2 RFC-0013 (`docs/rfc/RFC-0013-logic-programming.md`), logic programming, tabling and CLP(FD);
- Bund2's decisions register (`docs/registers/decisions.md`), cited as Bund2 D-n.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119, and only in
Part A.

---

## 0. Summary

`sq/1` is declarative, portable across the four implementations, and guaranteed to terminate.
It filters, ranks, groups and packs. It does not do user-defined rules, multi-way joins,
recursion, negation as failure or constraint solving, and it refuses `match` with `SMY-E461`
(`smysl-query/src/parse.rs:649`). That is by design, and this RFC does not change it.

Bund2 already has what `sq/1` leaves out. RFC-0014 gives it 28 `smysl.*` words over a library,
and RFC-0013 gives it Prolog-style logic, tabling, saturation, proof trees and CLP(FD), reached
from a library through `smysl.kb`.

This RFC adds **`smysl-bund`**, a boundary crate that runs a Bund2 script over a disposable copy
of a smysl library in a separate `bund2` process, and returns its answer as a typed value. It is
read-only with respect to every library the caller holds. It never links Bund2.

The draft promised more than that, and the check against Bund2 0.56.0 showed three of its
promises cannot be kept by the means it chose (Appendix A). Two change the design:

- **A word list is not a security boundary in Bund2.** A script can build a word's name from
  strings and call it, and Bund2's own register says evaluation "cannot be switched off by
  name-gating a word list" (Bund2 D16, `decisions.md:14263-14267`). The boundary is therefore
  the operating system: a process that cannot open the network, and can touch no file outside
  its temporary directory. Until that sandbox exists on a platform, the bridge runs only scripts
  its caller declares trusted (§5, D-B2).
- **Bund2 cannot write a string as JSON.** `json.from_value` refuses STRING and any LIST or MAP
  holding one (`bund2-value/src/lib.rs:997-1040`), and every `smysl.*` answer holds strings. The
  bridge therefore supplies its own encoder, written in Bund and appended to the script (§7).

## 1. Decisions

| # | Question | Decision | Rejected | Why |
|---|---|---|---|---|
| D-B1 | Linking or a process | **A process.** `smysl-bund` has no Bund2 crate in its `Cargo.toml`. | Linking `bund2-stdlib` | `bund2-stdlib` pins every smysl crate at `=1.15.0` (`crates/bund2-stdlib/Cargo.toml:135-149`), so it cannot share a `Store` type with 1.16 or later. Bund2 is `edition = "2024"`, `rust-version = "1.95.0"`, `publish = false`, with a git dependency (`algos`). And `bund2-stdlib` links a shell, filesystem walkers and OS randomness whatever its features. A process boundary crosses `smysl/1.0`, which both sides speak, and is the only place a wall clock and a sandbox can be applied without changing Bund2. |
| D-B2 | What keeps a script from the host | **The operating system** (§5). The word gate (§6) is a first check that names a mistake early, not a boundary. Until the sandbox exists on a platform, only `Trust::Trusted` runs there, and `Trust::Sandboxed` is refused with `SandboxUnavailable`. | A static allowlist or denylist of words as the boundary | Bund2 can assemble a call at run time: `make.call` / `call,` then `!` (`bund2-stdlib/src/values.rs:309-314, 760-763`); `resolve` (`singles.rs:1704-1730`); `bund.eval` and `!!`; `alias`; `kb.call`; and `compile lambda! !`, which runs source text even under `--noeval --noio` (`decisions.md:14244-14249, 14358-14366`). `"system." "shell" + call, !` names no forbidden token. |
| D-B3 | `--noio` | **Not passed.** | Running under `--noio` | `--noio` stubs 21 `smysl.*` words with `SM06`, every read-only one among them (`smysl/words.rs:740-761`); only `smysl.agent` and `smysl.parse` survive it. And it does not stub `time.*`, `sysinfo.*`, `input`, randomness or `exit`, so it would not be a sandbox even if the words survived. |
| D-B4 | How an answer leaves the process | **A Bund encoder the bridge appends** (§7), framed by a per-run sentinel and parsed by the bridge. | `json.from_value println` | It refuses strings. |
| D-B5 | How a live library is protected | **The process never sees it.** The bridge reads the library with this release's `smysl-text`, writes a fresh library in a private temporary directory, and points `bund2` there (§4). | Copying the library's files | A file copy carries what Bund2's 1.15 reader cannot open: a primary shard on redb is refused with `SM02` (`smysl-text` 1.15 `shards.rs:308-322`). Re-writing also drops lock files and index sidecars by construction instead of by an exclusion list. |
| D-B6 | Termination | **A wall clock enforced by the bridge**, which kills the process (`max_wall_ms`); and an inference cap for logic queries, through `kb.inferences`. | Relying on Bund2 | Bund2 has no step limit, fuel or timeout. Its only guard is an OOM flag (`bund2-api/src/memguard.rs`). `kb n kb.inferences` caps KB solving alone (`kb/store.rs:425-442`), and exceeding it is `E30`. |
| D-B7 | Version and publication | **The workspace version, published with the other crates** once the CLI uses it (BQ-P4). Not re-exported by the facade. Its API is frozen from its first publication, like every other crate's. | A separate `0.1.0` | `scripts/verify-dep-versions.py` requires every internal requirement to name the workspace version. And `smysl bund` in the CLI makes `smysl-bund` a dependency of the facade, so `smysl` cannot be published without it. |
| D-B8 | Name | **SMYSL-4.** Phases BQ-P1 to BQ-P4. | — | — |

## 2. What this does not change

- **Wire and kernel.** Nothing in `smysl/1.0`, the kernel schema or `sq/1` changes.
- **Purity.** No crate in `xtask check-purity`'s pure set depends on `smysl-bund`. `smysl-bund`
  is a boundary crate, like `smysl-provider`. It spawns a process, reads a clock and writes a
  temporary directory.
- **Sync first.** `smysl-bund` uses `std::process`, threads for its two pipe readers, and no
  async runtime.
- **Trust.** Nothing a script computes enters a store. A result is a value returned to the
  caller. A caller that wants to keep it writes it through the ordinary paths, and rule V1 and
  the trust ceiling apply there as everywhere else.

---

# Part A — Normative

## 3. Pipeline

```text
  (&Store | library root, script, &QueryPolicy)
      │
      ▼  §6  the word gate         size, nesting, and words named outright; a first check
      ▼  §4  the snapshot          a fresh library, written by this release, in a private TempDir
      ▼  §7  the wrapper           preamble + script + encoder, written to <tmp>/query.bund
      ▼  §5  the process           bund2, sandboxed (or trusted), env cleared, wall clock
      ▼  §7  the decoder           sentinel-framed answer → QueryValue; stderr → BundError
      ▼
  BundQueryResult
```

## 4. The snapshot

1. The bridge MUST create a private temporary directory per call and MUST remove it on every
   exit path, including a timeout and a panic.
2. **From a store** (`eval_store`, `eval_over_sq`): the bridge MUST create a library in the
   temporary directory with `Library::create` and append the store's records in store order with
   `Library::append_records`.
3. **From a library** (`eval_library`): the bridge MUST open the live library with this release's
   `Library::open`, which reads every shard including a primary shard on redb. It MUST then write
   the snapshot as in (2), and copy the object store's part texts and readings (`objects/t3`,
   `objects/r3`) for the parts the snapshot's manifests name, excluding redacted parts. It MUST NOT
   copy lock files, index sidecars or `series.redb`.
4. The snapshot's logs MUST be CBOR logs, never primary shards, because the `bund2` this RFC
   targets reads smysl through `smysl-text` 1.15 without `store-redb`.
5. The live library MUST NOT be opened for writing, locked, or passed to the process. (Opening it
   is subject to smysl's own rule: `Library::open` completes an interrupted redaction,
   `smysl-text/src/library.rs:660-700`. That is the same open every `smysl` command performs, and
   not something the bridge adds.)

The copy also protects against the 1.15 reader inside `bund2`. In 1.15, every read-only
`smysl.*` word opens through `Library::open`, which may unlink objects (`enforce_z`), and holds
no lock while it does (Bund2 `smysl/handle.rs:123-132`). Inside the snapshot, that is harmless.

## 5. The process

1. The bridge MUST start `bund2` with an absolute script path inside the snapshot directory, and
   MUST pass flags before the path. Everything after the path is the script's own arguments
   (Bund2 `bund2-cli/src/main.rs:550-560`).
2. The environment MUST be cleared and then set to exactly `LC_ALL=C`, `TZ=UTC`, `COLUMNS=80`,
   `NO_COLOR=1`, and `HOME` and `TMPDIR` pointing into the snapshot directory. The working
   directory MUST be the snapshot directory.
3. Standard output and standard error MUST each be read by a bounded reader. Past
   `max_output_bytes`, the process is killed and the call fails with `OutputTooLarge`.
4. After `max_wall_ms`, the process MUST be killed, along with its process group, and the call
   fails with `TimeoutExceeded`. No child process and no temporary directory may outlive the call.
5. **`Trust::Sandboxed`** MUST run the process so that it cannot:
   - open a network socket;
   - read a file outside the snapshot directory, the `bund2` binary and the system libraries it
     loads;
   - write a file outside the snapshot directory;
   - start a process other than itself.

   The mechanism is per platform. On macOS it is `sandbox-exec` with a generated profile. On
   Linux it is `bwrap` with every namespace unshared. On a platform without one, or where the
   tool is missing or cannot unshare its namespaces, `Trust::Sandboxed` MUST fail with
   `SandboxUnavailable` before any process starts.

   **As built (BQ-P3).**
   - **macOS.** The profile is `(deny default)`, with these allowed back:
     - exec of the `bund2` binary;
     - reads of the binary, the snapshot directory, `/usr/lib`, `/System/Library`, `/dev/null`
       and `/dev/urandom`;
     - metadata reads;
     - reads of `/` itself, without which `bund2` aborts at start. This shows the names at the
       root and nothing under them;
     - writes in the snapshot directory;
     - `sysctl-read`.

     A denied operation fails with `EPERM`. Bund2 reports this as `Operation not permitted`,
     which the bridge returns as `PolicyViolation` (§8).
   - **Linux.** `bwrap` binds read-only `/usr/lib`, `/usr/lib64`, `/lib`, `/lib64`,
     `/etc/ld.so.cache` and the binary. It binds the snapshot directory read-write and adds a
     fresh `/dev` and `/proc`. It also passes `--die-with-parent` and `--new-session`.
     - There is no `/bin` and no shell to start.
     - There is no network.
     - A path outside the snapshot does not exist, so it fails with `ENOENT` rather than a
       denial.
   - **Linux without Landlock.** The crate is `#![forbid(unsafe_code)]`, and namespaces with
     Landlock would need `pre_exec`. `bwrap` is the Linux mechanism, and a host without it, or
     whose user namespaces are restricted, gets `SandboxUnavailable`. Ubuntu 24.04 restricts
     them under AppArmor, so the CI job lifts the restriction.
6. **`Trust::Trusted`** runs without the sandbox, with everything else in this section. It is for
   scripts written by the person running them. It MUST NOT be the default for anything a model
   wrote. The CLI requires `--trusted` to select it (§10).

## 6. The word gate

The gate runs before the snapshot and costs nothing when it passes. It refuses a script early,
with the word named, so that a mistake is reported as a policy violation and not as a sandbox
denial three steps later. **It is not a boundary** (D-B2): a script that passes the gate is still
contained only by §5.

1. The gate MUST refuse a script over `max_script_bytes` or nested deeper than `max_nesting_depth`.
2. The gate MUST tokenise the script with string and comment literals respected, and refuse any
   token naming a word of §6.1, §6.2 or §6.3.
3. Under `WordPolicy::Allowlist`, the gate MUST also refuse any word not on the allowlist, and
   MUST refuse `smysl.open` outside the bridge's preamble.
4. A bare word directly inside `[ … ]` is data: Bund2 reads it as a STRING and does not run it.
   It is a clause's functor or atom, as in `[ intake W S ]`. The gate does not check it. A
   pointer or a lambda inside a list is checked as everywhere else. BQ-P1's gate checked these
   words, so it refused every KB script; BQ-P3 found and fixed this.

   **Known limit.** A KB bridge lambda reads clause variables as words (`[ test { T C > } ]`).
   The allowlist refuses those names, so such a script needs `WordPolicy::Denylist`.

### 6.1 Library writers and model words: always refused

| Words | Why |
|---|---|
| `smysl.create`, `smysl.unit`, `smysl.rel`, `smysl.label`, `smysl.append`, `smysl.text.add`, `smysl.retract`, `smysl.prove` | They write a library. `smysl.prove` writes its proofs as units (Bund2 `smysl/words.rs:617-638`); `smysl.kb` with `kb.prove` proves without writing. |
| `smysl.ingest`, `smysl.answer`, `smysl.ingest.text`, `smysl.commit` | They call a model, read provider configuration and keys, and in the last two cases write (`smysl-model` feature, `smysl/words.rs:650-729`). |

### 6.2 Words that run code by a name not in the script: always refused

`!`, `execute`, `execute.`, `!!`, `apply`, `lambda!`, `do`, `resolve`, `ptr`, `make.call`,
`call,`, `register`, `alias`, `alias=`, `unalias`, `unregister`, `compile`, `bund.eval`,
`bund.eval.`, `bund.eval-file`, `bund.eval-file.`, `use`, `use.`, `load`, `kb.call`, `kb.word`,
`kb.consult*`. Also `?`, `conditional` and `query`: a conditional's `type` may be `csv` or
`sqlite`, which read files, and the type is a STRING the script can build.

Refusing these makes the gate useful, not sound. A KB bridge or the conditional `smysl` word
still runs lambdas the script defines (Bund2 `smysl/query.rs:5-7`), and a lambda is ordinary code.

### 6.3 Host, clock and randomness: always refused

- Shell and processes: `system.shell`, `sh`, `sh.`, `system.setproctitle`, `vm.spawn`, `exit`,
  `bund.exit`.
- Files and fetching: `fs.*`, `fs_is_file.`, `rm`, `cp`, `mv`, `ls`, `ls.`, `cwd`, `filename`,
  `file`, `file.*`, `url`, `url.*`, `io.*`, `csv`, `csv.`, `sqlite`, `save`, `save.*`, `load.*`,
  `bootstrap`, `debug.profile`.
- Time, environment and host: `time.*`, `.timestamp`, `sleep`, `sleep.seconds`, `args`,
  `version`, `sysinfo.*`, `system.ip`, `system.ipv6`, `system.locale`, `debug.*`.
- Terminal and channels: `input*`, `password`, `display`, `io.banner`, `io.graph`,
  `bund.prompt`, `console.*`, `spinner.*`, `send`, `send.`, `send.quick`, `recv`, `bus.*`,
  `log.*`, and `json.path`, which prints as a side effect.
- Randomness and anything seeded from it: `rng.*`, `math.random.*`, `math.securerandom.*`,
  `string.random.*`, `id.uuid`, `id.ulid`, `dist.sample*`, `generator*`, `sim.gillespie`,
  `popgen.wf*`; and `fmt`, which reads the locale.

These lists are of Bund2 0.56.0 (Appendix A, item 7). Their authoritative form is
`crates/smysl-bund/src/words.rs`, read in BQ-P1 against `bund2 words` of a 0.56.0 build
(`crates/smysl-bund/words/bund2-0.56.0.txt`, 1,210 words). A test fails if a refused entry names
a word that build does not have. So the list cannot refuse a misspelling while the real word
stays allowed. Neither `stdin` nor a bare `call` exists in 0.56.0, though the first check named both. A
new Bund2 release is admitted only after its `bund2 words` is diffed against that file and read.

### 6.4 The default allowlist

`smysl.agent`, `smysl.parse`, `smysl.texts`, `smysl.check`, `smysl.write`, `smysl.get`,
`smysl.trace`, `smysl.status`, `smysl.salience`, `smysl.pack`, `smysl.dossier`, `smysl.measure`,
`smysl.query`, `smysl`, `smysl.kb`, and the RFC-0013 words `kb.new`, `kb!`, `kb.consult*`,
`kb.assert`, `kb.asserta`, `kb.retract`, `kb.merge`, `kb.clauses`, `kb.inferences`,
`kb.askable`, `kb.table`, `kb.query`, `kb.once`, `kb.holds`, `kb.prove`, `kb.saturate`, and the
CLP(FD) words. Also Bund2's stack, arithmetic, comparison, string, list, map and control words
that touch nothing outside the VM. In BQ-P1's form, the allowlist is every word of the 0.56.0
listing that §6.1 to §6.3 do not refuse, less `smysl.open`. A word a later Bund2 adds is not on
it.

`smysl.write` renders a library as surface text onto the stack; it writes no file
(`smysl/words.rs:394-408`). `smysl.measure` and `smysl.dossier` append nothing
(`smysl/research.rs:2, 313`). `kb.assert` and `kb.retract` change an in-memory KB, not a library.

## 7. The wrapper and the answer

The bridge writes `<tmp>/query.bund` as three parts:

1. **The preamble** binds the snapshot: `:lib "<snapshot root>" smysl.open var`, and for
   `eval_over_sq`, `:seed [ <uids> ] var`. The uids are written in their canonical 52-character
   form, since `Uid`'s `Display` writes the short form, which `Uid::parse` does not read. An empty
   answer is `:seed list var`, because Bund2 does not parse `[ ]`. A script reads them with `:lib var?` and `:seed var?`,
   Bund2's binding idiom (Bund2 `stdlib/src/check.rs:1496-1513`).
2. **The script**, verbatim.
3. **The encoder**: a line holding the opening sentinel, then `debug.display_stack`, then a
   line holding the closing sentinel. The sentinel is fresh for each call, and is drawn by the
   bridge, never by the script.

**Why `debug.display_stack` (decided in BQ-P1).** An encoder written in Bund cannot walk a map:
Bund2 0.56.0 has no word that lists a map's keys, `convert.to_list` of a MAP is `[]`, and `loop`
refuses one. `println` renders a string without quotes and a float lossily
(`0.30000000000000004` prints as `0.30000000000000007`). `debug.display_stack` instead prints
every stack value, bottom first, one row each, in Bund2's raw rendering
(`bund2-value/src/lib.rs`, `render`):

```text
Value { id: "…", stamp: <ms>, dt: <type>, q: <float>, data: <data>, attr: [ … ], curr: <n>, tags: { … } }
```

`data` is `I64(n)`, `F64(f)`, `Bool(b)`, `Null`, `String("…")`, `List([ … ])` or
`Map({"k": … })`. A string is Rust-escaped, so a row is one line whatever the string holds. A
float is `{:?}`, which reads back to the same bits. The rows do not wrap: at `COLUMNS=80`, on a
pipe, a 450-character row stays one line. `debug.display_stack` is refused to the script (§6.3);
the encoder is not the script.

The decoder:

1. MUST take the last row between the last pair of sentinel lines, which is the top of the
   stack, and ignore everything else the script printed;
2. MUST read past `id`, `stamp`, `dt`, `q`, `attr`, `curr` and `tags`. They are Bund2's
   bookkeeping, and `stamp` is a wall clock;
3. MUST produce a `QueryValue`, with map keys in `BTreeMap` order and a STRING that parses as a
   smysl uid (`b3:…`) returned as `QueryValue::Uid`. Any other `data`, a LAMBDA among them, is
   refused by name;
4. MUST, under `verify_uids`, fail with `NormalizationError` if a returned uid is not in the
   source store;
5. MUST, when the sentinels are missing, read the failure from the `Error: <message>` line that
   `--report trace` writes, and classify it (§8).

**What a run looks like from outside (BQ-P1).**
- A failing script exits 0. Bund2 sets an exit code only for `bund.exit` and for a failure to
  start (`bund2 help script`), so the exit status says nothing about the answer.
- The failure report goes to **standard output**, not standard error. The bridge runs with
  `--report trace --nocolor`, under which it begins `Error: <message>`.
- A script that ends normally prints nothing after itself; there is no stack dump to suppress.

Two runs over equal stores with equal scripts MUST return equal `QueryValue`s. `elapsed_ms` and
`warnings` are reported beside the value and are not part of that equality.

## 8. Errors

| Bund2 refusal | `BundError` |
|---|---|
| `SM00`, `SM03`, `SM05`, `SM11` | `InvalidScriptOrArgs` |
| `SM01`, `SM02`, `SM07`, `SM08`, `SM09`, `SM12` | `SmyslRefusal` |
| `SM04` | `SqQueryRefusal` |
| `SM06` | `ProcessFailed`; the bridge never passes `--noio`, so this is a defect |
| `SM10` | `LambdaFailure` |
| `SM13` | `PolicyViolation`; the model layer must never run in a query |
| `E30` (inference cap reached) | `InferenceCapReached` |
| `E50` (askable goal with no ask lambda) | `InvalidScriptOrArgs` |
| killed by the sandbox | `PolicyViolation`, with the denied operation when the platform names it |

The codes and their meanings are RFC-0014 §S8 (`RFC-0014-smysl-corpora.md:1805-1817`) and
RFC-0013 (`RFC-0013-logic-programming.md:876, 1083, 1091`).

**The code is read from the text (BQ-P1).** Bund2 keeps the `SMnn` code inside the process
(`stdlib/src/smysl/mod.rs`, `Refusal::for_word`). What it writes is `<WORD>: <text>`, where the
text follows §S8's template for the code: `no SMYSL library at` is SM01, `SMYSL refused it:`
SM02, `the query was refused:` SM04, `unknown <kind>:` SM05, and so on. The bridge recovers the
code from the template. SM03's template is the catch-all `<what> <reason>`, so a `smysl.*`
refusal matching no other template is SM03. An error from a word that is not `smysl.*` (an
unknown word, a type error) is `ScriptFailed`.

## 9. The library API

```rust
pub fn probe_bund_binary(explicit: Option<&Path>) -> Result<BundBinaryInfo, BundError>;
pub fn eval_store(store: &Store, script: &str, policy: &QueryPolicy) -> Result<BundQueryResult, BundError>;
pub fn eval_library(root: &Path, script: &str, policy: &QueryPolicy) -> Result<BundQueryResult, BundError>;
pub fn eval_over_sq(store: &Store, sq: &str, script: &str, policy: &QueryPolicy) -> Result<BundQueryResult, BundError>;
```

- `probe_bund_binary` reads `bund2 --version`, which prints `bund2 <version> (features: …)` and
  names `smysl` when it is compiled in (Bund2 `main.rs:56-58`, `help.rs:231`). It refuses a
  version below 0.56.0, and a binary without `smysl` with `BinaryMissingSmyslFeature`.
- `eval_over_sq` runs `sq` over the snapshot, opened as a library through `LibraryScope`, as
  `smysl q` does. A query then reads the same inside and outside Bund2 (`from library`, not
  `from store:…`).
- `QueryPolicy` holds the draft's fields (`bund_binary`, `word_policy`, `max_script_bytes`,
  `max_nesting_depth`, `max_output_bytes`, `max_wall_ms`, `verify_uids`) and adds `trust: Trust`
  and `max_inferences: Option<u64>`. When `max_inferences` is set, the preamble applies
  `kb.inferences` to every KB the script makes.

  **As built (BQ-P3).** A preamble cannot wrap `smysl.kb`, because a lambda that shadows a
  native word and calls it by name calls itself. So the bridge rewrites the script's text,
  using the gate's token offsets:
  - each `smysl.kb`, `kb!` and `kb.new` that runs where it stands is followed by
    `<cap> kb.inferences`;
  - each `kb.inferences` is preceded by `dup <cap> < { drop <cap> } if`, so a cap the script
    sets is lowered to the policy's.

  No line is added, so an error's line number is the script's. This is not a boundary, since
  a script can edit a KB's map by hand. The wall clock remains the bound. Each field
  has a `with_*` builder, since the struct is `#[non_exhaustive]`.
- `Trust` is `Sandboxed` (the default) or `Trusted`.
- `QueryValue`, `BundQueryResult` and `BundError` are as in the draft. `BundError` adds
  `InferenceCapReached`, `SandboxUnavailable`, `ScriptFailed` (a Bund2 error that is not a
  refusal) and `BinaryUnsupported` (a `bund2` older than 0.56.0, or a `--version` that cannot be
  read). Every public type is `#[non_exhaustive]`.
- Defaults: `max_script_bytes` 65,536; `max_nesting_depth` 32; `max_output_bytes` 1 MiB;
  `max_wall_ms` 2,000; `max_inferences` 1,000,000; `verify_uids` true;
  `word_policy` the allowlist of §6.4; `trust` `Sandboxed`.

## 10. CLI and TUI

Behind the facade's `bund` feature, off by default:

```sh
smysl bund --probe
smysl bund --library ./fenwick --script ./find_shaky.bund --format json
smysl bund -s ./notes.smy --sq 'from library where kind = claim' --eval '<script>' --timeout-ms 2000
```

- `--trusted` selects `Trust::Trusted`. Without it, a platform without the sandbox refuses the
  command and says so.
- Without `bund2`, or with one built without `smysl`, the command fails with a diagnostic naming
  `cargo build -p bund2-cli --features smysl`. No other command is affected.
- The output follows rule P: `--format json` or the surface form of the value, and nothing
  time-dependent on standard output.
- A TUI tab is optional and is not part of this RFC's exit criteria.

---

# Part B — Delivery

## 11. Crate

`crates/smysl-bund`: `version.workspace`, `edition.workspace`, `license.workspace` (MPL-2.0;
Bund2 is Apache-2.0 and is not linked). It depends on `smysl-core`, `smysl-graph`, `smysl-query`
and `smysl-text`, all with default features off, and adds `tempfile` as its only new dependency.
It compiles, and its unit tests pass, on a machine without `bund2`. It is listed in `UNPUBLISHED`
until BQ-P4, when it joins `PUBLISHED` with the release that first publishes it.

## 12. Phases

| Phase | Content | Exit criteria |
|---|---|---|
| **BQ-P1** spike and skeleton (**done**: findings in §7 and §8) | Against a local `bund2 --features smysl`: confirm the flags, that a script's stack is not dumped on stdout at exit (or the flag that stops it), the encoder's words (§7), and `bund2 words` → the lists of §6. Crate skeleton: `QueryPolicy`, the gate, `QueryValue`, the decoder, the error mapping. | The gate and the decoder unit-tested without `bund2`. The word lists regenerated and their diff against Appendix A recorded. |
| **BQ-P2** snapshot and runner (**done**) | §4 and §5 for `Trust::Trusted`: snapshot from a store and from a library (a redb library included), env, bounded readers, wall clock, process-group kill, cleanup. | Tests gated on `BUND2_BIN`. A timeout leaves no child and no directory. A live library's bytes, hashed before and after, are unchanged. |
| **BQ-P3** sandbox, determinism, and `max_inferences` (**done**) | §5.5 on macOS and on Linux; `SandboxUnavailable` elsewhere. Determinism replay over 20 or more scripts, including `smysl.kb` with `kb.query` and the CLP(FD) and `kb.inferences` examples of `bund_examples/smysl_demo_full/demo.bund`, over equal stores written in shuffled record order. | Adversarial suite: `"system." "shell" + call, !`, `compile`, a `url` fetch, a file read outside the snapshot, `time.now`, an infinite loop. Each is stopped by the sandbox, the gate or the clock, and each is named in the result. |
| **BQ-P4** CLI | `smysl bund` behind `bund`; `smysl-bund` joins `PUBLISHED`; the manual gains a section. | `cli-surface`, `doc-output` and the crate-features matrix cover it. Published with the release that ships it. |

## 13. Risks

| Risk | Mitigation |
|---|---|
| The sandbox profile is wrong on a platform | BQ-P3's adversarial suite runs on that platform in CI. Each case asserts the operation was denied, not merely that the script failed.<br><br>**As built.** The CI job `bund2-bridge` builds `bund2` at the pinned commit on Ubuntu and on macOS and runs the crate's tests with `SMYSL_BUND_REQUIRED`, which fails rather than skips when `bund2` or the sandbox is missing. The suite has two layers:<br>- **Through the gate** (`tests/run.rs`): each attack is refused with its word named, and the loop is stopped by the clock.<br>- **Past the gate** (`run::sandbox_tests`): each operation runs trusted and then sandboxed:<br>&nbsp;&nbsp;- a shell runs trusted and not sandboxed;<br>&nbsp;&nbsp;- a file outside is read trusted and not sandboxed;<br>&nbsp;&nbsp;- a file outside is written trusted and not sandboxed;<br>&nbsp;&nbsp;- a connection reaches a local listener trusted and not sandboxed. |
| Bund2 adds an impure word | The gate is not the boundary (D-B2). The word-list diff of §6.3 is read before a new Bund2 version is admitted by `probe_bund_binary`. |
| Bund2's pin moves past 1.15 | Nothing here depends on the pin. The snapshot is `smysl/1.0` and layout `smysl/library/2`, which 1.15 reads. A later reader reads more of it. |
| A 1.17 record that a 1.15 reader does not know | Read as Unknown or as an opaque extension unit. 1.17's timeline schema decodes as Unknown in 1.15 (`smysl-core` 1.15 `cbor/envelope.rs:1010, 1030`). BQ-P2 tests a library holding `x.tel/v1` units. |
| Process start cost | Accepted. Measured in BQ-P2 over F1 (21 records): 10.0 ms p50 a call, of which about 5 ms is the `bund2` process and the rest is writing the snapshot. Probing `bund2 --version` on every call made it 14.6 ms, so the probe is now cached per binary while its file is unchanged. BQ-P3, macOS, release build: 11.8 ms p50 trusted and 15.0 ms sandboxed, so `sandbox-exec` adds about 3 ms. Interactive queries stay with `sq/1`. |

## 14. Open questions

- **OQ-B1.** Does the snapshot need the object store at all? Only `smysl.texts` and the words that
  resolve a locator read part text. Copying objects only when the script names one of them would
  save the copy. Decided in BQ-P2 by measurement.
- **OQ-B2.** Should `Trust::Trusted` be refused when the script came from a model, by a flag on the
  call? The bridge cannot know where a script came from. The CLI can, when `ask` or an agent hands
  it a script. Decided before BQ-P4.

---

## Appendix A — What the check of the draft found

The draft (2026-11-20) was checked against smysl `fdd92ed` and Bund2 `development/0.56.0` at
`63fa39d`. Bund2 was read, not built or run.

| # | The draft said | What is true | Where this RFC changed |
|---|---|---|---|
| 1 | A static word gate, with an allowlist or denylist, enforces read-only, deterministic, host-free execution | A call can be assembled at run time (D-B2's row). Bund2's register says so (`decisions.md:14263-14267`). | §5 is the boundary; §6 is a first check |
| 2 | The epilogue emits the top of the stack as JSON | `json.from_value` refuses STRING, NODATA, and any container holding them (`bund2-value/src/lib.rs:997-1040`, `stdlib/src/json.rs:193-230`) | §7: a Bund encoder the bridge appends |
| 3 | Copying the library's files, minus locks, makes a safe snapshot | A copied primary shard on redb is refused by 1.15 (`SM02`) | §4: a snapshot written by this release, logs only |
| 4 | `smysl-bund` at `0.1.0`, excluded from the 1.x API | `verify-dep-versions.py` requires the workspace version, and a CLI command makes it a dependency of the published facade | D-B7 |
| 5 | `--noio` would stub only disk-reading words | It stubs 21 `smysl.*` words, every read-only one included (`smysl/words.rs:740-761`), and is not a sandbox | D-B3 |
| 6 | The denylist names `uuid`, `ulid`, `eval`, `import`, `csv.*`, `sqlite.*` | No such words. Bund2 has `id.uuid`, `id.ulid`, `bund.eval`/`!!`, `csv`/`csv.`, `sqlite`. `file` and `url` exist as bare words. | §6.3 |
| 7 | The denylist covers the impure words | It missed, among others: `sh`, `rm`, `cp`, `mv`, `ls`, `cwd`, `sleep`, `filename`, `save.*`, `load.*`, `bootstrap`, `time.*`, `args`, `exit`, `sysinfo.*`, `rng.*`, `dist.sample*`, `generator*`, `vm.spawn`, `send`, `recv`, `console.*`, `log.*`, `debug.*` (`stdlib/src/host.rs:2088-2172`, `world.rs:221-245`, `biomath/rng.rs`) | §6.3 |
| 8 | The demos open a library as `:lib "…" smysl.open var` | They create one: `:lib "mill" smysl.create var` (`bund_examples/smysl_demo/demo.bund:26`). Reading with `:lib var?` is right. `1000 kb.inferences` is in `smysl_demo_full/demo.bund:408`. | §7 keeps the idiom, and cites it correctly |
| 9 | Read-only words are read-only | They are, for the library's records. But the 1.15 open they go through can unlink redacted objects, holding no lock. | §4 says why the snapshot makes this harmless |
| 10 | Bund2 enforces termination | It has no step limit or timeout. `kb.inferences` caps KB solving only. | D-B6 |
| 11 | `bund2 <path> <flags>` | Flags after the path go to the script (`bund2-cli/src/main.rs:550-560`) | §5.1 |

Confirmed as the draft had it:
- the `=1.15.0` pins;
- the 28 `smysl.*` words, their names, and which of them write or call a model;
- SM00–SM13, with SM04, SM06, SM10 and SM13 as stated;
- the RFC-0013 words and CLP(FD), and `smysl.kb` as `handle scope → kb`;
- `bund2 --version` naming the `smysl` feature, and `bund2 words`;
- the layout marker that 1.15 and 1.18 share (`smysl/library/2`);
- the smysl API the draft named: `Store`, `Library::create`, `Library::open`,
  `Library::append_records`, `enforce_z`.

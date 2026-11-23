# RFC SMYSL-6 — an embedded language server (`smysl-lsp`, `smysl lsp-server`)

**Author:** Vladimir Ulogov
**Status:** draft, amended in review against the code (R1, 2026-11-23). LS-P1 and LS-P2 are
built (1.20); LS-P3 and LS-P4 are not.
**Target compatibility:** smysl 1.x, strictly additive; format `smysl/1.0`, unchanged.
**Crate:** `crates/smysl-lsp`, new, behind the facade's `lsp` feature.
**Distribution:** one binary. `smysl lsp-server` serves it.
**License:** MPL-2.0.
**Evidence baseline:** `dev/1.20.0` at `40cad29`. Every code pointer below was read there.

---

## 1. Summary and motivation

A `.smy` document is plain text that a person opens, reads, edits and saves without a dashboard.
Authoring or reviewing one, or writing an `sq/1` query, is still an edit → `smysl check` /
`smysl q` loop. A reviewer sees nothing inline: not a dangling reference (`SMY-E060`), not a
Rule M violation (`SMY-E030`), not a gist over its token budget (`SMY-E022`), not the grounds
behind a claim.

This RFC specifies **`smysl-lsp`**, a crate that puts the existing analysis (`smysl-core`'s
parser, `smysl-graph`'s store, `smysl-check`, `smysl-query`'s parser) behind the Language Server
Protocol 3.17, and **`smysl lsp-server`**, which serves it over stdio from the `smysl` binary.
One binary means the editor, the CLI and the checker cannot disagree about a version, and VS
Code, Neovim, Helix and Zed need only `smysl` on `PATH`.

## 2. Invariants

| | Requirement | How this RFC meets it |
|---|---|---|
| **G1. Semver and format** | `smysl/1.0`; `cargo-semver-checks` on every published crate | A new crate. Two fields added to `smysl_core::surface::ParseOutcome`, which is `#[non_exhaustive]` (§4.2). No wire, CBOR or surface change. |
| **G2. Purity** | Rule B (`xtask check-purity`): the pure crates link no runtime, socket or front end | No pure crate depends on `smysl-lsp`. `smysl-lsp` is not itself a pure crate: it reads stdin and writes stdout, as `smysl-bund` spawns a process. Its `analysis` module is pure, and `check-purity` enforces that with a module scan (§3.2). |
| **G3. Determinism** | Rule D: a pure operation is a function of its inputs | One document text and one workspace snapshot give the same diagnostics, hovers, completions and code actions, byte for byte. Everything is ordered by `BTreeMap` and by text position. |
| **G4. Epistemic safety** | A status is never raised to make a finding go away (the suggestion `SMY-E032` already prints: "never raise it") | The server never writes a store or a library and takes no lock. A code action may lower a unit's status to satisfy Rule M, and never raises one, deletes a rebuttal or resolves a contention (§5.6). |
| **G5. Library first** | Rule A: the CLI reaches the library only through the facade | `serve_stdio` and the pure `AnalysisHost` live in `smysl-lsp`, re-exported as `smysl::lsp` behind `lsp`, as `smysl::bund` is behind `bund`. `smysl lsp-server` is a thin dispatch (§6). |

## 3. The crate

### 3.1 Dependencies

```text
 smysl-core   smysl-graph   smysl-check   smysl-query
      └────────────┴─────────────┴─────────────┘
                          │
                  ┌───────▼────────┐
                  │   smysl-lsp    │  analysis (pure): line index, overlay, diagnostics,
                  │                │    hover, navigation, completion, formatting, actions
                  │                │  transport: JSON-RPC over stdio, debounce
                  └───────┬────────┘
                          │  `lsp` feature, re-exported as `smysl::lsp`
                  ┌───────▼────────┐
                  │  smysl (CLI)   │  `smysl lsp-server`
                  └────────────────┘
```

- `smysl-core`, `smysl-graph`, `smysl-check` and `smysl-query`, with `default-features = false`.
- **`serde_json`, and nothing else.** LSP is JSON-RPC 2.0 with a `Content-Length` header, and
  the server needs about a dozen of its messages. `lsp-types` (0.97) is large and breaks often
  (its URL type changed between 0.95 and 0.97), and `lsp-server` brings `crossbeam-channel`
  for what `std::sync::mpsc` does. Both would be dependencies to track for a protocol this
  small. `serde_json` is already in the workspace behind the readers' features, and the
  transport is `std` threads and channels. No async runtime (R1-5).

### 3.2 Two modules, one of them pure

- **`smysl_lsp::analysis`** is pure and synchronous: no `std::io`, `std::fs`, `std::net`,
  `std::thread`, `std::time`, `std::env`, `serde` or `serde_json`. It takes text and returns
  plain Rust values with positions already converted (§4.3).
- **`smysl_lsp::transport`** owns the protocol: framing, JSON, the reader thread, debouncing,
  the log file.
- **Enforced, not stated.** `check-purity` works per crate, and `smysl-lsp` cannot be a pure
  crate. So `xtask check-purity` gains a list of pure *modules*, with `smysl-lsp`'s
  `src/analysis/` its first entry, scanned for the symbols above (R1-4).

### 3.3 Wiring

- The facade gains `lsp = ["dep:smysl-lsp", "cli"]`, in `default`, and `pub use smysl_lsp as
  lsp` behind it. A `default-features = false` library build has none of it.
- `smysl-lsp` is unpublished until the release that ships it, in `UNPUBLISHED` beside where
  `smysl-bund` was.

## 4. Documents

### 4.1 Language ids

| id | extensions | analysis |
|---|---|---|
| `smysl` | `.smy`, `.smysl` | `smysl_core::surface::parse_surface`, a `Store` of its records, `smysl_check::check` |
| `smysl-query` | `.sq` | `smysl_query::parse`; `smysl_query::plan` against the workspace store, once there is one (LS-P4) |

### 4.2 Where a finding is in the text

`parse_surface`'s own diagnostics carry a byte `Span`. `check`'s name a unit
(`Subject::Unit`), a record (`Subject::Record`, the BLAKE3 of its encoding) or the store. The
parser knew where each record and each reference was, and kept none of it. 1.20 adds two
fields to `ParseOutcome` (R1-6):

- **`spans: Vec<Option<Span>>`**, parallel to `records`: each record's span, or `None` for a
  record with no single place (a label binding, a schema declaration, a manifest, the view);
- **`refs: Vec<(Span, Uid)>`**: every reference that resolved, with its span and target: a
  unit's `deps` and `grounds`, a relation's ends and note, a thread's steps, a commitment's
  target, the view's roots. A relation's ends now carry their own spans rather than the line's.

A unit finding goes to its record's span, a record finding to the record whose
`Store::record_hash` it names, and a store finding to the document's first line.

### 4.3 Positions

Spans are UTF-8 byte offsets. LSP 3.17 counts UTF-16 code units unless the client offers
`utf-8` in `general.positionEncodings`, in which case the server takes it. `analysis::LineIndex`
holds one document's line starts and converts both ways. A line ends at `\n`, `\r\n` or `\r`. A
position past a line's end clamps to it, a position inside a character clamps to its start,
and nothing panics on a buffer cut mid-character.

### 4.4 The overlay

Open documents live in `AnalysisHost`, a `BTreeMap` from URI to text and version, changed only by
`didOpen`, `didChange` (full or incremental) and `didClose`. Nothing is read from or written to
disk for an open document.

**The workspace (LS-P4)** is a read-only snapshot. `Library::open` is not read-only: it settles
migrations and runs `enforce_z`, which may unlink objects (`crates/smysl-text/src/library.rs`).
LS-P4 adds a read path that only reads the marker and the shard logs into a `Store`, and the
server uses that (R1-7).

## 5. What the server does

### 5.1 Diagnostics (`textDocument/publishDiagnostics`) — LS-P1

- **A `.smy` document.** Its parse diagnostics as they are. Then a `Store` of its records (with
  the workspace's, LS-P4) and `check` with the document's labels (`CheckOptions::with_labels`),
  the granularity taken from its view as `check` already does. Every finding keeps its code as
  the LSP `code` (`SMY-E030`), its message, and its suggestion appended. The severity maps
  error → 1, warning → 2, info → 3.
- **A `.sq` document.** `smysl_query::parse`'s diagnostic, if any. Planning (`SMY-E461` and the
  stage errors) needs an `Env` with a store, a scope and a reader, so it waits for LS-P4.
- **Debounced.** A burst of changes is checked once, `check_debounce_ms` (150) after the last.

### 5.2 Hover — LS-P2

On a unit's label, on a reference, or on a `b3:` uid in the text:

- the uid, schema, declared status and **effective status** (`smysl_graph`'s
  `effective_status`, which says `unfounded` after a retraction);
- the gist, and its tokens by `smysl_core::types::tokens` against the view's `l0_max`;
- its grounds and deps, how many units rebut it, and its source.

### 5.3 Navigation — LS-P2

- **Definition.** From a reference (`ParseOutcome::refs`) or a `b3:` uid, to the record that
  declares the unit.
- **References.** Every reference to the unit under the cursor.
- **Document symbols.** The `@doc` header, then each `@schema`, unit (its label or a short uid,
  with its gist), `@rel` and `@thread`, in text order. There is no `@view` record: the view is
  the `@doc` header (R1-3). A `@schema` declaration's span was not kept either, and LS-P2 adds
  it to `ParseOutcome::spans`.
- **What is under the cursor** is found in the parse, never by guessing at the text: a
  reference (`ParseOutcome::refs`), the label in a unit's own header, or a `b3:` uid written
  out, which hovers as "not declared in this document" when it is not. Nothing under the
  cursor answers `null`. A request without a position is `-32602 InvalidParams`.
- **Built in LS-P2,** and its gate met: `tests/cmd_lsp_server.rs` asserts the whole of each
  `hover`, `definition`, `references` and `documentSymbol` answer through the binary.

### 5.4 Completion — LS-P3

- **`.smy`.** After `@`: the kernel unit types and the declared extension schemas. Inside a
  header: its keys (`status`, `grounds`, `deps`, `source`, `lang`, `salience`). After
  `status:`: the five statuses an author may write (`unfounded` is reachable only by
  retraction, `SMY-E034`). Inside `grounds: [` and `deps: [`, and after `@rel`: the labels,
  each with its status and gist. Between `--` and `-->`: the fourteen kernel kinds and the
  declared `x.<domain>/<kind>`.
- **`.sq`.** The clauses, the filter fields, and the labels and schemas of the workspace.

### 5.5 Formatting — LS-P3

`write_surface` of the parse, only when nothing would be lost. That means no comments
(`ParseOutcome::comments`), no records skipped by recovery (`recovered`), and no error. Otherwise
the server answers with no edits and shows a message saying why. `.sq` is formatted by its
canonical printer.

### 5.6 Code actions — LS-P3

Allowed:

- **`SMY-E030`**: *lower the status to its weakest ground's*. It only ever lowers.
- **`SMY-E060`**: *replace with the nearest known label*, when one is within edit distance 2.
- **`SMY-W013`**, an unknown relation kind read as `elaborates`: *declare it*, by inserting
  `@schema x.<domain>/1 { version: 1, relations: ["x.<domain>/<kind>"] }`. The draft
  attached this action to "an undeclared extension schema"; `W013` is the relation kind
  (R1-1).

Forbidden, and tested as forbidden: raising any status, deleting or rewriting a `rebuts`
relation, writing a `@resolve`.

## 6. API and command

```rust
/// Configuration for `serve`. Built from `Default` and the `with_` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LspConfig {
    pub workspace_root: Option<PathBuf>,
    pub check_debounce_ms: u64, // 150
    pub log_file: Option<PathBuf>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum LspError {
    Transport(String),
    Io(std::io::Error),
}

/// Serve until the client's `exit`, over any byte streams.
pub fn serve(input: impl BufRead + Send + 'static, output: impl Write, config: LspConfig)
    -> Result<(), LspError>;

/// `serve` over stdin and stdout.
pub fn serve_stdio(config: LspConfig) -> Result<(), LspError>;
```

`serve` over arbitrary streams is what the tests drive, and `serve_stdio` is that over stdin and
stdout (R1-8). `LspConfig` is `#[non_exhaustive]`, so `with_workspace_root`,
`with_check_debounce_ms` and `with_log_file` build it.

```sh
smysl lsp-server [--stdio] [--library DIR] [--debounce-ms N] [--log-file PATH]
```

**stdout is the protocol's.** Everything else goes to stderr or `--log-file`. A build without
`lsp` answers `smysl lsp-server` with exit 2 and the feature to build with.

```toml
# Helix, languages.toml
[language-server.smysl]
command = "smysl"
args = ["lsp-server"]
```

## 7. Failure

| Concern | Answer |
|---|---|
| Keystroke latency | Changes are applied at once; analysis runs once a burst ends (§5.1). |
| A panic | Each request runs under `catch_unwind`; a panic answers `-32603 InternalError`, and the document's diagnostics are left as they were. |
| A comment lost by formatting | Refused whenever the parse dropped a comment or a record (§5.5). |
| An unknown method | `-32601 MethodNotFound` for a request; a notification is ignored, as the protocol says. |
| Shutdown | After `shutdown`, requests are `-32600 InvalidRequest`; `exit` ends with 0 after `shutdown`, 1 without. |

## 8. Phases

- **LS-P1 — the core and the command.** `crates/smysl-lsp` with `LineIndex`, the overlay,
  `AnalysisHost`, diagnostics for `.smy` and `.sq`, the transport, and `smysl lsp-server`. The
  parser's `spans` and `refs`. The module scan in `check-purity`. **Gate:** the purity and
  semver gates pass; UTF-16 and UTF-8 ranges are exact on CJK and emoji text; a stdio test drives
  `initialize` → `didOpen` → diagnostics → `shutdown` → `exit` through the real binary.
- **LS-P2 — hover and navigation.** **Gate:** the stdio test adds `hover`, `definition`,
  `references` and `documentSymbol`, and asserts each answer exactly.
- **LS-P3 — completion, formatting, code actions.** **Gate:** a property test over generated
  documents finds no action that raises a status, and no formatting that drops a comment.
- **LS-P4 — the workspace.** The read-only library read, cross-file resolution, `.sq` planning
  against it, and editor recipes in `Documentation/`. **Gate:** a library opened by the
  server is byte-identical afterwards, objects included.

## 9. Amendments

**R1, 2026-11-23: the draft read against the code.**

- **R1-1.** `SMY-W013` is "unknown relation kind treated as elaborates", not an undeclared
  extension schema. The `@schema` action declares the relation kind (§5.6).
- **R1-2.** The CLI crate is the workspace root's `Cargo.toml`, not `crates/smysl/Cargo.toml`.
  Documentation lives under `Documentation/`, not `docs/`.
- **R1-3.** The surface has no `@view` record; the view is the `@doc` header.
- **R1-4.** Purity is checked per crate, so "a pure module inside an impure crate" needed a
  module scan in `check-purity` to be true (§3.2). The draft's "Rule B, Guarantees A2–A3" are
  this gate. "Trust Ceiling" and "Principle P8" name nothing in this repository and are stated
  in words instead.
- **R1-5.** No `lsp-types`, `lsp-server`, `tower-lsp` or `tokio`: `serde_json` and `std` (§3.1).
- **R1-6.** Diagnostics from `check` carry no position. The parser now keeps each record's span and
  each reference's (§4.2). That is a public addition to `smysl-core`, in a `#[non_exhaustive]`
  struct.
- **R1-7.** `Library::open` writes (migrations, `enforce_z`). The workspace needs a read path
  that does not (§4.4).
- **R1-8.** The draft's "the facade never re-exports `smysl-lsp`" contradicts Rule A, which
  `check-purity` enforces: the CLI may only reach a crate through the facade. It is re-exported
  as `smysl::lsp`, behind its feature, as `smysl-bund` is. `serve`, over any streams, is added
  so the protocol can be tested without a process.

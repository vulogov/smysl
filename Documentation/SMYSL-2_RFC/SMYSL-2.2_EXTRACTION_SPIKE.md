# RFC SMYSL-2.2 — Extraction and proposition spike

**Status:** draft 1, **run in full on 2026-10-07**. Experiment protocol (non-normative); normative
rules are in SMYSL-2.3. The run covered 365 runs — the 335 of §3 plus the optional arm T, which
§3.3 gates on arm L being bit-identical at T=0 and which therefore qualified. Its decision table
(§3.7) returns *continue with a stated pivot*; the pivot is SMYSL-2.0 §1.1 and the results are in
`S0_RESULTS/`. The protocol was written against `d25ec9e`, before TX-P0, so several of its stated
facts no longer hold and six of its observations (O-1 to O-6) were already resolved when it ran;
`S0_RESULTS/S0_FINDINGS.md` records every departure, and `S0_REPORT.md` §5 the eight deviations
from the method as written.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** RFC SMYSL-2 draft 3, §7.4 (holder and mode), §7.7 (ingest over a text), §9
(proposition layer), §10.3 (estimator), §13.3 (`A1`, `A2`), §23 (GE-T2, GE-T3, GE-T4, GE-T5, GE-T9);
set decision D-12.
**Phases:** S0 (new, before TX-P1; may run in parallel with TX-P0)
**Depends on:** nothing new. Runs against `d25ec9e` as it is. Informs SMYSL-2.1 (TX-P0),
SMYSL-2.4 (TX-P5, TX-P7) and SMYSL-2.6 (TX-P8/P9 defaults).

## 0. Summary

Draft 3 rests on two assumptions nothing has measured: that model extraction is **stable**
enough for corpus-level numbers (GE-T2), and that a **proposition layer** of judged same-as edges
can be trusted (GE-T5). As planned, both are tested at TX-P5 and TX-P7, after the library, readers,
time engine and substrate index are built. If either fails, most of that work serves a pipeline
whose numbers do not mean anything.

This spike tests both first, cheaply, with **today's CLI** and throwaway scripts kept outside the
repository:
- 10 passages in six genres, en + ru, and three of them also in es, fr and de: 29 input files.
- 2 models (one local through Ollama, one hosted) × 5 runs × 29 inputs, through `smysl ingest`.
- Uid-set Jaccard within and between models.
- A human-judged same-as sample (two annotators, Krippendorff's α), which gives class-level
  agreement and a precision/recall pilot for the proposition layer without the layer existing.
- Attribution rates per language under normaliser V1, holder/mode errors on known
  reported-speech verses, and token cost per chapter per language.

The output is a short report, data files, and a filled decision table. Each outcome selects one of
four plan states: **continue as designed**, **pivot to consensus extraction (`attested:2`)**,
**scope narrative out**, or **reorder phases**. The spike's data then becomes fixtures for TX-P0,
TX-P1, TX-P5 and TX-P7 (§5).

Effort: about 7 working days, two annotators for about one day each, under 1.5M model tokens,
of which about 0.6M are hosted.

## 1. Scope

**In:**
- GE-T2 pilot (stability), GE-T5 pilot (same-as precision/recall), GE-T9 pilot (human ceiling,
  en+ru), GE-T3 pilot (language neutrality on the three five-language inputs).
- Two effects that are already known and are measured here, not fixed: F-3 (quote normalisation)
  and F-2 (byte-based token counts, which interact with `SMY-E022`).
- Reported-as-asserted errors (§7.4) under the shipped prompt, and under a one-paragraph prompt
  override (arm H).
- Token cost per chapter, per language and per model.

**Out:**
- Any change to the repository. Every script lives in `spike/`, outside `/tmp/smysl`.
- Structure-aligned windows, owned-range spans, ledger digests, `x.text/v1`, normaliser V2 and
  the script-aware estimator. These do not exist yet, and the spike measures the tree without them.
- Temperatures above 0. The CLI cannot set one (O-5). Arm T is optional and needs a local patch
  (§3.3).
- Full GE-T2/GE-T5/GE-T9 runs. These still run at their phases. The spike decides whether those
  phases are worth building in the order planned.

## 2. What exists today (verified against `d25ec9e`)

### 2.1 Facts the protocol depends on

| # | fact | where |
|---|---|---|
| E-1 | `smysl ingest [FILE]` flags: `--path auto\|surface\|json-ast`, `--granularity`, `--repair N` (default 2), `--max-output`, `--rung` (default `document`), `--prompt FILE`, `--dry-run`, `--yes`, `--hop`. There is no `--model` and no `--temperature`. | `cli()` and `cmd_ingest`, `src/main.rs` |
| E-2 | The provider config is always `<dir of --store, else cwd>/.smysl/config.hjson`. When the file is missing, the default is one local Ollama, `llama3.2`. | `load_config`, `project_file`, `root_beside` (`src/main.rs`); `Config::load`, `local_default` (`smysl-provider/src/config.rs`) |
| E-3 | Config keys per provider: `kind`, `endpoint`, `model`, `context_window`, `max_output`, `structured`, `api_key_env`/`api_key_cmd`, `timeout_secs`. Top-level keys: `routing` (task names `content-ingest`, `attest`, …), `fallback`, `ingest: {prompt, path}`. A literal key field is a hard error. | `Config::load` |
| E-4 | A named input file becomes each unit's source, `SourceRef(File, <path as typed>)` with `SourcePolicy::FillMissing`. The model then gets the "sourced" template `ingest.content.surface.sourced` v2. **The path string is in every uid.** | `cmd_ingest`; `Ingestor::template_for` (`smysl-ingest/src/lib.rs`) |
| E-5 | Recipe = BLAKE3(template id, version, provider id, model, granularity, quantised temperature, schema set, path, source). The CLI's model is the empty string. | `Conditions::recipe` (`smysl-ingest/src/recipe.rs`); `Ingestor::ingest` |
| E-6 | Temperature is `0.0` on every CLI call. | `IngestOptions::default`, `Request::new` (`smysl-provider/src/lib.rs`) |
| E-7 | Chunking: budget = `context_window − 1024 − max(max_output, context_window/2)` tokens, using `ceil(bytes/4)`. Splits fall at blank-line paragraphs, with one paragraph of overlap. | `Window::for_context`, `chunk` (`smysl-ingest/src/chunk.rs`) |
| E-8 | `--path auto` picks surface or json-ast from the provider's structured mode **and the input size**. The path is part of the recipe. | `path::choose` (`smysl-ingest/src/path.rs`) |
| E-9 | Each unit's `"ingest:quote"` is checked against the **chunk text** with normaliser V1. `Absent` gives `SMY-E307`, which is repaired. `Loose` gives `SMY-W308`, which passes. Only `SMY-E022` is unit-local. Any other unrepaired error degrades the **whole chunk** to one `@prose` unit carrying `"ingest:unrepaired": true` (`SMY-W304`). | `one_chunk`; `quote::verify` (`smysl-core/src/quote.rs`); `UNIT_LOCAL`, `degrade`, `salvage` (`smysl-ingest/src/repair.rs`) |
| E-10 | V1 folds case, whitespace, `' ‘ ’ ‛ " “ ” ‟` and dashes. It does not fold `« » „ ‚ ‹ ›` or `ё`. Reproduced in the spike harness: a quote `…в сердце своем` against source `…в сердце своём` is **`Absent`**. | `normalise`, `support` (`quote.rs`) |
| E-11 | Output: `.smysl/staged.smy` (surface) and `.smysl/staged.cbor`, overwritten on every run. Exit code 10 (`Staged`) or 11 (`StagedWithCorrections`). stderr carries `N chunk(s), N call(s), N unit(s), N weakened, N degraded, N token(s)` plus each diagnostic. | `cmd_ingest`; `stage::write` (`smysl-ingest/src/stage.rs`); `smysl-core/src/error.rs` |
| E-12 | `.smysl/usage.log` gets one JSON line per run: `in`, `out`, `estimated`, `retries`, `recipe`, with `"model":""`. | `LedgerEntry::to_line` (`smysl-provider/src/usage.rs`) |
| E-13 | `smysl attest --what gist-coverage\|warrant-plausibility\|granularity --sample N\|all <store>` prints YES/NO per unit. It does not write attestations. | `cmd_attest`; `What` (`smysl-ingest/src/attest.rs`) |
| E-14 | `smysl find QUERY STORE --engine lexical\|semantic\|hybrid --model DIR -n K --source PREFIX --json` prints `{"hits":[{uid, score, terms}]}`. Lexical search has no stemming: `Бога` does not match `Бог`. | `cmd_find` (`src/main.rs`) |
| E-15 | `smysl merge a.smy b.smy … -o pooled.cbor` pools runs. Labels reused across runs show up as `label-collision` contentions, which are harmless here. | reproduced |
| E-16 | The facade exports `parse_surface`, `canonical_uid`, `payload_strings`, `quote_support_span` and `QUOTE_KEY`. A 20-line harness gives exact V1 verdicts and byte spans (§4.2, compiled and run). `Uid`'s `Display` is the short form; use `Uid::canonical()`. | `src/lib.rs`; `smysl-core/src/ids.rs` |
| E-17 | `smysl-eval` has no Jaccard or α code to reuse. `tests/quoting_live.rs` sets the precedent of the **range test**: a between-arm difference counts only when it falls outside each arm's run-to-run range. | `crates/smysl-eval` |
| E-18 | `fixtures/embed-tiny` is a 50-word English toy vocabulary. `find --engine semantic` loads it and ranks, but it cannot embed Russian or the Bible. | reproduced |

Build (verified, 6 min 34 s on 2 cores):

```sh
cargo build --release --no-default-features --features cli,local,remote,semantic
```

- `remote` pulls `rustls` → `ring`, which needs a C compiler.
- `semantic` pulls `model2vec-rs 0.2.1` → `tokenizers` → `esaxx-rs`, which needs a C++ compiler
  (as `smysl-embed/Cargo.toml` says).
- The existing `target/release/smysl` (`cli` only) refuses `ingest`. The refusal message names
  `--features local`.

### 2.2 Observations for SMYSL-2.1

All are verified in code. O-2, O-3 and O-4 were reproduced end to end against a fake loopback
Ollama that logged requests.

| # | observation | effect on the spike |
|---|---|---|
| O-1 | `-C/--config` is declared as a global flag and never read. | One working directory per run (§3.4). |
| O-2 | The recipe hashes the provider id and an empty model, so changing `model:` under the same provider id keeps the recipe. | One provider id per model (OQ-33). |
| O-3 | The Ollama mapper sends only `temperature` and `num_predict`, never `num_ctx`. `context_window` steers chunking, while the server keeps its default context and may truncate silently. | Start the server with `OLLAMA_CONTEXT_LENGTH=32768` (unverified for the operator's Ollama version), or use a Modelfile with `PARAMETER num_ctx 32768`. Check `prompt_eval_count` in the proxy log. |
| O-4 | When a repair succeeds, the diagnostics of the earlier attempts are dropped, so a fabricated or falsely `Absent` quote that was repaired leaves no trace except `calls > chunks`. | A logging proxy for the local arm (§3.4). |
| O-5 | No CLI temperature. | T = 0 only (OQ-32). |
| O-6 | The help for `--yes` says "Commit the staged batch", but `--yes` only changes the exit code. Committing is `merge --staged`. | The spike never commits, and reads `staged.smy` directly. |

## 3. Design

### 3.1 Questions

| q | question | pilot of | main measure |
|---|---|---|---|
| Q1 | Does one model, rerun, extract the same propositions? | GE-T2 | `J_class` within model (M2) |
| Q2 | Do two models extract the same propositions? | GE-T2 | `J_class` between models |
| Q3 | Can same-as be judged reliably by people, and approximated by cheap candidates plus a threshold? | GE-T9, GE-T5 | α; precision at recall 0.7 |
| Q4 | How often is a reported proposition extracted as asserted? | §7.4 | RA rate (M4) |
| Q5 | How fair is attribution across languages under V1? | F-3, GE-T4 | false-`Absent` rate (M3) |
| Q6 | What does extraction cost per chapter and language, and does density depend on language? | §7.7 cost model, FC-6, GE-T3 | M5, M6 |

### 3.2 Corpus

Ten passages: two each of law, narrative, poetry and epistle, plus one essay and one chat. ★
marks the three inputs also run in es, fr and de. The alignment between them is free (§9.2).

| id | passage | genre | en | ru | es/fr/de | why |
|---|---|---|---|---|---|---|
| P1 ★ | Exodus 20 (26 v.) | law | KJV 1769 | Synodal 1876 | RV1909, LSG1910, Luther 1912 | imperatives; the commandments |
| P2 | Leviticus 19 (37 v.) | law | KJV | Synodal | — | dense casuistic law |
| P3 ★ | Genesis 4 (26 v.) | narrative | KJV | Synodal | RV1909, LSG1910, Luther 1912 | Cain's lie (4:9); events |
| P4 | 1 Kings 3 (28 v.) | narrative | KJV | Synodal | — | two women's contradictory claims (3:22–23) |
| P5 ★ | Psalm 14 (Synodal 13) (7 v.) | poetry | KJV | Synodal | RV1909, LSG1910, Luther 1912 | Ps 14:1, the standing reported-speech case |
| P6 | Ecclesiastes 1 (18 v.) | poetry/wisdom | KJV | Synodal | — | persona voice ("saith the Preacher") |
| P7 | James 2 (26 v.) | epistle | KJV | Synodal | — | argument against an imagined interlocutor (2:14, 2:18) |
| P8 | Romans 3 (31 v.) | epistle | KJV | Synodal | — | slander reported (3:8), chains of citation (3:10–18) |
| P9 | Essay, about 1,500 words | article | A. Maude tr., 1899 | original, 1897 | — | Tolstoy, *What Is Art?* ch. 3, opening: definitions of beauty reported, then rejected |
| P10 | Chat, 40 messages | chat | original | human translation | — | planted reported speech, a question and its answer, a retraction |

- **Sources** (identifiers unverified, confirm at download):
  - Bibles from eBible.org public-domain modules: `eng-kjv`, `russyn`, `spaRV1909`, `fraLSG`,
    `deu1912`.
  - The essay from Project Gutenberg, Russian and English editions.
  - P10 is written for the spike by the owner, released CC0, and translated by a person rather
    than a model, so that no hidden translation channel enters the corpus (§7.7 rule 5).
  - No PD text gives a *modern* article in both languages. P10 supplies the modern register.
- **Licences:** every input is public domain or CC0, so all of it can become a fixture (§5).

**Preparation** (`spike/prep.py`, one file per passage and language, written once and frozen):
- File name: `in/<lang>-<edition>-<Book>.<ch>.txt`, for example `in/ru-syn1876-Ps.13.txt`.
- UTF-8, NFC, LF, no BOM, final newline. **One verse per line, no blank lines**, so the file is
  one paragraph and one chunk (E-7). Each line is `<Book>.<ch>.<v> <text>`. The essay uses
  `Art.3.p<NN>`. The chat uses `Chat.m<NN> <speaker>: <text>`.
- Strip footnotes, Strong's numbers, italics markup and headings. The Synodal superscription
  stays inside verse 1, as printed.
- **Keep the edition's orthography exactly**. Record the count of `ё` per Russian file, because F-3
  depends on it (OQ-3).
- Assert that no file contains `<<<SMYSL-INPUT>>>` (the prompt fence).
- `in/manifest.tsv`: file, lang, edition, source URL, sha256, bytes, words, lines, `ё` count, and
  `smysl ingest --dry-run` tokens (`ceil(bytes/4)`, E-11).
- `in/align.tsv`: one row per aligned verse, for example `ru-syn1876 Ps.13.1 ⇄ en-kjv1769 Ps.14.1`.
  Written by hand. Equal verse counts per chapter are checked before use (Synodal Exodus 20
  numbering unverified).

Size: about 7.6k English words, and about 20k words over all 29 files. The largest Russian file
(Lev 19) is estimated at 16 KB, which is ≈ 4k estimator tokens and well inside one chunk (§3.3).

### 3.3 Arms

| arm | model | runs × inputs | purpose |
|---|---|---|---|
| **L** (local) | Ollama, proposed `qwen2.5:14b-instruct` (multilingual; tag unverified), provider id `spike-local-qwen14` | 5 × 29 | Q1–Q6. Raw answers logged through the proxy. |
| **R** (remote) | one hosted mapper, proposed `deepseek` / `deepseek-chat` (the model used by `smysl-eval` `prose_live.rs`), provider id `spike-remote-ds` | 5 × 29 | Q1–Q6, second model |
| **H** (holder prompt) | as L, with `--prompt spike/prompts/holder.hjson` | 1 × 29 | Q4: does wording alone fix reported speech? |
| **P** (packed) | as L, input = previous chapter + target chapter in one file | 1 × 16 (P1–P8, en+ru) | sensitivity to window context, as TX-P5's packed windows will have |
| T (optional) | as L, T = 0.7 through a local patch (one line in `cmd_ingest`, setting `opts.temperature` from `SPIKE_TEMPERATURE`, built in a scratch worktree and never committed) | 5 × 6 | run only if L is bit-identical across runs; it gives a sensitivity figure. The recipe hashes temperature, so T units are distinguishable. |

Pinned for all arms:
- `--path surface`, because `auto` would vary with input size (E-8).
- No `--granularity`, so the recipe says `standard` and the profile is `default`
  (`l0_max` = 30 estimator tokens = 120 bytes).
- Default `--repair 2`; rung `document`; the built-in sourced template (except arm H).
- `context_window: 32768`, `max_output: 8192` in **both** providers, so the windows are identical.
  The budget is 15,360 estimator tokens, which gives one chunk for every input. A run reporting
  more than one chunk is flagged.
- T = 0 (E-6).
- Arm order is interleaved: run 1 of every input, then run 2, and so on. Time-of-day effects on a
  hosted model then fall within runs, not between inputs.

Arm H's override replaces the system prompt as a whole (`PromptOverride`, `prompt.rs`). So
`holder.system.txt` is the text of `content_ingest_surface_sourced` v2 copied verbatim, plus one
paragraph:

> When the document reports what someone says, thinks or asks, the gist names who: write "The
> fool says there is no God", never "There is no God". A command stays a command.

The override id is `spike.holder`, version 1. The `ingest.` prefix is reserved.

### 3.4 Run procedure

Layout: `spike/runs/<arm>/<input>/r<k>/`. Each run directory holds `.smysl/config.hjson` and a
symlink `in -> ../../../../in`. Every unit's source is therefore the same string, `in/<file>`, in
every arm, and uids are comparable across runs and models (E-4).

`spike/config/local.hjson`:

```hjson
{
  providers: {
    spike-local-qwen14: {
      kind: ollama
      endpoint: "http://127.0.0.1:11435"
      model: "qwen2.5:14b-instruct"
      context_window: 32768
      max_output: 8192
      structured: json-schema
      timeout_secs: 900
    }
  }
  routing: { content-ingest: spike-local-qwen14, attest: spike-local-qwen14 }
  fallback: [spike-local-qwen14]
}
```

- Port 11435 is `spike/proxy.py`, which forwards to Ollama on 11434. It appends
  `{arm, input, run, attempt, request, response, prompt_eval_count, eval_count, ms}` to
  `raw/<arm>.jsonl`.
- The mapper's calls are non-streaming `POST /api/chat` (`OllamaProvider::complete`), so the
  proxy is about 40 lines.
- The endpoint is loopback, so `is_local()` holds and `--offline` still guards against egress.
- The design was checked against a fake server in exactly this position: two calls, a ё-repair,
  exit 10, and a usage line with `"model":""`.

`spike/config/remote.hjson` has the same shape with:
- `kind: deepseek`
- `endpoint: "https://api.deepseek.com"` (unverified base)
- `model: "deepseek-chat"`
- `api_key_env: DEEPSEEK_API_KEY`
- `structured: json-mode`

There is no proxy for arm R. Routing a hosted call through loopback would make `is_local()`
report it as local, so arm R's raw answers are not captured (§7).

Per run (`spike/run.sh`):

```sh
cd spike/runs/$ARM/$INPUT/r$K
$SMYSL $OFFLINE --noprogress ingest --path surface $PROMPT in/$INPUT.txt >stdout 2>stderr
echo $? >exit                       # expected 10 or 11
cp .smysl/staged.smy .smysl/staged.cbor .smysl/usage.log .
```

- `$OFFLINE` is `--offline` for L, H, P and T, and empty for R.
- Preflight, once per arm:
  - `smysl --offline providers --probe` (L only; loopback).
  - `smysl ingest --dry-run --path surface in/<largest>.txt`. It reports provider, egress, path,
    prompt, source and input tokens without calling anything.

### 3.5 Measurements

Each run's `staged.smy` goes through `qcheck` (§4.2) against its input file, giving `units.tsv`:
canonical uid, type, status, gist, quote, V1 verdict, byte span, and the locator of the line the
span starts in. Degraded `@prose` units (`ingest:unrepaired`) are counted and excluded from every
set.

**M1 — uid stability.**
- For each (model, input), Jaccard over the 10 run pairs; between models, the 25 cross pairs.
- `J_uid` uses canonical uids.
- `J_gist` uses the key (type, NFC + casefolded + whitespace-collapsed gist). The gap between the
  two is churn in status and in `ingest:quote`, which sits in the payload and therefore in identity.
- Reported as median and range per input, then per genre and language. Unit counts are reported
  the same way.

**M2 — classes without the layer.**
- *Pool:* every distinct unit of the 10 runs of one input. The units are pooled with
  `smysl merge … -o pool.cbor` (E-15).
- *Candidates:* each unit, against the pool, from four sources:
  - C0, identical `J_gist` key: auto-same, rung `computed`.
  - C1, `smysl find --json -n 5 --source in/<file> "<gist>" pool.cbor` (lexical, same language).
  - C2, the same with `--engine semantic --model $POTION`, where `$POTION` is the operator's copy
    of `minishlab/potion-multilingual-128M`. Loading it through `model2vec-rs 0.2` was
    **unverified** (F-11); **the run verified it** — the model loads and ranks, and the Python
    fallback was not needed. It does bridge en to ru, but at a markedly lower score than within
    one language, so a single global threshold does not behave the same both ways.
  - C3, anchored: both quotes located in the same or adjacent verse (spans from `qcheck`).
- *Cross-lingual* (P1, P3, P5): pairs en↔ru, en↔es, en↔fr and en↔de from C2 (with
  `--source in/<other file>`) and C3 (through `align.tsv`).
- *Control:* 100 random non-candidate pairs per language pair. These bound how many true pairs the
  candidates miss.
- *Edges:* adjudicated human `same` labels (§3.6), plus C0.
- *Classes:* `strict` policy, draft 3 §9.1 and D-7 — maximal cliques, built greedily in canonical
  uid order. `component` is also reported, with its diameter.
- `J_class(r1, r2) = |C(r1) ∩ C(r2)| / |C(r1) ∪ C(r2)|`, where `C(r)` is the set of class ids
  (smallest member uid) touched by run r. Units with no edge are singleton classes.
- Judging covers six pools per language: P1, P3, P5, P7, P9 and P10, in en and ru. The other four
  passages report M1 only.

**M3 — attribution under V1.**
- (a) Final units, both arms: shares of `Present`, `Loose` and no quote per language. Survivors are
  `Absent`-free by construction (E-9).
- (b) Every quote in every attempt, arm L, from the proxy log, under V1 and under **V2-emu**. V2-emu
  pre-folds both strings — `ё→е`, `« » „ ‚ ‹ ›`→`"`, `¿ ¡` deleted, `ß→ss`, `ʼ ′`→`'` — and then
  applies V1. It is an emulation for estimating F-3, **not** FC-7.
- False `Absent` = V1 `Absent` ∧ V2-emu not `Absent`.
- (c) Up to 50 V1-`Absent` quotes per language, labelled by A1:
  - normalisation
  - quote translated into another language (F-6)
  - quote joined across verses
  - paraphrase
  - fabrication
- Also reported:
  - repair rate, (calls − chunks) / chunks, from stderr
  - degraded chunks (`W304`)
  - `E022` unit-local degradations per language (F-2)
  - gists in a script other than the source's (F-6; for ru by script, for es/fr/de by a stopword
    test plus a spot check of 20)

**M4 — holder and mode.** Probe verses, all languages where present:

| probe | locator | reported content | holder |
|---|---|---|---|
| H1 | Ps 14:1 (Syn 13:1) | "There is no God" | the fool |
| H2 | Gen 4:9 | "I know not" (a lie) | Cain |
| H3 | Gen 4:13–14 | "my punishment is greater than I can bear" | Cain |
| H4 | 1 Kgs 3:22 | "the living is my son" (both women, contradictory) | each woman |
| H5 | 1 Kgs 3:26 | "divide it" | the second woman |
| H6 | Rom 3:8 | "Let us do evil, that good may come" | slanderers |
| H7 | Jas 2:14, 2:18 | "he hath faith" / "Thou hast faith" | an imagined speaker |
| H8 | Eccl 1:2 | "Vanity of vanities" | the Preacher (persona; reported separately as ambiguous) |
| H9 | Ex 20:13–17 | commandments | text, mode imperative (a mode error if stated as fact) |
| H10–H12 | Art.3 | three definitions of beauty the author rejects | named aestheticians |
| H13–H15 | Chat | "Bob says the deploy is safe"; a retracted claim; a question | speakers |

- Every unit whose span starts in a probe line is labelled by both annotators:
  - **RA**: reported content stated as the text's own claim
  - **RC**: attributed correctly in the gist
  - **RN**: stance noted
  - **MI**: a command stated as fact
  - **X**: other
- Not extracted: tallied per probe and run.
- `RA rate = RA / (RA + RC + RN)` per arm and language. α is computed on these labels.

**M5 — cost.**
- Per run: `in`, `out` and `calls` from `usage.log` and stderr. Per attempt, Ollama's
  `prompt_eval_count` and `eval_count` from the proxy.
- Reported per input as tokens per 1,000 words and per verse, for each model and language. The
  ratio ru/en from the model tokenizer is set beside the ratio from `ceil(bytes/4)`. The pair is
  calibration data for FC-6.
- Measured cost is set beside §7.7's planning figures.

**M6 — density by language (GE-T3 pilot).**
- `A1` = units per 1,000 source words, by type, for P1, P3 and P5 in five languages.
- Between-language variance of the per-language means is compared with the within-language,
  run-to-run variance, as GE-T3 frames it.

**M7 (optional).** `smysl attest --what granularity --sample 30 runs/L/<input>/r1/staged.smy`
on the six judged inputs. The share of `NO` estimates how many multi-assertion units there are,
which depress `J_class`.

### 3.6 Annotation protocol

- **Annotators:**
  - A1: the owner, Vladimir Ulogov (en, ru).
  - A2: a second en/ru reader, chosen before the runs. A2 has not seen the prompts or model
    identities.
  - es/fr/de cross-lingual pairs: a single annotator, A1 or an A3, reported without α.
- **Blinding:** pair order and side are shuffled. Model, arm, run and status are hidden. Each unit
  is shown as gist, quote and locator, with the verse text on demand.
- **Same-as rule** (from §9.1):
  - *same*: the two gists state one proposition, holder and mode included. They are true or false
    in the same circumstances.
  - A reported claim and the same content asserted are **not** same.
  - A conjunction and one of its conjuncts give *entails* (with direction), not same.
  - Translation equivalents are same.
- **Labels:** `same`, `entails→`, `←entails`, `related`, `different`, `unclear`.
- **Sheet** (`judge/sheet.tsv`): `pair_id, input, lang_a, lang_b, gist_a, quote_a, loc_a, gist_b,
  quote_b, loc_b, label, note, annotator, seconds`.
- **Calibration:**
  1. Both annotate 50 pairs that are not used in measurement.
  2. They discuss and amend the guideline once.
  3. Then the measured set: A1 and A2 each judge **all** en, ru and en↔ru pairs. The estimate is
     about 1,800 pairs at about 8 s each, ≈ 4 h per annotator, plus holder labels (≈ 300 units,
     ≈ 1 h).
- **Reliability:**
  - Krippendorff's α, nominal, two coders, on the binary `same`/not, on the six labels, and on the
    M4 labels.
  - 95% CI by 1,000 bootstrap resamples over pairs.
  - The minimum is 300 doubly-coded pairs. Fewer leave the CI too wide to compare with the 0.667
    threshold (estimate).
- **Adjudication:** A1 resolves disagreements **after** α is computed. Gold is the adjudicated
  label. α is the ceiling every model figure is reported against (GE-T9).

### 3.7 Decision table

- Thresholds mirror draft 3 §23.
- Point estimates decide. A 95% CI that straddles a threshold is reported as **inconclusive**, and
  the full GE at its phase decides.
- The spike never kills a phase on its own: it reorders work or changes defaults.

| # | measure | threshold | outcome → plan change |
|---|---|---|---|
| T1 | α (`same`/not), en and ru | < 0.667 after one guideline revision | Gold unreliable. GE-T5 and GE-T2 thresholds are restated relative to α (GE-T9), and SMYSL-2.4 TX-P7's exit test changes before TX-P7 starts. |
| T2 | `J_class` within model, median over judged inputs | ≥ 0.6 for both models | **Continue as designed** (GE-T2 expected to pass). |
| T3 | same | < 0.6 for either model | **Pivot to consensus extraction:** TX-P5 extracts twice per window (two models, or two runs), class measures default to `attested:2`, and GE-T2 reruns under that policy. |
| T4 | same | < 0.4 for both models | **Reorder phases:** TX-P7's same-as and class core moves before TX-P5's ingest work, and SMYSL-2.6 reports no `A*` until classes exist. |
| T5 | `J_class`, narrative (P3) vs law/epistle (P1, P7) | narrative < 0.6 while the others are ≥ 0.6, **or** RA rate on narrative probes > 25% under arm H | **Scope narrative out:** narrative is exploration-only in TX-P8/P9 defaults, and GE-T6 drops the narrative genre until x.text holder/mode is validated. |
| T6 | precision at recall 0.7 of (C2 cosine ≥ θ), and of (C3 ∧ cosine ≥ θ), within language | < 0.9 | Class measures ship as exploration only (GE-T5 rule). TX-P7 defaults to `attested:2` for corpus measures. |
| T7 | same, cross-lingual | < 0.9 | Cross-lingual classes wait for TX-P10's S1 embeddings. `anchored` alone is used for Bibles. |
| T8 | RA rate, arm L | > 10% | Holder/mode must be structural from TX-P5's first ingest, and a probe-set gate is added to TX-P5's exit tests. |
| T9 | RA rate, arm H | ≤ 5% where L > 10% | The holder paragraph goes into the TX-P0 ingest prompt change (with the language policy, F-6), before any later measurement. |
| T10 | V1 false-`Absent`, ru (M3b) | > 1% of quotes | Confirms FC-7 as a precondition (already TX-P0). Russian spike numbers carry this caveat. The cases become V2 test vectors. |
| T11 | `E022` degradations, ru | > 5% of units | Confirms FC-6 as a precondition. Russian `A1` is not compared with English until TX-P0 lands. |
| T12 | GE-T3 pilot | between-language variance > run variance | Reported as a finding, and cross-lingual `A*` is conditioned on it (GE-T3 rule). |
| T13 | ru/en model-token ratio vs estimator ratio | differ by > 20% | FC-6 weights are calibrated on the spike's M5 data. |
| T14 | measured tokens per chapter | > 2× §7.7 planning figure | The cost model is revised before TX-P5 is sized. |

Outcomes combine. In particular, T3 together with T5 means pivot and also scope narrative out.
The report states the resulting plan state in one line.

## 4. Implementation plan (the spike kit)

### 4.1 Layout (outside the repository)

```
spike/
  in/            29 input files, manifest.tsv, align.tsv          (frozen after prep)
  config/        local.hjson, remote.hjson
  prompts/       holder.hjson, holder.system.txt                  (arm H)
  prep.py        sources → in/ (NFC, one verse per line, checks)
  proxy.py       loopback logging proxy for Ollama
  run.sh         arms × inputs × runs, interleaved; resumable (skips a run dir with `exit`)
  qcheck/        Rust harness, §4.2
  collect.py     runs → units.tsv, runs.tsv, tokens.tsv
  candidates.py  pools (smysl merge), C0–C3, control pairs → judge/pairs.tsv, judge/sheet.tsv
  classes.py     strict/component classes, J_uid, J_gist, J_class, range test
  alpha.py       Krippendorff α (nominal) + bootstrap
  attrib.py      V1 / V2-emu verdicts over raw/*.jsonl, M3 tables
  report/        REPORT.md, decision.md, figures
```

Python 3.11 standard library only. The exception is the optional `model2vec` fallback for C2.

### 4.2 `qcheck` — exact V1 verdicts from the library

This was compiled and run against `d25ec9e`. It reproduces E-10 (`своем` against `своём` →
`Absent`; `нет Бога` inside `«нет Бога»` → `Present`).

```toml
[dependencies]
smysl = { path = "/tmp/smysl", default-features = false }
```

```rust
let out = smysl::parse_surface(&staged)?;
for r in &out.records {
    let Some(u) = r.as_unit() else { continue };
    let p = u.payload.as_deref().map(smysl::payload_strings).unwrap_or_default();
    let q = p.get(smysl::QUOTE_KEY).and_then(|s| s.iter().next().cloned());
    let (verdict, span) = q.as_deref()
        .map(|q| smysl::quote_support_span(q, &source))
        .unwrap_or((smysl::QuoteSupport::Absent, None));   // reported as NoQuote
    // emit smysl::canonical_uid(u).canonical(), u.schema, u.status, u.gist, q, verdict, span
}
```

- V2-emu runs the same binary on pre-folded copies of the quote and the source, so the code path
  is identical.
- Byte spans are offsets into the input file, because there is one chunk and its text is the
  file. Spans map to locators by line.

### 4.3 CLI surface used

| command | arm/step | verified |
|---|---|---|
| `ingest --path surface [--prompt F] [--offline] in/<f>.txt` | runs | flags read in `cmd_ingest`; flow reproduced against a fake Ollama |
| `ingest --dry-run …` | preflight, estimator tokens | run |
| `providers --probe` | preflight, L | `--probe` named in the `providers` output; not run against a live Ollama |
| `merge <staged…> -o pool.cbor` | pools | run |
| `find --json -n 5 --source P [--engine semantic --model D] Q pool.cbor` | C1, C2 | run (lexical; semantic with `embed-tiny`) |
| `attest --what granularity --sample 30 <staged.smy>` | M7 | flags read in `cmd_attest`; not run |

No new commands, flags, records or diagnostics.

## 5. Data, deliverables and fixtures

**Deliverables:**
- `report/REPORT.md`, at most 5 pages: questions, numbers with CIs, the filled decision table,
  the resulting plan state, and deviations.
- **Data:**

| file | rows |
|---|---|
| `in/manifest.tsv`, `in/align.tsv` | inputs, alignment |
| `runs.tsv` | arm, model, input, run, recipe, exit, chunks, calls, units, weakened, degraded, tokens in/out, wall ms |
| `units.tsv` | run, canonical uid, type, status, gist, quote, V1, V2-emu, span, locator |
| `raw/L.jsonl`, `raw/H.jsonl`, `raw/P.jsonl` | every local request and answer |
| `judge/pairs.tsv`, `judge/sheet-A1.tsv`, `judge/sheet-A2.tsv`, `judge/gold.tsv` | candidates, labels, adjudication |
| `classes.tsv` | uid → strict class id, component id |
| `holder.tsv` | probe, unit, labels A1/A2/gold |
| `tokens.tsv`, `metrics.json` | M5; every M1–M7 figure |

**Fixtures for later phases.** The spike data feeds the following:
- **TX-P0 (SMYSL-2.1):**
  - Each false-`Absent` case from M3 becomes an FC-7 test vector: V1 `Absent`, V2 `Present`.
  - The M5 ru/en token pairs calibrate FC-6.
  - O-1…O-6 go to the 2.1 backlog.
- **TX-P1 (SMYSL-2.4):** the 29 frozen inputs and `align.tsv` become reader and alignment fixtures
  under `fixtures/text/s0/`. The Psalm 14 ↔ 13 alignment is the versification test. The size is
  about 250 KB, so the inputs go under a path added to the package `exclude` list, or the crate
  stays under its 10 MB limit by a measured margin (`Cargo.toml` comment).
- **TX-P5:**
  - Arm L's `staged.smy` files are the "today" baseline. TX-P5's owned-range spans are checked
    against `qcheck` spans on the same inputs.
  - The M4 probe set becomes the holder/mode gate.
- **TX-P7:**
  - `gold.tsv` plus `classes.tsv` is the first conformance fixture for `strict` derivation (D-7).
    An implementation given the gold edges must reproduce `classes.tsv` exactly.
  - The gold seeds GE-T5 and GE-T9. GE-T9 still runs at 20 passages.
- **Admissibility:** model outputs are fixture material only if OQ-34 allows. Inputs, alignment,
  gold labels and classes over uids are the project's own data in any case.

## 6. Delivery steps, timeline, roles and cost

| day | step | exit test |
|---|---|---|
| 1 | `prep.py`; manifest; build with `cli,local,remote,semantic`; `qcheck`; proxy; dry runs | 29 files with checksums; `--dry-run` shows 1 chunk-sized input everywhere; the proxy log shows `prompt_eval_count` ≈ expected (no truncation, O-3) |
| 2 | Arms L, R, H, P (and T if triggered) | 290 + 29 + 16 runs, each with `exit` ∈ {10, 11} and `chunks = 1`; failures rerun once and logged |
| 3 | `collect`, M1, M3, M5, M6; candidates; sheets | `metrics.json` holds M1/M3/M5/M6; sheets generated and blinded |
| 4–5 | Calibration (50 pairs), then A1 and A2 judge; holder labels | Both sheets complete; α computed **before** adjudication |
| 6 | Adjudication, classes, `J_class`, PR curves (T6/T7), decision table | Every row of §3.7 filled or marked inconclusive |
| 7 | Report; fixture extraction; hand-over notes to 2.1/2.4/2.6 | `REPORT.md` reviewed by A2; fixtures listed in §5 exist |

**Roles:**
- Vladimir Ulogov: owner, runs, scripts, A1, adjudicator.
- A2: second annotator (en/ru), who also reviews the report.
- A3 (optional): es/fr/de single annotation.

**Cost** (estimates; the measured figures replace them in `REPORT.md`):

| item | tokens | wall clock |
|---|---|---|
| per run (en chapter) | ≈ 1.6k in (≈ 600 prompt + text) + ≈ 1.2k out, ×1.3 for repairs | 1–2 min local on a 16 GB GPU (unverified); about 10× on CPU |
| arm L, 145 runs | ≈ 0.55M | ≈ 3–4 h |
| arm R, 145 runs | ≈ 0.55M (≈ 0.37M in, 0.18M out) | ≈ 1 h |
| arms H + P, 45 runs | ≈ 0.2M | ≈ 1 h |
| optional LLM same-as judge (direct local Ollama script, outside smysl, labelled) | ≈ 0.6M local | ≈ 2 h |
| total | ≈ 1.3–1.9M, of which ≈ 0.55M hosted | — |

- Hosted spend: under USD 5 at the 2026 list prices of any of the four hosted mappers
  (unverified).
- Human time: about 6 h per annotator.

## 7. Risks and mitigations

| risk | mitigation |
|---|---|
| Ollama truncates the prompt silently (O-3) | Server context set to 32768. The proxy compares `prompt_eval_count` with the estimate, and a run more than 30% short is rerun. |
| Local T = 0 is bit-identical, so `J` within L = 1 means nothing | Reported as determinism, not as stability. Arm R carries Q1. Arm T gives sensitivity. |
| One fabricated or falsely-`Absent` quote degrades a whole chapter (E-9) | Counted, not hidden. Degraded runs stay in `runs.tsv`, and `J` is computed with and without them. |
| Russian gists over 120 bytes degrade (`E022`, F-2) | Measured (T11). The prompt is not changed, because the spike measures today's tree. |
| Arm R raw answers are not captured | M3b is reported for arm L only. Arm R reports survivor verdicts and repair rates. |
| Candidate generation misses true pairs, and `J_class` falls | Control pairs bound the miss rate, which is reported beside `J_class`. |
| ~~Potion model does not load in `model2vec-rs 0.2`~~ did not occur | F-11 verified in the run: it loads and ranks, no fallback used. |
| Annotator drift or fatigue | Calibration round, blinding, sessions of at most 90 min, `seconds` logged. |
| Model or tag changes during the runs | Model digest recorded (`ollama show`, hosted `model` echoed in the response). Runs are interleaved, so drift spreads across inputs. |
| Prompt-injection content in inputs | All inputs are PD scripture, an essay and our own chat. The fence check is in `prep.py`. |

## 8. Diagnostics allocated in this RFC

None. The spike adds no code to the repository and needs no diagnostic.

## 9. Open questions

**New in this RFC:**

| id | question |
|---|---|
| OQ-32 | **Resolved by SMYSL-2.1 H-12:** `ingest --temperature` ships in TX-P0. The spike's main arms stay at T = 0 for comparability; a T > 0 arm becomes possible without a local patch. |
| OQ-33 | **Resolved by SMYSL-2.1 H-9:** the recipe hashes the configured model. Recipes of earlier runs that relied on a config `model` change once; the spike runs either entirely before or entirely after H-9. |
| OQ-34 | Can staged units produced by a hosted model, or by an open-weights model under its own licence, be committed as fixtures to an MPL-2.0 repository? Or do fixtures keep only inputs, gold labels and uid-level data, regenerating model output on demand? |

**Draft-3 questions this spike informs:**
- **OQ-3:** the ё density of the chosen Synodal edition is recorded per file.
- **OQ-6:** M4 says whether holder/mode as payload is urgent.
- **OQ-7:** `strict` is used as D-7 makes it normative, and `component` is reported beside it.

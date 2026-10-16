# RFC SMYSL-2.9 — The `sq/1` query grammar

**Status:** draft 1, for discussion. Implementation RFC (non-normative); normative rules are in SMYSL-2.3.
**Author:** Vladimir Ulogov
**Date:** 2026-10-15
**Part of:** RFC set SMYSL-2 — see SMYSL-2.0 (index and roadmap).
**For:** crate `1.10.0` (`7b0bfc5`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Derived from:** SMYSL-2.5 §3.1 (the deltas), §3.2 (planner phase order), §3.3 (the operator-to-API table, which is the stage inventory), §3.9 (field resolution), §5.1 (the golden tests), §8 (diagnostics). **Replaces** SMYSL-2.5's dangling reference to "RFC SMYSL-2 draft 3, Appendix C", which is not in this repository.
**Phases:** TX-P6 step 1 (this document is its input).
**Depends on:** SMYSL-2.5 (everything above; this document adds no semantics it does not already fix); SMYSL-2.4 (EDTF parsing for time literals, `SMY-E410`; locators and aliases for `text(…)`); SMYSL-2.0 §6.1 (diagnostic registry).

---

## 0. Summary

SMYSL-2.5 specifies the `sq/1` query language completely except for its surface syntax. §3.3
binds every stage to a function, §3.2 fixes the order they execute in, §3.1 fixes the version
rule, the comment rule and keyword positioning, and §8 allocates the diagnostics. What it does
not carry is the grammar itself: it points at an appendix of a draft that is not in this
repository, so no parser can be written against it.

This document is that grammar. It is **recovered rather than invented**: every production below
comes from a surface form SMYSL-2.5 already writes down, and the places where it does not are
marked as additions and carried as open questions rather than settled quietly.

Three things it fixes beyond transcription:

1. **Two languages, not one.** The deterministic grammar is not asked to accept English, because
   SMYSL-2.5 §3.15 already specifies a model that translates English into it. Conflating them
   would make the pure layer ambiguous and leave the impure layer with no job.
2. **What "deterministic" means for a grammar**, as seven testable clauses (§3.3) rather than an
   adjective — including the three that only bite because this grammar is reimplemented in four
   languages.
3. **The canonical printer is a normaliser**, which is what makes saved-query identity, query
   `diff` and four-implementation agreement work at all.

## 1. Scope

**In:**
- The lexical structure of `sq/1`: tokens, modes, comments, case folding, string, number, time
  and uid literals.
- The complete grammar, as EBNF, for every stage in SMYSL-2.5 §3.3.
- Stage ordering, predicate structure and operator precedence.
- Canonical printing, the round-trip laws and query identity.
- The version and compatibility rule for `sq/N`.
- The accept/reject fixture layout and what a conformance consumer may rely on.
- Diagnostics for syntax, allocated from a new range (§8).

**Out:**
- What any stage *means* or which function it lowers onto: SMYSL-2.5 §3.3.
- Planner phases, purity labelling, RRF, bridges, lenses, dossiers, the NL front end:
  SMYSL-2.5 §3.2, §3.4–§3.15.
- EDTF syntax and validation: SMYSL-2.4's parser, reached through a time literal.
- `match`'s pattern language: OQ-50. This document specifies how `match` is **captured** without
  deciding what it captures (§4.9).
- Record layouts and the `x.query/v1` encoding: SMYSL-2.3.

## 2. What exists today

- No `smysl-query` crate, no `.sq` file and no BNF anywhere in the repository; nothing matching a
  draft-3 appendix was ever committed and later deleted. Verified 2026-10-15.
- **SMYSL-2.5 §2's probe is stale.** It records `--seed-check` as "declared as a global flag and
  read nowhere". It is read: `src/main.rs` declares it and the dispatcher decides per invocation.
  Part of TX-P6 step 7's exit is therefore already met, and §2 of that document should be
  corrected when it is next touched.
- The house parser idiom this grammar is written for is `smysl-core`'s `surface/lex.rs` and
  `surface/parse.rs`: hand-written, `Code`-tagged, span-carrying. SMYSL-2.5 §3.1 gives four
  reasons for matching it, and the fourth — that the grammar ports to Python, Go and JavaScript
  line by line — is the one that shapes this document.

## 3. Design

### 3.1 Two languages, not one

The requirement is that a query be deterministic and read as closely to human language as
possible. Those pull against each other only if one language has to do both jobs, and in this
RFC set it does not:

| layer | who writes it | purity | measured by |
|---|---|---|---|
| `sq/1`, this document | a person, or a dossier template | **pure**, deterministic | the accept/reject goldens (§5) |
| English | a person, translated by a model (SMYSL-2.5 §3.15) | model-dependent, labelled, printed before it runs, `E474` if it does not parse | GE-T12, at ≥ 70% equivalent results |

So the target for `sq/1` is **legible on sight, not dictatable**: a reader who has never seen the
language should understand a query without a manual; a writer may have to learn the shape. Every
readability decision below is taken only where it costs no determinism, and §3.3 says what that
means precisely.

The practical consequence is a rule:

> **QG-1 (no ambiguity for readability's sake).** Where a more English-like form would admit two
> parses, `sq/1` takes neither and the English-like form belongs to the front end.

### 3.2 Reading principles

Four, in the order they matter.

**Written order is evaluation order.** SQL's most-copied usability defect is that `SELECT` is
written first and evaluated nearly last, so the text does not describe what happens. `sq/1` is a
pipeline whose stages execute top to bottom in the order written, and the grammar *enforces* that
order (§4.2) rather than reordering silently. This is also the cheapest determinism win in the
document: with stage classes in a fixed sequence the parse is LL(1) and the planner's phase order
(SMYSL-2.5 §3.2) is the written order, so `--explain` prints the query back.

**Words where a word will do.** `during`, `before`, `after`, `overlaps`, `mentions`, `rests on`,
`depth 3`, `4000 tokens`, `in reading order`. Symbols are kept only where they are read the same
way by everyone: `=`, `!=`, `<`, `<=`, `>`, `>=`, `..` for a range, and the traversal arrows.

**One spelling per concept, chosen for the reader.** `sq/1` has **no synonyms** (OQ-80). The
temptation is real — accepting several spellings and printing one costs no determinism, because
nothing downstream can tell which was typed — but the cost is that four implementations must
carry an identical table or QG-7 breaks between ports, and a table is the easiest thing to let
drift. The benefit disappears once the one spelling is chosen for readability instead of copied
from a function name: the only synonym worth having was `rests_on` → `rests on`, and the right
fix is that **`rests on` is the only spelling** and the snake_case form — which exists because
`rests_on` is a Rust function — is not admitted.

**Say the quantity in the unit.** `limit 10`, `depth 2`, `pack 4000 tokens`. A bare number whose
unit is positional is the thing a reader has to look up.

A query under these principles:

```
from library
find "faith" ~lex
where lang = en and mode = reported
expand grounds depth 2
order by salience
limit 10
```

### 3.3 Determinism: the seven clauses

"Deterministic" is an adjective until it is split into things a test can fail. These are the
clauses; §5 says which harness owns each.

| clause | statement |
|---|---|
| **QG-2 unique parse** | One input text yields exactly one AST, or one `SMY-E460` with a span. The grammar is LL(1) under one token of lookahead, with the two documented exceptions in §4.12; no production is chosen by backtracking. |
| **QG-3 environment independence** | The parse does not depend on locale, timezone, the host's clock, environment variables, hash seeds or iteration order. The determinism matrix (`xtask determinism`) gains `sq` parse and print as an operation. |
| **QG-4 version independence** | The parse does not depend on the Unicode version of the runtime. Keyword recognition folds **ASCII only** (§4.1). |
| **QG-5 byte fidelity** | A string literal's value is the bytes it denotes. No normalisation, no case folding, no trimming — quotes and locators must match corpus bytes exactly, which is the rule GE-T2's gist already follows. |
| **QG-6 exact numbers** | Every numeric literal has an exact representation that four runtimes agree on. No binary floating point in the surface syntax (§4.1). |
| **QG-7 canonical printing** | `print` is idempotent and `parse` is its left inverse: for every accepted `q`, `parse(print(parse(q))) == parse(q)` **modulo spans** **and** `print(parse(print(parse(q)))) == print(parse(q))` exactly. The spans are excused because a span is an offset into the text it was parsed from and the printed text is a different string by construction — the printer discards the whitespace and comments those offsets index past, so equal spans would mean two different layouts were the same layout. What spans are held to instead is stronger where it can be: on canonical text, which is all content, the query's span is exactly `0..len`. |
| **QG-8 resolved at parse time** | Nothing whose meaning depends on when or where the query runs survives into the AST. A construct that would (a relative date) is either absent from the grammar or resolved during the parse, with the canonical print carrying the resolved value. |

QG-4, QG-5 and QG-6 exist because this grammar has four implementations, and each is a defect
that would appear in one port and not another:

- **QG-4.** All four runtimes case-fold independently of locale, so that is not the hazard. The
  hazard is that Unicode's case-folding tables *change between Unicode versions*, and four
  runtimes will not ship the same version. The Kelvin sign `K` (U+212A) folds to `k`; under
  Unicode folding a field name could collide with a keyword in one port and not another. ASCII
  folding is stable for the life of the format.
- **QG-5.** A string that is normalised on the way in matches nothing in a corpus that is not, and
  it fails *silently* — the worst available failure. §4.1 raises `SMY-W510` instead.
- **QG-6.** Shortest-round-trip formatting of binary floats has differed across these four
  runtimes historically, which would break QG-7 in exactly one of them. Decimals are parsed to a
  scaled integer with a fixed maximum of four fractional digits.

### 3.4 Canonical printing and query identity

The printer is not a pretty-printer, it is a **normaliser**: one query has one canonical text.
It discards comments and incidental whitespace, emits one stage per line with no `|`, and prints
every resolved value in place of whatever denoted it.

Two consequences, both load-bearing:

1. **A saved query's identity is the hash of its canonical print, never of the typed text**
   (SMYSL-2.5 §3.6, `x.query/v1`). Otherwise a changed comment or an added space makes a
   different query, and reruns and `diff` stop being about the question.
2. **Query `diff` is a diff of canonical prints**, so it shows what changed in the question
   rather than in the typing.
3. **A saved query's canonical print carries the scope that was resolved when it was saved**, even
   though `from` is optional in the grammar (§4.3, OQ-79). This is QG-8 applied to scope: a query
   that ran against a particular library records which, so two reruns are comparable and a saved
   question is never ambiguous about what it was asked of.

### 3.5 Versioning and compatibility

A query without `using sq/N` runs under `sq/1`; `SUPPORTED_SQ = ["sq/1"]`; anything else is
`SMY-E427` (SMYSL-2.5 §3.1). The compatibility rule this document adds is the one that is
otherwise gotten wrong in two years:

> **QG-9.** Adding a keyword is **not** an additive change if the word could previously have
> appeared where the new keyword is now recognised. Since keywords are reserved **only at
> stage-head position** (§4.1), a new stage name is the only breaking addition, and it breaks
> only queries whose stage head was that bareword — which no `sq/1` query can have, because an
> unrecognised stage head is `SMY-E460` today.

So: new stages are additive, new field names are always additive, new *operators* and new
keywords recognised inside a stage body are **not**, and need `sq/2`.

## 4. The grammar

Notation: `=` defines, `|` alternates, `{ }` is zero or more, `[ ]` is optional, `( )` groups,
`" "` is a literal, `(* *)` is a comment. Terminals are lowercase; nonterminals are hyphenated.
Whitespace separates tokens and is otherwise insignificant except where §4.1 says.

### 4.1 Lexical structure

**Case.** Keywords are recognised after folding with ASCII-only lowercasing (QG-4). Identifiers,
field names, values and strings are **not** folded and compare byte-exactly.

**Keyword reservation.** A word is a keyword only where §4.2 places it: stage heads at
stage-head position, operators inside the stage that defines them. Everywhere else — notably as a
field name — a keyword spelling is an ordinary identifier, so a payload key named `limit` works.

**Comments.** `#` begins a comment, to end of line, **only at the start of a token** (at line
start or after whitespace). `text(kjv-1769#Ps.14)` keeps its `#`; `find "x"  # by title` does not
(SMYSL-2.5 §3.1).

**Stage separation.** A newline or the token `|` separates stages; both are accepted and the
canonical printer emits a newline (OQ-75). Since stage heads are reserved at stage-head position,
neither is required for an unambiguous parse, but one of them must be present: two stages on one
line with no `|` is `SMY-E460`, because the alternative is an error message that cannot say where
the first stage ended.

**A separator inside an incomplete construct is whitespace.** A newline ends a stage only where a
stage could end: after `and`, after a comma, or anywhere else the parser is waiting on an operand,
it is skipped. This is the rule Go and Swift use for semicolons, and without it "a separator is
required" would mean "a stage must fit on one line", which a long `where` cannot. A run of
newlines and `|` collapses to one separator, so a blank line between stages is not an empty stage
— and a **trailing** separator is therefore accepted, exactly as a trailing newline is. SMYSL-2.5
§5.1 lists "dangling `|`" as a reject fixture, which it was under a mandatory `|`; under OQ-75 it
is not an error, and the reject set carries seventeen near-misses of its own instead
(`fixtures/sq/reject`, counted 2026-10-15).

```
string      = '"' { char | escape } '"' ;
escape      = "\\" ( '"' | "\\" | "n" | "t" ) ;
char        = (* any byte except '"' and '\\'; no normalisation, QG-5 *) ;
integer     = digit { digit } ;                       (* ASCII only, no sign, no separators *)
decimal     = digit { digit } "." digit [ digit [ digit [ digit ] ] ] ;  (* ≤ 4 places, QG-6 *)
ident       = alpha { alpha | digit | "-" | "_" } ;   (* ASCII *)
field-name  = ident { ( ":" | "." ) ident } ;         (* text:mode, payload.code.kind *)
schema-id   = ident { ( "." | "/" ) ident } ;         (* x.text/v1 *)
uid         = "b3:" base32 { base32 } ;
alias       = ident { ( "-" | "." ) ident } ;
param       = "$" ident ;
```

**Strings** carry bytes (QG-5). A literal that is not in NFC is accepted unchanged and raises
`SMY-W510`, because a non-NFC quote will silently match nothing.

**Numbers.** `integer` everywhere a count appears. `decimal` only in `near(…)`; it is parsed to a
scaled integer (value × 10⁴) and printed back from it, so QG-6 and QG-7 hold.

**Time literals** are lexed in a time context only — after an axis keyword — so a range's `..`
cannot collide with anything else (SMYSL-2.5 §3.1). Their contents go to SMYSL-2.4's EDTF parser,
and a failure is `SMY-E410`.

```
time-value  = edtf | edtf ".." edtf ;
edtf        = (* an EDTF string, validated by SMYSL-2.4; SMY-E410 on failure *) ;
```

**An `edtf` token contains no `.`**, and the lexer ends one at the first it meets. The claim
above that `..` "cannot collide with anything else" was true of the stage and false of the
value: stopping only at a *pair* of dots leaves the split point ambiguous, because
`2026-09-01...2026-09-15` lexes as `2026-09-01.` `..` `2026-09-15` and prints back as
`2026-09-01` `..` `.2026-09-15` — a different pair of values from the same text, which is
QG-7's first equation failing. `sq_parse` found it on its second run. No EDTF form uses a
single `.`, so excluding it costs nothing and a lone dot is refused by name with `SMY-E460`
(`fixtures/sq/reject/dot-in-time-value.sq`).

One EDTF form is unreachable as a consequence, and it is worth naming rather than discovering:
level-2's **open interval** `1760-12-03/..` cannot be written in `sq/1`, because `..` is the
range operator. The forms it stands for have spellings here already — `said after
1760-12-03` is the open interval — so nothing is lost but the EDTF spelling of it, and a
`time-value` that admitted both would be a grammar in which `..` means two things.

There are **no relative time words** (`today`, `last week`) in `sq/1`: by QG-8 they would have to
be resolved at parse time, and a saved query whose text said `last week` but whose meaning was a
fixed fortnight would be a trap. The front end (§3.1) is where "last week" belongs.

### 4.2 Query and stage order

```
query       = [ header ] expression { ordering } { shaping } [ output ] ;
header      = using-clause { using-clause } ;
using-clause= "using" ( sq-version | schema-pin ) ;
sq-version  = "sq" "/" integer ;                       (* else SMY-E427 *)
schema-pin  = schema-id "@" integer ;                  (* using x.text/v1@1 *)

expression  = term { set-op term } ;                   (* left-associative *)
set-op      = "union" | "intersect" | "except" ;
term        = "(" expression ")" | pipeline ;

pipeline    = [ scope ] [ source ] { filter } { time-stage } { traversal } { closure } ;
```

A `pipeline` must contain at least one element. The order of the five classes is **enforced**: a
`where` after an `expand` is `SMY-E460` naming both stages, not a reordering. This is QG-2's main
mechanism and SMYSL-2.5 §3.2's phase order written as syntax.

`ordering`, `shaping` and `output` sit outside `expression` because SMYSL-2.5 §3.2 runs set
operations (phase 8) before ordering and shape (phase 9); a sort inside one arm of a `union`
would be discarded, so the grammar does not let it be written.

### 4.3 Scope

```
scope       = "from" scope-item { "," scope-item } ;
scope-item  = "store" ":" name
            | "collection" ":" name
            | "import" ":" name
            | "channel" ":" name
            | "channels"
            | "library"
            | "texts" ;
name        = ident | string ;
```

The plural/singular split is the rule SMYSL-2.5 §3.3's own list follows: a bare plural is every
one of that kind in scope (`texts`, `channels`), and a singular with a colon is that one
(`collection:`, `import:`, `channel:`). That document writes bare `channel`, which is the only
member of the list not following its four neighbours; this grammar reads it as `channels`
(OQ-76).

`from` is optional in the grammar and the host supplies a default when it is absent (the CLI's
positional store argument). A dossier template may omit it so one template serves many scopes
(OQ-79).

### 4.4 Sources

```
source      = find | about | text-src | uid-src | view-src
            | pointer-src | metric-src | question-src | match-stage ;

find        = "find" ( string | param ) [ engine ] [ "lang" ":" lang-sel ] ;
engine      = "~lex" | "~sem" | "~hybrid" ;
lang-sel    = "any" | ident ;                          (* a source option, not a field *)

about       = "about" ( "entity" "(" ( string | param ) ")"
                      | "class"  "(" uid ")" ) ;
text-src    = "text" "(" alias ( "#" locator | "@" time-value ) ")" ;
locator     = (* bytes to the closing paren, balanced; '#' inside is not a comment *) ;
uid-src     = "uid" ( uid | param ) ;
view-src    = "view" view-id ;
view-id     = "v" "/" ident { ( "." | "/" ) ident } ;
pointer-src = "pointer" "(" string ")" ;
metric-src  = "metric"  "(" string ")" ;
question-src= "question" ( string | param ) ;
```

### 4.5 Filters

Predicate structure is where a grammar either prevents misreadings or causes them. `and` binds
tighter than `or` in most languages and nobody verifies it, so `sq/1` **does not let the two be
mixed without parentheses** — the grammar has no production for it, and the diagnostic says so.

```
filter      = "where" predicate ;
predicate   = and-chain | or-chain | unary ;
and-chain   = unary "and" unary { "and" unary } ;
or-chain    = unary "or"  unary { "or"  unary } ;
unary       = [ "not" ] atom ;
atom        = "(" predicate ")" | comparison | match-pred | near-pred ;

comparison  = field-ref compare-op value ;
compare-op  = "=" | "!=" | "<" | "<=" | ">" | ">=" | "in" ;
value       = string | integer | decimal | uid | bareword | param | list ;
list        = "[" value { "," value } "]" ;
bareword    = ident ;                                  (* en, reported, attested *)

match-pred  = "matches" ( string | param ) ;
near-pred   = "near" "(" ( string | param ) "," decimal ")" ;

field-ref   = [ schema-id "@" integer ":" ] field-name ;   (* qualified reads raw, §3.9 *)
```

`where kind`, `schema`, `status`, `lang`, `commitment`, `holder`, `speaker`, `mode` and any
payload key are all just `field-ref`, so the grammar has one rule and SMYSL-2.5 §3.3 keeps the
list of what resolves. Status comparisons use `Status: Ord`, so `status >= attested` is a
`comparison` like any other.

### 4.6 Time

```
time-stage  = axis time-op time-value ;
axis        = "said" | "observed" ;
time-op     = "during" | "overlaps" | "before" | "after" ;
```

### 4.7 Traversals and closures

```
traversal   = "expand" edge-spec [ "depth" integer ] ;
edge-spec   = "->" kind | "<-" kind | "-[" kind "]-" | named-edge ;
kind        = field-name ;                             (* SMY-E462 if undeclared *)
named-edge  = "grounds" | "deps" | "rests on" | "dependents" | "lineage" | "mentions" ;

closure     = "with" closure-item { "," closure-item } ;
closure-item= "rebuttals" | "grounds" | "holders" | "speakers" | "spans" | "datings" ;
```

`with lens none` disables lensing for qualified access on a stage (SMYSL-2.5 §3.1), which does
not say *which* stage. It attaches to the four that can hold a qualified field reference —
`where`, `order by`, `group by` and `compare by` — and nowhere else, so the modifier is local to
the thing it modifies and the grammar gains one production rather than a general suffix:

```
lens-mod    = "with" "lens" "none" ;
```

**This resolution has a prerequisite in SMYSL-2.5, not here.** §3.1 says the modifier "disables
lensing for qualified access only", while §3.9 says a qualified `x.text/v1@1:text:mode` "reads
revision 1 raw, no lens" unconditionally. If §3.9 is right the modifier does nothing, so either
qualified access is normally lensed forward from the named revision and this modifier makes it
raw, or §3.1's sentence is stale. That is a question about meaning rather than syntax, and
OQ-77 records it against SMYSL-2.5.

### 4.8 Ordering, shape and output

```
ordering    = "order" "by" sort-key { "," sort-key }
            | "in" "reading" "order"
            | "in" "time" "order" [ "as" "recorded" ]
            | "group"   "by" field-ref
            | "compare" "by" field-ref
            | "align"   "by" "class" [ policy ] ;
sort-key    = ( "salience" | field-ref ) [ "ascending" | "descending" ] ;
policy      = ident ;

shaping     = "limit" ( integer | param )
            | "pack" ( integer | param ) "tokens" [ "as" "view" ] ;

output      = "count" | "units" | "json" | "show" | "spans"
            | "measure" measure-id { "," measure-id } ;
measure-id  = field-name ;
```

The direction is spelled in full: `order by salience descending`, not `desc`. Determinism does
not turn on it — ties break by uid everywhere an order is partial (SMYSL-2.5 §3.3) — so this is
a reading decision, and an abbreviation buys nothing a reader wants (OQ-81). The default is
**ascending for every key, uniformly**, including `salience`. A per-key default would read
better in the common case and was rejected for it: an ordering whose direction depends on which
field was named is the kind of surprise that makes people stop trusting the order. Expect
`order by salience descending` to be the usual form and write it out in templates.

`in time order` reads SMYSL-2.4's **effective** time; `as recorded` selects the observed axis
instead (OQ-78). Ordering is the only construct here with no axis word in it — a time predicate
already says `said` or `observed` (§4.6) — which is why the modifier belongs to this stage and
not to that one.

### 4.9 `match`, parsed but not interpreted

`match` is a **source**: it names a set, which is what §4.2 lets a source do, and draft 1 of
this document defined it as a free-standing "stage" that §4.2's pipeline had no slot for. The
gap was found by implementing §4.4 against it (TX-P6 step 1) and is fixed above rather than
argued with.

SMYSL-2.5 refuses `match` with `SMY-E461` and leaves its pattern language to OQ-50 — but a parser
cannot refuse what it cannot parse. The resolution is to capture without interpreting:

```
match-stage = "match" raw-pattern ;
raw-pattern = (* tokens to the end of the stage, with (), [] and {} balanced *) ;
```

The AST node holds the raw bytes and their span. The printer emits them back unchanged, so QG-7
holds; the planner raises `SMY-E461`. When OQ-50 is decided, the pattern language replaces
`raw-pattern` and no other production moves.

### 4.10 Parameters

`$name` is a value anywhere `value` admits one, and in the three source positions marked above.
It is bound by the caller and recorded in the saved query (SMYSL-2.5 §3.1). An unbound parameter
at execution is `SMY-E460` with the span of the `$name`.

### 4.11 No synonyms

There is no synonym table (§3.2, OQ-80). One concept has one spelling, chosen to read well:
`limit` and not `first`/`top`, `expand` and not `follow`, **`rests on`** and not `rests_on`.

`!=` and `not … =` are **not** two spellings of one thing and both are kept: once `not` may wrap
a parenthesised predicate the two stop agreeing, so they are different operators that happen to
coincide on a single comparison.

### 4.12 The two lookahead exceptions

QG-2 claims LL(1) with two documented exceptions, both resolved by the lexer and neither by
backtracking:

1. **`in`** is both a comparison operator (`lang in [en, ru]`) and the head of two orderings
   (`in reading order`, `in time order`). They never occur in the same position: `in` at
   stage-head is an ordering, `in` after a `field-ref` is an operator.
2. **`with`** heads both a closure stage and the `lens none` modifier. One token of lookahead
   (`lens`) separates them, and `lens` is not a `closure-item`.

## 5. Tests, fixtures and harnesses

This section refines SMYSL-2.5 §5.1 rather than replacing it.

**`fixtures/sq/accept/*.sq` with `*.ast` and `*.sqc`.** Every production in §4, the examples in
this document, comments, `#` inside a locator, multi-word keywords, parameters, both order
directions. Two expectations per fixture, because they fail for different reasons: `.ast` is the
tree and `.sqc` is the canonical text, so a printer change that leaves the tree alone is still a
reviewable diff. The `.ast` dump leaves **spans out** — a span is a byte offset, so every
expectation would churn on an edit to the whitespace above it, and spans are asserted where they
are actually read, in the reject goldens.

**`fixtures/sq/reject/*.sq` with `*.diag`**, one expected code, span and message per file. The
message is compared and not only the code: a grammar meant to read like English owes a diagnostic
that reads like one. Fifteen as of TX-P6 step 1 — unterminated string, `depth` without a number,
`using sq/2` (`E427`), `and`/`or` mixed without parentheses, a source after a filter, two stages
on one line, two sources in one pipeline, an abbreviated `desc`, `rests_on`, an unknown scope, a
`/` in a field name, an unbalanced `match`, an unknown escape, a decimal with five places, and
`with lens none` on a line of its own.

Two of SMYSL-2.5 §5.1's are **not** here and neither is an omission. "Dangling `|`" is not an
error under OQ-75 (§4.1). An unbound `$x` is refused at *execution*, not at parse (§4.10): the
parser has no bindings to check against, so the fixture belongs to the step that does.

> **The reject set must contain a near-miss of every accept fixture.** For a hand-written
> recursive-descent parser the risk is never ambiguity — it is unintended acceptance, which no
> accept fixture can detect.

**The suite is external, like `fixtures/conformance/`:** inputs plus expected outputs, no Rust in
the tree. That directory is deliberately language-free so a second implementation validates
against the same artifacts, and this grammar has three more implementations coming. Regenerated
with `SMYSL_BLESS=1` (render's convention, as SMYSL-2.5 §5.1 says).

**Laws.** QG-7's two equations over every accept fixture, plus the canonical-span law the
first equation excuses spans from: `parse(print(q))` closes the query's span at the last word.
QG-4 as a test that a name containing U+212A does not fold onto a keyword, and as a source
guard that `to_lowercase` appears nowhere in the crate.

QG-3 belongs in the determinism matrix (`xtask determinism`), which runs a **command** under
permuted locale, timezone and hash seed — and there is no `smysl q` to run until step 7, so the
matrix entry lands with it. What stands in the meantime is not a weaker check of the same thing
but a different and stronger one: a source guard that the crate never names `std::env`,
`SystemTime`, `Instant::now` or `RandomState`, so there is nothing in a parse for a permutation
to change.

**Fuzz.** `fuzz/fuzz_targets/sq_parse.rs` (SMYSL-2.5 §5.3): never panics; accepted text
re-prints and re-parses to an equal AST, modulo spans; the canonical text is exactly the span
of the query parsed from it. TX-P6 step 1's exit asks for ten minutes clean. It is seeded from
`fixtures/sq` — the accept cases are valid seeds and the reject cases are one mutation from
valid — and registered in the Makefile's `FUZZ_TARGETS`, without which `make fuzz` would
compile it and never run it, which is the defect `readers` already supplied once.

**It took four runs to get a clean ten minutes, and each of the three findings was real.** All
three are worth recording, because none was reachable from a fixture and each is a different
kind:

1. **A span that swallowed a trailing newline.** `from library\n` closed the query at 13 in a
   text whose last word ends at 12, because the span was closed at the lexer's position and
   peeking had already moved it past the separator. A caret a line below the word it is about.
   The fix is a separate `prev_end` — the end of the last *consumed* token, with separators and
   end-of-input excluded — and the law in §5 that pins it.
2. **An ambiguous split in a time range**, §4.1 above: fixed by ending an `edtf` token at any
   `.`.
3. **A panic**, which is guarantee A1 itself: `near("x", N)` for an integer `N` near `i64`'s
   ceiling multiplied by the scale unchecked. The lexer's own decimal path had the guard; the
   integer path in the parser did not. Both now answer `SMY-E460 number out of range`.

The first two are the parse goldens' blind spot stated plainly: a fixture compares a tree
against a committed dump, and the dump leaves spans out so it does not churn on a whitespace
edit. That is the right trade for a golden and it means the fixtures cannot see a span defect
at all. The third is the blind spot of any hand-written corpus — nobody writes
`near("x", 9223372036854775807)` as an example of anything.

## 6. Delivery steps

This document is the input to **TX-P6 step 1**, which it leaves unchanged in scope and splits
only for review:

1. `src/lex.rs` + `src/ast.rs`: tokens, modes, spans, the AST types of §4. *Exit:* lexer goldens
   over the accept corpus.
2. `src/parse.rs`: recursive descent over §4. *Exit:* `fixtures/sq/accept` parse, `fixtures/sq/reject`
   produce the expected code, span and message.
3. The canonical printer. *Exit:* QG-7's two equations over every accept fixture.
4. `sq_parse` fuzz target and the QG-3/QG-4 source guards. *Exit:* ten minutes clean — reached
   on the fourth run, after the three defects §5 lists. The determinism-matrix entry moves to
   step 7, where the command it would run exists.

## 7. Risks and mitigations

| risk | mitigation |
|---|---|
| The grammar is recovered from a table, so a form draft 3 had may be missing | Every production cites the SMYSL-2.5 text it came from; anything with no citation is marked an addition and carried as an OQ. A form found later is additive under QG-9 if it is a stage |
| Four implementations drift | The conformance suite is language-free (§5) and QG-4/5/6 name the three drifts that are otherwise invisible until a port disagrees |
| A later reader re-adds synonyms for readability | §3.2 and §4.11 record that the table was considered and dropped, and why: four ports must agree on it or QG-7 breaks between them |
| `match` is specified as a hole | §4.9 captures it with balanced delimiters, so OQ-50 can be decided without reopening the grammar |
| Enforced stage order annoys | It is the planner's order (SMYSL-2.5 §3.2). The diagnostic names both stages and the order they must take |

## 8. Diagnostics allocated in this RFC (SMY-E/W510–519)

| code | meaning |
|---|---|
| W510 | a string literal is not in NFC; it is used byte-exactly as written and may match nothing |

E511–W519 are free. Syntax errors are `SMY-E460` and stage availability is `SMY-E461`, both from
SMYSL-2.5 §8; this document adds no code it does not use.

Codes from other documents used here: E410 (EDTF), E427 (`sq` version), E460 (syntax, unbound
parameter), E461 (stage unavailable), E462 (undeclared relation kind), E421/E422 (field
resolution and retired fields, SMYSL-2.5 §3.9).

## 9. Open questions

All eight were **decided 2026-10-15**, and each row keeps what was rejected, because a decision
without its alternative is not reviewable. Three of them changed the draft; they are marked.

| id | decision |
|---|---|
| OQ-75 | **Decided: a separator is required, either a newline or `|`; the printer emits a newline.** It is the only form serving both a shell one-liner and a multi-line template, where trailing pipes are the dangling-`|` error SMYSL-2.5 §5.1 already names. *Rejected:* `|` mandatory (noisy in files); newline only (breaks one-liners); and **no separator at all**, which parses unambiguously — stage heads are reserved at stage-head position — and was rejected because a required separator is what lets a diagnostic say *"`where` may not follow `order by`"* instead of guessing where the previous stage ended, and because it widens QG-9's blast radius for every future keyword |
| OQ-76 | **Decided: `channels` bare for all, `channel:name` for one** (§4.3). *Changed the draft.* SMYSL-2.5 §3.3 writes bare `channel`, the only member of its list not following the plural-bare/singular-colon rule its four neighbours follow. *Rejected:* bare-only (cannot name one); colon-only (no way to ask about all) |
| OQ-77 | **Decided: the modifier attaches to `where`, `order by`, `group by` and `compare by`** — the four stages that can hold a qualified field reference (§4.7). *Rejected:* any stage (too broad); a header clause (not local to what it modifies); per-field (noisy). **Carried back to SMYSL-2.5:** its §3.1 and §3.9 disagree about whether qualified access is lensed at all, and until that is settled the modifier's *meaning* is open even though its placement is not |
| OQ-78 | **Decided: `as recorded` modifies `in time order`** (§4.8); `in time order` alone reads SMYSL-2.4's effective time. *Rejected:* attaching it to the time predicate, which already carries its axis in `said`/`observed` and so would make the modifier redundant; a query-level clause, which would act at a distance |
| OQ-79 | **Decided: `from` is optional, and a saved query's canonical print records the scope resolved when it was saved** (§3.4, §4.3). This is QG-8 applied to scope. *Rejected:* always required (boilerplate on every CLI call, and it can disagree with the positional store argument); optional with nothing recorded (leaves a saved question ambiguous and two reruns incomparable) |
| OQ-80 | **Decided: no synonyms** (§3.2, §4.11). *Changed the draft.* The cost is not determinism, it is that four implementations must carry an identical table or QG-7 breaks between ports. The benefit disappears once the one spelling is chosen for a reader rather than copied from a function name: `rests on` is the sole spelling and `rests_on` is not admitted. *Rejected:* the three-row table; an open table |
| OQ-81 | **Decided: `ascending`/`descending` in full, default ascending for every key** (§4.8). *Changed the draft.* Determinism does not turn on it, so this is a reading decision and the abbreviation buys nothing. *Rejected:* `asc`/`desc`; and a per-key default — which reads better in the common case and was rejected because an ordering whose direction depends on which field was named is the kind of surprise that makes people stop trusting the order |
| OQ-82 | **Decided: keep the balanced-delimiter capture** (§4.9). It buys one real thing — a user who read about `match` gets `SMY-E461` "not available in this build or phase" rather than `SMY-E460` "unexpected word". The objection, that raw text is not normalised so two queries differing only in `match` whitespace hash differently, is unreachable **provided saving a query requires a successful plan**, since planning always refuses `match`; that precondition should be confirmed in SMYSL-2.5 §3.6. *Rejected:* deciding the Cypher subset now (OQ-50 is not ready); removing `match` from `sq/1` (safe under QG-9, but loses the diagnostic) |

Open elsewhere, raised here: **OQ-50** (`match`'s pattern language, SMYSL-2.5) and the §3.1/§3.9
contradiction OQ-77 records against SMYSL-2.5 §3.9.

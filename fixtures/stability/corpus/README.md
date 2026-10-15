# GE-T2's corpus: extraction stability, per language

SMYSL-2.4 §5.5's **GE-T2** asks how stable this build's extraction is over a corpus, measured as
`identical-span` agreement between the two passes of one window and reported **per language**.
The instrument was met in TX-P5 step 5 against a scripted provider; the corpus was owed, because
a figure about a *model* cannot be answered by a provider that says what the test told it to say.

This is the corpus. Five files, one per tier-1 language, named by the language tag the harness
reads off the file stem.

## Where it comes from

Every passage is a public-domain verse-per-line edition from [eBible](https://ebible.org/), and
each file is three passages concatenated in a fixed order: narrative, then law, then poetry.

| lang | edition | narrative | law | poetry | distribution |
|---|---|---|---|---|---|
| de | luther1912 | Gen.4 | Ex.20 | Ps.14 | `deu1912_vpl.zip` |
| en | kjv1769 | Gen.4 | Ex.20 | Ps.14 | `eng-kjv_vpl.zip` |
| es | rv1909 | Gen.4 | Ex.20 | Ps.14 | `spaRV1909_vpl.zip` |
| fr | lsg1910 | Gen.4 | Ex.20 | Ps.14 | `fraLSG_vpl.zip` |
| ru | syn1876 | Gen.4 | Ex.20 | **Ps.13** | `russyn_vpl.zip` |

Each distribution is at `https://ebible.org/Scriptures/<name>`.

Russian carries **Ps.13** where the others carry Ps.14. That is not a different psalm: it is the
Septuagint/Synodal numbering against the Masoretic, the same published convention TX-P4 step 4's
`align` table already encodes as its KJV ↔ Synodal clause. The text is the same poem.

## Why these passages, and not others

The passages are **not chosen here**. They are the S0 extraction spike's (SMYSL-2.2), fixed
before any of this code existed, and the rule that picks these three out of the spike's eleven
is mechanical: *every reference all five tier-1 languages carry*. The spike's other eight
references exist only in English and Russian, so including them would have given two languages
five genres the other three lack — and GE-T2 reports per **language**, which would then have been
a figure about genre. The same discipline as `fixtures/attribution/probes/`: the authored choice
is removed rather than defended.

What that buys is a corpus identical in shape across all five languages — 26 verses of narrative,
26 of law, 7 of poetry, **59 lines in every file** — so the only thing varying between the five
reported figures is the language.

## Checking it

Every source file was verified against the `sha256` column of the spike's own `manifest.tsv`
before assembly (29 of 29). The assembled files:

| file | sha256 (first 16) | lines | bytes |
|---|---|---|---|
| `de.txt` | `bcdcf7330fe8e100` | 59 | 7487 |
| `en.txt` | `2cbf36281a2be1a3` | 59 | 7476 |
| `es.txt` | `989f2cca181d291c` | 59 | 7143 |
| `fr.txt` | `b56b850e17d8532d` | 59 | 7822 |
| `ru.txt` | `e0803e07d31b6e84` | 59 | 10954 |

Each line begins with its own reference (`Gen.4.1 `, `Ps.13.6 `), as the verse-per-line
distributions write it. The harness windows at level `line`, so one line is one node, and the
prompt carries the locators while the quote is checked without them.

## Running it

```text
SMYSL_INGEST_LIVE=1   consent to spend a credential
SMYSL_GE_T2=<dir>     another corpus, in place of this one
```

`SMYSL_INGEST_LIVE` is required and has no default: the measurement costs money, so it is never
taken by accident. `SMYSL_GE_T2` defaults to this directory. Without the first, the harness
reports the clause as owed rather than passing quietly.

The figure is **reported, never thresholded** (§5.5): the computed form has no bar, because the
bar it had — 0.6 on `J_class` — belongs to the judged form, and that waits for GE-T9's human α.

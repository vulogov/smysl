# Reader fixtures

One small sample per TX-P1 reader, and the identities each one produces.

`crates/smysl-text/tests/readers.rs` reads every sample, cuts it into parts, builds a reading
and a manifest, and compares the result against the `.expected` file beside it — **and writes
that file when `SMYSL_BLESS=1` is set**. Generated and checked by the same plain test, for the
reason `fixtures/library/wire/` is: a generator that only ever writes leaves the expectations
agreeing with a build nobody ran.

What is pinned is every identity a corpus is addressed by — the tid of each part, the structure
hash of the reading, its rdid, and the mid of the manifest over them. If a reader's output
changes by one byte, all four move, and that is the point: a reader is the thing a library's
identities are *derived from*, so a silent change in one is a silent change in every tid taken
with it (SMYSL-2.4 §7, the reason reader dependencies are pinned with `=`).

## The samples

| file | reader | what it is |
|---|---|---|
| `gen1.usfm` | `usfm/1` | Genesis 1:1-5, King James Version (1769). Public domain. |
| `gen1.osis` | `osis/1` | the same five verses as OSIS, container form. Public domain. |
| `gen1-milestone.osis` | `osis/1` | the same five verses again, milestone form. Public domain. |
| `gen1.zefania` | `zefania/1` | Genesis 1:1-3 from Luther's 1912 revision. Public domain. |
| `notes.md` | `md/1` | written for this fixture. |
| `notes.txt` | `txt/1` | the same prose as plain text, written for this fixture. |
| `chat.json` | `json/1` | a three-message exchange, written for this fixture. |

The scripture samples are editions whose copyright has expired, which is also what lets them be
`licence: public-domain` with `carry: text` in the expected manifests — the licence gate
(`SMY-E402`) is exercised by the same test, and a fixture it refused would be a fixture that
could not be distributed with this repository.

The two OSIS samples exist as a pair on purpose: they are different XML with the same text, and
the test asserts that their tids are **equal**. Half the OSIS Bibles in circulation use each
form, and a corpus built from one has to align with a corpus built from the other.

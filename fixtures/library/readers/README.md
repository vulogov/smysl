# Reader fixtures

One small sample per reader, and the identities each one produces.

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
| `telegram.json` | `telegram/1` | a Telegram Desktop export of a two-day reading group, written for this fixture. |
| `whatsapp.txt` | `whatsapp/1` | the same conversation as a WhatsApp `_chat.txt`, written for this fixture. |
| `slack.zip` | `slack/1` | the same conversation as a Slack workspace export, written for this fixture. |

The scripture samples are editions whose copyright has expired, which is also what lets them be
`licence: public-domain` with `carry: text` in the expected manifests — the licence gate
(`SMY-E402`) is exercised by the same test, and a fixture it refused would be a fixture that
could not be distributed with this repository.

## The chat samples

The three are two days of the same imagined conversation in three formats, which is deliberate:
they are the nearest thing this repository can hold to GE-T1's "two chat exports", and the three
of them together exercise what the formats disagree about rather than what they share. Each has
English and Russian messages, a sender whose name is **not** in the text, and something the
reader drops and reports (`lossy: true`): a Telegram service message and a photo, WhatsApp's
encryption notice, Slack's `channel_join` and its `users.json`.

Two of them carry a deliberate trap, and the `.expected` files are where it shows:

- **`slack.zip`'s `general/2024-01-15.json` holds a message timestamped `02:00Z`**, which belongs
  to the **16th**. Slack writes one file per channel per *local* day and the format's parts are
  UTC days, so a reader that trusted the file name would cut a part on a boundary that is in no
  field of the data. The expectation has that message as `general.20240116.1`.
- **`telegram.json` carries both `date` and `date_unixtime`**, and they disagree, because `date`
  is a local wall clock with no offset in it. Only the second is an instant.

`slack.zip` is a real deflated archive with the UTF-8 name flag set, written by Python's
`zipfile` with a fixed timestamp so that rebuilding it gives the same bytes. Its members:

```
channels.json              general, reading — names and ids
users.json                 read by nothing: a display name is never a speaker
general/2024-01-15.json    4 messages, one of them a channel_join
general/2024-01-16.json    2 messages
reading/2024-01-16.json    1 message
```

It is also the only fixture in this tree that is not text, and it is read by two suites: the
reader expectations here, and `tests/limits.rs`, whose archive cap cases need an archive that a
real zipper wrote rather than one assembled by the code under test.

The two OSIS samples exist as a pair on purpose: they are different XML with the same text, and
the test asserts that their tids are **equal**. Half the OSIS Bibles in circulation use each
form, and a corpus built from one has to align with a corpus built from the other.

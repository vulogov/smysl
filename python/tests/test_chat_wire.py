"""GE-T1's chat half: the identities of a chat corpus, recomputed here.

``fixtures/library/wire/`` gives this port a scripture-shaped expression whose records were
hand-built in ``smysl-core`` — correctly, in a crate with no reader. ``chat-wire`` gives it the
records a real ``text add`` emitted from three chat exports, which is where the three identities
are derived over things scripture has none of: segment tables with speakers, timestamps and
platform ids in them, ``mul`` manifests, and ``raw`` metadata inside the mid.

This port implements no reader and is not asked to. It decodes the records and hashes them.
"""

from __future__ import annotations

import json
from pathlib import Path

import smysl

WIRE = Path(__file__).parents[2] / "fixtures" / "library" / "chat-wire"
IDS = json.loads((WIRE / "ids.json").read_text())
RECORDS = (WIRE / "records.cbor").read_bytes()


def _records():
    return smysl.decode_store(RECORDS)


def test_there_are_vectors_to_check():
    assert IDS["expressions"], "a suite with no fixture passes vacuously"
    assert len(IDS["expressions"]) == 3, "one expression per export"


def test_the_chat_fixture_round_trips():
    """C-Read, over records a reader produced rather than records a test hand-built."""
    records = _records()
    assert smysl.encode_store(records) == RECORDS
    codes = sorted({r.code for r in records})
    assert codes == [14, 15, 18]


def test_every_identity_is_recomputed_from_the_records():
    """**The exit.** Each tid, structure hash, rdid and mid, derived here and compared.

    Derived from the records decoded and **re-encoded**, not from the bytes as they arrived:
    hashing the input would check the hash, and hashing what this implementation's own encoder
    produces checks the encoder too. A mismatch then says which of the two disagreed.
    """
    records = _records()
    by_mid = {}
    for r in records:
        if r.code == 14:
            manifest = smysl.Manifest.decode(r.body)
            by_mid[smysl.mid(smysl.encode_one(r.body)).hex()] = manifest
    texts = {
        smysl.PartText.decode(r.body).tid.hex(): smysl.PartText.decode(r.body)
        for r in records
        if r.code == 15
    }
    readings = {}
    for r in records:
        if r.code == 18:
            reading = smysl.PartReading.decode(r.body)
            readings[smysl.rdid(smysl.encode_one(r.body)).hex()] = reading

    for expression in IDS["expressions"]:
        manifest = by_mid.get(expression["mid_hex"])
        assert manifest is not None, f"no manifest hashes to {expression['mid_hex']}"
        assert manifest.fields["reader"] == expression["reader"]
        assert manifest.fields["lang"] == "mul", "a chat export is not one language"
        assert len(manifest.parts) == len(expression["parts"])

        for entry, want in zip(manifest.parts, expression["parts"]):
            assert entry.tid.hex() == want["tid_hex"]
            assert entry.length == want["length"]
            assert entry.structure.hex() == want["structure_hex"]
            assert entry.rdid.hex() == want["rdid_hex"]

            # The part's own bytes hash to the tid the manifest records.
            part = texts[want["tid_hex"]]
            assert smysl.tid(part.text).hex() == want["tid_hex"]
            assert len(part.text) == want["length"]

            # And the reading's table hashes to the structure hash.
            reading = readings[want["rdid_hex"]]
            assert reading.structure_hash().hex() == want["structure_hex"]
            assert len(reading.rows()) == want["segments"]
            reading.verify_entry(entry)


def test_a_chat_reading_names_who_said_what():
    """The thing scripture has none of, and the reason this fixture exists.

    A segment row's speaker, timestamp and platform ids are inside the structure hash and
    therefore inside the rdid — so a port that dropped them would still decode the record and
    would disagree about the identity.
    """
    speakers, observed, ids = 0, 0, 0
    for r in _records():
        if r.code != 18:
            continue
        for row in smysl.PartReading.decode(r.body).rows():
            speakers += "speaker" in row
            observed += "observed" in row
            ids += "ids" in row
    assert speakers > 0, "a chat reading names its speakers"
    assert observed > 0, "and when each message was sent"
    assert ids > 0, "and the platform's own id for it"


def test_raw_metadata_is_inside_the_mid():
    """Two of the three exports carry a reader's header as manifest key 16.

    Carried as opaque canonical CBOR, so nothing here decodes it — and that is exactly why it is
    worth checking: a port that dropped an opaque key would re-encode a shorter body and compute
    a different mid, with nothing else to notice.
    """
    with_raw = 0
    for r in _records():
        if r.code != 14:
            continue
        if 16 in r.body:
            with_raw += 1
            assert smysl.mid(smysl.encode_one(r.body)).hex() in {
                e["mid_hex"] for e in IDS["expressions"]
            }
    assert with_raw == 2, "telegram and slack carry `raw`; whatsapp has none"

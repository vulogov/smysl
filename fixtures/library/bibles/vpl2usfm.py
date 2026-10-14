#!/usr/bin/env python3
r"""eBible verse-per-line -> USFM, one file per book, for GE-T1's corpus.

The text is not touched: a line `GEN 1:1 In the beginning...` becomes `\v 1 In the
beginning...` under the `\c 1` it belongs to, with `\id <USFM code>` at the top. The only
judgement here is the book-code mapping (eBible writes the older Paratext codes: JOH, MAR,
JAM) and the decision to carry the 66-book canon and skip the deuterocanon, which this
build's book table does not hold.
"""
import os, sys, re, collections

# eBible's code -> the USFM code `smysl_text::readers::books` knows.
RENAME = {"SOL": "SNG", "EZE": "EZK", "JOE": "JOL", "NAH": "NAM",
          "MAR": "MRK", "JOH": "JHN", "PHI": "PHP", "JAM": "JAS",
          "1JO": "1JN", "2JO": "2JN", "3JO": "3JN"}
CANON = """GEN EXO LEV NUM DEU JOS JDG RUT 1SA 2SA 1KI 2KI 1CH 2CH EZR NEH EST JOB PSA PRO
ECC SNG ISA JER LAM EZK DAN HOS JOL AMO OBA JON MIC NAM HAB ZEP HAG ZEC MAL MAT MRK LUK JHN
ACT ROM 1CO 2CO GAL EPH PHP COL 1TH 2TH 1TI 2TI TIT PHM HEB JAS 1PE 2PE 1JN 2JN 3JN JUD
REV""".split()
ORDER = {c: n for n, c in enumerate(CANON)}
LINE = re.compile(r"^([0-9A-Z]{3})\s+(\d+):(\d+)\s*(.*)$")

def convert(src, out):
    books = collections.OrderedDict()
    skipped = collections.Counter()
    for raw in open(src, encoding="utf-8"):
        raw = raw.rstrip("\n")
        if not raw.strip():
            continue
        m = LINE.match(raw)
        if not m:
            skipped["unparsed"] += 1
            continue
        code, ch, v, text = m.group(1), int(m.group(2)), int(m.group(3)), m.group(4).strip()
        code = RENAME.get(code, code)
        if code not in ORDER:
            skipped[code] += 1
            continue
        if not text:
            skipped["empty"] += 1
            continue
        books.setdefault(code, collections.OrderedDict()).setdefault(ch, []).append((v, text))
    os.makedirs(out, exist_ok=True)
    for code in sorted(books, key=lambda c: ORDER[c]):
        path = os.path.join(out, "%02d-%s.usfm" % (ORDER[code] + 1, code))
        with open(path, "w", encoding="utf-8") as f:
            f.write("\\id %s\n" % code)
            for ch in sorted(books[code]):
                f.write("\\c %d\n" % ch)
                for v, text in books[code][ch]:
                    f.write("\\p\n\\v %d %s\n" % (v, text))
    verses = sum(len(vs) for b in books.values() for vs in b.values())
    print("%-12s %2d books, %5d chapters, %6d verses, skipped %s"
          % (os.path.basename(out), len(books),
             sum(len(b) for b in books.values()), verses,
             ", ".join("%s=%d" % kv for kv in sorted(skipped.items())) or "nothing"))

if __name__ == "__main__":
    convert(sys.argv[1], sys.argv[2])

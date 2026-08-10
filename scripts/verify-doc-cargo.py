"""Replay the manual's `cargo` transcripts, and check its feature table against the manifest.

`verify-doc-output.py` replays the documented `smysl` commands. It cannot replay the `cargo`
ones — they are a different program, and its skip rules pass over them — so nothing ever
checked them, and they drifted exactly as you would expect of text nobody verifies.

0.14 found three stale claims in the manual and every one was in a block this script covers:

  * `$ cargo build` showed `smysl v0.1.0` while the crate was 0.13.0;
  * `$ cargo tree --no-default-features` predated `smysl-retrieve` becoming a plain
    dependency, so it omitted `bm25`, `fxhash` and `byteorder` entirely;
  * the feature table said `default` turns on `tui`, which it does not and never did —
    `Cargo.toml`'s own comment calls `tui` "deliberately absent".

The first of those had drifted *again* by 1.1, one release after being fixed by hand. A
version number in prose goes stale every release; that is not a reason to remove it, it is a
reason to check it.

    python3 scripts/verify-doc-cargo.py

Exits non-zero on drift, so it can be a gate rather than a report — the lesson
`verify-doc-output.py` records about fifteen phantom mismatches nobody read.

# What is normalised, and why each one has to be

Cargo's output is not byte-reproducible, so a raw comparison would fail on every run and teach
people to ignore it. Three things vary and none of them is what the manual is claiming:

  * **Timings.** `in 0.37s` is not a fact about the project.
  * **The absolute path.** `(/Users/gandalf/Src/smysl)` is one machine's checkout.
  * **Cargo's own progress.** Which crates needed recompiling depends on what was already
    built, and `cargo xtask determinism` documents what the *gate* printed rather than that
    cargo linked it first. `Compiling`, `Finished` and `Running` lines are dropped from both
    sides.

What is left after that is what the transcript is actually asserting: dependency structure,
error text, and the output of the xtask gates.

Dropping the progress lines leaves `cargo build` with nothing to compare — its whole body is
`Compiling` and `Finished`. The version inside them is still a claim, and it is the one that
goes stale every single release, so it is checked separately against the manifest rather than
left unchecked. A gate that cannot tell "nothing was wrong" from "nothing was checked" is the
failure this repository keeps rediscovering.
"""

import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)

# `#screen(caption: "$ cmd")[` then a fenced block, with the fence on either line.
BLOCK = re.compile(
    r'#screen\(caption:\s*"(?P<cap>(?:[^"\\]|\\.)*)"\s*\)\[\s*\n?```\n(?P<out>.*?)```',
    re.S,
)

TIMING = re.compile(r'\bin \d+(\.\d+)?(s|ms|m \d+s)\b')
DURATION = re.compile(r'\[\s*\d+\.\d+s\]')
CRATE_PATH = re.compile(r' \(<root>[^)]*\)')
VERSION = re.compile(r'\bsmysl(?:-[a-z]+)? v(\d+\.\d+\.\d+)')

# Cargo's own progress, which the manual trims because it is not what the transcript shows.
# `cargo xtask determinism` documents what the *gate* printed, not that cargo linked it first.
PROGRESS = ('Compiling', 'Finished', 'Running `', 'Blocking', 'Updating', 'Downloaded',
            'Downloading', 'Locking', 'Adding')


def norm(text):
    """Strip what varies between two correct runs, and cargo's own chatter."""
    out = []
    for line in text.strip().split('\n'):
        line = line.replace(ROOT, '<root>')
        line = TIMING.sub('in <time>', line)
        line = DURATION.sub('[<time>]', line)
        line = CRATE_PATH.sub('', line)
        if line.strip().startswith(PROGRESS):
            continue
        out.append(line.rstrip())
    while out and not out[-1]:
        out.pop()
    return out


def crate_version():
    """The workspace version, read once from the manifest."""
    if not hasattr(crate_version, '_v'):
        with open('Cargo.toml') as fh:
            crate_version._v = re.search(r'^version\s*=\s*"([^"]+)"', fh.read(),
                                         re.M).group(1)
    return crate_version._v


def run(cmd):
    """Run a cargo command and return stdout+stderr as one stream, as a terminal shows it."""
    p = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    return (p.stdout or '') + (p.stderr or '')


def check_transcripts():
    """Replay every `$ cargo …` block that is a single command."""
    ran = skipped = 0
    bad, stale = [], []
    for path in sorted(f for f in os.listdir('Documentation/manual') if f.endswith('.typ')):
        full = os.path.join('Documentation/manual', path)
        with open(full) as fh:
            src = fh.read()
        for m in BLOCK.finditer(src):
            cap = m.group('cap')
            if not cap.startswith('$ cargo'):
                continue
            cmd = cap[2:].strip()
            # Compound commands chain a `smysl` invocation onto a build; the build half is
            # not what the transcript is showing, and replaying the whole thing would need
            # the fixtures that `verify-doc-output.py` already handles.
            if '&&' in cmd or '|' in cmd or '>' in cmd:
                skipped += 1
                continue

            raw = m.group('out')
            claimed = norm(raw)
            actual = norm(run(cmd))
            ran += 1

            if claimed != actual:
                bad.append((path, cmd, claimed, actual))

            # Cargo's progress lines are dropped above, which for `cargo build` leaves nothing
            # to compare — its whole body is `Compiling` and `Finished`. The version inside
            # them is still a claim, and it is the one that goes stale every single release,
            # so it is checked separately rather than left unchecked.
            for v in VERSION.findall(raw):
                if v != crate_version():
                    stale.append((path, cmd, v))
    return ran, skipped, bad, stale


def check_feature_table():
    """The manual's feature table against `[features]` in the manifest.

    This is the defect the transcripts would not have caught: a table saying `default` turns
    on `tui` when the manifest says otherwise. The table is prose about a fact, and the fact
    is three lines away in a file nobody diffs against it.
    """
    with open('Cargo.toml') as fh:
        manifest = fh.read()
    feats = manifest.split('[features]', 1)[1].split('\n[', 1)[0]
    real = {}
    for line in feats.split('\n'):
        line = line.split('#')[0].strip()
        if not line or '=' not in line:
            continue
        name, rest = line.split('=', 1)
        real[name.strip()] = set(re.findall(r'"([^"]+)"', rest))

    bad = []
    # Only the table that *claims* to be a transcription is checked against the manifest.
    #
    # The manual has more than one feature table, and the others are prose — they describe
    # what a feature is for, mentioning `ratatui` and `crossterm` in a sentence about the TUI.
    # Comparing those to `[features]` produced eleven mismatches and not one of them was a
    # defect, which is the shape of check that teaches people to ignore checks. The table in
    # chapter 27 says "transcribed here exactly, edges and all", and that claim is fair to
    # test; the marker is what scopes this rather than a filename.
    MARKER = 'transcribed here exactly'
    for path in sorted(f for f in os.listdir('Documentation/manual') if f.endswith('.typ')):
        full = os.path.join('Documentation/manual', path)
        with open(full) as fh:
            src = fh.read()
        if MARKER not in src:
            continue
        src = src[src.index(MARKER):]
        for name, claimed in re.findall(r'\(\[`([a-z-]+)`\], \[(.*?)\]\),', src, re.S):
            if name not in real:
                continue
            # The row names what the feature turns on, in backticks, before the em dash that
            # begins the explanation.
            head = claimed.split('—')[0]
            named = set(re.findall(r'`([a-z][a-z0-9:_/-]*)`', head))
            expected = set(real[name])
            # A row may name another crate's feature (`smysl-render/typst`); the manifest
            # spells those the same way, so they compare directly, but a row that summarises
            # a group of them with a brace is prose and is left alone.
            if any('{' in c for c in claimed.split('—')[:1]):
                continue
            if named != expected:
                bad.append((path, name, sorted(expected), sorted(named)))
    return bad


def main():
    ran, skipped, bad, stale = check_transcripts()
    feature_bad = check_feature_table()

    print(f'cargo transcripts: ran {ran}, skipped {skipped}, MISMATCHED {len(bad)}')
    print(f'stale version strings: {len(stale)}  (crate is {crate_version()})')
    print(f'feature table rows: MISMATCHED {len(feature_bad)}')
    print()

    for path, cmd, claimed, actual in bad:
        print(f'=== {path}  |  $ {cmd}')
        import difflib
        for line in list(difflib.unified_diff(claimed, actual, 'claimed', 'actual',
                                              lineterm='', n=1))[2:14]:
            print(line)
        print()

    for path, name, expected, named in feature_bad:
        print(f'=== {path}  |  feature `{name}`')
        print(f'    manifest: {expected}')
        print(f'    manual:   {named}')
        print()

    for path, cmd, v in stale:
        print(f'=== {path}  |  $ {cmd}')
        print(f'    documents version {v}; the crate is {crate_version()}')
        print()

    sys.exit(1 if (bad or stale or feature_bad) else 0)


if __name__ == '__main__':
    main()

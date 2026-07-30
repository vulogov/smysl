"""Re-run every documented command that operates on a corpus fixture and compare it
against the output the manual claims.

The manual quotes ~190 command outputs. Nothing checked them, so when 0.2 changed what
`check` reports — a labelled unit now yields two records — 34 documented outputs became
wrong at once and the only way to find out was to read all of them. This finds the ones
that operate on a shipped fixture and can therefore be replayed.

    python3 scripts/verify-doc-output.py

Requires a *default-features* build. This bit me while writing it: a stale
`--all-features` binary made a correct `SMY-W202` claim look like drift, because
`exact-pack` was compiled in when the doc assumed it was not.

Known limits, all of them "skipped" rather than silently passed:

  * commands with a pipe, redirect or `$(…)` are not replayed;
  * commands naming a file the chapter built earlier in its own narrative are skipped,
    since the setup is prose;
  * the docs trim long output with `…` and wrap long lines to the page, so a reported
    mismatch is often formatting rather than drift — read the diff before believing it.

What it does catch reliably is a changed *number*, which is the failure mode that
actually happens."""
import re, subprocess, glob, os, sys

ROOT = '/Users/gandalf/Src/smysl'
os.chdir(ROOT)
SM = './target/debug/smysl'

# #screen(caption: "$ cmd")[ ``` ...output... ``` ]
BLOCK = re.compile(
    r'#screen\(caption:\s*"(?P<cap>[^"]*)"\s*\)\[\s*\n```\n(?P<out>.*?)```',
    re.S)

mismatches, ran, skipped = [], 0, 0
for f in sorted(glob.glob('Documentation/manual/*.typ')):
    src = open(f).read()
    for m in BLOCK.finditer(src):
        cap, claimed = m.group('cap').strip(), m.group('out')
        if not cap.startswith('$ smysl') or 'fixtures/corpus' not in cap:
            skipped += 1
            continue
        # Captions carry human annotations like "(--grounds is the default)"; strip them.
        cmd = re.sub(r'\s{2,}\(.*\)$', '', cap[2:]).strip()
        cmd = cmd.replace('smysl', SM, 1)
        # Skip anything naming a file the chapter built earlier in its own narrative.
        paths = [t for t in cmd.split() if ('/' in t and not t.startswith('-')
                                            and not t.startswith('b3:'))]
        if any(not os.path.exists(t) for t in paths):
            skipped += 1
            continue
        # commands with shell extras are not safely replayable here
        if any(t in cmd for t in ('|', '>', '<', '&&', ';', '$(')):
            skipped += 1
            continue
        p = subprocess.run(cmd, shell=True, capture_output=True)
        try:
            actual = (p.stderr + p.stdout).decode('utf-8')
        except UnicodeDecodeError:
            # rule P: stdout is CBOR on a pipe. Only stderr is comparable text.
            actual = p.stderr.decode('utf-8', 'replace')
        ran += 1
        # normalise: trailing whitespace only
        norm = lambda s: '\n'.join(l.rstrip() for l in s.strip().split('\n'))
        if norm(actual) != norm(claimed):
            mismatches.append((f, cap, norm(claimed), norm(actual)))

print(f'ran {ran}, skipped {skipped}, MISMATCHED {len(mismatches)}\n')
import difflib
for f, cap, exp, act in mismatches:
    print(f'=== {os.path.basename(f)}  |  {cap}')
    d = list(difflib.unified_diff(exp.split('\n'), act.split('\n'),
                                  'claimed', 'actual', lineterm='', n=0))
    print('\n'.join(d[2:12]) if len(d) > 2 else '(whitespace only)')
    print()

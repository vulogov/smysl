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
  * the docs trim long output with `...`, abbreviate a caption with `…`, wrap long lines
    to the page, quote one stream of two, or name a build this script is not running.

All five are now understood and handled rather than reported as drift, so a mismatch here
means the manual and the binary genuinely disagree. Verified by breaking one documented
count by a single character and confirming this script catches it."""
import re, subprocess, glob, os, sys

# Derived from this file's location, not hardcoded. It was an absolute path to one
# developer's home directory, so the script ran nowhere else — including CI, where it
# failed on `os.chdir` before comparing a single transcript.
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)
SM = './target/debug/smysl'

# #screen(caption: "$ cmd")[ ``` ...output... ``` ]
# `\"` inside the caption is part of the caption, not its end. The first version stopped at
# any quote, so a documented command containing a quoted argument — `find "connection pool"`
# — was truncated to nonsense, failed the `fixtures/corpus` test, and was silently skipped.
# Two transcripts were unverified the moment they were written, which is precisely the drift
# this script exists to catch.
BLOCK = re.compile(
    r'#screen\(caption:\s*"(?P<cap>(?:[^"\\]|\\.)*)"\s*\)\[\s*\n```\n(?P<out>.*?)```',
    re.S)

mismatches, ran, skipped, excerpts = [], 0, 0, 0
for f in sorted(glob.glob('Documentation/manual/*.typ')):
    src = open(f).read()
    for m in BLOCK.finditer(src):
        # Undo the Typst escaping so the command is what a shell would receive.
        cap = m.group('cap').replace('\\"', '"').strip()
        claimed = m.group('out')
        if not cap.startswith('$ smysl') or 'fixtures/corpus' not in cap:
            skipped += 1
            continue
        # A caption carrying an ellipsis is an abbreviation of the command, not the command
        # — the block itself spells it out across several lines. Replaying the caption would
        # run something the manual never claimed to run.
        if '…' in cap:
            skipped += 1
            continue
        # An annotation naming a build this script is not running describes a different
        # binary. Replaying it against the default build compares two different programs.
        if 'built with' in cap:
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
        dec = lambda b: b.decode('utf-8', 'replace')
        # Twice, deliberately. The first run points both streams at one pipe, so the lines
        # arrive in *write* order — which is what a terminal shows and therefore what a
        # transcript copied from one looks like. Concatenating two separately-captured
        # streams does not reproduce that: `check --granularity` writes its view line to
        # stdout, its warnings to stderr, and its summary back to stdout, so either
        # concatenation reorders a transcript that was correct all along. This alone
        # accounted for a third of the "drift" this script used to report.
        merged = subprocess.run(cmd, shell=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT)
        # The second run keeps them apart, because a block may quote one stream alone: a
        # command whose stdout is a store prints its report on stderr, and rule P makes that
        # stdout CBOR on a pipe, which is not comparable text at all.
        p = subprocess.run(cmd, shell=True, capture_output=True)
        candidates = [dec(merged.stdout), dec(p.stderr), dec(p.stdout)]
        ran += 1
        # normalise: trailing whitespace only
        def norm(s):
            lines = [l.rstrip() for l in s.strip().split('\n')]
            # Rejoin a gist wrapped for the page. A printed transcript has a column limit
            # the terminal does not, so the manual breaks a long `~ …` line and indents the
            # remainder by two spaces — which is the surface continuation rule, so joining it
            # back is reading the transcript as the format defines it rather than papering
            # over a difference.
            out = []
            for l in lines:
                if out and out[-1].startswith('~ ') and l.startswith('  ') and l[2:3] != ' ':
                    out[-1] += ' ' + l.strip()
                else:
                    out.append(l)
            return '\n'.join(out)
        # A build that lacks the feature a command needs answers by saying so. Comparing
        # that against a transcript taken from a build that has it compares two different
        # programs — the same reason a caption annotated "built with …" is skipped. This
        # one is detected from the answer rather than the caption, because the manual does
        # not annotate it: `make doc-output` builds default features on purpose, and
        # `attest` needs `--features local`.
        if any('this build has no' in c for c in candidates):
            ran -= 1
            skipped += 1
            continue
        want = norm(claimed)
        if any(want == norm(c) for c in candidates):
            continue
        # A lone `...` line is the manual explicitly declaring an elision. Honour it as
        # "any run of lines here", so a block that says it is showing part of the output is
        # checked on the part it shows.
        if any(l.strip() in ('...', '…') for l in want.split('\n')):
            import fnmatch
            pat = '\n'.join('*' if l.strip() in ('...', '…') else
                            l.replace('*', '[*]').replace('?', '[?]').replace('[', '[[]')
                            for l in want.split('\n'))
            pat = re.sub(r'\n\*\n', '\n*', pat)
            if any(fnmatch.fnmatchcase(norm(c), pat) for c in candidates):
                excerpts += 1
                continue
        # Not identical to any stream — try the weaker claim, which is the one a screen
        # block actually makes: *every line shown is still produced, in the order shown*.
        # Manual transcripts elide (a `pack --explain` block quotes the report and discusses
        # the store it wrote separately, two paragraphs down), and an exact-equality check
        # can never pass one of those. It catches what matters — changed wording, changed
        # uids, changed counts, a line that stopped being emitted — and permits only the
        # addition of lines the block never claimed to show.
        def covers(actual):
            it = iter(norm(actual).split('\n'))
            return all(any(a == line for a in it) for line in want.split('\n'))
        if any(covers(c) for c in candidates):
            excerpts += 1
            continue
        mismatches.append((f, cap, want, norm(candidates[0])))

print(f'ran {ran}, skipped {skipped}, excerpt-matched {excerpts}, '
      f'MISMATCHED {len(mismatches)}\n')
import difflib
for f, cap, exp, act in mismatches:
    print(f'=== {os.path.basename(f)}  |  {cap}')
    d = list(difflib.unified_diff(exp.split('\n'), act.split('\n'),
                                  'claimed', 'actual', lineterm='', n=0))
    print('\n'.join(d[2:12]) if len(d) > 2 else '(whitespace only)')
    print()

# Exit non-zero on drift, so this can be a gate rather than a report.
#
# It ran as a report through 0.3 and nobody looked, which is how fifteen "mismatches" sat
# there — every one of them an artifact of how this script captured output rather than a
# manual that had gone stale. A check with a permanent backlog of false positives teaches
# people to ignore it, and then it catches nothing when something real breaks.
sys.exit(1 if mismatches else 0)

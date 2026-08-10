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
# The binary under test. Overridable so `tests/doc_output.rs` can point this at the binary
# cargo just built for it — `CARGO_BIN_EXE_smysl` — rather than at whatever happens to be in
# target/debug. That distinction is the whole reason the test exists: `cargo-mutants` rebuilds
# the binary with a mutation applied, and a check that replays a *stale* binary would report
# every mutant as caught while testing none of them.
SM = os.environ.get('SMYSL_BIN', './target/debug/smysl')
# Absolute, because tutorial commands run with their working directory set to the fixture
# folder and a relative `./target/debug/smysl` would point at nothing from there. Resolved
# after the chdir above, so it is still the binary this script was told to use.
SM = os.path.abspath(SM)

# #screen(caption: "$ cmd")[ ``` ...output... ``` ]
# `\"` inside the caption is part of the caption, not its end. The first version stopped at
# any quote, so a documented command containing a quoted argument — `find "connection pool"`
# — was truncated to nonsense, failed the `fixtures/corpus` test, and was silently skipped.
# Two transcripts were unverified the moment they were written, which is precisely the drift
# this script exists to catch.
# The `\n?` is load bearing, and its absence hid twenty-six blocks for the life of this
# script. Typst accepts the fence on the line after `#screen(...)[` or on the same line, and
# chapter 22–24 writes it the second way throughout:
#
#     #screen(caption: "$ smysl render …")[```
#
# Requiring the newline meant those blocks matched *nothing*. They were not skipped — skipping
# is counted and reported — they were invisible, so the denominator itself was wrong: "46 of
# 168" was really 46 of 194, and every render transcript in the book had gone unchecked since
# the chapter was written. `verify-doc-cargo.py` was written later and got this right, which
# is how the difference surfaced: two scripts scanning one book disagreed about how many
# blocks are in it.
#
# A checker that cannot see part of its subject reports a clean bill for the part it can.
BLOCK = re.compile(
    r'#screen\(caption:\s*"(?P<cap>(?:[^"\\]|\\.)*)"\s*\)\[\s*\n?```\n(?P<out>.*?)```',
    re.S)

def tutorial_dir(chapter_path, cap):
    """Where a chapter's committed tutorial files live, if this command's inputs are all there.

    Gate 7's larger half: 100 of the manual's commands name a file the *prose* asks the reader
    to create — `first.smy`, `cycle.smy` — so the script had nothing to run them against.

    Only files with a single state are committed. One filename often has several: chapter 1
    creates `first.smy` broken, fixes it, finds it unformatted and then rewrites it in place,
    so four commands name one path and expect four different files. Committing one of those
    would make the other three report drift that is not there, which this script's own history
    says is worse than no check — `fixtures/tutorial/README.md` records which files are held
    back and why.

    Returns None unless *every* bare filename the command names is present, so a command that
    mixes a committed file with an uncommitted one stays skipped rather than half-checked.
    """
    stem = os.path.basename(chapter_path).replace('.typ', '')
    d = os.path.join('fixtures', 'tutorial', stem)
    if not os.path.isdir(d):
        return None
    cmd = re.sub(r'\s{2,}\(.*\)$', '', cap[2:]).strip()
    toks = cmd.split()
    named = [t for i, t in enumerate(toks[1:], 1)
             if not t.startswith('-') and not t.startswith('b3:')
             and '.' in t and '/' not in t and not toks[i - 1] in ('-o', '--output')]
    if not named:
        return None
    return d if all(os.path.exists(os.path.join(d, t)) for t in named) else None


def is_label(tok):
    """A smysl label — `t/brief`, `c/pool-saturation`, `tiktoken/cl100k` — not a file.

    The path test below is "contains a slash", and smysl spells labels `kind/name`, so
    twenty-two commands were skipped as naming a file that does not exist when the token was
    never a filename: `--thread t/brief`, `--id v/two-roots`, `--focus c/pool-saturation`,
    `--tokenizer tiktoken/cl100k`, and `trace c/pool-saturation` positionally. Every render
    transcript in chapter 22–24 was held out by this alone.

    The discriminator is the dot. Every file the manual names carries an extension —
    `F1-incident.smy`, `project/store2.cbor` — and no label does; a label is one slash with a
    kind on the left. Checking the flag in front instead would need a list of every
    label-taking option, which is a list that goes stale silently.
    """
    return '.' not in tok and tok.count('/') == 1


mismatches, ran, skipped, excerpts = [], 0, 0, 0
for f in sorted(glob.glob('Documentation/manual/*.typ')):
    src = open(f).read()
    for m in BLOCK.finditer(src):
        # Undo the Typst escaping so the command is what a shell would receive.
        cap = m.group('cap').replace('\\"', '"').strip()
        claimed = m.group('out')
        # A command must name something this script can supply: a corpus fixture, or a
        # tutorial file committed under `fixtures/tutorial/<chapter>/`.
        #
        # The tutorial half is gate 7's "commit the files as fixtures". What it deliberately
        # does *not* do is change what the page tells the reader to type. The chapter still
        # says `smysl check cycle.smy`, the fixture is the same bytes the chapter prints, and
        # the command runs with its working directory set to the fixture folder — so the
        # diagnostics still say `cycle.smy: error: …` and the transcript compares as written.
        # Substituting a path into the command would have changed the output it produces and
        # made every such transcript drift on its own filename.
        cwd = tutorial_dir(f, cap) if 'fixtures/corpus' not in cap else None
        if not cap.startswith('$ smysl') or ('fixtures/corpus' not in cap and cwd is None):
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
        #
        # Inputs only. The value after `-o` is a file the command *writes*, and requiring it
        # to exist made replayability depend on leftovers: `merge … -o /tmp/incident.cbor`
        # ran only because an earlier replay had created that file, so a clean machine and a
        # dirty one disagreed about how many blocks were covered. Six commands were in that
        # state, and the count moved between runs without anything changing.
        toks = cmd.split()
        # `i` starts at 1: token 0 is the binary this script was told to run, not an input the
        # manual named. It matters because `SMYSL_BIN` may be absolute — `tests/doc_output.rs`
        # passes `CARGO_BIN_EXE_smysl`, which always is — and the absolute-path rule below
        # would then match the program itself and skip every command in the book. It did:
        # `ran 0, skipped 168`, reported as a pass, because the test asserted only that the
        # script had produced a summary line. Both halves are fixed; this is the half in here.
        paths = [t for i, t in enumerate(toks)
                 if i and '/' in t and not t.startswith('-') and not t.startswith('b3:')
                 and not toks[i - 1] in ('-o', '--output')
                 and not is_label(t)]
        # An absolute path as an *input* is narrative state, not something this script can
        # guarantee. It may exist because an earlier replayed command in this very run wrote
        # it — `merge … -o /tmp/incident.cbor` does — and then the manual's transcript
        # describes it in a state nobody reproduced: "one byte flipped in the sidecar". The
        # command then runs and the comparison is meaningless rather than absent.
        if any(t.startswith('/') for t in paths):
            skipped += 1
            continue
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
        # `run_in` is the fixture folder for a tutorial command and None otherwise. The
        # binary is resolved to an absolute path first, or changing directory would leave a
        # relative `./target/debug/smysl` pointing at nothing.
        run_in = os.path.abspath(cwd) if cwd else None
        merged = subprocess.run(cmd, shell=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, cwd=run_in)
        # The second run keeps them apart, because a block may quote one stream alone: a
        # command whose stdout is a store prints its report on stderr, and rule P makes that
        # stdout CBOR on a pipe, which is not comparable text at all.
        p = subprocess.run(cmd, shell=True, capture_output=True, cwd=run_in)
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
        # A trailing `exit N` line is the manual's notation for the exit status, not output
        # the program printed. Four blocks use it, all in chapter 22–24 — the chapter this
        # script could not see until the block regex was widened, so it had never met the
        # convention.
        #
        # Checked rather than dropped. Discarding the line would make the comparison pass on a
        # command that stopped failing, and the four places the book bothers to state an exit
        # code are exactly the places it matters: a missing render target, an unreadable
        # profile, a threshold breach.
        want = norm(claimed)
        want_lines = want.split('\n')
        if want_lines and re.fullmatch(r'exit \d+', want_lines[-1].strip()):
            expected_code = int(want_lines[-1].strip().split()[1])
            if merged.returncode != expected_code:
                mismatches.append((f, cmd, want,
                                   f'{norm(dec(merged.stdout))}\n'
                                   f'exit {merged.returncode}'))
                continue
            want = '\n'.join(want_lines[:-1]).rstrip()
        if any(want == norm(c) for c in candidates):
            continue
        # A lone `...` line is the manual explicitly declaring an elision. Honour it as
        # "any run of lines here", so a block that says it is showing part of the output is
        # checked on the part it shows.
        #
        # A caption annotated `(excerpt)` says the same thing about the *edges*: the block is a
        # window onto the output, so what precedes and follows it is elided too, and a trailing
        # `. ...` mid-line is a sentence cut short rather than a line of its own. Without this,
        # a block that declares itself an excerpt is compared as though it were complete and
        # reports drift for the four lines it never claimed to show — a false positive, which
        # this script's own history says is worse than no check at all.
        is_excerpt = '(excerpt)' in cap
        if is_excerpt or any(l.strip() in ('...', '…') for l in want.split('\n')):
            import fnmatch

            def esc(l):
                l = l.replace('*', '[*]').replace('?', '[?]').replace('[', '[[]')
                # An inline ellipsis ending a line: the rest of that line is elided.
                return re.sub(r'\s*(\.\.\.|…)$', '*', l)

            pat = '\n'.join('*' if l.strip() in ('...', '…') else esc(l)
                            for l in want.split('\n'))
            pat = re.sub(r'\n\*\n', '\n*', pat)
            if is_excerpt:
                pat = '*' + pat + '*'
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

# Every committed tutorial fixture must still be the bytes its chapter prints.
#
# This is the property gate 7 asks for and the only thing that keeps the arrangement honest:
# the reader copies the fenced block, the script replays the fixture, and nothing forces them
# to stay the same file. Edit the chapter and the fixture goes stale silently — the script
# would keep passing against bytes no reader ever sees, which is a check measuring its own
# fixture rather than the book.
stale = []
for fixture in sorted(glob.glob('fixtures/tutorial/*/*')):
    chapter = os.path.join('Documentation', 'manual',
                           os.path.basename(os.path.dirname(fixture)) + '.typ')
    body = open(fixture).read()
    if body not in open(chapter).read():
        stale.append((fixture, chapter))
for fixture, chapter in stale:
    print(f'=== {fixture}')
    print(f'    is not printed anywhere in {os.path.basename(chapter)} — the reader and this')
    print(f'    script no longer see the same file. Update whichever is wrong.')
    print()

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
sys.exit(1 if (mismatches or stale) else 0)

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
import re, subprocess, glob, os, sys, json, shutil, tempfile, atexit

# Reader-edits that could not be applied. Collected rather than raised: one broken
# anchor should say so and let the rest of the book still be checked, but it must
# reach the exit status — a declared edit that silently did not apply leaves the
# workspace a state behind and reports the mismatch against the command instead.
edit_errors = []

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

def named_files(cap):
    """The bare filenames a caption's command names as inputs."""
    cmd = re.sub(r'\s{2,}\(.*\)$', '', cap[2:]).strip()
    toks = cmd.split()
    return [t for i, t in enumerate(toks[1:], 1)
            if not t.startswith('-') and not t.startswith('b3:')
            and '.' in t and '/' not in t and not toks[i - 1] in ('-o', '--output')]


# A chapter is a sequence, not a set: what a command sees depends on what ran before it
# ---------------------------------------------------------------------------------------
#
# Gate 7's larger half: 100 of the manual's commands name a file the *prose* asks the reader to
# create — `first.smy`, `cycle.smy` — so the script had nothing to run them against.
#
# Committing those files got 12 of them in 1.1. The other 22 were held back because **one
# filename has several states**: chapter 1 has the reader create `first.smy` broken, fix it,
# find it unformatted, then rewrite it in place with `fmt --write`. Four commands name one path
# and expect four different files, so committing any one makes the other three report drift
# that is not there. That was recorded as a decision about the book — split the files and the
# chapters would have to name a different one at each step, changing what they teach.
#
# It was not a decision about the book. It was the fixtures being keyed by filename when the
# chapter's state is keyed by *position*. Two of those four states cannot be committed at all,
# which is what makes the point: state 2 is printed as a **fragment** the reader pastes in, and
# state 3 as a **diff**. Neither is ever printed as a file, so neither can satisfy the rule that
# a fixture is the bytes the chapter prints.
#
# So they are not committed. The chapter is replayed as a chapter: a scratch copy of the
# committed files, walked in document order, with `fmt --write` allowed to actually write. A
# later state is *derived* rather than recorded, which is a stronger claim than a fixture makes
# — the file a command runs against is one the book's own instructions produced.
#
# The reader's hand-edits are the one thing a command cannot produce, and they are declared in
# `edits.json` by **prose anchor**, never by content: the edit body is read out of the chapter
# at run time. Nothing is duplicated, so nothing can go stale — change the page and the anchor
# stops resolving, which is an error rather than a silent pass against bytes no reader sees.

def next_fence(src, at):
    """The next fenced block after `at`, as (body, end_offset)."""
    open_at = src.find('```', at)
    if open_at < 0:
        return None, at
    body_at = src.index('\n', open_at) + 1
    close_at = src.find('```', body_at)
    if close_at < 0:
        return None, at
    return src[body_at:close_at], close_at


def stanza_bounds(body, match):
    """The surface stanza beginning with `match`: from its `@` line to the next one."""
    start = body.find(match)
    if start < 0:
        return None
    nxt = re.search(r'^@', body[start + 1:], re.M)
    return (start, start + 1 + nxt.start() if nxt else len(body))


def load_edits(stem, src):
    """This chapter's declared reader-edits, each resolved to a document position and a body.

    An edit whose anchor is missing, ambiguous, or matches nothing in the file it names is an
    error rather than a skip. A declared edit that quietly does not apply would leave the
    workspace in state N while the transcript expects N+1, and the mismatch would be reported
    against the *command* — sending the reader looking for a bug in the binary.
    """
    path = os.path.join('fixtures', 'tutorial', stem, 'edits.json')
    if not os.path.exists(path):
        return []
    out = []
    for fname, steps in json.load(open(path)).items():
        # `_`-prefixed keys are prose. JSON has no comments and this file has to explain
        # itself, so the convention is the alternative to a second document nobody updates.
        if fname.startswith('_'):
            continue
        for step in steps:
            anchor = step['anchor']
            if src.count(anchor) != 1:
                edit_errors.append(
                    f'{stem}/edits.json: anchor {anchor!r} appears {src.count(anchor)} times '
                    f'in the chapter; it must appear exactly once')
                continue
            at = src.index(anchor)
            body, _ = next_fence(src, at)
            if body is None:
                edit_errors.append(
                    f'{stem}/edits.json: no fenced block follows anchor {anchor!r}')
                continue
            out.append({'file': fname, 'at': at, 'body': body, **step})
    return sorted(out, key=lambda e: e['at'])


def apply_edit(ws, edit):
    """Apply one reader-edit to the workspace, or record why it could not be."""
    target = os.path.join(ws, edit['file'])
    # `from` is the chapter that builds a file up under a series of names. Chapter 4 has the
    # reader write `step1.smy`, add a claim and call it `step2.smy`, then add a definition and
    # call it `step3.smy` — three filenames, one document, each state printed only as the
    # stanza that was added. None of the three later states can be a committed fixture, for
    # the same reason chapter 1's cannot: the page prints the increment, not the file.
    src_name = edit.get('from')
    if src_name and not os.path.exists(target):
        origin = os.path.join(ws, src_name)
        if not os.path.exists(origin):
            edit_errors.append(
                f"{edit['file']}: derives from {src_name}, which the chapter has not created yet")
            return
        shutil.copy(origin, target)
    if not os.path.exists(target):
        edit_errors.append(f"{edit['file']}: the edit names a file the chapter never created")
        return
    body = open(target).read()
    if edit['op'] == 'replace-stanza':
        span = stanza_bounds(body, edit['match'])
        if span is None:
            edit_errors.append(
                f"{edit['file']}: no stanza matching {edit['match']!r} to replace")
            return
        body = body[:span[0]] + edit['body'].strip('\n') + '\n' + body[span[1]:]
    elif edit['op'] == 'append':
        body = body.rstrip('\n') + '\n\n' + edit['body'].strip('\n') + '\n'
    else:
        edit_errors.append(f"{edit['file']}: unknown edit op {edit['op']!r}")
        return
    open(target, 'w').write(body)


def open_workspace(chapter_path):
    """A scratch copy of this chapter's committed tutorial files, or None if it has none.

    A copy rather than the fixture folder itself, because commands now write. `fmt --write` is
    the whole point of the arrangement, and running it where the fixtures live would rewrite a
    tracked file every time the gate ran.
    """
    stem = os.path.basename(chapter_path).replace('.typ', '')
    d = os.path.join('fixtures', 'tutorial', stem)
    if not os.path.isdir(d):
        return None, []
    ws = tempfile.mkdtemp(prefix=f'smysl-doc-{stem}-')
    # A tree, not a flat list. Not every file the reader creates has a bare name: chapter 29
    # hand-simulates an `ingest` by placing a reviewed batch at `.smysl/staged.smy`, which is
    # the file `merge --staged` reads, and the chapter prints it in full like any other.
    shutil.copytree(d, ws, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns('README.md', 'edits.json'))
    return ws, stem


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


mismatches, ran, skipped, excerpts, effects = [], 0, 0, 0, 0
for f in sorted(glob.glob('Documentation/manual/*.typ')):
    src = open(f).read()
    ws, stem = open_workspace(f)
    if ws:
        atexit.register(shutil.rmtree, ws, True)
    pending = load_edits(stem, src) if ws else []
    for m in BLOCK.finditer(src):
        # The reader's edits land where the prose puts them, before the next command runs.
        while pending and pending[0]['at'] < m.start():
            apply_edit(ws, pending.pop(0))
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
        cwd = None
        if ws and 'fixtures/corpus' not in cap:
            named = named_files(cap)
            # Every named file must be present *now*. A command whose input the chapter has
            # not yet created stays skipped rather than being run against an absent file —
            # and, since the workspace evolves, "not yet" is a real distinction from "never".
            if named and all(os.path.exists(os.path.join(ws, t)) for t in named):
                cwd = ws
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
        # A caption annotated `(then …)` says the block shows what comes *after* the command,
        # not what the command printed: `$ smysl fmt --write first.smy  (then diff against the
        # original)` is followed by a diff. The command still has to run — it is what produces
        # the state the rest of the chapter is written against — but comparing its output to a
        # diff would report drift on a block that never claimed to be its output.
        #
        # Counted separately from `skipped`, because it did run and it did have an effect. A
        # command lumped in with the skips would look like coverage that was declined.
        if re.search(r'\(\s*then\b', cap):
            effects += 1
            continue
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
for fixture in sorted(glob.glob('fixtures/tutorial/*/**', recursive=True)):
    if os.path.isdir(fixture) or os.path.basename(fixture) in ('README.md', 'edits.json'):
        continue
    # `fixtures/tutorial/<chapter>/…` — the chapter is the first path element, however deep
    # the fixture sits, so `.smysl/staged.smy` is checked against its chapter like any other.
    chapter = os.path.join('Documentation', 'manual',
                           fixture.split(os.sep)[2] + '.typ')
    body = open(fixture).read()
    if body not in open(chapter).read():
        stale.append((fixture, chapter))
for fixture, chapter in stale:
    print(f'=== {fixture}')
    print(f'    is not printed anywhere in {os.path.basename(chapter)} — the reader and this')
    print(f'    script no longer see the same file. Update whichever is wrong.')
    print()

for e in edit_errors:
    print(f'=== declared reader-edit did not apply')
    print(f'    {e}')
    print()

print(f'ran {ran}, skipped {skipped}, excerpt-matched {excerpts}, '
      f'ran-for-effect {effects}, MISMATCHED {len(mismatches)}\n')
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
sys.exit(1 if (mismatches or stale or edit_errors) else 0)

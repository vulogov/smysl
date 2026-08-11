# smysl - developer entry points.
#
# The check targets mirror .github/workflows/ci.yml exactly. That is the point: a local
# check that differs from CI is a check that lets you push red.
#
#   make            # help
#   make ci         # everything CI runs, except the jobs needing a server
#
# ---------------------------------------------------------------------------
# Toolchain
# ---------------------------------------------------------------------------
#
# CI resolves `dtolnay/rust-toolchain@stable` to the newest stable release, so the compiler
# that lints your code is whatever Rust shipped most recently - not whatever is active
# here. This repository has already been broken by exactly that: two clippy lints exist in
# 1.97 and not in 1.94, the local checks were clean, and CI went red on a push that had
# been verified twice. Pinning the checks to the same channel removes the surprise.
#
#   make lint                     # your installed stable
#   make lint TOOLCHAIN=+1.97.1   # one specific version
#   make lint TOOLCHAIN=          # whatever is active (may differ from CI)
#
# **`+stable` is only CI's stable if you have updated it.** rustup pins the `stable`
# toolchain at whatever it downloaded last; CI fetches the newest on every run. Run
# `make update` to pull them level, and `make toolchain` to see whether they are.
TOOLCHAIN ?= +stable
CARGO     := cargo $(TOOLCHAIN)

# The feature sets CI's test-matrix covers. A combination that builds only in one of them
# is a combination nobody builds.
#
# `--features cli` is here even though CI's test-matrix does not name it, because the
# determinism job *builds* it — its permutations run
# `cargo run --no-default-features --features cli`. Nothing else compiled that combination,
# so a function that went dead in it went unnoticed, and with `-D warnings` the build
# failed rather than warned. The determinism job then reported "rule D failed" for three
# releases running, when nothing about determinism was wrong: the binary would not compile.
MATRIX := \
	--no-default-features \
	@ \
	--all-features \
	--no-default-features@--features@cli \
	--no-default-features@--features@tui \
	--no-default-features@--features@semantic \
	--no-default-features@--features@local \
	--no-default-features@--features@remote \
	--no-default-features@--features@exact-pack \
	--no-default-features@--features@render-typst,render-html

.DEFAULT_GOAL := help
.PHONY: help all rebuild release test lint clippy fmt fix test-matrix gates purity update seed-fuzz fuzz-build \
        determinism conformance eval live-ollama live-hosted doc fuzz clean sweep \
        commit ci toolchain eval-live eval-semantic docs doc-output doc-cargo spec-tables seed-fuzz fuzz-long

help: ## Show this help
	@echo "smysl - make targets"
	@echo
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'
	@echo
	@echo "toolchain: cargo $(TOOLCHAIN)   (override with TOOLCHAIN=)"

# ---------------------------------------------------------------------------
# Everyday
# ---------------------------------------------------------------------------

all: ## Build the workspace with default features
	$(CARGO) build --workspace

rebuild: clean all ## Clean, then build

release: ## Optimised build of the CLI
	$(CARGO) build --release

test: ## Full test suite, every feature on
	$(CARGO) test --workspace --all-features

fmt: ## Format the workspace in place
	$(CARGO) fmt --all

fix: fmt ## Format, then apply the clippy fixes that can be applied mechanically
	$(CARGO) clippy --workspace --all-features --all-targets --fix --allow-dirty

doc: ## Build API documentation, no dependencies
	$(CARGO) doc --workspace --all-features --no-deps

# What docs.rs renders, gated the way clippy is.
#
# Publishing made this consequential: rustdoc warnings are broken links on a page strangers
# read first, and six had accumulated unnoticed. One was not cosmetic - `Usage` was
# documented as being built through `Usage::new`, a constructor that does not exist, which
# is precisely the "documentation that matches the binary" defect READINESS gate 7 tracks.
# It survived because nothing ever failed on it.
doc-gate: ## Rustdoc with warnings denied, as docs.rs would show it
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --all-features --no-deps
	@echo "doc-gate: no rustdoc warnings"

# The three books in Documentation/ are tracked as PDFs as well as sources, because they
# are deliverables people are handed rather than artefacts people build. Tracking the
# output of a build makes it drift, so this target is what keeps the two in step.
# The last **published** version, and what is published.
#
# Published, not tagged. `cargo-semver-checks` fetches the baseline from crates.io, so this
# moves when a release reaches the registry and not when it is cut.
#
# That distinction cost two cycles. It sat at 0.9.0 through 0.10 and 0.11, both cut without
# being published, so every breaking change was measured against a version two releases old and
# the `ContextExceeded` repair sat parked behind it. Pointing it at an unpublished version does
# not fail loudly either — it turns all twelve crates into "version not found in registry", a
# red job saying nothing about the API. 0.10.0 and 0.12.0 were never published and never will
# be; everything in them shipped in the release after.
BASELINE  := 1.0.0
PUBLISHED := smysl-core smysl-graph smysl-check smysl-pack smysl-thread smysl-render \
             smysl-retrieve smysl-embed smysl-provider smysl-ingest smysl-tui smysl

# The library crates behind the facade: everything published except the facade itself.
LIBRARIES := $(filter-out smysl,$(PUBLISHED))

# The twenty-two subcommands, for `cli-surface`. Written out rather than read from `--help`,
# for the same reason `tests/dispatch.rs` writes them out: a list derived from the binary
# shrinks silently when the binary does, and the regenerated golden would then record the
# absence as if it were the intent.
COMMAND_NAMES := fmt check pack merge diff trace view bundle thread salience find retract \
                 render import relink compact ingest attest providers usage reindex ui

DOCS := SMYSL_MANUAL SMYSL_FORMAT_GUIDE SMYSL_RATIONALE SMYSL_RATIONALE_PRESENTATION

#
# `SOURCE_DATE_EPOCH` is what makes that work. Typst stamps a creation time into every PDF,
# so an unpinned rebuild changes all three files whether or not a word changed, and a
# tracked PDF would show up dirty after any `make docs`. Pinned, the output is byte-identical
# for identical sources - so a diff in a PDF means the document actually changed.
docs: ## Rebuild the PDFs in Documentation/ from their typst sources
	@command -v typst >/dev/null || { echo "typst is not installed: https://typst.app"; exit 1; }
	@set -e; for d in $(DOCS); do \
		echo "==> $$d"; \
		SOURCE_DATE_EPOCH=0 typst compile Documentation/$$d.typ Documentation/$$d.pdf; \
	done
	@echo "docs: rebuilt $(words $(DOCS)) document(s), reproducibly"

# ---------------------------------------------------------------------------
# The CI jobs, individually
# ---------------------------------------------------------------------------

# The public contract, and whether it moved.
#
# Publishing turned every re-exported name into something people build against. These two
# targets answer different questions and both are needed: `api-check` says the *list* changed,
# `semver` says the change was breaking. A rename shows up in the first; adding
# `#[non_exhaustive]` to a struct shows up only in the second.
cli-surface: ## Regenerate tests/cli-surface.txt, the recorded CLI argument surface
	@$(CARGO) build --quiet
	@{ sed -n '1,/^# Regenerate with/p' tests/cli-surface.txt; \
	   for c in $(COMMAND_NAMES); do \
	     ./target/debug/smysl $$c --help 2>&1 \
	       | grep -oE "^ +(-[a-zA-Z], )?--[a-z0-9-]+|^ +[<[][A-Z.]+[]>]" \
	       | sed -E 's/^ +//; s/^-[a-zA-Z], //' \
	       | while read -r a; do printf '%-10s %s\n' "$$c" "$$a"; done; \
	   done; } > tests/cli-surface.txt.new
	@mv tests/cli-surface.txt.new tests/cli-surface.txt
	@echo "cli-surface: recorded $$(grep -vc '^#' tests/cli-surface.txt) argument(s) across $(words $(COMMAND_NAMES)) commands"

api: ## Regenerate the recorded public surfaces, both ends
	@command -v cargo-public-api >/dev/null || { echo "cargo install cargo-public-api"; exit 1; }
	@{ sed -n '1,/^# Regenerate with/p' tests/public-api.txt; \
	   $(CARGO) public-api --all-features --simplified 2>/dev/null; } > tests/public-api.txt.new
	@mv tests/public-api.txt.new tests/public-api.txt
	@{ sed -n '1,/^# Regenerate with/p' tests/public-api-pure.txt; \
	   $(CARGO) public-api --no-default-features --simplified 2>/dev/null; } > tests/public-api-pure.txt.new
	@mv tests/public-api-pure.txt.new tests/public-api-pure.txt
	@{ sed -n '1,/^# Regenerate with/p' tests/public-api-counts.txt; \
	   for c in $(LIBRARIES); do \
	     printf '%-16s %s\n' "$$c" \
	       "$$($(CARGO) public-api -p $$c --all-features --simplified 2>/dev/null | wc -l | tr -d ' ')"; \
	   done; } > tests/public-api-counts.txt.new
	@mv tests/public-api-counts.txt.new tests/public-api-counts.txt
	@echo "api: recorded $$($(CARGO) public-api --all-features --simplified 2>/dev/null | wc -l | tr -d ' ') names at --all-features, $$($(CARGO) public-api --no-default-features --simplified 2>/dev/null | wc -l | tr -d ' ') pure"
	@echo "api: recorded $(words $(LIBRARIES)) per-crate surface counts"

# Both ends, and the nesting between them.
#
# One file was not enough. Recording only `--all-features` freezes the maximum, so a name could
# stop being reachable on default features while the recorded surface never moved — and the
# README recommends `default-features = false` for the pure library, which had nothing checking
# it at all.
api-check: ## Fail if either recorded surface moved, or if they stop nesting
	@command -v cargo-public-api >/dev/null || { echo "cargo install cargo-public-api"; exit 1; }
	@$(CARGO) public-api --all-features --simplified 2>/dev/null > /tmp/smysl-api-all.txt
	@$(CARGO) public-api --no-default-features --simplified 2>/dev/null > /tmp/smysl-api-pure.txt
	@$(CARGO) public-api --simplified 2>/dev/null > /tmp/smysl-api-def.txt
	@grep -v '^#' tests/public-api.txt | grep -v '^$$' > /tmp/smysl-api-all-was.txt
	@grep -v '^#' tests/public-api-pure.txt | grep -v '^$$' > /tmp/smysl-api-pure-was.txt
	@diff -u /tmp/smysl-api-all-was.txt /tmp/smysl-api-all.txt \
	  || { echo "api-check: the --all-features surface moved. If deliberate, run 'make api'."; exit 1; }
	@diff -u /tmp/smysl-api-pure-was.txt /tmp/smysl-api-pure.txt \
	  || { echo "api-check: the pure surface moved. If deliberate, run 'make api'."; exit 1; }
	@sort /tmp/smysl-api-all.txt > /tmp/a.s; sort /tmp/smysl-api-def.txt > /tmp/d.s; sort /tmp/smysl-api-pure.txt > /tmp/p.s
	@test -z "$$(comm -13 /tmp/a.s /tmp/d.s)" \
	  || { echo "api-check: default features expose a name --all-features does not"; exit 1; }
	@test -z "$$(comm -13 /tmp/d.s /tmp/p.s)" \
	  || { echo "api-check: --no-default-features exposes a name default does not"; exit 1; }
	@{ for c in $(LIBRARIES); do \
	     printf '%-16s %s\n' "$$c" \
	       "$$($(CARGO) public-api -p $$c --all-features --simplified 2>/dev/null | wc -l | tr -d ' ')"; \
	   done; } > /tmp/smysl-api-counts.txt
	@grep -v '^#' tests/public-api-counts.txt | grep -v '^$$' > /tmp/smysl-api-counts-was.txt
	@diff -u /tmp/smysl-api-counts-was.txt /tmp/smysl-api-counts.txt \
	  || { echo "api-check: a crate's public surface changed size. If deliberate, run 'make api'."; exit 1; }
	@echo "api-check: both surfaces match, pure <= default <= all-features, and $(words $(LIBRARIES)) crate sizes unmoved"

# `--release-type patch` is load bearing. Without it, 0.9 -> 0.10 on a 0.x crate is a
# breaking-allowed bump and cargo-semver-checks skips every check: "0 checks: 0 pass, 254
# skip", reported as a pass. Forcing patch makes the 223 checks actually run. A gate that
# green-lights by skipping is the failure this project keeps finding.
# Crates with a **deliberate** break this cycle, and why.
#
# **The meaning of this list changed at 1.0.0. An entry in it now means a 2.0.**
#
# Before 1.0 it was a cycle-scoped ledger: a break was allowed, recorded here, and cleared when
# publication made it the baseline. Nine of twelve crates were in it for 0.13, deliberately,
# because that was the last cycle in which narrowing was free. Phase 3 of ROAD_TO_1.0.md then
# asked for two consecutive published cycles with it empty, and 0.14 and 0.15 delivered them.
#
# After 1.0 there is no such thing as a break that costs only a cycle. `API_CONTRACT.md` is the
# promise the version number makes: the facade's names and every public item behind them move
# only with a major version. So adding a crate below is not paperwork — it is a decision to
# ship smysl 2.0, and it should be taken that way or not at all.
#
# What to do instead, in rough order of preference:
#
#   * Add rather than change. Most public types are `#[non_exhaustive]` as of §1.1 precisely so
#     that a new field is not a break; that audit is what let the `smysl/1.0` format migration
#     land inside a quiet cycle instead of costing one.
#   * Deprecate rather than remove. `#[deprecated]` is not a break; deletion is.
#   * If a thing genuinely should not have been public, note it here and leave it. Hiding an
#     item is removal as far as cargo-semver-checks is concerned — there are lints named
#     `struct_now_doc_hidden` and `pub_module_level_const_now_doc_hidden` — so the narrowing
#     done in §1.2 S2 and S4 is not available any more. It was done before 1.0 for that reason.
#
# `--release-type patch` below forbids any break, which is the sensitive setting and the only
# one that ran the checks at all while the crate was 0.x — on a 0.x crate the real release type
# permits breaking, so cargo-semver-checks skipped all 223 and reported a pass. At 1.x it is
# still the right setting for a different reason: it makes an accidental break fail the job
# rather than quietly imply a version bump nobody chose.
#
# Empty is the normal state and now the only comfortable one.
SEMVER_BREAKING :=

semver: ## Report API breakage against the last published version
	@command -v cargo-semver-checks >/dev/null || { echo "cargo install cargo-semver-checks"; exit 1; }
	@set -e; for c in $(PUBLISHED); do \
		case " $(SEMVER_BREAKING) " in \
			*" $$c "*) continue;; \
		esac; \
		$(CARGO) semver-checks check-release --baseline-version $(BASELINE) \
			--release-type patch -p $$c; \
	done
	@# Reported, not gated. Until 0.13 these were `continue`d with a one-line SKIP, which
	@# meant a crate with one deliberate break had *nothing* watching it — a second,
	@# unintended break in the same crate rode along invisibly for the rest of the cycle.
	@# Running them and printing what comes back costs one command and makes the question
	@# answerable: are these the breaks that were meant?
	@if [ -n "$(SEMVER_BREAKING)" ]; then \
		echo; \
		echo "=== deliberate breaks this cycle: reported, not gated on ==="; \
		for c in $(SEMVER_BREAKING); do \
			echo; echo "--- $$c"; \
			$(CARGO) semver-checks check-release --baseline-version $(BASELINE) \
				--release-type patch -p $$c 2>&1 \
				| grep -E '^--- failure|Summary semver' || true; \
		done; \
		echo; \
		echo "Each must match a reason recorded above SEMVER_BREAKING in this file."; \
		echo "Empty SEMVER_BREAKING at the release cut."; \
	fi

lint: ## fmt --check and clippy -D warnings, as CI runs them
	$(CARGO) fmt --all -- --check
	RUSTFLAGS="-D warnings" $(CARGO) clippy --workspace --all-features --all-targets -- -D warnings

clippy: ## Clippy alone, without the formatting check
	RUSTFLAGS="-D warnings" $(CARGO) clippy --workspace --all-features --all-targets -- -D warnings

test-matrix: ## Every feature combination CI builds
	@set -e; for f in $(MATRIX); do \
		args=$$(echo "$$f" | tr '@' ' ' | sed 's/^ *//'); \
		echo "==> cargo test --workspace $$args"; \
		RUSTFLAGS="-D warnings" $(CARGO) test --workspace $$args; \
	done

purity: ## Rules A and B: the library stays synchronous and offline
	$(CARGO) xtask check-purity

determinism: ## Rule D: pure operations are bit-reproducible
	$(CARGO) xtask determinism

gates: purity determinism ## Both xtask gates
	@echo "gates: purity and determinism clean"

doc-cargo: ## Replay the manual's `cargo` transcripts and check its feature table
	@# The blocks `doc-output` cannot reach. It replays `smysl` commands; these are a
	@# different program, so its skip rules pass over them and nothing checked them at all.
	@#
	@# 0.14 found three stale claims in the manual and every one was in a block this covers —
	@# a `cargo build` transcript from 0.1.0, a dependency tree that predated smysl-retrieve
	@# becoming a plain dependency, and a feature table claiming `default` turns on `tui`.
	@# The first had gone stale *again* by 1.1, one release after being fixed by hand, which
	@# is the argument: a version number in prose drifts every release.
	python3 scripts/verify-doc-cargo.py

spec-tables: ## Fail if the format's constants and the document that defines them disagree
	@# The gate 1.2.0 needed and did not have. Four facts a C-Produce implementer cannot
	@# proceed without — the status integers, the source sub-map's layout, the kind enum and
	@# the base32 alphabet — were in no section of the spec, and three implementations
	@# "agreed" on all four because all three had decoded the same fixture to learn them.
	@#
	@# It parses the spec's tables rather than quoting them, which is the distinction that
	@# matters: until this existed, every implementation read SMYSL_FORMAT_SPEC.md only to
	@# assert it contained the string "Deterministic CBOR", while READINESS reported the
	@# §2.2 and §3.1 tables as checked against the document.
	python3 scripts/verify-spec-tables.py

doc-output: ## Replay the manual's documented commands against the real binary
	@echo "Needs a default-features build: a stale --all-features binary reports false drift."
	$(CARGO) build
	python3 scripts/verify-doc-output.py

conformance: ## The conformance suite, as CI runs it
	$(CARGO) test --test conformance_fixtures --no-default-features

eval: ## The SM-P15 evaluation harness over the corpus (smysl arm, no model)
	$(CARGO) test -p smysl-eval

# ---------------------------------------------------------------------------
# Live provider tests
# ---------------------------------------------------------------------------
#
# `SMYSL_OLLAMA=required` turns a skip into a failure. A live test that quietly skips is a
# live test nobody runs, which is how a mapper stays broken without anyone noticing.

live-ollama: ## Live tests against a local Ollama (needs `ollama serve`)
	SMYSL_OLLAMA=required $(CARGO) test -p smysl-provider --features ollama
	SMYSL_OLLAMA=required $(CARGO) test -p smysl-ingest --features ollama

# The retrieval comparison. Three engines over one query set, so the tables can be read side
# by side — which is the only reason to run it. A number from a different query set says
# nothing about the number it is being compared with.
#
# The model is not in this repository and this build cannot fetch one: `hf-hub` is compiled
# out of `model2vec-rs`, deliberately, so nothing here reaches the network. Get one however
# you normally would, for example:
#
#     pip install huggingface_hub
#     hf download minishlab/potion-base-8M --local-dir ~/models/potion-base-8M
#
# Any Model2Vec model works; `potion-base-8M` is small and the one the numbers below were
# taken with. The directory needs `tokenizer.json`, `model.safetensors` and `config.json`.
eval-semantic: ## Semantic vs lexical vs hybrid over the shared query set
	@test -n "$(SMYSL_EMBED_MODEL)" || { \
		echo "set SMYSL_EMBED_MODEL to a Model2Vec directory — see the comment above this"; \
		echo "target in the Makefile for one way to obtain one."; exit 2; }
	@echo "Queries: fixtures/retrieval/queries.tsv — the same set the lexical evaluation uses."
	SMYSL_EMBED_MODEL=$(SMYSL_EMBED_MODEL) $(CARGO) test -p smysl-embed --release \
		--test evaluation -- --nocapture

eval-live: ## Both eval arms, prose baseline included (needs GEMINI_API_KEY)
	@echo "Runs a model at every hop of the prose arm, and again to judge the result."
	@echo "Skips without a key rather than inventing a baseline."
	SMYSL_EVAL_LIVE=required $(CARGO) test -p smysl-eval --test prose_live -- --nocapture

live-hosted: ## Live ingest gate against whichever hosted providers have keys set
	@echo "Runs whichever of GEMINI_API_KEY / DEEPSEEK_API_KEY / OPENAI_API_KEY /"
	@echo "ANTHROPIC_API_KEY are set. The rest are skipped and named in the output."
	@echo "Spends tokens: SMYSL_INGEST_LIVE opts in, since 0.2 a key alone does not."
	SMYSL_INGEST_LIVE=1 $(CARGO) test -p smysl-ingest --features gemini,deepseek,openai,anthropic \
		--test providers_live -- --nocapture --test-threads=1

# The two parser targets, plus three that fuzz the *algebra*: rule U's join-semilattice
# laws, pack's constraints C1-C7, and rule L with guarantee A1 across the pipeline. The
# properties are the ones the seeded tests already assert; what changes is that coverage
# feedback drives the search instead of a fixed seed and 200 blind rounds.
FUZZ_TARGETS := surface cbor merge_algebra pack_constraints pipeline pack_exact

# Seeds, not a corpus. The repo already holds inputs worth starting from — the corpus
# fixtures are real `.smy` documents, and `fuzz/artifacts/` holds every input that has ever
# broken something — so seeding costs no repo weight and skips the minutes a cold run spends
# rediscovering that `@claim` exists. Measured: a cold 60s run of `surface` reaches 2093
# coverage points; seeded with these it found a *new defect* in under sixty seconds, in the
# `@thread` gist path, which cold runs had never reached.
#
# A minimised corpus was the obvious alternative and is not viable: `cargo fuzz cmin` takes
# `cbor` from 6780 inputs to 2093, and 2093 inputs is still 8.2 MB. Seeds stay small on
# purpose.
fuzz-build: ## Compile the fuzz targets without running them
	@# `fuzz/` is its own workspace, so `cargo check --workspace` from the root never
	@# compiles it and `make ci` never saw it. That is not theoretical: 0.13's
	@# `#[non_exhaustive]` audit broke `pack_exact.rs`, every local check passed, and CI's
	@# fuzz job went red on a push that had been verified twice.
	@#
	@# Compiling is enough to catch it and costs seconds. *Running* the fuzzers needs
	@# nightly and a minute per target, which is why that stays its own job.
	@#
	@# Not piped through `tail`. A pipeline's exit status is the last command's, so
	@# `cargo check ... | tail -5` prints the error and returns 0 — a gate that reports a
	@# failure and passes anyway, which is the exact shape of defect this file keeps finding.
	cd fuzz && $(CARGO) check --all-targets
	@echo "fuzz-build: the fuzz targets still compile against the library"

seed-fuzz: ## Copy the repo's own inputs into each fuzz corpus
	@set -e; for t in $(FUZZ_TARGETS); do \
		mkdir -p fuzz/corpus/$$t; \
		cp -f fuzz/artifacts/$$t/* fuzz/corpus/$$t/ 2>/dev/null || true; \
	done; \
	mkdir -p fuzz/corpus/surface; cp -f fixtures/corpus/*.smy fuzz/corpus/surface/ 2>/dev/null || true
	@echo "seeded from fixtures/corpus and fuzz/artifacts"

fuzz: seed-fuzz ## Fuzz every target for 60s each, as CI does (nightly)
	@echo "The parser targets existed from the start and nothing ran them, which is how two"
	@echo "stack overflows survived to 0.3 and eight more defects to 0.4. Sixty seconds each"
	@echo "catches a regression; finding something new takes the long run below."
	@set -e; for t in $(FUZZ_TARGETS); do \
		echo "==> $$t"; \
		cargo +nightly fuzz run $$t -- -max_total_time=60; \
	done

# Deliberately *not* seeded, and deliberately not depending on `seed-fuzz`. A warm corpus
# explores deeply around what it already knows; a cold one lands somewhere arbitrary. Both
# 0.4.0's duplicate-key defect and 0.5.0's schema-declaration defect came from a cold run,
# so the two searches are kept as two searches.
fuzz-long: ## Fuzz one target from cold until interrupted: make fuzz-long T=merge_algebra
	cargo +nightly fuzz run $(or $(T),surface) $(shell mktemp -d)

# ---------------------------------------------------------------------------
# Housekeeping
# ---------------------------------------------------------------------------

clean: ## Remove build artefacts
	$(CARGO) clean

sweep: ## Remove build artefacts older than three days across all projects
	cargo install cargo-sweep
	cargo sweep --time 3 --recursive

commit: ## Commit with aic and push
	aic -ac
	git push

# ---------------------------------------------------------------------------
# Everything
# ---------------------------------------------------------------------------

ci: lint doc-gate api-check test-matrix gates conformance fuzz-build doc-cargo spec-tables ## Everything CI runs, bar the jobs needing a server
	@echo
	@echo "ci: green."
	@echo "Not covered here: the ollama job (needs a running server - see make live-ollama)"
	@echo "and no-network (needs Linux user namespaces, so it cannot run on macOS)."

update: ## Pull the local stable toolchain level with the one CI fetches
	rustup update stable
	rustup component add --toolchain stable clippy rustfmt

toolchain: ## Show the toolchain these targets use against the newest stable
	@echo "make targets use: cargo $(TOOLCHAIN)"
	@printf '  '; $(CARGO) --version
	@printf '  '; $(CARGO) clippy --version 2>/dev/null || echo "clippy: not installed"
	@echo
	@echo 'CI uses dtolnay/rust-toolchain@stable, resolved fresh on every run.'
	@echo 'Your local stable is whatever rustup downloaded last; it does not update itself.'
	@echo 'If the version above is behind, CI lints your code with a compiler you never'
	@echo 'ran, and can fail on a push you verified. "make update" pulls them level.'

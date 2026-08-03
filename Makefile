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
.PHONY: help all rebuild release test lint clippy fmt fix test-matrix gates purity update seed-fuzz \
        determinism conformance eval live-ollama live-hosted doc fuzz clean sweep \
        commit ci toolchain eval-live eval-semantic docs doc-output seed-fuzz fuzz-long

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
# The last published version, and what is published. Both move at a release cut.
BASELINE  := 0.9.0
PUBLISHED := smysl-core smysl-graph smysl-check smysl-pack smysl-thread smysl-render \
             smysl-retrieve smysl-embed smysl-provider smysl-ingest smysl-tui smysl

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
api: ## Regenerate the recorded public surface
	@command -v cargo-public-api >/dev/null || { echo "cargo install cargo-public-api"; exit 1; }
	@{ sed -n '1,/^# Regenerate with/p' tests/public-api.txt; \
	   $(CARGO) public-api --all-features --simplified 2>/dev/null; } > tests/public-api.txt.new
	@mv tests/public-api.txt.new tests/public-api.txt
	@echo "api: recorded $$($(CARGO) public-api --all-features --simplified 2>/dev/null | wc -l | tr -d ' ') names"

api-check: ## Fail if the public surface moved without being recorded
	@command -v cargo-public-api >/dev/null || { echo "cargo install cargo-public-api"; exit 1; }
	@$(CARGO) public-api --all-features --simplified 2>/dev/null > /tmp/smysl-api-now.txt
	@grep -v '^#' tests/public-api.txt | grep -v '^$$' > /tmp/smysl-api-was.txt
	@diff -u /tmp/smysl-api-was.txt /tmp/smysl-api-now.txt \
	  || { echo "api-check: the public surface moved. If deliberate, run 'make api'."; exit 1; }
	@echo "api-check: the public surface matches what is recorded"

# `--release-type patch` is load bearing. Without it, 0.9 -> 0.10 on a 0.x crate is a
# breaking-allowed bump and cargo-semver-checks skips every check: "0 checks: 0 pass, 254
# skip", reported as a pass. Forcing patch makes the 223 checks actually run. A gate that
# green-lights by skipping is the failure this project keeps finding.
semver: ## Report API breakage against the last published version
	@command -v cargo-semver-checks >/dev/null || { echo "cargo install cargo-semver-checks"; exit 1; }
	@set -e; for c in $(PUBLISHED); do \
		$(CARGO) semver-checks check-release --baseline-version $(BASELINE) \
			--release-type patch -p $$c; \
	done

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

ci: lint doc-gate api-check test-matrix gates conformance ## Everything CI runs, bar the jobs needing a server
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

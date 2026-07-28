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
MATRIX := \
	--no-default-features \
	@ \
	--all-features \
	--no-default-features@--features@local \
	--no-default-features@--features@remote \
	--no-default-features@--features@exact-pack \
	--no-default-features@--features@render-typst,render-html

.DEFAULT_GOAL := help
.PHONY: help all rebuild release test lint clippy fmt fix test-matrix gates purity update \
        determinism conformance eval live-ollama live-hosted doc fuzz clean sweep \
        commit ci toolchain eval-live

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

# ---------------------------------------------------------------------------
# The CI jobs, individually
# ---------------------------------------------------------------------------

lint: ## fmt --check and clippy -D warnings, as CI runs them
	$(CARGO) fmt --all -- --check
	RUSTFLAGS="-D warnings" $(CARGO) clippy --workspace --all-features --all-targets -- -D warnings

clippy: ## Clippy alone, without the formatting check
	RUSTFLAGS="-D warnings" $(CARGO) clippy --workspace --all-features --all-targets -- -D warnings

test-matrix: ## Every feature combination CI builds
	@set -e; for f in $(MATRIX); do \
		args=$$(echo "$$f" | tr '@' ' ' | sed 's/^ *//'); \
		echo "==> cargo test --workspace $$args"; \
		$(CARGO) test --workspace $$args; \
	done

purity: ## Rules A and B: the library stays synchronous and offline
	$(CARGO) xtask check-purity

determinism: ## Rule D: pure operations are bit-reproducible
	$(CARGO) xtask determinism

gates: purity determinism ## Both xtask gates
	@echo "gates: purity and determinism clean"

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

eval-live: ## Both eval arms, prose baseline included (needs GEMINI_API_KEY)
	@echo "Runs a model at every hop of the prose arm, and again to judge the result."
	@echo "Skips without a key rather than inventing a baseline."
	SMYSL_EVAL_LIVE=required $(CARGO) test -p smysl-eval --test prose_live -- --nocapture

live-hosted: ## Live ingest gate against whichever hosted providers have keys set
	@echo "Runs whichever of GEMINI_API_KEY / DEEPSEEK_API_KEY / OPENAI_API_KEY /"
	@echo "ANTHROPIC_API_KEY are set. The rest are skipped and named in the output."
	$(CARGO) test -p smysl-ingest --features gemini,deepseek,openai,anthropic \
		--test providers_live -- --nocapture --test-threads=1

fuzz: ## Fuzz the surface parser (nightly; runs until interrupted)
	cargo +nightly fuzz run surface

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

ci: lint test-matrix gates conformance ## Everything CI runs, bar the jobs needing a server
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

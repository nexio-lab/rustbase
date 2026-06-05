# RustBaas — developer Makefile.
#
# `make help` lists every target. Most are thin wrappers around `cargo`
# and `bun` so contributors don't have to remember the exact invocation
# (and so CI / release behaviour matches one canonical source).

.DEFAULT_GOAL := help

.PHONY: help fmt clippy test check audit build docker docker-run \
        ui-dev docs-dev docs-build e2e e2e-install install-hooks \
        setup-dev changelog release release-push

help: ## Show this help
	@awk 'BEGIN {FS=":.*?## "} \
	     /^[a-zA-Z][a-zA-Z0-9_-]*:.*?## / \
	     {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}' \
	    $(MAKEFILE_LIST)

# ----- everyday loop --------------------------------------------------

fmt: ## cargo fmt --all
	cargo fmt --all

clippy: ## cargo clippy -D warnings
	cargo clippy --workspace --all-targets -- -D warnings

test: ## cargo test --workspace
	cargo test --workspace

check: ## fmt-check + clippy + tests (mirrors CI)
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

audit: ## cargo audit (transitive vulnerability scan)
	cargo audit

# ----- build artifacts ------------------------------------------------

build: ## Release binary at target/release/rustbase
	cargo build --release --bin rustbase

docker: ## Build the local docker image (linux/amd64 only)
	docker build -t rustbase:local .

docker-run: ## Run the local docker image, exposing :8080 and binding ./data
	docker run --rm -p 8080:8080 -v $(PWD)/data:/home/rustbase/data rustbase:local

# ----- dev servers ----------------------------------------------------

ui-dev: ## SvelteKit dashboard dev server on :5173 (proxies API to :8080)
	bun --cwd ui run dev

docs-dev: ## VitePress docs dev server
	bun --cwd docs run dev

docs-build: ## Build the docs site to docs/.vitepress/dist
	bun --cwd docs run build

# ----- end-to-end -----------------------------------------------------

e2e-install: ## One-time: install Playwright's Chromium browser
	bun --cwd ui run e2e:install

e2e: ## Playwright end-to-end suite against a throw-away rustbase boot
	bun --cwd ui run e2e

# ----- repo hygiene ---------------------------------------------------

install-hooks: ## Wire .githooks/ as core.hooksPath for this clone
	./scripts/install-hooks.sh

setup-dev: ## First-time bootstrap: toolchain check + hooks + warm caches
	./scripts/setup-dev.sh

# ----- release --------------------------------------------------------

changelog: ## (Re)draft [Unreleased] from commits since the last tag — useful as a starting point you then polish by hand
	@./scripts/changelog.sh

release: ## Cut a release. Usage: make release V=X.Y.Z [REGEN_CHANGELOG=1]
	@V=$(V) REGEN_CHANGELOG=$(REGEN_CHANGELOG) ./scripts/release.sh

release-push: ## Push the staged release. Usage: make release-push V=X.Y.Z
	@test -n "$(V)" || { echo "usage: make release-push V=X.Y.Z"; exit 1; }
	git push origin main
	git push origin v$(V)
	@echo
	@echo "✓ Pushed. Track the release pipeline at:"
	@echo "    https://github.com/pjonaszik/rustbase/actions/workflows/release.yml"

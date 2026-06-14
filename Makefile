CARGO ?= cargo
# Package name (for `cargo uninstall`); the installed command is `lgo`.
BIN   := local-git-ops

.DEFAULT_GOAL := help
.PHONY: help build release install uninstall run test lint fmt fmt-check check clean

help: ## Show available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

build: ## Debug build
	$(CARGO) build

release: ## Optimized release build
	$(CARGO) build --release

install: ## Install the `lgo` command into ~/.cargo/bin
	$(CARGO) install --path . --locked

uninstall: ## Remove the installed binary
	$(CARGO) uninstall $(BIN)

run: ## Run against the current directory (pass flags via ARGS="...")
	$(CARGO) run --quiet -- $(ARGS)

test: ## Run unit and integration tests
	$(CARGO) test

lint: ## Clippy with warnings denied
	$(CARGO) clippy --all-targets -- -D warnings

fmt: ## Format sources
	$(CARGO) fmt

fmt-check: ## Verify formatting without changing files
	$(CARGO) fmt --check

check: fmt-check lint test ## Full CI-equivalent gate

clean: ## Remove build artifacts
	$(CARGO) clean

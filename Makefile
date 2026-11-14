# Secure Dev Workspace — Rust-first build and validation
.PHONY: build image-export image-import test lint format check build-web web-tools env-example \
        secrets-init smoke smoke-build install-hooks update-tools check-tools purge-env-history \
        lint-sh audit clean host install-host test-host

RUST_MANIFEST := src/rust/Cargo.toml
WASM_TARGET := wasm32-unknown-unknown
WASM_BINDGEN_VERSION := $(shell awk '$$0 == "name = \"wasm-bindgen\"" { found=1; next } found && /^version = / { gsub(/[\" ]/, "", $$3); print $$3; exit }' src/rust/Cargo.lock)
WEB_DIST := src/rust/target/sdw-web

build:
	docker compose build

# The tag docker compose builds: dev-workspace:<WORKSPACE_VARIANT>, from the environment, else .env,
# else compose's own default (trixie).
WORKSPACE_VARIANT ?= $(or $(shell sed -n 's/^WORKSPACE_VARIANT=//p' .env 2>/dev/null | head -1),trixie)

image-export:
	docker save dev-workspace:$(WORKSPACE_VARIANT) -o workspace.tar

image-import:
	docker load -i workspace.tar

test:
	cargo test --manifest-path $(RUST_MANIFEST) --workspace --locked

lint:
	cargo fmt --manifest-path $(RUST_MANIFEST) --all --check
	cargo clippy --manifest-path $(RUST_MANIFEST) --all-targets -- -D warnings
	cargo clippy --manifest-path $(RUST_MANIFEST) -p sdw-web --target $(WASM_TARGET) -- -D warnings

format:
	cargo fmt --manifest-path $(RUST_MANIFEST) --all

check: lint test lint-sh test-host

# One-time local prerequisites for compiling the browser UI. Docker and CI install the same pinned
# tools themselves; this target is only for host-side development.
web-tools:
	rustup target add $(WASM_TARGET)
	cargo install wasm-bindgen-cli --locked --version $(WASM_BINDGEN_VERSION)

# Browser product code is Rust/WASM. wasm-bindgen emits a small standards-required JavaScript
# loader, with TypeScript declarations explicitly disabled. Output is under Cargo target/.
build-web:
	cargo build --manifest-path $(RUST_MANIFEST) --profile wasm --locked -p sdw-web --target $(WASM_TARGET)
	command -v wasm-bindgen >/dev/null 2>&1 || { echo "missing wasm-bindgen; run 'make web-tools'" >&2; exit 1; }
	[ ! -e $(WEB_DIST) ] || rm -r $(WEB_DIST)
	mkdir -p $(WEB_DIST)/pkg
	wasm-bindgen src/rust/target/$(WASM_TARGET)/wasm/sdw_web.wasm \
		--target web --no-typescript --out-dir $(WEB_DIST)/pkg
	cp -R src/rust/web/static/. $(WEB_DIST)/
	test ! -e $(WEB_DIST)/pkg/sdw_web.d.ts

env-example:
	cargo run -q --manifest-path $(RUST_MANIFEST) -p sdw-cli -- env-example

# Write PASSWORD and CLASH_SUBSCRIPTION_URL from .env into ./secrets/ (0600) for
# docker-compose.secrets.yml, so they can leave the container's environment. Prints no secret.
secrets-init:
	deploy/scripts/secrets-init.sh

# One-time, per clone: refuse a commit that adds a secrets file or contains a value from .env.
install-hooks:
	git config core.hooksPath deploy/scripts/git-hooks

# Third-party binaries baked into the image are pinned with checksums under src/image/tools/.
update-tools:
	deploy/scripts/update-tools.sh

check-tools:
	deploy/scripts/update-tools.sh --check

# Remove .env from the whole git history (rewrites history; dry run unless YES=1).
purge-env-history:
	deploy/scripts/purge-env-history.sh $(if $(YES),--yes)

lint-sh:
	@set -e; git ls-files -co --exclude-standard '*.sh' | while IFS= read -r file; do \
		[ ! -f "$$file" ] || shellcheck -x -S warning "$$file"; \
	done; shellcheck -x -S warning deploy/scripts/git-hooks/pre-commit

audit:
	cd src/rust && cargo audit

# Boot a disposable image next to local stand-ins and exercise the full stack.
IMAGE ?= dev-workspace:$(WORKSPACE_VARIANT)
smoke:
	src/tests/smoke/run.sh --image $(IMAGE) --services

smoke-build:
	src/tests/smoke/run.sh --build --services

host:
	cargo build --release --locked --manifest-path $(RUST_MANIFEST) -p sdw-host

install-host: host
	mkdir -p $(HOME)/.local/bin
	install -m 0755 src/rust/target/release/sdw-host $(HOME)/.local/bin/sdw-host

test-host:
	cargo build --locked --manifest-path $(RUST_MANIFEST) -p sdw-host
	bash src/tests/scripts/host-orchestration.test.sh
	bash src/tests/scripts/destroy.test.sh

clean:
	cargo clean --manifest-path $(RUST_MANIFEST)
	rm -f workspace.tar

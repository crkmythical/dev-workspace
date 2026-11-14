# Secure Dev Workspace — Build & Test
.PHONY: build image-export image-import dev test lint format env-example secrets-init smoke smoke-build \
        install-hooks update-tools check-tools purge-env-history lint-sh audit clean

build:
	docker compose build

# The tag docker compose builds: dev-workspace:<WORKSPACE_VARIANT>, from the environment, else .env,
# else compose's own default (trixie).
WORKSPACE_VARIANT ?= $(or $(shell sed -n 's/^WORKSPACE_VARIANT=//p' .env 2>/dev/null | head -1),trixie)

image-export:
	docker save dev-workspace:$(WORKSPACE_VARIANT) -o workspace.tar

image-import:
	docker load -i workspace.tar

dev:
	cd src/packages/spa && bun run dev

test:
	cd src && bun test

lint:
	cd src && bunx biome check .

format:
	cd src && bunx biome format . --write

build-spa:
	cd src/packages/spa && bun run build
env-example:
	cargo run -q --manifest-path src/rust/Cargo.toml -p sdw-cli -- env-example

# Write PASSWORD and CLASH_SUBSCRIPTION_URL from .env into ./secrets/ (0600) for
# docker-compose.secrets.yml, so they can leave the container's environment. Prints no secret.
secrets-init:
	deploy/scripts/secrets-init.sh

# One-time, per clone: refuse a commit that adds a secrets file or contains a value from .env.
install-hooks:
	git config core.hooksPath deploy/scripts/git-hooks

# The third-party tools baked into the image are pinned, with checksums, in src/image/tools/*.env.
#   make update-tools   move them to the latest releases (then review `git diff src/image/tools`)
#   make check-tools    change nothing; fail if any pin is behind
update-tools:
	deploy/scripts/update-tools.sh

check-tools:
	deploy/scripts/update-tools.sh --check

# Remove .env from the whole git history (rewrites history; backs up first; dry run unless YES=1).
purge-env-history:
	deploy/scripts/purge-env-history.sh $(if $(YES),--yes)

# shellcheck over every shell script in the repository (warnings and above).
lint-sh:
	git ls-files -co --exclude-standard '*.sh' | xargs shellcheck -x -S warning deploy/scripts/git-hooks/pre-commit

# Known vulnerabilities in the Rust and JavaScript dependencies (needs cargo-audit; bun audit is built in).
audit:
	cd src/rust && cargo audit
	cd src && bun audit --prod

# Boots an image next to a fake upstream and checks it (src/tests/smoke/README.md). Needs Docker.
#   make smoke IMAGE=dev-workspace:trixie    test another image (default: the one `make build` made)
#   make smoke-build                         build the production (Kali) image first
IMAGE ?= dev-workspace:$(WORKSPACE_VARIANT)
smoke:
	src/tests/smoke/run.sh --image $(IMAGE) --services

smoke-build:
	src/tests/smoke/run.sh --build --services

clean:
	rm -f workspace.tar
	rm -rf src/packages/*/node_modules src/node_modules

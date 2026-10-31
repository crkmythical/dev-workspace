# Secure Dev Workspace — Build & Test
.PHONY: build image-export image-import dev test lint format env-example smoke smoke-build clean

build:
	docker compose build

image-export:
	docker save dev-workspace:$${WORKSPACE_VARIANT:-latest} -o workspace.tar

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

# Boots an image next to a fake upstream and checks it (src/tests/smoke/README.md). Needs Docker.
#   make smoke IMAGE=dev-workspace:trixie    test an image you already have
#   make smoke-build                         build the production (Kali) image first
IMAGE ?= dev-workspace:latest
smoke:
	src/tests/smoke/run.sh --image $(IMAGE) --services

smoke-build:
	src/tests/smoke/run.sh --build --services

clean:
	rm -f workspace.tar
	rm -rf src/packages/*/node_modules src/node_modules

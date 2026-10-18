# Secure Dev Workspace — Build & Test
.PHONY: build image-export image-import dev test lint format env-example smoke smoke-build clean

build:
	docker compose build

image-export:
	docker save dev-workspace:$${WORKSPACE_VARIANT:-latest} -o workspace.tar

image-import:
	docker load -i workspace.tar

dev:
	bun run --watch packages/server/src/index.ts

test:
	bun test

lint:
	bunx biome check .

format:
	bunx biome format . --write

build-spa:
	cd packages/spa && bun run build
env-example:
	bun run packages/cli/scripts/gen-env-example.ts

# Boots an image next to a fake upstream and checks it (tests/smoke/README.md). Needs Docker.
#   make smoke IMAGE=dev-workspace:trixie    test an image you already have
#   make smoke-build                         build the production (Kali) image first
IMAGE ?= dev-workspace:latest
smoke:
	tests/smoke/run.sh --image $(IMAGE) --services

smoke-build:
	tests/smoke/run.sh --build --services

clean:
	rm -f workspace.tar
	rm -rf packages/*/node_modules node_modules

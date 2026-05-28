# Secure Dev Workspace — Build & Test
.PHONY: build image-export image-import dev test lint format clean

build:
	docker compose build

image-export:
	docker save dev-workspace -o workspace.tar

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

clean:
	rm -f workspace.tar
	rm -rf packages/*/node_modules node_modules

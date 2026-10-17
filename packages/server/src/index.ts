/**
 * Sync service — Hono + Bun.serve()
 * Composition root: reads the environment once, wires the real adapters, serves.
 * Serves: SPA static files, encrypted upload API, authenticated listing, doctor
 * endpoint, WebSocket events.
 */
import { SYNC_SERVICE_PORT } from "@sdw/core/sync/layout";
import { loadConfig } from "@sdw/core/workspace";
import { createSyncDeps } from "@sdw/infra";
import { createApp } from "./app.ts";

const { features } = loadConfig(process.env);
const app = createApp({ features, sync: createSyncDeps() });

// WebSocket clients
const wsClients = new Set<unknown>();

// Bun.serve with WS support
const server = Bun.serve({
  port: SYNC_SERVICE_PORT,
  hostname: "127.0.0.1",
  fetch(req, server) {
    const url = new URL(req.url);
    if (url.pathname === "/sync/api/events") {
      const upgraded = server.upgrade(req);
      if (upgraded) return undefined;
      return new Response("WebSocket upgrade failed", { status: 400 });
    }
    return app.fetch(req);
  },
  websocket: {
    open(ws) {
      wsClients.add(ws);
    },
    message() {},
    close(ws) {
      wsClients.delete(ws);
    },
  },
});

console.log(`sync-service listening on 127.0.0.1:${SYNC_SERVICE_PORT}`);

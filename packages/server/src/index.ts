/**
 * Sync service — Hono + Bun.serve()
 * Serves: SPA static files, encrypted upload/download API, doctor endpoint, WebSocket events.
 */
import { Hono } from "hono";
import { SHARED_DIR, SYNC_SERVICE_PORT } from "../../core/src/constants.ts";
import { destructRoute } from "./routes/destruct.ts";
import { doctorRoute } from "./routes/doctor.ts";
import { downloadRoute } from "./routes/download.ts";
import { uploadRoute } from "./routes/upload.ts";

const app = new Hono();

// Static SPA
app.get("/sync/", async (c) => {
  const file = Bun.file("/opt/workspace/packages/spa/dist/index.html");
  if (await file.exists()) return c.html(await file.text());
  return c.html("<h1>Sync Page</h1><p>Build with: bun run build:spa</p>");
});
app.get("/sync/assets/*", async (c) => {
  const filePath = c.req.path.replace("/sync/", "/opt/workspace/packages/spa/dist/");
  const file = Bun.file(filePath);
  if (await file.exists()) return new Response(file);
  return c.notFound();
});

// API routes
app.post("/sync/api/upload", uploadRoute);
app.get("/sync/api/download", downloadRoute);
app.get("/sync/api/doctor", doctorRoute);
app.post("/sync/api/destruct", destructRoute);

// Content type parser for image/png (encrypted payloads)
app.use("/sync/api/upload", async (c, next) => {
  await next();
});

// WebSocket clients
const wsClients = new Set<any>();

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

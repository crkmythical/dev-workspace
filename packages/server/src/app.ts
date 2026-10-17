import { SYNC_API, type SyncDeps } from "@sdw/core/sync";
/**
 * The sync service's HTTP surface. Dependencies come in as parameters, so the
 * very same app runs in production (real adapters) and in tests (temp-dir
 * adapters) with nothing mocked in between.
 */
import { SPA_DIST_DIR } from "@sdw/core/sync/layout";
import type { FeatureFlags } from "@sdw/core/workspace";
import { Hono } from "hono";
import { destructRoute } from "./routes/destruct.ts";
import { doctorRoute } from "./routes/doctor.ts";
import { downloadRoute } from "./routes/download.ts";
import { uploadRoute } from "./routes/upload.ts";

export interface AppDeps {
  features: FeatureFlags;
  sync: SyncDeps;
  /** Where the compiled SPA lives. */
  spaDistDir?: string;
}

export function createApp({ features, sync, spaDistDir = SPA_DIST_DIR }: AppDeps) {
  const app = new Hono();

  // Static SPA
  app.get("/sync/", async (c) => {
    const file = Bun.file(`${spaDistDir}/index.html`);
    if (await file.exists()) return c.html(await file.text());
    return c.html("<h1>Sync Page</h1><p>Build with: bun run build:spa</p>");
  });
  app.get("/sync/assets/*", async (c) => {
    const file = Bun.file(c.req.path.replace("/sync/", `${spaDistDir}/`));
    if (await file.exists()) return new Response(file);
    return c.notFound();
  });

  // API routes
  app.post(SYNC_API.upload, (c) => uploadRoute(c, features, sync));
  app.get(SYNC_API.list, (c) => downloadRoute(c, features, sync));
  app.get(SYNC_API.doctor, (c) => doctorRoute(c, features));
  app.post(SYNC_API.destruct, (c) => destructRoute(c, features));

  return app;
}

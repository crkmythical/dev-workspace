import { readFile, readdir, stat } from "node:fs/promises";
import path from "node:path";
import type { Context } from "hono";
import { PNG_HEADER, SHARED_DIR } from "../../../core/src/constants.ts";
import { checkRelativePath } from "../../../core/src/path-safety.ts";

export async function downloadRoute(c: Context) {
  const filePath = c.req.query("path");

  if (!filePath) {
    // List files in shared directory
    try {
      const entries = await readdir(SHARED_DIR);
      const files = [];
      for (const entry of entries) {
        if (entry.startsWith(".")) continue;
        const fullPath = path.join(SHARED_DIR, entry);
        const s = await stat(fullPath);
        if (s.isFile()) {
          files.push({ name: entry, size: s.size, mtime: s.mtimeMs });
        }
      }
      return c.json({ files });
    } catch {
      return c.json({ files: [] });
    }
  }

  // Download specific file. `filePath` is attacker-influenced query input and
  // must be validated to stay within SHARED_DIR (defense-in-depth traversal).
  const pathCheck = checkRelativePath(filePath);
  if (!pathCheck.ok) {
    return c.json({ error: "not-found" }, 404);
  }
  const fullPath = path.join(SHARED_DIR, pathCheck.normalized);
  try {
    const data = await readFile(fullPath);
    // Wrap with PNG header for camouflage
    const camouflaged = Buffer.concat([Buffer.from(PNG_HEADER), data]);
    return new Response(camouflaged, {
      headers: { "Content-Type": "image/png" },
    });
  } catch {
    return c.json({ error: "not-found" }, 404);
  }
}

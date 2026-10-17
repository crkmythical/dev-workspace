import { defineConfig } from "vite";

// @sdw/core is resolved through the workspace link and the `exports` map in its
// package.json, the same way TypeScript and Bun resolve it. No alias: a second
// resolution rule here would silently diverge from the one the type checker uses.
export default defineConfig({
  root: __dirname,
  base: "/sync/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});

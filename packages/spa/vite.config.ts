import { defineConfig } from "vite";
import { resolve } from "path";

export default defineConfig({
  root: __dirname,
  base: "/sync/",
  resolve: {
    alias: {
      "@sdw/core": resolve(__dirname, "../core/src"),
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});

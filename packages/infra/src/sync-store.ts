/**
 * UploadStore on the filesystem: chunks are appended to `.uploading-<id>` files
 * in a staging directory, then renamed into the shared directory.
 *
 * Ids and relative paths have already been validated by the use case; the
 * directories are injectable so the whole path can run against a temp dir.
 */
import crypto from "node:crypto";
import { existsSync } from "node:fs";
import { appendFile, mkdir, readFile, readdir, rename, stat, unlink } from "node:fs/promises";
import path from "node:path";
import type { SharedFile, UploadStore } from "@sdw/core/sync";
import { SHARED_DIR, UPLOAD_TRACKING_DIR } from "@sdw/core/sync/layout";

export interface FsUploadStoreOptions {
  sharedDir?: string;
  uploadDir?: string;
}

export function createFsUploadStore(opts: FsUploadStoreOptions = {}): UploadStore {
  const sharedDir = opts.sharedDir ?? SHARED_DIR;
  const uploadDir = opts.uploadDir ?? UPLOAD_TRACKING_DIR;
  const tempPath = (id: string) => path.join(uploadDir, `.uploading-${id}`);

  return {
    async appendChunk(fileId, data) {
      await mkdir(uploadDir, { recursive: true });
      await mkdir(sharedDir, { recursive: true });
      await appendFile(tempPath(fileId), data);
    },

    async hasChunks(fileId) {
      return existsSync(tempPath(fileId));
    },

    async sha256Hex(fileId) {
      return crypto
        .createHash("sha256")
        .update(await readFile(tempPath(fileId)))
        .digest("hex");
    },

    async discard(fileId) {
      await unlink(tempPath(fileId)).catch(() => {});
    },

    async commit(fileId, relativePath) {
      const finalPath = path.join(sharedDir, relativePath);
      await mkdir(path.dirname(finalPath), { recursive: true });
      await rename(tempPath(fileId), finalPath);
    },

    async list() {
      let entries: string[];
      try {
        entries = await readdir(sharedDir);
      } catch {
        return [];
      }
      const files: SharedFile[] = [];
      for (const name of entries) {
        if (name.startsWith(".")) continue;
        const s = await stat(path.join(sharedDir, name));
        if (s.isFile()) files.push({ name, size: s.size, mtime: s.mtimeMs });
      }
      return files;
    },
  };
}

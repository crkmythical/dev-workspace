import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import {
  ReplayWindow,
  SYNC_API,
  type SyncDeps,
  buildChunkRequest,
  buildFinalizeRequest,
  buildListAuth,
  deriveSyncKey,
} from "@sdw/core/sync";
import { createFileKeyProvider, createFsUploadStore, nodeAead } from "@sdw/infra";
import { createApp } from "../src/app.ts";

/**
 * End to end through the real HTTP app: a WebCrypto client (the code the SPA
 * runs) builds genuine requests; the app, the use cases and the Node/filesystem
 * adapters answer them. Only the directories are redirected to a temp dir.
 */

const PASSPHRASE = "end to end passphrase";
let root: string;
let passphraseFile: string;
let deps: SyncDeps;

beforeEach(() => {
  root = mkdtempSync(path.join(tmpdir(), "sdw-e2e-"));
  passphraseFile = path.join(root, "sync-passphrase");
  deps = {
    keys: createFileKeyProvider(passphraseFile),
    aead: nodeAead,
    store: createFsUploadStore({
      sharedDir: path.join(root, "shared"),
      uploadDir: path.join(root, "shared", ".uploads"),
    }),
    replay: new ReplayWindow(),
  };
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

const on = { sync: true, remoteDestruct: false };
const off = { sync: false, remoteDestruct: false };
const unlock = () => writeFileSync(passphraseFile, PASSPHRASE);
const post = (
  app: ReturnType<typeof createApp>,
  req: { headers: Record<string, string>; body: Uint8Array },
) => app.request(SYNC_API.upload, { method: "POST", headers: req.headers, body: req.body });

async function uploadFile(
  app: ReturnType<typeof createApp>,
  name: string,
  content: string,
  id = "f1",
) {
  const key = await deriveSyncKey(PASSPHRASE);
  const data = new TextEncoder().encode(content);
  const hash = [
    ...new Uint8Array(await crypto.subtle.digest("SHA-256", data as Uint8Array<ArrayBuffer>)),
  ]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  const chunk = await post(app, await buildChunkRequest(key, Date.now(), id, 0, data));
  const fin = await post(
    app,
    await buildFinalizeRequest(key, Date.now(), {
      fileId: id,
      filePath: name,
      contentSha256: hash,
      totalChunks: 1,
    }),
  );
  return { chunk, fin };
}

describe("feature gating", () => {
  it("every sync endpoint is a plain 404 while SYNC_ENABLED is off", async () => {
    const app = createApp({ features: off, sync: deps });
    unlock();
    expect((await app.request(SYNC_API.upload, { method: "POST", body: "x" })).status).toBe(404);
    expect((await app.request(SYNC_API.list)).status).toBe(404);
    expect((await app.request(SYNC_API.destruct, { method: "POST", body: "{}" })).status).toBe(404);
  });

  it("the doctor says 'disabled' rather than 'vault-locked'", async () => {
    const res = await createApp({ features: off, sync: deps }).request(SYNC_API.doctor);
    expect(await res.json()).toMatchObject({ state: "disabled", sync_enabled: false });
  });
});

describe("a real upload over HTTP", () => {
  it("stores the file in the shared directory with the exact bytes sent", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    const { chunk, fin } = await uploadFile(app, "notes/today.txt", "line one\nline two\n");
    expect(chunk.status).toBe(200);
    expect(await chunk.json()).toEqual({ status: "chunk-received", chunk_idx: 0 });
    expect(fin.status).toBe(200);
    expect(await fin.json()).toEqual({ status: "upload-complete", path: "notes/today.txt" });
    expect(readFileSync(path.join(root, "shared", "notes", "today.txt"), "utf-8")).toBe(
      "line one\nline two\n",
    );
    expect(existsSync(path.join(root, "shared", ".uploads", ".uploading-f1"))).toBe(false);
  });

  it("answers 503 while the vault is locked (no passphrase file), 200 once it is unlocked", async () => {
    const app = createApp({ features: on, sync: deps });
    const key = await deriveSyncKey(PASSPHRASE);
    const req = await buildChunkRequest(key, Date.now(), "f", 0, new TextEncoder().encode("x"));
    const locked = await post(app, req);
    expect(locked.status).toBe(503);
    expect(await locked.json()).toEqual({ error: "vault-locked" });

    unlock();
    expect(
      (
        await post(
          app,
          await buildChunkRequest(key, Date.now(), "f", 0, new TextEncoder().encode("x")),
        )
      ).status,
    ).toBe(200);
  });

  it("locking the vault again takes the key away immediately", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    const key = await deriveSyncKey(PASSPHRASE);
    const ok = await post(
      app,
      await buildChunkRequest(key, Date.now(), "a", 0, new TextEncoder().encode("x")),
    );
    expect(ok.status).toBe(200);
    rmSync(passphraseFile); // the vault unmounts: the file disappears
    const after = await post(
      app,
      await buildChunkRequest(key, Date.now(), "b", 0, new TextEncoder().encode("x")),
    );
    expect(after.status).toBe(503);
  });

  it("maps a replayed request to 400", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    const req = await buildChunkRequest(
      await deriveSyncKey(PASSPHRASE),
      Date.now(),
      "f",
      0,
      new TextEncoder().encode("x"),
    );
    expect((await post(app, req)).status).toBe(200);
    const again = await post(app, req);
    expect(again.status).toBe(400);
    expect(await again.json()).toEqual({ error: "replay-detected" });
  });

  it("maps a wrong passphrase to 400 and writes nothing", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    const wrong = await deriveSyncKey("not the passphrase");
    const res = await post(
      app,
      await buildChunkRequest(wrong, Date.now(), "f", 0, new TextEncoder().encode("x")),
    );
    expect(res.status).toBe(400);
    expect(await res.json()).toEqual({ error: "aead-failure" });
    expect(existsSync(path.join(root, "shared"))).toBe(false);
  });

  it("refuses a path that tries to leave the shared directory", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    const key = await deriveSyncKey(PASSPHRASE);
    await post(
      app,
      await buildChunkRequest(key, Date.now(), "f", 0, new TextEncoder().encode("x")),
    );
    const fin = await post(
      app,
      await buildFinalizeRequest(key, Date.now(), {
        fileId: "f",
        filePath: "../escape.txt",
        contentSha256: "",
        totalChunks: 1,
      }),
    );
    expect(fin.status).toBe(400);
    expect(await fin.json()).toEqual({ error: "bad-file-path" });
    expect(existsSync(path.join(root, "escape.txt"))).toBe(false);
  });
});

describe("listing", () => {
  it("returns the shared files to a client that proves the sync key", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    await uploadFile(app, "a.txt", "12345");
    const key = await deriveSyncKey(PASSPHRASE);
    const res = await app.request(SYNC_API.list, { headers: await buildListAuth(key, Date.now()) });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { files: Array<{ name: string; size: number }> };
    expect(body.files.map((f) => [f.name, f.size])).toEqual([["a.txt", 5]]);
  });

  it("is 401 without the authenticator, so file names are not exposed", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    await uploadFile(app, "secret-name.txt", "x");
    const res = await app.request(SYNC_API.list);
    expect(res.status).toBe(401);
    expect(await res.text()).not.toContain("secret-name");
  });

  it("no longer serves file content at all (the old plaintext ?path= download)", async () => {
    const app = createApp({ features: on, sync: deps });
    unlock();
    await uploadFile(app, "a.txt", "plaintext-content");
    const key = await deriveSyncKey(PASSPHRASE);
    const res = await app.request(`${SYNC_API.list}?path=a.txt`, {
      headers: await buildListAuth(key, Date.now()),
    });
    expect(res.status).toBe(404);
    expect(await res.text()).not.toContain("plaintext-content");
  });
});

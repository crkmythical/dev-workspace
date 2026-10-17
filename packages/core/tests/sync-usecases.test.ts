import { describe, expect, it } from "bun:test";
import {
  buildChunkRequest,
  buildFinalizeRequest,
  buildListAuth,
  deriveSyncKey,
  deriveSyncKeyBytes,
} from "../src/sync/client.ts";
import {
  type AeadPort,
  ReplayWindow,
  type SharedFile,
  type SyncDeps,
  type SyncKey,
  type UploadStore,
  buildAad,
  listShared,
  receiveUpload,
  wrapPng,
} from "../src/sync/index.ts";

/**
 * The receive use cases against an in-memory store and a WebCrypto "server" AEAD.
 *
 * A REAL client (the same code the browser SPA runs) builds every request, so the
 * ordering guarantees are tested against genuine envelopes: nothing reaches the
 * store before authentication, and each failure is the named code the HTTP layer
 * maps to a status.
 */

const NOW = 1_700_000_000_000;
const PASSPHRASE = "correct horse battery staple";

/** AES-GCM open via WebCrypto: a second, independent implementation next to Node's. */
const webAead: AeadPort = {
  async open(key, nonce, ciphertext, tag, aad) {
    const k = await crypto.subtle.importKey(
      "raw",
      key as Uint8Array<ArrayBuffer>,
      "AES-GCM",
      false,
      ["decrypt"],
    );
    const sealed = new Uint8Array(ciphertext.length + tag.length);
    sealed.set(ciphertext, 0);
    sealed.set(tag, ciphertext.length);
    try {
      return new Uint8Array(
        await crypto.subtle.decrypt(
          {
            name: "AES-GCM",
            iv: nonce as Uint8Array<ArrayBuffer>,
            additionalData: aad as Uint8Array<ArrayBuffer>,
            tagLength: 128,
          },
          k,
          sealed,
        ),
      );
    } catch {
      return null;
    }
  },
};

class MemoryStore implements UploadStore {
  staged = new Map<string, Uint8Array[]>();
  committed = new Map<string, Uint8Array>();
  calls: string[] = [];
  failOn: string | null = null;

  async appendChunk(fileId: string, data: Uint8Array) {
    this.calls.push(`append:${fileId}`);
    if (this.failOn === "append") throw new Error("disk full");
    this.staged.set(fileId, [...(this.staged.get(fileId) ?? []), data]);
  }
  async hasChunks(fileId: string) {
    return this.staged.has(fileId);
  }
  private joined(fileId: string): Uint8Array {
    const parts = this.staged.get(fileId) ?? [];
    const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
    let off = 0;
    for (const p of parts) {
      out.set(p, off);
      off += p.length;
    }
    return out;
  }
  async sha256Hex(fileId: string) {
    const digest = await crypto.subtle.digest(
      "SHA-256",
      this.joined(fileId) as Uint8Array<ArrayBuffer>,
    );
    return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
  }
  async discard(fileId: string) {
    this.calls.push(`discard:${fileId}`);
    this.staged.delete(fileId);
  }
  async commit(fileId: string, relativePath: string) {
    this.calls.push(`commit:${fileId}->${relativePath}`);
    this.committed.set(relativePath, this.joined(fileId));
    this.staged.delete(fileId);
  }
  async list(): Promise<SharedFile[]> {
    return [...this.committed].map(([name, data]) => ({ name, size: data.length, mtime: NOW }));
  }
}

async function setup(opts: { locked?: boolean; windowSize?: number } = {}) {
  const keyBytes: SyncKey = await deriveSyncKeyBytes(PASSPHRASE);
  const store = new MemoryStore();
  const deps: SyncDeps = {
    keys: { current: async () => (opts.locked ? null : keyBytes) },
    aead: webAead,
    store,
    replay: new ReplayWindow(opts.windowSize),
  };
  return { deps, store, clientKey: await deriveSyncKey(PASSPHRASE) };
}

const toRequest = (parts: { headers: Record<string, string>; body: Uint8Array }, nowMs = NOW) => ({
  body: parts.body,
  op: parts.headers["X-Sync-Op"],
  ts: Number.parseInt(parts.headers["X-Sync-Ts"], 10),
  fileId: parts.headers["X-Sync-FileId"],
  chunkIdx: parts.headers["X-Sync-ChunkIdx"],
  nowMs,
});

const bytes = (s: string) => new TextEncoder().encode(s);
const sha256 = async (data: Uint8Array) =>
  [...new Uint8Array(await crypto.subtle.digest("SHA-256", data as Uint8Array<ArrayBuffer>))]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

describe("receiveUpload: a complete upload", () => {
  it("stages every chunk, then commits the file at the requested path", async () => {
    const { deps, store, clientKey } = await setup();
    const part1 = bytes("hello ");
    const part2 = bytes("world");

    const r1 = await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW, "f1", 0, part1)),
      deps,
    );
    const r2 = await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW + 1, "f1", 1, part2)),
      deps,
    );
    expect(r1).toEqual({ ok: true, result: { status: "chunk-received", chunk_idx: 0 } });
    expect(r2).toEqual({ ok: true, result: { status: "chunk-received", chunk_idx: 1 } });

    const fin = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, NOW + 2, {
          fileId: "f1",
          filePath: "docs/a.txt",
          contentSha256: await sha256(bytes("hello world")),
          totalChunks: 2,
        }),
      ),
      deps,
    );
    expect(fin).toEqual({ ok: true, result: { status: "upload-complete", path: "docs/a.txt" } });
    expect(new TextDecoder().decode(store.committed.get("docs/a.txt"))).toBe("hello world");
  });

  it("finalizes without a checksum when the client sends none", async () => {
    const { deps, store, clientKey } = await setup();
    await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW, "f2", 0, bytes("x"))),
      deps,
    );
    const fin = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, NOW + 1, {
          fileId: "f2",
          filePath: "x.bin",
          contentSha256: "",
          totalChunks: 1,
        }),
      ),
      deps,
    );
    expect(fin.ok).toBe(true);
    expect(store.committed.has("x.bin")).toBe(true);
  });
});

describe("receiveUpload: nothing reaches storage before authentication", () => {
  it("a locked vault is refused before anything is parsed", async () => {
    const { deps, store, clientKey } = await setup({ locked: true });
    const out = await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x"))),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "vault-locked" });
    expect(store.calls).toEqual([]);
  });

  it("a body too short to hold header, nonce and tag is rejected", async () => {
    const { deps, store } = await setup();
    const out = await receiveUpload(
      {
        body: wrapPng(new Uint8Array(10)).subarray(0, 40),
        op: "put-chunk",
        ts: NOW,
        fileId: "f",
        chunkIdx: "0",
        nowMs: NOW,
      },
      deps,
    );
    expect(out).toEqual({ ok: false, code: "too-short" });
    expect(store.calls).toEqual([]);
  });

  it("a tampered ciphertext fails authentication and stores nothing", async () => {
    const { deps, store, clientKey } = await setup();
    const parts = await buildChunkRequest(clientKey, NOW, "f", 0, bytes("secret"));
    parts.body[parts.body.length - 20] ^= 0xff;
    const out = await receiveUpload(toRequest(parts), deps);
    expect(out).toEqual({ ok: false, code: "aead-failure" });
    expect(store.calls).toEqual([]);
  });

  it("an envelope sealed for one file id cannot be replayed under another (AAD binding)", async () => {
    const { deps, store, clientKey } = await setup();
    const parts = await buildChunkRequest(clientKey, NOW, "victim", 0, bytes("x"));
    const out = await receiveUpload({ ...toRequest(parts), fileId: "attacker" }, deps);
    expect(out).toEqual({ ok: false, code: "aead-failure" });
    expect(store.calls).toEqual([]);
  });

  it("an envelope sealed with the wrong passphrase fails authentication", async () => {
    const { deps, store } = await setup();
    const wrong = await deriveSyncKey("a different passphrase");
    const out = await receiveUpload(
      toRequest(await buildChunkRequest(wrong, NOW, "f", 0, bytes("x"))),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "aead-failure" });
    expect(store.calls).toEqual([]);
  });

  it("an op the client did not seal fails authentication (op is part of the AAD)", async () => {
    const { deps } = await setup();
    const parts = await buildChunkRequest(await deriveSyncKey(PASSPHRASE), NOW, "f", 0, bytes("x"));
    const out = await receiveUpload({ ...toRequest(parts), op: "put-finalize" }, deps);
    expect(out).toEqual({ ok: false, code: "aead-failure" });
  });
});

describe("receiveUpload: replay and timestamps", () => {
  it("the same envelope sent twice is a replay the second time", async () => {
    const { deps, clientKey } = await setup();
    const req = toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x")));
    expect((await receiveUpload(req, deps)).ok).toBe(true);
    expect(await receiveUpload(req, deps)).toEqual({ ok: false, code: "replay-detected" });
  });

  it("a timestamp outside the tolerance window is rejected", async () => {
    const { deps, clientKey } = await setup();
    const stale = toRequest(
      await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x")),
      NOW + 10 * 60_000,
    );
    expect(await receiveUpload(stale, deps)).toEqual({ ok: false, code: "replay-detected" });
  });

  it("a non-numeric timestamp is rejected, not waved through the tolerance check", async () => {
    const { deps, clientKey } = await setup();
    const req = {
      ...toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x"))),
      ts: Number.NaN,
    };
    expect(await receiveUpload(req, deps)).toEqual({ ok: false, code: "bad-timestamp" });
  });

  it("every request of a slow multi-chunk upload carries its own timestamp and is accepted", async () => {
    const { deps, clientKey } = await setup();
    // 4 minutes between the first chunk and the finalize: fine, each ts is fresh.
    const t0 = NOW;
    const t1 = NOW + 2 * 60_000;
    const t2 = NOW + 4 * 60_000;
    expect(
      (
        await receiveUpload(
          toRequest(await buildChunkRequest(clientKey, t0, "slow", 0, bytes("a")), t0),
          deps,
        )
      ).ok,
    ).toBe(true);
    expect(
      (
        await receiveUpload(
          toRequest(await buildChunkRequest(clientKey, t1, "slow", 1, bytes("b")), t1),
          deps,
        )
      ).ok,
    ).toBe(true);
    const fin = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, t2, {
          fileId: "slow",
          filePath: "s.bin",
          contentSha256: "",
          totalChunks: 2,
        }),
        t2,
      ),
      deps,
    );
    expect(fin.ok).toBe(true);
  });

  it("(why the SPA must not reuse a timestamp) a first-chunk timestamp is stale by the finalize", async () => {
    const { deps, clientKey } = await setup();
    const reused = NOW;
    const lateNow = NOW + 6 * 60_000; // beyond the 5 minute tolerance
    const fin = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, reused, {
          fileId: "f",
          filePath: "f.bin",
          contentSha256: "",
          totalChunks: 1,
        }),
        lateNow,
      ),
      deps,
    );
    expect(fin).toEqual({ ok: false, code: "replay-detected" });
  });
});

describe("replay window: unauthenticated requests cannot flush it", () => {
  const WINDOW = 4;

  /** A request that is well-formed but forged: right shape, fresh nonce, wrong key. */
  async function forgedUpload(i: number) {
    const wrong = await deriveSyncKey(`attacker-${i}`);
    return toRequest(await buildChunkRequest(wrong, NOW, `forged-${i}`, 0, bytes("x")));
  }

  it("a flood of forged uploads does not push a genuine nonce out of the window", async () => {
    const { deps, clientKey } = await setup({ windowSize: WINDOW });
    const genuine = toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x")));
    expect((await receiveUpload(genuine, deps)).ok).toBe(true);

    // More forgeries than the window holds: if they could claim slots, the genuine
    // nonce would be evicted and the replay below would be accepted.
    for (let i = 0; i < WINDOW * 3; i++) {
      expect(await receiveUpload(await forgedUpload(i), deps)).toEqual({
        ok: false,
        code: "aead-failure",
      });
    }
    expect(await receiveUpload(genuine, deps)).toEqual({ ok: false, code: "replay-detected" });
  });

  it("a flood of forged list authenticators does not either", async () => {
    const { deps, clientKey } = await setup({ windowSize: WINDOW });
    const h = await buildListAuth(clientKey, NOW);
    const genuine = { ts: NOW, auth: h["X-Sync-Auth"], nowMs: NOW };
    expect((await listShared(genuine, deps)).ok).toBe(true);

    for (let i = 0; i < WINDOW * 3; i++) {
      const forged = await buildListAuth(await deriveSyncKey(`attacker-${i}`), NOW);
      expect(await listShared({ ts: NOW, auth: forged["X-Sync-Auth"], nowMs: NOW }, deps)).toEqual({
        ok: false,
        code: "unauthorized",
      });
    }
    expect(await listShared(genuine, deps)).toEqual({ ok: false, code: "unauthorized" });
  });

  it("a request that fails authentication can be retried: it never claimed its nonce", async () => {
    const { deps, clientKey } = await setup();
    const good = await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x"));
    const tampered = new Uint8Array(good.body);
    tampered[tampered.length - 1] ^= 0xff; // flip a tag bit, keep the nonce
    expect(await receiveUpload(toRequest({ ...good, body: tampered }), deps)).toEqual({
      ok: false,
      code: "aead-failure",
    });
    // The intact envelope carries the same nonce and is still accepted.
    expect((await receiveUpload(toRequest(good), deps)).ok).toBe(true);
  });
});

describe("receiveUpload: untrusted ids and paths", () => {
  it("rejects a file id that is not one flat, safe segment", async () => {
    for (const id of ["../etc/passwd", "a/b", "/abs", "", ".."]) {
      const { deps, store, clientKey } = await setup();
      const out = await receiveUpload(
        toRequest(await buildChunkRequest(clientKey, NOW, id, 0, bytes("x"))),
        deps,
      );
      expect(out).toEqual({ ok: false, code: "bad-file-id" });
      expect(store.calls).toEqual([]);
    }
  });

  it("rejects a finalize path that escapes the shared directory", async () => {
    const { deps, store, clientKey } = await setup();
    await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x"))),
      deps,
    );
    const out = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, NOW + 1, {
          fileId: "f",
          filePath: "../../etc/cron.d/x",
          contentSha256: "",
          totalChunks: 1,
        }),
      ),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "bad-file-path" });
    expect(store.committed.size).toBe(0);
  });

  it("rejects a finalize whose decrypted metadata names a traversing file id", async () => {
    const { deps, clientKey } = await setup();
    const out = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, NOW, {
          fileId: "../x",
          filePath: "a.txt",
          contentSha256: "",
          totalChunks: 1,
        }),
      ),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "bad-file-id" });
  });
});

describe("receiveUpload: finalize failures", () => {
  it("finalizing a file with no staged chunks is refused", async () => {
    const { deps, clientKey } = await setup();
    const out = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, NOW, {
          fileId: "ghost",
          filePath: "g.bin",
          contentSha256: "",
          totalChunks: 1,
        }),
      ),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "no-chunks" });
  });

  it("a checksum mismatch discards the staged file and commits nothing", async () => {
    const { deps, store, clientKey } = await setup();
    await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("real"))),
      deps,
    );
    const out = await receiveUpload(
      toRequest(
        await buildFinalizeRequest(clientKey, NOW + 1, {
          fileId: "f",
          filePath: "f.bin",
          contentSha256: "0".repeat(64),
          totalChunks: 1,
        }),
      ),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "checksum-mismatch" });
    expect(store.calls).toContain("discard:f");
    expect(store.committed.size).toBe(0);
    expect(store.staged.has("f")).toBe(false);
  });

  it("an unknown op, once authenticated, is named as such", async () => {
    const { deps, clientKey } = await setup();
    // seal an envelope for an op the server does not implement
    const key = clientKey;
    const aad = buildAad("rename", NOW, "f", "0");
    const sealed = new Uint8Array(
      await crypto.subtle.encrypt(
        {
          name: "AES-GCM",
          iv: new Uint8Array(12).fill(7),
          additionalData: aad as Uint8Array<ArrayBuffer>,
          tagLength: 128,
        },
        key,
        bytes("x"),
      ),
    );
    const body = wrapPng(new Uint8Array([...new Uint8Array(12).fill(7), ...sealed]));
    const out = await receiveUpload(
      { body, op: "rename", ts: NOW, fileId: "f", chunkIdx: "0", nowMs: NOW },
      deps,
    );
    expect(out).toEqual({ ok: false, code: "unknown-op" });
  });

  it("a storage failure is reported as internal, with no detail", async () => {
    const { deps, store, clientKey } = await setup();
    store.failOn = "append";
    const out = await receiveUpload(
      toRequest(await buildChunkRequest(clientKey, NOW, "f", 0, bytes("x"))),
      deps,
    );
    expect(out).toEqual({ ok: false, code: "internal" });
  });
});

describe("listShared: listing needs proof of the sync key", () => {
  async function withFile() {
    const ctx = await setup();
    await receiveUpload(
      toRequest(await buildChunkRequest(ctx.clientKey, NOW, "f", 0, bytes("12345"))),
      ctx.deps,
    );
    await receiveUpload(
      toRequest(
        await buildFinalizeRequest(ctx.clientKey, NOW + 1, {
          fileId: "f",
          filePath: "a.txt",
          contentSha256: "",
          totalChunks: 1,
        }),
      ),
      ctx.deps,
    );
    return ctx;
  }

  it("returns the files for a valid authenticator", async () => {
    const { deps, clientKey } = await withFile();
    const headers = await buildListAuth(clientKey, NOW + 2);
    const out = await listShared(
      { ts: Number(headers["X-Sync-Ts"]), auth: headers["X-Sync-Auth"], nowMs: NOW + 2 },
      deps,
    );
    expect(out).toEqual({ ok: true, files: [{ name: "a.txt", size: 5, mtime: NOW }] });
  });

  it("refuses a missing, malformed or wrongly keyed authenticator with one code", async () => {
    const { deps } = await withFile();
    const wrong = await buildListAuth(await deriveSyncKey("another passphrase"), NOW + 2);
    const attempts = [
      { ts: NOW + 2, auth: null },
      { ts: NOW + 2, auth: "!!!not base64!!!" },
      { ts: NOW + 2, auth: "AAAA" },
      { ts: Number(wrong["X-Sync-Ts"]), auth: wrong["X-Sync-Auth"] },
    ];
    for (const a of attempts) {
      expect(await listShared({ ...a, nowMs: NOW + 2 }, deps)).toEqual({
        ok: false,
        code: "unauthorized",
      });
    }
  });

  it("an authenticator cannot be replayed", async () => {
    const { deps, clientKey } = await withFile();
    const h = await buildListAuth(clientKey, NOW + 2);
    const req = { ts: Number(h["X-Sync-Ts"]), auth: h["X-Sync-Auth"], nowMs: NOW + 2 };
    expect((await listShared(req, deps)).ok).toBe(true);
    expect(await listShared(req, deps)).toEqual({ ok: false, code: "unauthorized" });
  });

  it("an authenticator for a different timestamp is refused (the timestamp is bound)", async () => {
    const { deps, clientKey } = await withFile();
    const h = await buildListAuth(clientKey, NOW + 2);
    expect(await listShared({ ts: NOW + 3, auth: h["X-Sync-Auth"], nowMs: NOW + 3 }, deps)).toEqual(
      { ok: false, code: "unauthorized" },
    );
  });

  it("an upload envelope cannot be passed off as a list authenticator", async () => {
    const { deps, clientKey } = await withFile();
    const up = await buildChunkRequest(clientKey, NOW + 5, "g", 0, bytes("x"));
    const asAuth = btoa(String.fromCharCode(...up.body.subarray(33)));
    expect(await listShared({ ts: NOW + 5, auth: asAuth, nowMs: NOW + 5 }, deps)).toEqual({
      ok: false,
      code: "unauthorized",
    });
  });

  it("a locked vault is unavailable", async () => {
    const { deps, clientKey } = await setup({ locked: true });
    const h = await buildListAuth(clientKey, NOW);
    expect(await listShared({ ts: NOW, auth: h["X-Sync-Auth"], nowMs: NOW }, deps)).toEqual({
      ok: false,
      code: "vault-locked",
    });
  });
});

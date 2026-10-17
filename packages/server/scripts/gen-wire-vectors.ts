#!/usr/bin/env bun
/**
 * Generate cross-language wire-protocol golden vectors from the AUTHORITATIVE
 * TypeScript implementation (packages/infra sync-crypto + packages/core sync).
 *
 * The Rust PoC (rust/sync-service) consumes the SAME JSON in its parity tests,
 * so both ends are proven byte-for-byte compatible on:
 *   - HKDF-SHA256 key derivation
 *   - AES-256-GCM (nonce + ciphertext||tag layout)
 *   - AAD string framing
 *   - the PNG-camouflage upload body
 *   - the list authenticator (base64(nonce || tag) over an empty message)
 *   - relative-path validation
 *
 * Deterministic (fixed nonces + patterned plaintext) so regenerating is stable.
 *
 * Run from repo root: bun run packages/server/scripts/gen-wire-vectors.ts
 */
import crypto from "node:crypto";
import {
  buildAad,
  buildChunkAad,
  buildListAuth,
  checkRelativePath,
  deriveSyncKey,
  sealEnvelope,
  toBase64,
} from "@sdw/core/sync";
import { HKDF_INFO, HKDF_SALT, PNG_HEADER } from "@sdw/core/sync/protocol";
import { decrypt, deriveKey } from "@sdw/infra";

const hex = (b: Uint8Array | Buffer): string => Buffer.from(b).toString("hex");

/** Encrypt with an explicit nonce so the vector is reproducible. */
function encryptFixed(key: Buffer, nonce: Buffer, plaintext: Buffer, aad: Uint8Array): Buffer {
  const cipher = crypto.createCipheriv("aes-256-gcm", key, nonce);
  cipher.setAAD(Buffer.from(aad));
  const ct = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  return Buffer.concat([ct, tag]); // WebCrypto/wire layout: ciphertext || tag
}

// ── HKDF ──────────────────────────────────────────────────────────────────
const passphrases = ["correct horse battery staple", "p", "日本語パスフレーズ🔐"];
const hkdf = passphrases.map((passphrase) => ({
  passphrase,
  salt: HKDF_SALT,
  info: HKDF_INFO,
  key_hex: hex(deriveKey(passphrase)),
}));

// ── AAD framing ───────────────────────────────────────────────────────────
const aadCases: Array<{ op: string; ts: number; extra: string[] }> = [
  { op: "put-chunk", ts: 1700000000000, extra: ["file-abc", "0"] },
  { op: "put-finalize", ts: 1699999999999, extra: ["file-xyz", "3"] },
  { op: "doctor", ts: 0, extra: [] },
];
const aad = aadCases.map((c) => ({
  ...c,
  aad_hex: hex(buildAad(c.op, c.ts, ...c.extra)),
}));

// ── AEAD + PNG body ───────────────────────────────────────────────────────
const key0 = deriveKey("correct horse battery staple");
const nonceA = Buffer.from([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
const nonceB = Buffer.from([11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
const pt = {
  empty: Buffer.alloc(0),
  hello: Buffer.from("hello world", "utf8"),
  bytes256: Buffer.from(Array.from({ length: 256 }, (_, i) => i)),
  k4: Buffer.from(Array.from({ length: 4096 }, (_, i) => i % 256)),
};
const aeadCases = [
  {
    label: "chunk-empty",
    ts: 1700000000000,
    file_id: "abc123",
    chunk_idx: 0,
    nonce: nonceA,
    plaintext: pt.empty,
  },
  {
    label: "chunk-hello",
    ts: 1700000000001,
    file_id: "abc123",
    chunk_idx: 1,
    nonce: nonceB,
    plaintext: pt.hello,
  },
  {
    label: "chunk-bytes256",
    ts: 1700000000002,
    file_id: "deadbeef",
    chunk_idx: 7,
    nonce: nonceA,
    plaintext: pt.bytes256,
  },
  {
    label: "chunk-4k",
    ts: 1700000000003,
    file_id: "deadbeef",
    chunk_idx: 2,
    nonce: nonceB,
    plaintext: pt.k4,
  },
];
const aead = aeadCases.map((c) => {
  const aadBytes = buildChunkAad(c.ts, c.file_id, c.chunk_idx);
  const ctWithTag = encryptFixed(key0, c.nonce, c.plaintext, aadBytes);
  const body = Buffer.concat([Buffer.from(PNG_HEADER), c.nonce, ctWithTag]);
  return {
    label: c.label,
    op: "put-chunk",
    ts: c.ts,
    file_id: c.file_id,
    chunk_idx: c.chunk_idx,
    key_hex: hex(key0),
    aad_hex: hex(aadBytes),
    nonce_hex: hex(c.nonce),
    plaintext_hex: hex(c.plaintext),
    ciphertext_with_tag_hex: hex(ctWithTag),
    body_hex: hex(body),
  };
});

// ── List authenticator ──────────────────────────────────────────────────────
// X-Sync-Auth = base64(nonce || tag): an AEAD over an EMPTY message, AAD "list|<ts>".
// Sealed with the production client code and a pinned nonce so the file is stable.
const clientKey = await deriveSyncKey("correct horse battery staple");
const listAuthCases = [
  { label: "list-a", ts: 1700000000000, nonce: nonceA },
  { label: "list-b", ts: 1700000000123, nonce: nonceB },
];
const listAuth = await Promise.all(
  listAuthCases.map(async (c) => {
    const aadBytes = buildAad("list", c.ts);
    const envelope = await sealEnvelope(clientKey, new Uint8Array(0), aadBytes, c.nonce);
    return {
      label: c.label,
      ts: c.ts,
      key_hex: hex(key0),
      nonce_hex: hex(c.nonce),
      aad_hex: hex(aadBytes),
      auth_b64: toBase64(envelope),
    };
  }),
);

// ── Path safety ───────────────────────────────────────────────────────────
const pathInputs = [
  "",
  "a/b/c",
  "a//b/./c",
  "../etc",
  "a/../b",
  "/abs",
  "a\\b",
  "foo\u0000bar",
  "./",
  "a/b/",
  ".",
];
const paths = pathInputs.map((input) => ({ input, result: checkRelativePath(input) }));

// ── Self-check: TS must round-trip its own vectors ──────────────────────────
for (const v of aead) {
  const body = Buffer.from(v.body_hex, "hex");
  const raw = body.subarray(PNG_HEADER.length);
  const nonce = raw.subarray(0, 12);
  const ctwt = raw.subarray(12);
  const ct = ctwt.subarray(0, ctwt.length - 16);
  const tag = ctwt.subarray(ctwt.length - 16);
  const out = decrypt(
    Buffer.from(v.key_hex, "hex"),
    Buffer.from(nonce),
    Buffer.from(ct),
    Buffer.from(tag),
    Buffer.from(v.aad_hex, "hex"),
  );
  if (out.toString("hex") !== v.plaintext_hex) {
    throw new Error(`TS round-trip failed for ${v.label}`);
  }
}

// The list vectors must be what the production builder produces, and the Node side
// must accept them: sealed with a pinned nonce above, built with a random one here.
for (const v of listAuth) {
  const raw = Buffer.from(v.auth_b64, "base64");
  if (raw.length !== 28) throw new Error(`list auth ${v.label}: expected nonce||tag (28 bytes)`);
  const out = decrypt(
    Buffer.from(v.key_hex, "hex"),
    raw.subarray(0, 12),
    Buffer.alloc(0),
    raw.subarray(12),
    Buffer.from(v.aad_hex, "hex"),
  );
  if (out.length !== 0) throw new Error(`list auth ${v.label}: message must be empty`);
}
{
  const live = await buildListAuth(clientKey, 1700000000000);
  const raw = Buffer.from(live["X-Sync-Auth"], "base64");
  const out = decrypt(
    key0,
    raw.subarray(0, 12),
    Buffer.alloc(0),
    raw.subarray(12),
    Buffer.from(buildAad("list", 1700000000000)),
  );
  if (raw.length !== 28 || out.length !== 0) throw new Error("buildListAuth shape drifted");
}

const outDoc = { png_header_hex: hex(PNG_HEADER), hkdf, aad, aead, list_auth: listAuth, paths };
await Bun.write(
  "rust/sync-service/testdata/wire-vectors.json",
  `${JSON.stringify(outDoc, null, 2)}\n`,
);
console.log("✓ TS self-check passed; wrote rust/sync-service/testdata/wire-vectors.json");
console.log(
  `  ${hkdf.length} hkdf, ${aad.length} aad, ${aead.length} aead, ${listAuth.length} list-auth, ${paths.length} path vectors`,
);

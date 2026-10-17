import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";
import {
  PNG_HEADER,
  buildChunkAad,
  deriveSyncKeyBytes,
  parseUploadBody,
  sealEnvelope,
  wrapPng,
} from "@sdw/core/sync";
import { decrypt, deriveKey, encrypt } from "../src/sync-crypto.ts";

/**
 * The browser client (WebCrypto, the code the SPA runs) against the server's Node
 * implementation, and both against the golden vectors the Rust PoC also
 * reproduces. Before this file existed, nothing proved the two ends agreed.
 */

const VECTORS = JSON.parse(
  readFileSync(
    path.resolve(import.meta.dir, "../../../rust/sync-service/testdata/wire-vectors.json"),
    "utf-8",
  ),
) as {
  png_header_hex: string;
  hkdf: Array<{ passphrase: string; key_hex: string }>;
  aead: Array<{
    label: string;
    ts: number;
    file_id: string;
    chunk_idx: number;
    key_hex: string;
    aad_hex: string;
    nonce_hex: string;
    plaintext_hex: string;
    ciphertext_with_tag_hex: string;
    body_hex: string;
  }>;
};

const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
const unhex = (s: string) => new Uint8Array(Buffer.from(s, "hex"));

describe("key derivation: WebCrypto client == Node server == golden vectors", () => {
  for (const v of VECTORS.hkdf) {
    it(`agrees for ${JSON.stringify(v.passphrase)}`, async () => {
      expect(hex(await deriveSyncKeyBytes(v.passphrase))).toBe(v.key_hex);
      expect(hex(deriveKey(v.passphrase))).toBe(v.key_hex);
    });
  }
});

describe("envelope: the client seals, the server opens", () => {
  it("round-trips across the two implementations, for several sizes", async () => {
    const passphrase = "interop passphrase";
    const key = await crypto.subtle.importKey(
      "raw",
      await deriveSyncKeyBytes(passphrase),
      "AES-GCM",
      false,
      ["encrypt"],
    );
    for (const size of [0, 1, 15, 16, 17, 4096, 1_000_003]) {
      const plaintext = new Uint8Array(size).map((_, i) => (i * 31) % 251);
      const aad = buildChunkAad(1_700_000_000_000, "file", 3);
      const body = wrapPng(await sealEnvelope(key, plaintext, aad));
      const parsed = parseUploadBody(body);
      expect(parsed).not.toBeNull();
      const opened = decrypt(
        deriveKey(passphrase),
        parsed!.nonce,
        parsed!.ciphertext,
        parsed!.tag,
        aad,
      );
      expect(hex(opened)).toBe(hex(plaintext));
    }
  });

  it("round-trips the other way: the server seals, the client opens", async () => {
    const passphrase = "interop passphrase";
    const aad = buildChunkAad(1, "f", 0);
    const { nonce, ciphertext, tag } = encrypt(
      deriveKey(passphrase),
      Buffer.from("from the server"),
      aad,
    );
    const key = await crypto.subtle.importKey(
      "raw",
      await deriveSyncKeyBytes(passphrase),
      "AES-GCM",
      false,
      ["decrypt"],
    );
    // WebCrypto expects the tag appended to the ciphertext
    const sealed = new Uint8Array(ciphertext.length + tag.length);
    sealed.set(ciphertext, 0);
    sealed.set(tag, ciphertext.length);
    const plain = await crypto.subtle.decrypt(
      {
        name: "AES-GCM",
        iv: nonce as Uint8Array<ArrayBuffer>,
        additionalData: aad as Uint8Array<ArrayBuffer>,
        tagLength: 128,
      },
      key,
      sealed,
    );
    expect(new TextDecoder().decode(plain)).toBe("from the server");
  });
});

describe("golden vectors: the shared core framing reproduces the recorded bytes", () => {
  it("PNG header", () => {
    expect(hex(PNG_HEADER)).toBe(VECTORS.png_header_hex);
  });

  for (const v of VECTORS.aead) {
    it(`[${v.label}] wrapPng(encodeEnvelope(...)) equals the recorded body`, async () => {
      const key = await crypto.subtle.importKey("raw", unhex(v.key_hex), "AES-GCM", false, [
        "encrypt",
      ]);
      // pinned nonce => deterministic ciphertext, comparable to the recorded one
      const envelope = await sealEnvelope(
        key,
        unhex(v.plaintext_hex),
        unhex(v.aad_hex),
        unhex(v.nonce_hex),
      );
      expect(hex(envelope.subarray(12))).toBe(v.ciphertext_with_tag_hex);
      expect(hex(wrapPng(envelope))).toBe(v.body_hex);
    });

    it(`[${v.label}] the server opens the recorded body`, () => {
      const parsed = parseUploadBody(unhex(v.body_hex));
      expect(parsed).not.toBeNull();
      const opened = decrypt(
        unhex(v.key_hex),
        parsed!.nonce,
        parsed!.ciphertext,
        parsed!.tag,
        unhex(v.aad_hex),
      );
      expect(hex(opened)).toBe(v.plaintext_hex);
    });
  }
});

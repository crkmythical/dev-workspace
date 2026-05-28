import { describe, it, expect } from "bun:test";
import { deriveKey, encrypt, decrypt, keyFingerprint } from "../src/crypto.ts";
import { buildAad } from "../../core/src/aad.ts";

describe("server/crypto", () => {
  it("deriveKey returns 32 bytes deterministically", () => {
    const k1 = deriveKey("test");
    const k2 = deriveKey("test");
    expect(k1.length).toBe(32);
    expect(k1.equals(k2)).toBe(true);
  });

  it("encrypt/decrypt round-trip", () => {
    const key = deriveKey("pass");
    const plaintext = Buffer.from("hello world");
    const aad = buildAad("test", 123);
    const { nonce, ciphertext, tag } = encrypt(key, plaintext, aad);
    const decrypted = decrypt(key, nonce, ciphertext, tag, aad);
    expect(decrypted.equals(plaintext)).toBe(true);
  });

  it("tampered ciphertext throws", () => {
    const key = deriveKey("pass");
    const aad = buildAad("test", 123);
    const { nonce, ciphertext, tag } = encrypt(key, Buffer.from("secret"), aad);
    ciphertext[0] ^= 0xff;
    expect(() => decrypt(key, nonce, ciphertext, tag, aad)).toThrow();
  });

  it("wrong AAD throws", () => {
    const key = deriveKey("pass");
    const aad1 = buildAad("op1", 1);
    const aad2 = buildAad("op2", 2);
    const { nonce, ciphertext, tag } = encrypt(key, Buffer.from("data"), aad1);
    expect(() => decrypt(key, nonce, ciphertext, tag, aad2)).toThrow();
  });

  it("keyFingerprint returns 32 hex chars", () => {
    const key = deriveKey("test");
    const fp = keyFingerprint(key);
    expect(fp.length).toBe(32);
    expect(/^[0-9a-f]{32}$/.test(fp)).toBe(true);
  });
});

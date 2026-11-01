import { describe, expect, test } from "bun:test";
import { deriveSyncKeyBytes } from "../src/sync/client.ts";
import { toHex } from "../src/sync/envelope.ts";

// Pin the browser sync-key derivation to the SAME frozen golden vector the Rust side checks in
// rust/sync-service/testdata/wire-vectors.json (sync_kdf[0]) via rust/.../tests/parity.rs. The fast
// Rust CI pins the server; this pins the LIVE TS client, so a drift in the salt or iteration count
// (packages/core/src/sync/protocol.ts) fails here in unit CI instead of only in the heavy smoke
// suite. If the wire format is changed deliberately, update both sides and this vector together.
describe("sync key derivation (PBKDF2-HMAC-SHA256), pinned cross-language", () => {
  test("matches the frozen wire vector for a known passphrase", async () => {
    const key = await deriveSyncKeyBytes("correct horse battery staple");
    expect(toHex(key)).toBe("608557e1f4f768ee5b6caa81988db2ad8169f992528c8d9ffe8ae09ab071bdc2");
  });
});

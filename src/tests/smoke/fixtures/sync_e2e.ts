// The REAL sync protocol client (core) against the REAL server in the container, through Caddy.
// Copied into the container (keeping its path relative to the repository root, because of the
// imports) and run there by checks/30-sync.sh:
//   bun run tests/smoke/fixtures/sync_e2e.ts [base] [passphrase]
import {
  SYNC_API,
  buildChunkRequest,
  buildFinalizeRequest,
  buildListAuth,
  deriveSyncKey,
} from "../../../packages/core/src/sync/index.ts";

const base = process.argv[2] ?? "http://127.0.0.1:8080";
const passphrase = process.argv[3] ?? "smoke-master-pass-123";
const key = await deriveSyncKey(passphrase);
const wrong = await deriveSyncKey("not-the-passphrase");

// Everything except /auth/* and /sync/api/destruct sits behind the ingress auth gateway (Caddy
// forward_auth -> /auth/verify). Log in once and send the session cookie on every request, as a
// browser would after the login page. The sync protocol's own proof (list auth) is checked on top.
const loginReply = await fetch(`${base}/auth/login`, {
  method: "POST",
  redirect: "manual",
  headers: { "content-type": "application/x-www-form-urlencoded" },
  body: new URLSearchParams({ password: passphrase, next: "/" }),
});
const gateCookie = /sdw_session=[^;]+/.exec(loginReply.headers.get("set-cookie") ?? "")?.[0] ?? "";
if (!gateCookie) {
  console.log(`FAIL  the auth gateway accepts the master password   [status ${loginReply.status}]`);
  process.exit(1);
}
const gated = (init: RequestInit = {}): RequestInit => ({
  ...init,
  headers: { ...((init.headers as Record<string, string> | undefined) ?? {}), cookie: gateCookie },
});

let failed = 0;
function check(name: string, ok: boolean, detail = "") {
  if (!ok) failed++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? `   [${detail}]` : ""}`);
}

const text = (s: string) => new TextEncoder().encode(s);
async function sha256(data: Uint8Array) {
  const d = await crypto.subtle.digest("SHA-256", data as Uint8Array<ArrayBuffer>);
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
const post = (p: { headers: Record<string, string>; body: Uint8Array }) =>
  fetch(base + SYNC_API.upload, gated({ method: "POST", headers: p.headers, body: p.body }));

type Reply = { status: number; body: Record<string, unknown> };
const json = async (r: Response): Promise<Reply> => ({
  status: r.status,
  body: (await r.json().catch(() => ({}))) as Record<string, unknown>,
});

// /sync/api/doctor reveals sync/vault state, so it is behind the gate like the rest of the API.
const doctor = await json(await fetch(base + SYNC_API.doctor, gated()));
check(
  "doctor reports ready, with an available sync key",
  doctor.body.state === "ready" && doctor.body.sync_key_available === true,
  JSON.stringify(doctor.body),
);

// --- a complete two-chunk upload ---
const fileId = crypto.randomUUID();
const whole = text("hello from the smoke run\n");
const c1 = await json(
  await post(await buildChunkRequest(key, Date.now(), fileId, 0, text("hello from the "))),
);
const c2 = await json(
  await post(await buildChunkRequest(key, Date.now(), fileId, 1, text("smoke run\n"))),
);
check(
  "chunk 0 accepted",
  c1.status === 200 && c1.body.status === "chunk-received",
  JSON.stringify(c1),
);
check(
  "chunk 1 accepted",
  c2.status === 200 && c2.body.status === "chunk-received",
  JSON.stringify(c2),
);
const fin = await json(
  await post(
    await buildFinalizeRequest(key, Date.now(), {
      fileId,
      filePath: "smoke-hello.txt",
      contentSha256: await sha256(whole),
      totalChunks: 2,
    }),
  ),
);
check(
  "finalize commits the file",
  fin.status === 200 && fin.body.status === "upload-complete",
  JSON.stringify(fin),
);

// --- listing needs proof of the key ---
const listed = await json(
  await fetch(base + SYNC_API.list, gated({ headers: await buildListAuth(key, Date.now()) })),
);
const files = (listed.body.files ?? []) as { name: string }[];
const names = files.map((f) => f.name);
check(
  "authenticated listing shows the uploaded file",
  listed.status === 200 && names.includes("smoke-hello.txt"),
  JSON.stringify(names),
);
const anon = await json(await fetch(base + SYNC_API.list, gated()));
check("listing without an authenticator is 401", anon.status === 401, JSON.stringify(anon));
const badList = await json(
  await fetch(base + SYNC_API.list, gated({ headers: await buildListAuth(wrong, Date.now()) })),
);
check("listing with the wrong key is 401", badList.status === 401, JSON.stringify(badList));
const content = await fetch(`${base}${SYNC_API.list}?path=smoke-hello.txt`, gated());
check(
  "the old plaintext ?path= download is gone (404)",
  content.status === 404,
  `status ${content.status}`,
);

// --- replay and forgery ---
const replayable = await buildChunkRequest(key, Date.now(), crypto.randomUUID(), 0, text("x"));
const first = await post(replayable);
const second = await json(await post(replayable));
check("an envelope is accepted once", first.status === 200);
check(
  "the same envelope again is a replay (400)",
  second.status === 400 && second.body.error === "replay-detected",
  JSON.stringify(second),
);
const forged = await json(
  await post(await buildChunkRequest(wrong, Date.now(), crypto.randomUUID(), 0, text("x"))),
);
check(
  "an envelope sealed with another key fails authentication (400)",
  forged.status === 400 && forged.body.error === "aead-failure",
  JSON.stringify(forged),
);
const stale = await json(
  await post(
    await buildChunkRequest(key, Date.now() - 10 * 60_000, crypto.randomUUID(), 0, text("x")),
  ),
);
check("a 10 minute old timestamp is refused (400)", stale.status === 400, JSON.stringify(stale));

// --- hostile paths never reach the filesystem ---
const evilId = crypto.randomUUID();
await post(await buildChunkRequest(key, Date.now(), evilId, 0, text("x")));
const evil = await json(
  await post(
    await buildFinalizeRequest(key, Date.now(), {
      fileId: evilId,
      filePath: "../../etc/cron.d/evil",
      contentSha256: "",
      totalChunks: 1,
    }),
  ),
);
check(
  "a traversing target path is rejected (400)",
  evil.status === 400 && evil.body.error === "bad-file-path",
  JSON.stringify(evil),
);
const evilFile = await json(
  await post(await buildChunkRequest(key, Date.now(), "../escape", 0, text("x"))),
);
check(
  "a traversing file id is rejected (400)",
  evilFile.status === 400 && evilFile.body.error === "bad-file-id",
  JSON.stringify(evilFile),
);

console.log(failed === 0 ? "\nALL SYNC CHECKS PASSED" : `\n${failed} SYNC CHECK(S) FAILED`);
process.exit(failed === 0 ? 0 : 1);

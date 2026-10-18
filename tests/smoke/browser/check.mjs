// Browser checks of the smoke test: a real Chrome drives the pages the way a person would.
//
//   SMOKE_BASE      where the workspace is published   (default http://127.0.0.1:18090)
//   SMOKE_PASSWORD  its master password                 (default: the smoke one)
//   SMOKE_STACK     desktop stacks in the image: kasmvnc | selkies | both (default kasmvnc)
//   CHROME_PATH     the browser binary (default: looked for in the usual places)
//   SHOTS_DIR       where screenshots go (default ./shots)
//
// Prints PASS / FAIL / SKIP lines like the shell checks do. Exit status 0 unless something failed.
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { chromium } from "playwright-core";

const base = process.env.SMOKE_BASE ?? "http://127.0.0.1:18090";
const password = process.env.SMOKE_PASSWORD ?? "smoke-master-pass-123";
const stack = process.env.SMOKE_STACK ?? "kasmvnc";
const shotsDir = process.env.SHOTS_DIR ?? path.join(import.meta.dirname, "shots");

const CANDIDATES = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/google-chrome-stable",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
];
const executablePath = process.env.CHROME_PATH ?? CANDIDATES.find((p) => existsSync(p));

let failed = 0;
const pass = (name) => console.log(`  PASS  ${name}`);
const skip = (name) => console.log(`  SKIP  ${name}`);
const fail = (name, detail = "") => {
  failed++;
  console.log(`  FAIL  ${name}${detail ? `\n        ${String(detail).slice(0, 400)}` : ""}`);
};
const check = (name, ok, detail) => (ok ? pass(name) : fail(name, detail));
const step = (title) => console.log(`\n== ${title}`);

if (!executablePath) {
  skip("no Chrome or Chromium found (set CHROME_PATH)");
  process.exit(0);
}
mkdirSync(shotsDir, { recursive: true });

const browser = await chromium.launch({ executablePath, headless: true });

/**
 * What a screenshot shows, judged in a scratch page: how many distinct colours it has on a grid,
 * what share of the grid is not black, and a signature that changes when the picture does.
 */
async function analyse(png) {
  const page = await browser.newPage();
  try {
    return await page.evaluate(async (b64) => {
      const img = new Image();
      img.src = `data:image/png;base64,${b64}`;
      await img.decode();
      const canvas = document.createElement("canvas");
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext("2d");
      ctx.drawImage(img, 0, 0);
      const seen = new Set();
      let dark = 0;
      let total = 0;
      let signature = 0;
      for (let y = 0; y < canvas.height; y += 10) {
        for (let x = 0; x < canvas.width; x += 10) {
          const [r, g, b] = ctx.getImageData(x, y, 1, 1).data;
          seen.add(`${r},${g},${b}`);
          total++;
          if (r + g + b < 24) dark++;
          signature = (signature * 31 + r * 3 + g * 5 + b * 7 + x + y) % 1000000007;
        }
      }
      return { colors: seen.size, lit: 1 - dark / total, signature };
    }, png.toString("base64"));
  } finally {
    await page.close();
  }
}

/**
 * Screenshots of `page` until one shows a desktop (many colours, not black) or the time is up.
 * A freshly started session takes a while to paint: XFCE first, then the stream catches up.
 */
async function waitForDesktop(page, file, seconds) {
  const deadline = Date.now() + seconds * 1000;
  let seen;
  for (;;) {
    const shot = await page.screenshot({ path: path.join(shotsDir, file) });
    seen = await analyse(shot);
    if ((seen.colors >= 12 && seen.lit > 0.02) || Date.now() > deadline) return seen;
    await page.waitForTimeout(3000);
  }
}
const looksLikeDesktop = (seen) => seen.colors >= 12 && seen.lit > 0.02;

async function section(title, body) {
  step(title);
  try {
    await body();
  } catch (err) {
    fail(`${title}: unexpected error`, err?.stack ?? err);
  }
}

// --- the sync page ------------------------------------------------------------------------------
await section("sync page: a real browser uploads a file", async () => {
  const context = await browser.newContext({ viewport: { width: 1000, height: 700 } });
  const page = await context.newPage();
  // A real folder picker needs a person at a native dialog. The page gets the browser's private
  // file system as "the chosen folder", which is a genuine FileSystemDirectoryHandle.
  await page.addInitScript(() => {
    window.showDirectoryPicker = async () => navigator.storage.getDirectory();
  });
  page.on("dialog", (dialog) => dialog.accept(password));
  const logText = () => page.locator("#log").innerText();

  await page.goto(`${base}/sync/`);
  await page
    .locator("#st")
    .filter({ hasText: "Set passphrase to start" })
    .waitFor({ timeout: 15000 });
  pass("the page loads and asks for the passphrase");

  await page.getByRole("button", { name: "Set passphrase" }).click();
  await page.locator("#log").filter({ hasText: "Key set" }).waitFor({ timeout: 10000 });
  pass("the key is derived in the browser (WebCrypto, secure context)");

  await page.getByRole("button", { name: "Choose folder" }).click();
  await page.locator("#log").filter({ hasText: "Folder selected" }).waitFor({ timeout: 10000 });
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const handle = await root.getFileHandle("browser-upload.txt", { create: true });
    const writable = await handle.createWritable();
    await writable.write("uploaded by a real browser\n");
    await writable.close();
  });
  try {
    await page
      .locator("#log")
      .filter({ hasText: "↑ browser-upload.txt" })
      .waitFor({ timeout: 30000 });
    pass("the file is noticed, sealed and uploaded without a click");
  } catch {
    fail("the file is noticed, sealed and uploaded without a click", await logText());
  }
  check("the page did not report an error", !/Error|✗/.test(await logText()), await logText());

  await page.reload();
  try {
    await page.locator("#log").filter({ hasText: "Key restored" }).waitFor({ timeout: 15000 });
    pass("after a reload the key comes back from IndexedDB");
  } catch {
    fail("after a reload the key comes back from IndexedDB", await logText());
  }
  await context.close();
});

// --- the emergency panel -------------------------------------------------------------------------
await section("emergency page: a wrong passphrase gets nowhere", async () => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${base}/sync/#emergency`);
  const button = page.locator("#ep-btn");
  await button.waitFor({ timeout: 15000 });
  check("Execute is disabled at first", await button.isDisabled());
  await page.locator("#ep-pass").fill("not-the-destruct-passphrase");
  await page.locator("#ep-confirm").fill("destroy");
  check(
    "and still disabled when the confirmation is not exactly DESTROY",
    await button.isDisabled(),
  );
  await page.locator("#ep-confirm").fill("DESTROY");
  check("enabled once both are filled in", await button.isEnabled());
  await button.click();
  const status = page.locator("#ep-status");
  try {
    await status.filter({ hasText: "Incorrect passphrase" }).waitFor({ timeout: 15000 });
    pass("a wrong passphrase is answered with 'incorrect passphrase or not configured'");
  } catch {
    fail(
      "a wrong passphrase is answered with 'incorrect passphrase or not configured'",
      await status.innerText(),
    );
  }
  check(
    "the inputs stay locked afterwards (no repeated triggers)",
    await page.locator("#ep-pass").isDisabled(),
  );
  await context.close();
});

// --- the desktops --------------------------------------------------------------------------------
if (stack === "kasmvnc" || stack === "both") {
  await section("desktop /vnc/ (KasmVNC): the picture and the input", async () => {
    const anonymous = await browser.newContext();
    const refused = await anonymous.request.get(`${base}/vnc/`);
    check(
      "without credentials the page is refused",
      refused.status() === 401,
      `status ${refused.status()}`,
    );
    await anonymous.close();

    const context = await browser.newContext({
      httpCredentials: { username: "user", password },
      viewport: { width: 1280, height: 720 },
    });
    const page = await context.newPage();
    const sockets = [];
    page.on("websocket", (ws) => sockets.push(ws.url()));
    const response = await page.goto(`${base}/vnc/`);
    check(
      "with the master password the client loads",
      response?.status() === 200 && (await page.title()) === "KasmVNC",
    );
    try {
      await page.waitForFunction(() => document.body.innerText.includes("Connected"), null, {
        timeout: 60000,
      });
      pass("the client connects over the WebSocket and says so");
    } catch {
      fail(
        "the client connects over the WebSocket and says so",
        (await page.locator("body").innerText()).slice(0, 300),
      );
    }
    check(
      "through /websockify",
      sockets.some((u) => u.endsWith("/websockify")),
      sockets.join(" "),
    );
    const seen = await waitForDesktop(page, "vnc-desktop.png", 90);
    check(
      `the remote screen shows a desktop, not a blank (${seen.colors} colours, ${(seen.lit * 100).toFixed(0)}% lit)`,
      looksLikeDesktop(seen),
      JSON.stringify(seen),
    );

    // The panel's Applications menu opens on a click: the remote session reacts to input.
    await page.mouse.click(50, 12);
    let after = seen;
    for (let i = 0; i < 6 && after.signature === seen.signature; i++) {
      await page.waitForTimeout(2500);
      after = await analyse(await page.screenshot({ path: path.join(shotsDir, "vnc-menu.png") }));
    }
    check(
      "a click on the panel changes the remote screen (input reaches the session)",
      after.signature !== seen.signature,
      JSON.stringify({ seen, after }),
    );
    await context.close();
  });
}

if (stack === "selkies" || stack === "both") {
  await section("desktop /desktop/ (Selkies): the picture", async () => {
    const context = await browser.newContext({
      httpCredentials: { username: "user", password },
      viewport: { width: 1280, height: 720 },
    });
    const page = await context.newPage();
    const sockets = [];
    page.on("websocket", (ws) => sockets.push(ws.url()));
    const response = await page.goto(`${base}/desktop/`);
    check("the client loads", response?.status() === 200, `status ${response?.status()}`);
    const seen = await waitForDesktop(page, "selkies-desktop.png", 90);
    check(`a WebSocket to the stream is open (${sockets.length})`, sockets.length > 0);
    check(
      `the stream shows a desktop (${seen.colors} colours, ${(seen.lit * 100).toFixed(0)}% lit)`,
      looksLikeDesktop(seen),
      JSON.stringify(seen),
    );
    await context.close();
  });
}

await browser.close();
writeFileSync(path.join(shotsDir, ".keep"), "");
process.exit(failed === 0 ? 0 : 1);

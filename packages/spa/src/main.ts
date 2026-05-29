/**
 * Sync Page SPA — E2E encrypted file sync via File System Access API.
 * Uses WebCrypto AES-256-GCM with HKDF-derived key from vault passphrase.
 * Files are PNG-camouflaged for DLP evasion.
 */
import { PNG_HEADER, CHUNK_SIZE, SIZE_LIMIT, NONCE_LENGTH, TAG_LENGTH } from "../../core/src/constants.ts";

let syncKey: CryptoKey | null = null;
let dirHandle: FileSystemDirectoryHandle | null = null;
let pollTimer: ReturnType<typeof setInterval> | null = null;
let baseline: Map<string, { mtime: number; size: number }> = new Map();

const app = document.getElementById("app")!;
const HKDF_SALT = "sync-key-v1";
const HKDF_INFO = "aead-key";

// --- Crypto (WebCrypto) ---
async function deriveKey(passphrase: string): Promise<CryptoKey> {
  const enc = new TextEncoder();
  const keyMaterial = await crypto.subtle.importKey("raw", enc.encode(passphrase), "HKDF", false, ["deriveKey"]);
  return crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: enc.encode(HKDF_SALT), info: enc.encode(HKDF_INFO) },
    keyMaterial, { name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"],
  );
}

async function encryptPayload(key: CryptoKey, plaintext: ArrayBuffer, aadStr: string): Promise<Uint8Array> {
  const nonce = crypto.getRandomValues(new Uint8Array(NONCE_LENGTH));
  const aad = new TextEncoder().encode(aadStr);
  const combined = new Uint8Array(await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce, additionalData: aad, tagLength: TAG_LENGTH * 8 }, key, plaintext,
  ));
  // Format: nonce(12) + ciphertext + tag(16) — tag is appended by WebCrypto
  const result = new Uint8Array(NONCE_LENGTH + combined.length);
  result.set(nonce, 0);
  result.set(combined, NONCE_LENGTH);
  return result;
}

function wrapPng(data: Uint8Array): Uint8Array {
  const result = new Uint8Array(PNG_HEADER.length + data.length);
  result.set(PNG_HEADER, 0);
  result.set(data, PNG_HEADER.length);
  return result;
}

// --- Init ---
async function init() {
  if (!("showDirectoryPicker" in window)) {
    app.innerHTML = `<h1 style="color:#89b4fa">Sync</h1><p>Use Chrome/Edge for file sync.</p>`;
    return;
  }

  // Multi-tab lock
  const bc = new BroadcastChannel("sdw-sync-lock");
  let isActive = true;
  bc.postMessage({ type: "claim" });
  bc.onmessage = (e) => {
    if (e.data?.type === "claim" && isActive) bc.postMessage({ type: "active" });
    if (e.data?.type === "active") {
      app.innerHTML = `<h1 style="color:#89b4fa">Sync</h1><p>Another tab is already syncing. Close it first.</p>`;
      isActive = false;
    }
  };
  await new Promise(r => setTimeout(r, 300)); // Wait for responses
  if (!isActive) return;

  render();
  try {
    const stored = await loadHandle();
    if (stored) {
      const perm = await stored.queryPermission({ mode: "readwrite" });
      if (perm === "granted") { dirHandle = stored; log("Folder restored"); }
    }
    const storedKey = await loadKey();
    if (storedKey) { syncKey = storedKey; log("Key restored"); }
    if (syncKey && dirHandle) { startPolling(); setStatus("idle"); }
    else if (!syncKey) setStatus("Set passphrase to start");
    else setStatus("Choose folder to start");
  } catch {}
}

function render() {
  app.innerHTML = `
    <h1 style="color:#89b4fa">Sync</h1>
    <div id="status" style="display:inline-flex;align-items:center;gap:8px;padding:6px 12px;border-radius:6px;background:#2a2a3c;border:1px solid #45475a;margin-bottom:16px">
      <span id="dot" style="width:8px;height:8px;border-radius:50%;background:#f9e2af"></span>
      <span id="st">Initializing...</span>
    </div>
    <div style="display:flex;gap:8px;margin-bottom:24px;flex-wrap:wrap">
      <button id="bf">Choose folder</button>
      <button id="bk">Set passphrase</button>
      <button id="bl">Clear key</button>
      <button id="bd">Diagnostics</button>
    </div>
    <div style="font-size:12px;text-transform:uppercase;color:#a6adc8;margin-bottom:8px">Activity</div>
    <div id="log"></div>
  `;
  const btnStyle = "padding:8px 16px;border:1px solid #45475a;border-radius:6px;background:#2a2a3c;color:#cdd6f4;cursor:pointer";
  document.querySelectorAll("button").forEach(b => b.setAttribute("style", btnStyle));
  document.getElementById("bf")!.onclick = chooseFolder;
  document.getElementById("bk")!.onclick = setPassphrase;
  document.getElementById("bl")!.onclick = async () => {
    syncKey = null;
    const db = await openDB();
    const tx = db.transaction("kv", "readwrite");
    tx.objectStore("kv").delete("key");
    tx.oncomplete = () => db.close();
    if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
    setStatus("Key cleared — set passphrase to resume");
    log("Key cleared from browser");
  };
  document.getElementById("bd")!.onclick = () => window.open("/sync/api/doctor", "_blank");
}

async function chooseFolder() {
  try {
    const h = await (window as any).showDirectoryPicker({ mode: "readwrite" });
    dirHandle = h;
    await saveHandle(h);
    log("Folder selected");
    if (syncKey) { startPolling(); setStatus("idle"); }
  } catch {}
}

async function setPassphrase() {
  const pass = prompt("Vault passphrase (same as unlock-vault):"); if (!pass) return;
  try {
    syncKey = await deriveKey(pass);
    await saveKey(syncKey);
    log("Key set");
    if (dirHandle) { startPolling(); setStatus("idle"); }
  } catch { log("Key derivation failed"); }
}

function startPolling() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(poll, document.hidden ? 10000 : 2000);
  document.addEventListener("visibilitychange", () => {
    if (pollTimer) clearInterval(pollTimer);
    if (syncKey && dirHandle) pollTimer = setInterval(poll, document.hidden ? 10000 : 2000);
  });
}

async function poll() {
  if (!dirHandle || !syncKey) return;
  try {
    const current = await scanDir(dirHandle);

    // If no baseline (first run or cache cleared), fetch remote state to avoid full re-upload
    if (baseline.size === 0) {
      try {
        const resp = await fetch("/sync/api/download");
        if (resp.ok) {
          const { files } = await resp.json() as any;
          for (const f of files || []) {
            baseline.set(f.name, { mtime: 0, size: f.size }); // Mark as already synced
          }
        }
      } catch {}
    }

    const changes: { path: string; handle: FileSystemFileHandle }[] = [];
    for (const [p, entry] of current) {
      const prev = baseline.get(p);
      if (!prev || prev.mtime !== entry.mtime || prev.size !== entry.size) {
        changes.push({ path: p, handle: entry.handle });
      }
    }
    if (changes.length === 0) return;
    setStatus("syncing");
    for (const c of changes) await uploadFile(c.path, c.handle);
    baseline = new Map([...current].map(([k, v]) => [k, { mtime: v.mtime, size: v.size }]));
    setStatus("idle");
  } catch (e: any) { log(`Error: ${e.message}`); setStatus("error"); }
}

async function uploadFile(filePath: string, fileHandle: FileSystemFileHandle) {
  if (!syncKey) return;
  const file = await fileHandle.getFile();
  if (file.size > SIZE_LIMIT) { log(`Skip (>500MB): ${filePath}`); return; }

  const fileId = crypto.randomUUID();
  const totalChunks = Math.ceil(file.size / CHUNK_SIZE) || 1;
  const ts = Date.now();

  // Upload chunks
  for (let i = 0; i < totalChunks; i++) {
    const chunk = await file.slice(i * CHUNK_SIZE, (i + 1) * CHUNK_SIZE).arrayBuffer();
    const aadStr = `put-chunk|${ts}|${fileId}|${i}`;
    const encrypted = await encryptPayload(syncKey, chunk, aadStr);
    const camouflaged = wrapPng(encrypted);

    const resp = await fetch("/sync/api/upload", {
      method: "POST",
      headers: {
        "Content-Type": "image/png",
        "X-Sync-Op": "put-chunk",
        "X-Sync-Ts": String(ts),
        "X-Sync-FileId": fileId,
        "X-Sync-ChunkIdx": String(i),
      },
      body: camouflaged,
    });
    if (!resp.ok) { log(`✗ chunk ${i} failed`); return; }
  }

  // Finalize
  const contentHash = await hashFile(file);
  const finalizePayload = JSON.stringify({ file_id: fileId, file_path: filePath, content_sha256: contentHash, total_chunks: totalChunks });
  const finalizeAad = `put-finalize|${ts}|${fileId}|${totalChunks}`;
  const encrypted = await encryptPayload(syncKey, new TextEncoder().encode(finalizePayload).buffer, finalizeAad);
  const camouflaged = wrapPng(encrypted);

  const resp = await fetch("/sync/api/upload", {
    method: "POST",
    headers: {
      "Content-Type": "image/png",
      "X-Sync-Op": "put-finalize",
      "X-Sync-Ts": String(ts),
      "X-Sync-FileId": fileId,
      "X-Sync-ChunkIdx": String(totalChunks),
    },
    body: camouflaged,
  });
  if (resp.ok) log(`↑ ${filePath}`);
  else log(`✗ finalize failed: ${filePath}`);
}

async function hashFile(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const hash = await crypto.subtle.digest("SHA-256", buf);
  return Array.from(new Uint8Array(hash)).map(b => b.toString(16).padStart(2, "0")).join("");
}

async function scanDir(handle: FileSystemDirectoryHandle, prefix = ""): Promise<Map<string, { mtime: number; size: number; handle: FileSystemFileHandle }>> {
  const entries = new Map<string, { mtime: number; size: number; handle: FileSystemFileHandle }>();
  for await (const [name, child] of (handle as any).entries()) {
    if (name.startsWith(".")) continue;
    const p = prefix ? `${prefix}/${name}` : name;
    if (child.kind === "file") {
      const f: File = await child.getFile();
      entries.set(p, { mtime: f.lastModified, size: f.size, handle: child });
    } else if (child.kind === "directory") {
      const sub = await scanDir(child, p);
      for (const [k, v] of sub) entries.set(k, v);
    }
  }
  return entries;
}

function setStatus(s: string) {
  const dot = document.getElementById("dot");
  const st = document.getElementById("st");
  if (!dot || !st) return;
  const colors: Record<string, string> = { idle: "#a6e3a1", syncing: "#89b4fa", error: "#f38ba8" };
  dot.style.background = colors[s] || "#f9e2af";
  st.textContent = s;
}

function log(msg: string) {
  const el = document.getElementById("log");
  if (!el) return;
  const e = document.createElement("div");
  e.style.cssText = "padding:4px 0;border-bottom:1px solid #45475a;font-size:12px;color:#a6adc8";
  e.textContent = `${new Date().toLocaleTimeString()} ${msg}`;
  el.prepend(e);
  if (el.children.length > 50) el.lastChild?.remove();
}

// IDB persistence
const DB = "sdw-sync";
function openDB(): Promise<IDBDatabase> {
  return new Promise((res, rej) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => { if (!req.result.objectStoreNames.contains("kv")) req.result.createObjectStore("kv"); };
    req.onsuccess = () => res(req.result);
    req.onerror = () => rej(req.error);
  });
}
async function saveHandle(h: FileSystemDirectoryHandle) { const db = await openDB(); const tx = db.transaction("kv", "readwrite"); tx.objectStore("kv").put(h, "handle"); return new Promise<void>(r => { tx.oncomplete = () => { db.close(); r(); }; }); }
async function loadHandle(): Promise<FileSystemDirectoryHandle | undefined> { const db = await openDB(); return new Promise(r => { const tx = db.transaction("kv", "readonly"); const req = tx.objectStore("kv").get("handle"); req.onsuccess = () => { db.close(); r(req.result); }; req.onerror = () => { db.close(); r(undefined); }; }); }
async function saveKey(k: CryptoKey) { const db = await openDB(); const tx = db.transaction("kv", "readwrite"); tx.objectStore("kv").put(k, "key"); return new Promise<void>(r => { tx.oncomplete = () => { db.close(); r(); }; }); }
async function loadKey(): Promise<CryptoKey | undefined> { const db = await openDB(); return new Promise(r => { const tx = db.transaction("kv", "readonly"); const req = tx.objectStore("kv").get("key"); req.onsuccess = () => { db.close(); r(req.result); }; req.onerror = () => { db.close(); r(undefined); }; }); }

document.addEventListener("DOMContentLoaded", init);

// --- Emergency Destruct Panel (Task 5.1 / 5.2) ---
// Activated via URL hash: /sync/#emergency

function renderEmergencyPanel() {
  app.innerHTML = `
    <div style="max-width:480px;margin:40px auto;padding:24px;border:2px solid #f38ba8;border-radius:12px;background:#1e1e2e">
      <div style="background:#f38ba8;color:#1e1e2e;padding:12px 16px;border-radius:8px;font-weight:bold;margin-bottom:20px">
        ⚠ EMERGENCY DESTRUCT — This will permanently destroy all workspace data
      </div>
      <div style="margin-bottom:12px">
        <input id="ep-pass" type="password" placeholder="Destruction passphrase"
          style="width:100%;padding:10px;border:1px solid #45475a;border-radius:6px;background:#2a2a3c;color:#cdd6f4;box-sizing:border-box" />
      </div>
      <div style="margin-bottom:16px">
        <input id="ep-confirm" type="text" placeholder='Type "DESTROY" to confirm'
          style="width:100%;padding:10px;border:1px solid #45475a;border-radius:6px;background:#2a2a3c;color:#cdd6f4;box-sizing:border-box" />
      </div>
      <button id="ep-btn" disabled
        style="width:100%;padding:12px;border:none;border-radius:6px;background:#585b70;color:#bac2de;font-weight:bold;cursor:not-allowed">
        Execute
      </button>
      <div id="ep-status" style="margin-top:16px;text-align:center;font-size:14px"></div>
    </div>
  `;

  const passInput = document.getElementById("ep-pass") as HTMLInputElement;
  const confirmInput = document.getElementById("ep-confirm") as HTMLInputElement;
  const btn = document.getElementById("ep-btn") as HTMLButtonElement;
  const status = document.getElementById("ep-status")!;

  function updateBtn() {
    const ready = passInput.value.length > 0 && confirmInput.value === "DESTROY";
    btn.disabled = !ready;
    btn.style.background = ready ? "#f38ba8" : "#585b70";
    btn.style.color = ready ? "#1e1e2e" : "#bac2de";
    btn.style.cursor = ready ? "pointer" : "not-allowed";
  }

  passInput.addEventListener("input", updateBtn);
  confirmInput.addEventListener("input", updateBtn);

  btn.addEventListener("click", async () => {
    passInput.disabled = true;
    confirmInput.disabled = true;
    btn.disabled = true;
    status.textContent = "Executing...";
    status.style.color = "#f9e2af";

    try {
      const resp = await fetch("/sync/api/destruct", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ passphrase: passInput.value }),
      });

      if (resp.status === 200) {
        status.textContent = "✓ Environment destroyed";
        status.style.color = "#a6e3a1";
      } else if (resp.status === 429) {
        status.textContent = "✗ Rate limited. Wait 60 seconds.";
        status.style.color = "#fab387";
      } else {
        status.textContent = "✗ Incorrect passphrase or not configured";
        status.style.color = "#fab387";
      }
    } catch {
      status.textContent = "✗ Connection failed";
      status.style.color = "#f38ba8";
    }
    // Do not re-enable inputs (prevent repeated triggers).
  });
}

function checkEmergencyHash() {
  if (location.hash === "#emergency") {
    renderEmergencyPanel();
  }
}

window.addEventListener("hashchange", checkEmergencyHash);
// Also check on initial load (after DOMContentLoaded fires init)
setTimeout(checkEmergencyHash, 350);

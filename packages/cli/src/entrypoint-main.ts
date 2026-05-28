#!/usr/bin/env bun
import { CLASH_HTTP_PORT, CLASH_SOCKS_PORT } from "@sdw/core/constants";
/**
 * entrypoint-main — Container startup logic (called from entrypoint.sh)
 *
 * 1. Cleanup stale FUSE mount
 * 2. Fetch Clash subscription
 * 3. Configure git globals
 * 4. Tune inotify
 * 5. Start Clash and wait for readiness
 * 6. Probe egress
 * 7. exec supervisord
 */
import { $ } from "bun";

// 1. Cleanup stale FUSE mount
await $`fusermount -uz /workspace`.quiet().nothrow();
await $`fusermount -uz /pentest/rootfs`.quiet().nothrow();

// 2. Fetch subscription (must bypass proxy since Clash isn't ready yet)
const subUrl = process.env.CLASH_SUBSCRIPTION_URL;
if (subUrl) {
  const fetchProc = Bun.spawn(
    [
      "curl",
      "-fsSL",
      "--noproxy",
      "*",
      "--max-time",
      "30",
      subUrl,
      "-o",
      "/etc/clash/config.yaml.new",
    ],
    {
      stdout: "pipe",
      stderr: "pipe",
      env: {
        ...process.env,
        http_proxy: "",
        https_proxy: "",
        all_proxy: "",
        HTTP_PROXY: "",
        HTTPS_PROXY: "",
        ALL_PROXY: "",
      },
    },
  );
  const fetchExit = await fetchProc.exited;
  if (fetchExit === 0) {
    await $`mv /etc/clash/config.yaml.new /etc/clash/config.yaml`.quiet();
    console.log("Clash subscription updated.");
  } else {
    console.warn("WARNING: Subscription fetch failed, using cached config.");
    await $`rm -f /etc/clash/config.yaml.new`.quiet();
  }
}

// 3. Git globals (optional — skip if not configured)
if (process.env.GIT_USER_NAME) {
  await $`git config --global user.name "${process.env.GIT_USER_NAME}"`.quiet();
}
if (process.env.GIT_USER_EMAIL) {
  await $`git config --global user.email "${process.env.GIT_USER_EMAIL}"`.quiet();
}
await $`git config --global http.proxy http://127.0.0.1:7890`.quiet();
await $`git config --global https.proxy http://127.0.0.1:7890`.quiet();

// 4. inotify
await $`sysctl -w fs.inotify.max_user_watches=524288`.quiet().nothrow();

// 5. Start Clash
console.log("Starting Clash...");
Bun.spawn(["/usr/bin/clash", "-d", "/etc/clash"], { stdout: "ignore", stderr: "ignore" });

// Wait for readiness
let ready = false;
for (let i = 0; i < 30; i++) {
  const probe = await $`nc -z 127.0.0.1 ${CLASH_HTTP_PORT}`.quiet().nothrow();
  if (probe.exitCode === 0) {
    ready = true;
    break;
  }
  await Bun.sleep(1000);
}
console.log(ready ? "Clash ready." : "WARNING: Clash not ready after 30s.");

// 6b. Auto-select proxy node (switch from DIRECT to first available proxy)
if (ready) {
  try {
    const resp = await fetch("http://127.0.0.1:9090/proxies");
    if (resp.ok) {
      const data = (await resp.json()) as any;
      const selector = data.proxies?.["🚀 节点选择"];
      if (selector && selector.now === "🎯 全球直连" && selector.all?.length > 1) {
        // Try to find a url-test/fallback group first (auto-selects fastest)
        const autoGroup = selector.all.find(
          (n: string) => n.includes("自动") || n.includes("auto") || n.includes("url-test"),
        );
        // Otherwise pick first real proxy node
        const proxyNode =
          autoGroup ||
          selector.all.find(
            (n: string) =>
              !n.includes("直连") &&
              !n.includes("DIRECT") &&
              !n.includes("拦截") &&
              !n.includes("REJECT") &&
              !n.includes("净化") &&
              !n.includes("漏网"),
          );
        if (proxyNode) {
          await fetch(
            "http://127.0.0.1:9090/proxies/%F0%9F%9A%80%20%E8%8A%82%E7%82%B9%E9%80%89%E6%8B%A9",
            {
              method: "PUT",
              body: JSON.stringify({ name: proxyNode }),
            },
          );
          console.log(`Proxy auto-selected: ${proxyNode}`);
        }
      }
    }
  } catch {}
}

// 7. Egress probe
let egress = false;
for (let i = 0; i < 3; i++) {
  const probe =
    await $`curl -fsSL --proxy socks5://127.0.0.1:${CLASH_SOCKS_PORT} --max-time 10 https://www.google.com -o /dev/null`
      .quiet()
      .nothrow();
  if (probe.exitCode === 0) {
    egress = true;
    break;
  }
  await Bun.sleep(2000);
}
console.log(egress ? "Egress confirmed." : "WARNING: Egress probe failed.");

// 7. exec supervisord
console.log("Starting supervisord...");
const proc = Bun.spawn(["/usr/bin/supervisord", "-c", "/etc/supervisor/supervisord.conf"], {
  stdout: "inherit",
  stderr: "inherit",
});
await proc.exited;

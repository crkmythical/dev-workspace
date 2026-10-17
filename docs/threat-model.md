# Threat Model

## Adversary Classes

| Adversary | Capability | Mitigations |
|-----------|-----------|-------------|
| **Network observer** | Inspect traffic on local/ISP network | Traffic that goes through Clash is encrypted to the proxy node (routing is best effort, see Egress below); the tunnel uses TLS |
| **Host compromise** | Read host filesystem, Docker volumes | Vault content is encrypted at rest (gocryptfs). The passphrase is NOT memory-only (see Non-Guarantees): whoever can read the container's filesystem, environment or logs can unlock it |
| **Container escape** | Break out of Docker isolation | Not privileged, but `CAP_SYS_ADMIN` (FUSE) and `seccomp:unconfined` (the desktop's bwrap sandbox) weaken isolation. Treat the container as a moderate boundary; use a dedicated VM for stronger separation |
| **GitHub breach** | Access vault repository contents | Repository contains only gocryptfs ciphertext; AES-256 |
| **Sync wire observer** | Intercept sync traffic | AEAD encryption + PNG camouflage; no plaintext metadata |
| **Physical access** | Access to powered-off machine | Vault locked on container stop; workspace content is not on disk in the clear. The credentials listed under Non-Guarantees are |
| **Cloudflare compromise** | Access tunnel traffic | End-to-end encryption between browser and container; CF sees TLS only |

## Trust Boundaries

1. **Passphrase → vault**: the passphrase (typed, or `PASSWORD`) is handed to gocryptfs; in the browser the sync key derived from it (HKDF) is a non-extractable CryptoKey. See Non-Guarantees for where the passphrase also lives
2. **Container → Host**: Docker isolation; single mapped port (localhost only)
3. **Container → Internet**: Proxy-aware traffic goes through Clash (`http_proxy` and friends point at it). This is best effort and is **not enforced** (see Egress)
4. **Sync SPA → Server**: AEAD-encrypted envelopes; server cannot read plaintext
5. **Vault → GitHub**: Only ciphertext pushed; git history contains no plaintext

## Residual Risks

| Risk | Likelihood | Impact | Acceptance |
|------|-----------|--------|------------|
| Memory dump while unlocked | Low | High | Mitigated: core dumps disabled, key zeroed on lock |
| gocryptfs vulnerability | Very Low | Critical | Monitor CVEs; passphrase rotation available |
| Clash subscription compromise | Medium | Medium | Traffic visible to proxy provider; use trusted provider |
| Docker Desktop vulnerability | Low | High | Keep updated; consider VM alternative |
| Browser IndexedDB key extraction | Low | Medium | Key marked non-extractable; cleared on rotation |

## Security Invariants

1. Workspace content is written only inside the FUSE mount. Each writer checks that the vault is mounted first (`desktop-start`, `backup-watcher`, `vault-sync`, notifications, the vault-gated steps of the boot plan, and sync uploads, which need the key that only exists while unlocked), so nothing lands in the container's own filesystem while locked. The check is per writer, not one central guard
2. The sync wire protocol reveals no file paths, names, or content in plaintext. Listing the shared directory needs proof of the sync key, and file content is never served back
3. Replay protection claims a nonce only after the envelope has authenticated, so forged requests cannot flush genuine nonces out of the bounded window
4. Container destruction leaves no recoverable traces on host (best effort; see Self-Destruct)

## Non-Guarantees

What this system does NOT promise, so nobody relies on it:

- **Egress is best effort, not enforced.** `http_proxy`, `https_proxy` and `all_proxy` point at Clash, so proxy-aware tools use it. Nothing blocks a process that ignores them, or a command run under `noproxy <cmd>`, from connecting directly: there is no firewall rule, and all processes run as root, so per-user rules would not help either. Enforcing egress needs a separate, mark-based firewall layer. DNS servers are set directly in `docker-compose.yml`.
- **Egress state is observed and reported, never assumed.** At start-up the proxy is brought up and probed. The result is `confirmed`, `degraded` (proxy up, probe failed) or `down` (proxy not listening). A non-confirmed result does **not** stop the workspace; it is shown in the start-up banner, in `doctor` (live), in `/var/run/egress-status`, and as `.notifications/egress-degraded.md` inside the vault once it is unlocked
- **The vault passphrase is also the master password**, and it is not memory-only: it comes from `PASSWORD` (so it is in the container environment and in `.env`), or is generated; it is printed in the start-up banner (so it reaches the container log) and kept in code-server's `config.yaml` on the container filesystem. With `SYNC_ENABLED` it is additionally stored inside the vault. Anyone who can read the container filesystem, its environment or its logs can unlock the vault
- **The Clash subscription URL** is a credential too; it lives in `.env` and in the `clash-config` volume, outside the vault

## Self-Destruct Attack Surface

### 机制

| 触发方式 | 认证 | 攻击面 |
|----------|------|--------|
| CLI (`self-destruct`) | 容器 exec 权限 (Docker socket) | 与 `docker exec` 等价 |
| API (`POST /sync/api/destruct`) | bcrypt-hashed destruct passphrase | 暴力破解 |
| SPA (`#emergency`) | 同 API | 同 API (前端仅为 UI) |

### 防御措施

| 威胁 | 缓解 |
|------|------|
| 暴力破解 destruct passphrase | 速率限制: 3 次/分钟/IP; bcrypt cost=12 (~250ms/verify) |
| 信息泄露 (passphrase 正确性) | 所有失败统一返回 404 (无区分) |
| 重放攻击 | 无状态 API，但 bcrypt 验证本身是幂等的；销毁后 inert 模式阻止重复执行 |
| 未授权远程触发 | Cloudflare Access OAuth 前置认证 + destruct passphrase 双因素 |
| 宿主机残留 | host-watcher 检测 marker → `destroy.sh --paranoid` (清理 Docker 缓存、shell 历史、DNS) |
| 密钥派生弱点 | HKDF-SHA256 with domain-separated salt ("destruct-key-v1") |
| 容器重启后恢复 | `completed` marker 持久化在 named volume → entrypoint 检测 → inert mode |

### 不可防御场景

- 攻击者已知 vault passphrase → 可推导 destruct passphrase (设计如此: 同一信任根)
- 物理访问 + 已开机 + vault 已解锁 → 可直接读取明文 (与无 self-destruct 时相同)
- Docker socket 暴露 → 可 exec 任意命令 (与无 self-destruct 时相同)

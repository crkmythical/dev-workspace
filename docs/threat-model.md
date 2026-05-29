# Threat Model

## Adversary Classes

| Adversary | Capability | Mitigations |
|-----------|-----------|-------------|
| **Network observer** | Inspect traffic on local/ISP network | Clash proxy encrypts all egress; tunnel uses TLS |
| **Host compromise** | Read host filesystem, Docker volumes | Vault encrypted at rest (gocryptfs); passphrase in memory only |
| **Container escape** | Break out of Docker isolation | Minimal attack surface; no privileged mode; CAP_SYS_ADMIN only for FUSE |
| **GitHub breach** | Access vault repository contents | Repository contains only gocryptfs ciphertext; AES-256 |
| **Sync wire observer** | Intercept sync traffic | AEAD encryption + PNG camouflage; no plaintext metadata |
| **Physical access** | Access to powered-off machine | Vault locked on container stop; no plaintext on disk |
| **Cloudflare compromise** | Access tunnel traffic | End-to-end encryption between browser and container; CF sees TLS only |

## Trust Boundaries

1. **Passphrase → Memory**: User enters passphrase; key derived via HKDF; held in process memory only
2. **Container → Host**: Docker isolation; single mapped port (localhost only)
3. **Container → Internet**: All traffic via Clash; fail-closed on Clash failure
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

1. No plaintext credential or workspace content exists on disk outside the FUSE mount
2. All network egress traverses Clash — no direct internet access from container
3. Vault passphrase is never written to disk, logs, or environment variables
4. Sync wire protocol reveals no file paths, names, or content in plaintext
5. Container destruction leaves no recoverable traces on host

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

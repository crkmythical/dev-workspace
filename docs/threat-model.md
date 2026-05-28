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

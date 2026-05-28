# Architecture

## Monorepo Structure

```
packages/
├── core/       Shared pure logic (types, constants, AAD, reconcile, ReplayWindow)
├── server/     Hono + Bun.serve() sync service (upload, download, doctor, WebSocket)
├── cli/        Container CLI tools (init-vault, unlock-vault, lock-vault, vault-sync, doctor, setup)
└── spa/        Browser file sync SPA (File System Access API + E2E encryption)
```

## Runtime Stack

- **Bun** — TypeScript runtime (replaces Node.js)
- **Hono** — HTTP framework (12,640 req/s)
- **Caddy** — Reverse proxy (code-server + sync-service on single port)
- **Clash/mihomo** — Encrypted egress proxy
- **gocryptfs** — Disk encryption (AES-256, 4KB blocks)
- **supervisord** — Process management
- **Zulu JDK 17** — Java runtime (via mise)

## Security Layers

```
L5: E2E encryption (WebCrypto AES-256-GCM)     ← Sync file transfer
L4: code-server password                        ← Access control
L3: Cloudflare Tunnel (TLS)                     ← Network encryption
L2: gocryptfs (AES-256)                         ← Disk encryption
L1: Clash proxy (VLESS+WS+TLS)                  ← Traffic obfuscation
```

## Data Flow

```
Browser → Cloudflare Tunnel → Caddy(:8080) → code-server(:8082) / sync-service(:8081)
Container egress → Clash(:7890) → Proxy nodes → Internet
Vault sync → git push (via Clash) → GitHub (encrypted ciphertext)
```

## Key Design Decisions

1. Single container (FUSE mount shared across all services)
2. Fail-closed egress (no Clash = no internet)
3. No plaintext on disk (gocryptfs FUSE only in memory)
4. PNG camouflage on sync wire (DLP evasion)
5. Replay protection (ring buffer nonce tracking)
6. LaunchAgent for tunnel (no sudo, auto-start on login)

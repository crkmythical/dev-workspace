# Migration Guide: Fresh Mac Mini Setup

## Prerequisites

- macOS 13+ (Ventura or later)
- Docker Desktop installed
- `git`, `curl` available (pre-installed on macOS)
- Cloudflare account with a domain
- GitHub account with SSH key configured
- Existing vault repo (for migration) or empty private repo (for fresh start)

## Step 1: Clone the Workspace Project

```bash
git clone <this-repo-url> ~/dev-workspace
cd ~/dev-workspace
```

## Step 2: Run Bootstrap

### Migration (existing vault)

```bash
./bootstrap.sh --vault-repo git@github.com:you/dev-vault.git
```

### Fresh Start (new vault)

```bash
./bootstrap.sh --init
```

## Step 3: Configure Environment

Bootstrap creates `.env` from `.env.example`. Fill in:

- `CLASH_SUBSCRIPTION_URL` — your proxy subscription
- `CLOUDFLARE_TUNNEL_TOKEN` — from tunnel creation
- `VAULT_GIT_REPO` — your private vault repo URL
- `GIT_USER_NAME` / `GIT_USER_EMAIL` — git identity

## Step 4: Docker & Power Setup

Bootstrap runs these automatically, but you can re-run:

```bash
bash scripts/setup-docker.sh       # Docker memory/disk + pmset
bash scripts/setup-cloudflared.sh  # Tunnel login/create/route
```

## Step 5: Start and Initialize

```bash
# Container starts automatically from bootstrap
# For migration:
docker exec -it dev-workspace unlock-vault

# For fresh start:
docker exec -it dev-workspace init-vault
# Then:
docker exec -it dev-workspace unlock-vault
```

## Step 6: Verify

```bash
docker exec -it dev-workspace doctor
```

All checks should show ✓. Access code-server at `https://workspace.example.com/`.

## Step 7: Configure Sync (Optional)

Open `https://workspace.example.com/sync/` in Chrome/Edge. Select your local sync folder and enter the vault passphrase when prompted.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Docker build fails | Check network; try `make image-import` with pre-built tar |
| Tunnel not reachable | Run `scripts/setup-cloudflared.sh` again; check DNS propagation |
| Vault unlock fails | Verify passphrase; check `/vault/cipher/gocryptfs.conf` exists |
| Clash not connecting | Check subscription URL; try updating: restart container |
| Sync page shows "vault locked" | Run `unlock-vault` in container terminal |

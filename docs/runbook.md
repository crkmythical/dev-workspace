# Operations Runbook

## First-Time Setup

```bash
cp .env.example .env          # Subscription URL pre-filled
docker compose build          # ~5 min first time
docker compose up -d
docker exec -it dev-workspace init-vault
docker exec -it dev-workspace unlock-vault
./scripts/setup-cloudflared.sh
```

## Daily Operations

```bash
docker exec -it dev-workspace unlock-vault   # Start of day
docker exec -it dev-workspace lock-vault     # End of day
docker exec -it dev-workspace doctor         # Health check
```

## Access

- Local: http://localhost:18080
- Remote: https://workspace.cicd.dpdns.org
- File sync: https://workspace.cicd.dpdns.org/sync/

## Password Rotation

```bash
docker exec -it dev-workspace lock-vault
docker exec -it dev-workspace change-password
docker exec -it dev-workspace unlock-vault  # Use new password
# Then: Sync Page → Clear key → Set passphrase (new password)
```

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Clash not connecting | Check subscription URL; restart container |
| Vault unlock fails | Verify passphrase; check gocryptfs.conf exists |
| Sync Page idle | Modify a file in the synced folder; check browser console |
| code-server 502 | Wait 10s after container start; check `docker logs` |

## Destroy

```bash
./destroy.sh --force              # Standard
./destroy.sh --force --paranoid   # Deep clean (history, DNS, creds)
```

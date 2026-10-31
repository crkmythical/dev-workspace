# 🔒 Vault Locked

Your workspace is encrypted. To unlock, open a terminal (Ctrl+`) and run:

```
unlock-vault
```

You will be prompted for your vault passphrase.
After unlocking, this file will be replaced by your workspace contents.

## Quick Reference

| Command | Description |
|---------|-------------|
| `unlock-vault` | Unlock encrypted workspace |
| `lock-vault` | Lock workspace (encrypt) |
| `doctor` | Health check all services |
| `init-vault` | First-time vault setup |

## Troubleshooting

- If `unlock-vault` fails: check your passphrase
- If proxy not working: run `doctor` to check Clash status
- Need Java? Already installed: `java -version` (Zulu 17)

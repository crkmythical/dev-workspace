# Requirements Document

## Introduction

在紧急情况下（如被要求交出设备、被搜查等），需要能够从容器内部触发数据销毁，而不依赖宿主机 shell 访问。由于容器内无法删除 Docker volume 或容器本身，策略是：容器内擦除所有密钥和敏感数据，使得即使 volume 被取证分析也无法恢复明文；同时可选地通知宿主机执行完整销毁。

核心安全论断：`gocryptfs.conf` 是唯一能够将密码转化为解密能力的主密钥文件。一旦被擦除，即使知道密码、拿到全部密文，也无法恢复数据。

## Glossary

- **Self_Destruct**: 容器内执行的紧急数据擦除命令，删除所有密钥文件和密文数据
- **Host_Watcher**: 宿主机上监听容器退出事件的守护脚本，验证自毁标记后执行完整的 `destroy.sh`
- **Key_Erasure**: 删除 `gocryptfs.conf`（主密钥文件），使得即使密码正确也无法解密
- **Data_Overwrite**: 对密文文件执行覆写操作，作为 defense-in-depth（SSD/overlay2 下不保证物理擦除，但密钥已丢失使其无意义）
- **Destruct_Key**: 从 vault passphrase 通过 HKDF 独立派生的销毁认证密钥，用于远程触发验证
- **Inert_Mode**: 自毁完成后容器进入的惰性状态——entrypoint 检测到标记文件后 `sleep infinity`，不再启动任何服务

## Requirements

### Requirement 1: 容器内自毁命令

**User Story:** 作为开发者，我希望在容器内执行一个命令即可擦除所有敏感数据，这样即使无法访问宿主机也能确保数据不可恢复。

#### Acceptance Criteria

1. THE CLI SHALL provide a `self-destruct` command executable inside the container via `docker exec` or code-server terminal
2. WHEN `self-destruct` is executed without `--force` flag, IT SHALL prompt for confirmation (type 'destroy' to confirm)
3. WHEN confirmed, THE command SHALL execute the following erasure sequence in strict order:
   a. Stop vault-sync cron immediately (`supervisorctl stop vault-sync-cron`) to prevent a concurrent sync cycle from pushing a "deleted" commit to GitHub
   b. Unmount gocryptfs FUSE at `/workspace` via `unmountVault()` (progressive escalation: normal → fuser kill → lazy)
   c. Unmount gocryptfs FUSE at `/pentest/rootfs` via `unmountVault()` (same escalation, handles chroot bind mounts)
   d. Shred and delete `/vault/cipher/gocryptfs.conf` — **Point of No Return**
   e. Shred and delete `/pentest/cipher/gocryptfs.conf` (if exists)
   f. (Unless `--skip-shred`) Recursively shred then delete all files in `/vault/cipher/` and `/pentest/cipher/`
   g. Shred and delete `/etc/clash/config.yaml` (proxy node information)
   h. Delete SSH keys and git credentials (`/root/.ssh`, `/root/.gitconfig`, `/root/.git-credentials`)
   i. Clear shell history (truncate `~/.bash_history`)
   j. Write `/var/run/self-destruct/completed` marker file (enables Inert_Mode on next container start)
   k. (destroyCore returns here — remaining steps are caller responsibility)
   l. CLI caller: `supervisorctl stop all` then send SIGTERM to PID 1
   m. Server caller: send SIGTERM to PID 1 directly (cannot stop its own process via supervisorctl)
4. THE command SHALL support `--force` flag to skip confirmation
5. THE command SHALL support `--skip-shred` flag for fast mode (only key erasure + deletion, no overwrite passes; completes < 3 seconds)
6. THE command SHALL support `--remote` flag to delete the GitHub vault repo BEFORE unmounting (reads token from `/workspace/.credentials/github-token` while vault is still accessible); if vault not mounted or token not available, skip with warning
7. AFTER execution, THE container SHALL be in Inert_Mode — any future restart results in `sleep infinity` rather than service startup
8. THE command SHALL print a progress indicator showing each phase's completion status to stdout only (never to persistent logs)
9. THE `destroyCore()` function SHALL NOT call `process.exit()` — it is a library function that returns after completing all phases; the caller (CLI or server) handles process termination separately

### Requirement 2: 密钥优先擦除策略

**User Story:** 作为开发者，我希望即使擦除过程被中断（断电、强制关机），只要主密钥文件被删除，数据就不可恢复。

#### Acceptance Criteria

1. THE erasure sequence SHALL prioritize `gocryptfs.conf` deletion as the FIRST destructive action after unmounting (step 3d)
2. THE `gocryptfs.conf` files SHALL be overwritten with 3 passes of random data then zeros before unlinking, using `shred -n3 -z` (< 1KB file, effectively instant)
3. AFTER `gocryptfs.conf` is deleted, THE remaining cipher files are cryptographically useless — this is the system's fundamental security guarantee regardless of filesystem behavior (overlay2 CoW, SSD wear leveling)
4. IF the process is interrupted at any point after step 3d, THE vault data SHALL remain unrecoverable
5. THE command SHALL print timestamp and explicit "POINT OF NO RETURN" message to stdout after key erasure succeeds

### Requirement 3: 宿主机联动销毁

**User Story:** 作为开发者，我希望容器自毁可以自动触发宿主机的完整清理，实现彻底销毁整个环境。

#### Acceptance Criteria

1. THE self-destruct process SHALL terminate the container by sending SIGTERM to PID 1 (tini) after writing the completed marker
2. THE Host_Watcher SHALL be a lightweight script monitoring Docker container die events (`docker events --filter event=die --filter container=dev-workspace`)
3. WHEN a die event is detected, THE Host_Watcher SHALL verify destruction by checking whether the `completed` marker exists in the `destruct-state` volume (via `docker run --rm -v <project>_destruct-state:/s alpine test -f /s/completed`)
4. IF the marker exists, THE Host_Watcher SHALL execute `destroy.sh --force --paranoid`
5. THE Host_Watcher SHALL be installable as a macOS LaunchAgent (user-level, no sudo required for installation)
6. THE Host_Watcher SHALL be optional — self-destruct is fully useful standalone without host-side automation
7. THE Host_Watcher setup SHALL be offered as an optional step during `bootstrap.sh` execution
8. THE mechanism uses Docker event API + volume inspection; no bind mounts or magic exit codes required (avoids false positives from normal container crashes)

### Requirement 4: 远程触发销毁

**User Story:** 作为开发者，我希望能从任何设备（手机、另一台电脑）远程触发容器自毁，以应对无法物理接触设备的紧急情况。

#### Acceptance Criteria

1. THE sync-service SHALL expose a `POST /sync/api/destruct` endpoint that accepts a destruction passphrase
2. THE destruct passphrase SHALL be derived independently from vault passphrase: `HKDF(vault_passphrase, salt="destruct-key-v1", info="self-destruct")` producing a 32-byte key, then Base64-encoded as the passphrase string
3. THE destruct passphrase's bcrypt hash SHALL be stored in a dedicated Docker named volume (`destruct-state`) at `/var/run/self-destruct/key-hash`, ensuring persistence across container restarts without exposing data to the host filesystem
4. THE hash SHALL be written during the post-unlock hook in `unlock-vault.ts` — derived from the passphrase that successfully unlocked the vault
5. WHEN vault is locked, remote destruct SHALL still work (hash persists independently of vault mount state in the named volume)
6. THE endpoint SHALL verify the provided passphrase against the stored bcrypt hash using `Bun.password.verify()`
7. WHEN verification succeeds, THE endpoint SHALL invoke the self-destruct core logic with `--force --skip-shred` (fast mode for remote — network may be cut at any moment)
8. THE endpoint SHALL be rate-limited: max 3 failed attempts per minute per source; on rate limit, respond 429
9. THE endpoint SHALL respond with 404 to any request where hash file doesn't exist OR passphrase is wrong (no information leakage about endpoint existence or passphrase validity)
10. THE Sync Page SPA SHALL include an emergency destruct panel accessible via URL fragment `#emergency` (bookmarkable, works on mobile/any device)
11. THE emergency panel SHALL require: (a) destruct passphrase input, (b) typing "DESTROY" to confirm, then send POST request
12. On success: display "Environment destroyed" final state; on 404: show generic "Failed" (no hint about wrong passphrase vs missing hash)

### Requirement 5: 安全性约束

**User Story:** 作为开发者，我希望自毁机制本身不会成为攻击向量或降低系统安全性。

#### Acceptance Criteria

1. THE `self-destruct` command SHALL NOT write any sensitive data to persistent storage during execution (progress output to stdout only, which goes to Docker log driver with size cap)
2. THE destruct key hash (bcrypt) stored in the volume is computationally irreversible — knowing the hash does not enable deriving the passphrase or the vault passphrase
3. THE primary security guarantee SHALL be key erasure (deleting `gocryptfs.conf`), NOT data overwrite — this is explicitly documented because `shred` is unreliable on overlay2/SSD, and we do not make false promises about physical erasure
4. THE remote destruct endpoint SHALL NOT be triggerable by Cloudflare Access-authenticated users who do not know the destruction passphrase (OAuth compromise ≠ destruct capability)
5. THE Inert_Mode marker (`/var/run/self-destruct/completed`) SHALL reside in the `destruct-state` volume so it survives container restarts and prevents restart loops
6. THE entrypoint.sh SHALL check for the Inert_Mode marker as its first action and `exec sleep infinity` if found, preventing any service from starting in a destroyed container
7. IF the `--remote` flag is used and GitHub repo deletion succeeds, THE system SHALL have eliminated both the local keys AND the remote ciphertext backup, providing maximum destruction depth

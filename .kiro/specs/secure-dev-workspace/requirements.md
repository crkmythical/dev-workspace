# Requirements Document

## Introduction

A secure, encrypted, containerized development workspace system deployed on a Mac Mini in a company server room. The system provides a browser-accessible code-server IDE via Cloudflare Tunnel, encrypts all data at rest with gocryptfs, and routes all outbound traffic (especially GitHub push/pull) through a Clash (mihomo) proxy to bypass network restrictions. The architecture ensures that company monitoring only sees Docker Desktop process activity and encrypted HTTPS connections.

## Glossary

- **Workspace_Container**: The primary Docker container running on the Mac Mini, housing Clash, gocryptfs, code-server, and the development toolchain
- **Clash_Proxy**: The mihomo-based proxy service inside the container that routes all outbound traffic through the user's VLESS+WS+TLS subscription
- **Vault**: The gocryptfs-encrypted Docker volume storing all sensitive workspace data; ciphertext at rest, plaintext only via FUSE mount
- **Code_Server**: The browser-based VS Code IDE (code-server) exposed to the user through Cloudflare Tunnel
- **Cloudflare_Tunnel**: The cloudflared daemon on the Mac Mini host that creates a secure tunnel from Cloudflare's edge to the container's code-server port
- **Cloudflare_Access**: Cloudflare's Zero Trust access control layer providing OAuth authentication before reaching code-server
- **FUSE_Mount**: The filesystem-in-userspace mount point (`/workspace`) where gocryptfs decrypts the Vault contents in memory
- **Subscription_URL**: The user's Clash-format proxy subscription URL providing proxy node configurations
- **Client_Container**: An optional minimal Docker container on the company laptop running only Clash to mask SNI/domain patterns when accessing code-server
- **Parallels_VM**: An optional Parallels Desktop virtual machine on the company laptop providing full OS-level isolation with Clash proxy, as an alternative to the Client_Container approach
- **Entrypoint_Script**: The container's entrypoint.sh that orchestrates service startup (Clash → wait for connectivity → optional vault unlock)
- **Unlock_Script**: The `unlock-vault.sh` script that prompts for the gocryptfs password and mounts the decrypted filesystem at /workspace
- **Lock_Script**: The `lock-vault.sh` script that unmounts the FUSE mount, leaving only ciphertext
- **Backup_Script**: The `backup-vault.sh` script that creates encrypted backups of the Vault to a specified location

## Requirements

### Requirement 1: Container Image Build

**User Story:** As a developer, I want a single Docker image containing all required tools and services, so that I can deploy the entire workspace with one container.

#### Acceptance Criteria

1. THE Workspace_Container SHALL include the following runtime components: bash, python3, java17 (JDK), jf CLI, git, node, and code-server
2. THE Workspace_Container SHALL use mise (https://mise.jdx.dev) as the runtime version manager for development tools (python, java, node, etc.), with versions declared in a `.mise.toml` configuration file
3. THE Workspace_Container SHALL include Clash_Proxy (mihomo) binary configured to start as a background service
4. THE Workspace_Container SHALL include gocryptfs binary and FUSE utilities for encrypted filesystem support
5. THE Workspace_Container SHALL be built from a Dockerfile that installs core infrastructure (mise, clash, gocryptfs, code-server, supervisord) with pinned versions AND pre-installs a default set of development tools via mise (ensuring offline usability), while allowing users to change tool versions at runtime via `.mise.toml` without rebuilding the image (requires network through Clash_Proxy)
6. THE Workspace_Container SHALL run with `--cap-add SYS_ADMIN --device /dev/fuse` to enable FUSE mount operations
7. THE Workspace_Container SHALL run processes as root user inside the container (acceptable for an isolated development environment) to allow unrestricted package installation and system configuration

### Requirement 2: Clash Proxy Configuration and Startup

**User Story:** As a developer, I want all container outbound traffic routed through my Clash proxy subscription, so that GitHub access and other restricted services work transparently.

#### Acceptance Criteria

1. WHEN the Workspace_Container starts, THE Entrypoint_Script SHALL launch Clash_Proxy with the configuration from `/etc/clash/config.yaml`
2. WHEN a Subscription_URL is provided via environment variable, THE Entrypoint_Script SHALL fetch the latest proxy configuration from the Subscription_URL on startup
3. IF the Subscription_URL fetch fails, THEN THE Clash_Proxy SHALL fall back to the cached local configuration file
4. THE Clash_Proxy SHALL expose a SOCKS5 proxy on port 7891 and an HTTP proxy on port 7890 within the container
5. THE Clash_Proxy SHALL enable DNS interception (`dns.enable: true` with `enhanced-mode: fake-ip` or `redir-host`) to prevent DNS query leakage to the company network
6. THE Clash_Proxy configuration SHALL include bypass rules for localhost, Docker internal networks (172.16.0.0/12), and link-local addresses to avoid breaking internal connectivity
7. THE Clash_Proxy configuration SHALL support user-customizable routing rules (via a rules file in the config volume) to allow direct connections for public resources (npm registry, Maven Central, etc.) while routing sensitive traffic (GitHub, private services) through proxy nodes
8. THE Workspace_Container SHALL set `http_proxy`, `https_proxy`, and `all_proxy` environment variables pointing to the Clash_Proxy ports
9. THE Workspace_Container SHALL configure git to use the Clash_Proxy for all remote operations via `git config --global http.proxy` and `git config --global https.proxy`
10. WHEN deploying for the first time without network access to the Subscription_URL, THE user SHALL provide an initial Clash configuration file (pre-downloaded) in the clash config volume

### Requirement 3: gocryptfs Vault Initialization

**User Story:** As a developer, I want to initialize an encrypted vault on first setup, so that all my workspace data is encrypted at rest from the beginning.

#### Acceptance Criteria

1. WHEN the `init-vault.sh` script is executed and no existing vault is detected, THE script SHALL create a new gocryptfs encrypted directory in the Docker volume mount point
2. WHEN initializing the vault, THE script SHALL prompt the user for a password (the password itself is never written to unencrypted disk; it MAY be cached inside the vault — i.e. only readable when the vault is unlocked — to allow the sync service to re-derive its AEAD key without prompting on every container restart)
3. THE Dockerfile SHALL create the FUSE mount point directory `/workspace` at build time (`RUN mkdir -p /workspace`)
4. IF an existing vault is detected at the target path, THEN THE script SHALL print an informational message and exit without modifying the existing vault
5. THE Vault ciphertext SHALL reside in a named Docker volume (persisted across container restarts)

### Requirement 4: Vault Unlock and Lock Lifecycle

**User Story:** As a developer, I want to manually unlock and lock my encrypted workspace, so that plaintext data only exists in memory while I am actively working.

#### Acceptance Criteria

1. WHEN the user executes `unlock-vault.sh` inside the container terminal, THE Unlock_Script SHALL prompt for the gocryptfs password and mount the decrypted filesystem at `/workspace`
2. IF the password is incorrect, THEN THE Unlock_Script SHALL print an error and exit with a non-zero code without mounting
3. IF the vault is already mounted, THEN THE Unlock_Script SHALL print an informational message and exit without re-mounting
4. WHEN the user executes `lock-vault.sh`, THE Lock_Script SHALL unmount the FUSE_Mount at `/workspace`
5. IF there are open file handles on `/workspace` when locking, THEN THE Lock_Script SHALL warn the user and offer a force-unmount option (`fusermount -uz`)
6. WHEN the container stops or is removed, THE FUSE_Mount SHALL automatically disappear, leaving only ciphertext in the Docker volume

### Requirement 5: code-server Configuration

**User Story:** As a developer, I want a browser-accessible VS Code IDE that relies on Cloudflare Access for authentication, so that I can securely code from any device without managing separate passwords.

#### Acceptance Criteria

1. THE Code_Server SHALL listen on container loopback `127.0.0.1:8082` with authentication disabled (`--auth none`); a Caddy reverse proxy on `0.0.0.0:8080` fronts both code-server (`/`) and the sync service (`/sync/*`); access restriction is enforced at the Docker port mapping level (`127.0.0.1:{TUNNEL_HOST_PORT}:8080`) and Cloudflare_Access layer
2. THE Code_Server SHALL be configured to use `/workspace` as the default workspace directory
3. THE Code_Server SHALL start automatically via supervisord after Clash_Proxy is running
4. THE Code_Server SHALL be accessible only through the Cloudflare_Tunnel (Caddy port 8080 mapped to host as `127.0.0.1:{TUNNEL_HOST_PORT}:8080`, never `0.0.0.0`)
5. THE Code_Server SHALL have auto-save enabled by default to prevent data loss from WebSocket disconnections or OAuth token expiry
6. THE Code_Server SHALL store its user data (extensions, settings, state) in `/workspace/.code-server/` so that extensions and preferences persist in the Vault and survive container rebuilds
7. THE Code_Server extensions marketplace access SHALL route through Clash_Proxy (inheriting the container's proxy environment variables)
8. WHEN the Vault is not yet unlocked, THE `/workspace` directory SHALL contain a `README-UNLOCK.md` file (created at image build time) instructing the user to run `unlock-vault` in the terminal

### Requirement 6: Cloudflare Tunnel Setup

**User Story:** As a developer, I want the Mac Mini host to run a Cloudflare Tunnel that forwards traffic to the container's code-server, so that I can access my workspace from any browser via a custom domain.

#### Acceptance Criteria

1. THE `setup-cloudflared.sh` script SHALL guide the user through authenticating cloudflared with their Cloudflare account
2. THE `setup-cloudflared.sh` script SHALL create a named tunnel and configure it to forward traffic to `localhost:{code-server-mapped-port}`
3. WHEN the Mac Mini boots, THE cloudflared daemon SHALL start automatically (via launchd or Docker)
4. THE Cloudflare_Tunnel SHALL be configured to require Cloudflare_Access OAuth authentication before forwarding requests to code-server
5. THE docker-compose.yml SHALL map the container's Caddy port (8080) to a host port bound to `127.0.0.1` only (`127.0.0.1:{TUNNEL_HOST_PORT}:8080`), accessible by cloudflared but not from any other network interface; Caddy demuxes traffic by path between code-server and sync-service

### Requirement 7: Docker Compose Orchestration (Server)

**User Story:** As a developer, I want a single `docker-compose up` command to start the entire workspace stack, so that deployment and recovery are simple.

#### Acceptance Criteria

1. THE server docker-compose.yml SHALL define the Workspace_Container service with all required volume mounts, port mappings, and capabilities
2. THE server docker-compose.yml SHALL define a named volume for the gocryptfs ciphertext data
3. THE server docker-compose.yml SHALL configure the container to restart automatically (`restart: unless-stopped`)
4. THE server docker-compose.yml SHALL load sensitive configuration from a `.env` file (Subscription_URL, tunnel token, etc.)
5. THE server docker-compose.yml SHALL set memory reservation (`mem_reservation: 8g`) rather than hard limits to avoid OOM kills while leaving headroom for the Docker VM system

### Requirement 8: Entrypoint Orchestration

**User Story:** As a developer, I want the container startup to be automated and resilient, so that services start in the correct order and recover from transient failures.

#### Acceptance Criteria

1. WHEN the container starts, THE Entrypoint_Script SHALL detect and clean up any stale FUSE mounts at `/workspace` (from previous non-graceful shutdowns) before proceeding
2. WHEN the container starts, THE Entrypoint_Script SHALL start Clash_Proxy first and wait until the proxy port is accepting connections
3. WHEN Clash_Proxy is ready, THE Entrypoint_Script SHALL verify outbound connectivity through the proxy (e.g., curl a test URL)
4. IF outbound connectivity verification fails after retries, THEN THE Entrypoint_Script SHALL log a warning and continue startup (allowing manual troubleshooting)
5. WHEN connectivity is confirmed, THE Entrypoint_Script SHALL start Code_Server
6. THE Entrypoint_Script SHALL NOT automatically unlock the Vault (requires manual user action for security)
7. THE Entrypoint_Script SHALL use a process supervisor (supervisord or s6-overlay) to manage Clash and code-server, ensuring individual process crashes trigger restart without killing the container
8. THE Workspace_Container SHALL configure Docker logging to `driver: "none"` or `driver: "local"` with strict size limits to prevent sensitive information from accumulating in host-accessible log files

### Requirement 9: Vault Persistence and Sync

**User Story:** As a developer, I want my encrypted vault automatically backed up to a remote Git repository, so that migration to a new machine is a single `git clone` away and I never lose data.

#### Acceptance Criteria

1. THE Vault ciphertext directory SHALL be initialized as a Git repository, separate from the workspace config repo (NOT a submodule — they have independent lifecycles and access policies)
2. A periodic sync process (cron or timer inside the container) SHALL automatically commit and push vault ciphertext changes to a private GitHub repository through Clash_Proxy at a configurable interval (default: every 30 minutes)
3. THE sync process SHALL use incremental git commits (only changed ciphertext blocks), minimizing transfer size
4. THE sync process SHALL run silently in the background; failures SHALL be logged but SHALL NOT interrupt the user's work
5. IF the sync process fails consecutively for more than a configurable threshold (default: 3 attempts), THE system SHALL display a warning message in the code-server terminal alerting the user to check proxy status
6. THE sync process SHALL check whether the Vault is currently mounted before attempting to commit/push; IF the Vault is not mounted, THE sync SHALL skip the current cycle silently
7. WHEN the vault contains large binary files that exceed GitHub's file size limits, THE system SHALL use Git LFS for those files
8. THE vault Git repository SHALL be configured with `.gitattributes` to handle binary ciphertext blocks efficiently and SHALL periodically run `git gc` to control repository size
9. THE vault Git repository SHALL exclude the `/workspace/shared/` directory and common ephemeral files (`.DS_Store`, `*.tmp`, `*.swp`) from automatic sync to GitHub, preventing transient file transfers from polluting the version history; users MAY explicitly include specific files in shared/ if desired
10. ON a new machine, THE migration workflow SHALL be: run bootstrap script → git clone vault repo → docker compose up → unlock vault → fully restored

### Requirement 10: Client Container (Optional)

**User Story:** As a developer, I want an optional lightweight proxy container on my company laptop, so that I can mask the SNI/domain pattern when accessing my workspace through the browser.

#### Acceptance Criteria

1. WHERE the client container is deployed, THE Client_Container SHALL run only Clash_Proxy with the user's subscription configuration
2. WHERE the client container is deployed, THE Client_Container SHALL expose a local SOCKS5/HTTP proxy for the host browser to use
3. THE client docker-compose.yml SHALL be independent from the server docker-compose.yml
4. THE Client_Container SHALL be minimal in resource usage (no development tools, no code-server)

### Requirement 11: Parallels Desktop VM (Optional Alternative)

**User Story:** As a developer, I want an optional Parallels Desktop VM on my company laptop as an alternative to the Docker client container, so that I have full OS-level isolation with Clash proxy and can also perform local development tasks in a completely sandboxed environment.

#### Acceptance Criteria

1. WHERE Parallels Desktop is available on the company laptop, THE Parallels_VM SHALL provide a lightweight Linux guest OS (e.g., Ubuntu 22.04) with Clash_Proxy installed and configured
2. THE Parallels_VM SHALL route all guest OS network traffic through Clash_Proxy, ensuring the host macOS and company monitoring cannot inspect traffic content
3. THE Parallels_VM SHALL expose a SOCKS5/HTTP proxy port to the host macOS (via Parallels shared networking), allowing the host browser to optionally route traffic through the VM's Clash
4. THE Parallels_VM MAY include a full development toolchain for local isolated development (as an alternative to remote code-server)
5. THE Parallels_VM disk image SHALL be stored on the company laptop; users SHOULD be aware that company endpoint agents may detect the VM process and scan the disk image file (though contents are within the VM filesystem)
6. THE Parallels_VM SHALL support snapshot and restore for quick environment recovery
7. THE Parallels_VM approach SHALL be documented as an alternative to the Client_Container, with a comparison of trade-offs (heavier resource usage, better isolation, local development capability vs. remote-only)

### Requirement 12: Environment Configuration

**User Story:** As a developer, I want all sensitive and environment-specific values externalized to a `.env` file, so that the configuration is portable and secrets are not committed to version control.

#### Acceptance Criteria

1. THE `.env.example` file SHALL document all required and optional environment variables with placeholder values
2. THE `.env.example` SHALL include: `CLASH_SUBSCRIPTION_URL`, `CLOUDFLARE_TUNNEL_TOKEN`, `TUNNEL_HOST_PORT`, `MEMORY_RESERVATION`, `VAULT_GIT_REPO`, `VAULT_SYNC_INTERVAL`, `TZ`, `GIT_USER_NAME`, and `GIT_USER_EMAIL`
3. THE `.env` file SHALL be listed in `.gitignore` to prevent accidental commit of secrets
4. IF a required environment variable is missing at container startup, THEN THE Entrypoint_Script SHALL print a clear error message identifying the missing variable and exit with a non-zero code

### Requirement 13: Security Model Enforcement

**User Story:** As a developer, I want the system to enforce a strict security model, so that company monitoring cannot access my plaintext workspace data.

#### Acceptance Criteria

1. THE Workspace_Container SHALL ensure that plaintext workspace data exists only in the FUSE_Mount (process memory), never written to unencrypted disk
2. THE Workspace_Container SHALL route all outbound internet traffic through Clash_Proxy (no direct connections to GitHub or other restricted services), excluding localhost and Docker internal networks
3. THE Vault ciphertext SHALL be stored inside Docker Desktop's Linux VM disk image (Docker.raw), adding an additional layer of indirection from the host filesystem
4. THE Code_Server SHALL not store authentication tokens or passwords on disk (relies entirely on Cloudflare_Access)
5. WHEN the container is stopped, THE system SHALL guarantee no plaintext workspace data remains (FUSE mount disappears with container process)

### Requirement 14: Docker Host Setup

**User Story:** As a developer, I want a setup script for the Mac Mini host, so that Docker Desktop and required host-level configurations are properly initialized.

#### Acceptance Criteria

1. THE `setup-docker.sh` script SHALL verify Docker Desktop is installed and running
2. THE `setup-docker.sh` script SHALL configure Docker Desktop VM memory allocation (recommend 12G for 16G host)
3. THE `setup-docker.sh` script SHALL verify that the Docker daemon is accessible and can pull images
4. THE `setup-docker.sh` script SHALL recommend disabling Docker Desktop automatic updates to prevent unexpected VM resets that could affect volumes
5. THE `setup-docker.sh` script SHALL verify Docker Desktop VM disk size is adequate (recommend ≥ 100G) and advise increasing if insufficient
6. IF Docker Desktop is not installed, THEN THE script SHALL print installation instructions and exit with a non-zero code

### Requirement 15: Git Proxy Integration

**User Story:** As a developer, I want git operations to transparently use the Clash proxy, so that I can push to and pull from GitHub without manual proxy configuration each time.

#### Acceptance Criteria

1. THE Workspace_Container SHALL configure `git config --global http.proxy socks5://127.0.0.1:7891` at container startup
2. THE Workspace_Container SHALL configure `git config --global https.proxy socks5://127.0.0.1:7891` at container startup
3. WHEN the user runs `git push` or `git pull` inside the container, THE git client SHALL route traffic through Clash_Proxy automatically
4. IF Clash_Proxy is not running when a git operation is attempted, THEN THE git client SHALL fail with a connection error (no fallback to direct connection)
5. THE Workspace_Container SHALL store SSH keys and/or GitHub tokens inside the Vault (accessible only after unlock); THE Unlock_Script SHALL execute a post-unlock hook that symlinks `/workspace/.credentials/ssh/` to `~/.ssh` and configures the git credential helper automatically; IF the credentials directory does not exist (first-time use), THE hook SHALL print setup instructions and continue without error

### Requirement 16: Container Environment Configuration

**User Story:** As a developer, I want the container to have correct timezone, locale, and shell configuration, so that git commits have correct timestamps and the development experience is comfortable.

#### Acceptance Criteria

1. THE Workspace_Container SHALL configure timezone via the `TZ` environment variable (default: `Asia/Shanghai`, configurable in `.env`)
2. THE Workspace_Container SHALL set locale to `en_US.UTF-8` to support international characters in filenames and code
3. THE Workspace_Container SHALL include a reasonable shell configuration (bash/zsh with prompt showing current directory) and tmux for persistent terminal sessions that survive browser disconnections
4. THE Workspace_Container SHALL configure git `user.name` and `user.email` from environment variables at startup

### Requirement 17: Bootstrap and Migration

**User Story:** As a developer, I want to set up or migrate my entire workspace on a new machine with a single command, so that recovery from hardware failure or machine replacement takes minutes, not hours.

#### Acceptance Criteria

1. A `bootstrap.sh` script SHALL be hosted in the workspace config GitHub repo and executable via `curl | bash` pattern
2. THE `bootstrap.sh` script SHALL: verify prerequisites (Docker, git) → clone the config repo → clone the vault repo → run `docker compose up` → prompt user to unlock vault
3. AFTER bootstrap completes and vault is unlocked, THE workspace SHALL be fully functional with all code, data, credentials, and tool configurations restored
4. THE bootstrap script SHALL accept `--vault-repo URL` for migration mode (clone existing vault repo) and `--init` for fresh-deployment mode (create empty vault directory; user runs `init-vault` after first compose-up); migration mode is the default
5. THE entire migration process SHALL require only: network access, the vault password, and GitHub authentication (SSH key or token provided interactively)

### Requirement 18: One-Command Destroy

**User Story:** As a developer, I want to completely destroy all workspace traces with a single command, so that in an emergency or upon leaving the company, no recoverable data remains on the machine.

#### Acceptance Criteria

1. A `destroy.sh` script SHALL stop and remove the Workspace_Container, all associated Docker volumes (vault, backup, clash-config), and the Docker image
2. THE `destroy.sh` script SHALL remove the cloudflared tunnel configuration and deauthorize the tunnel from Cloudflare (if cloudflared CLI is available)
3. THE `destroy.sh` script SHALL delete the project directory (docker-compose.yml, .env, scripts, etc.) from the host filesystem
4. THE `destroy.sh` script SHALL prompt for confirmation before executing (with a `--force` flag to skip confirmation for scripted/emergency use)
5. AFTER `destroy.sh` completes, THE host machine SHALL have no recoverable workspace data — no volumes, no images, no configuration files, no logs
6. THE `destroy.sh` script SHALL print a summary of what was removed and confirm complete destruction
7. THE `destroy.sh` script SHALL accept a `--remote` flag to also delete the vault Git repository from GitHub (requires GitHub token with delete permission)

### Requirement 19: Browser-Native Bidirectional Folder Sync

**User Story:** As a developer, I want a designated folder on my company laptop to be transparently synced with a folder in the remote container, so that file transfer feels like Parallels Desktop's shared folders without requiring any installed software, and works under DLP restrictions.

#### Acceptance Criteria

1. THE Workspace_Container SHALL expose a sync service at `https://{tunnel-domain}/sync` that serves a single-page web application (the "Sync Page")
2. THE Sync Page SHALL use the browser's File System Access API (`showDirectoryPicker`) to request user permission to read/write a designated local folder once; permission SHALL be persisted via IndexedDB and automatically restored on subsequent visits
3. WHEN the Sync Page is open in a browser tab, IT SHALL poll the local folder every 2 seconds (foreground) / 10 seconds (background) using File System Access API to detect file additions, modifications, and deletions; this polling is unavoidable because the API does not expose change events. Server-to-browser updates SHALL be pushed via WebSocket (no polling), per R19.16.
4. WHEN a local file change is detected, THE Sync Page SHALL upload the change to the container's `/workspace/shared/` directory via authenticated HTTPS using a temporary filename (`.uploading-{uuid}`) and atomically rename to final name only after upload completes (preventing partial-file visibility)
5. WHEN a container-side file change occurs, THE Sync Page SHALL receive a WebSocket notification and immediately apply the change (download/delete) to the local folder
6. THE sync service SHALL maintain bidirectional consistency: deletions, renames, and modifications propagate in both directions
7. THE Sync Page SHALL display a sync status indicator (idle, syncing, error, vault-locked, auth-expired) and a recent activity log
8. THE sync service SHALL support files up to a configurable size limit (default: 500MB per file); IF a file exceeds the limit, THE Sync Page SHALL reject it client-side with a clear error message before any upload attempt; chunked upload with progress reporting SHALL be used for files within the limit
9. WHEN there is a conflict (both sides modified the same file), THE Sync Page SHALL preserve both versions with a `.conflict-{timestamp}` suffix, display a prominent warning badge, and provide a UI to view differences and resolve conflicts
10. THE Sync Page SHALL be accessible only through Cloudflare_Access authentication, sharing the same security perimeter as code-server
11. THE Sync Page SHALL be embeddable as an iframe inside code-server for unified UX (single browser tab for both IDE and file sync)
12. WHEN the browser tab is reopened after being closed, THE Sync Page SHALL perform a full reconciliation scan comparing local and remote folder state, automatically syncing any changes missed during the closed period
13. WHEN the browser is not Chromium-based (Safari, Firefox) and File System Access API is unavailable, THE Sync Page SHALL display an informational message and fall back to a manual upload/download UI (drag-drop and click-to-download)
14. WHEN Cloudflare_Access OAuth token expires and sync requests return 401, THE Sync Page SHALL display a re-authentication prompt and resume sync automatically after the user re-authenticates
15. WHEN the Vault transitions from locked to unlocked state, THE Sync Page SHALL automatically resume sync without requiring the user to refresh the page (via WebSocket/SSE state notification)
16. THE sync protocol SHALL use WebSocket-driven event notifications for the server-to-browser direction (replacing what would otherwise be HTTP polling for remote changes); the browser-to-server direction still requires periodic local-folder scanning per R19.3 because the File System Access API has no native watch. This split minimizes request frequency and avoids traffic patterns that could trigger DLP behavioral analysis.
17. WHEN the user selects a local folder, THE Sync Page SHALL warn against selecting system directories or the home directory root, and SHALL recommend a dedicated subfolder (e.g., `~/Documents/dev-shared/`) to prevent accidental exposure of unrelated files
18. THE Sync Page SHALL detect if another active Sync Page session exists for the same local folder (via shared IndexedDB lock or BroadcastChannel) and refuse to start a second concurrent session, preventing double-upload and write conflicts
19. WHEN the browser tab is in the background (not focused), THE local folder polling interval SHALL increase (e.g., from 2s to 10s) to reduce CPU and battery usage
20. WHEN the user enters the vault passphrase in the Sync Page for the first time, THE Sync Page SHALL perform a verification handshake (encrypt a test vector and have the server validate it) before accepting the passphrase, providing immediate feedback if it is incorrect
21. THE Sync Page SHALL display distinct status messages for different states: vault-not-initialized (instructs to run init-vault), vault-locked (instructs to run unlock-vault), sync-key-not-set (instructs to enter passphrase in browser), syncing, idle, error
22. THE container's `/workspace/shared/` directory SHALL be intended for transient file transfers only; users SHOULD NOT place large code projects (with thousands of files like `node_modules`) in this directory to avoid exhausting Linux inotify watch limits; THE container SHALL configure `fs.inotify.max_user_watches` to a reasonable value (e.g., 524288) to handle moderate workloads

### Requirement 20: End-to-End Encryption Against TLS Inspection

**User Story:** As a developer, I want file transfers to be encrypted end-to-end above the TLS layer, so that even if the company performs TLS man-in-the-middle inspection (using a corporate root CA on my laptop), my file content remains private.

#### Acceptance Criteria

1. ALL file content transferred via the Sync Page SHALL be encrypted in the browser using WebCrypto API (AES-256-GCM) before being sent over the network
2. THE encryption key SHALL be derived from the gocryptfs vault password via HKDF with a context-specific salt (e.g., `"sync-key-v1"`), so the user manages a single passphrase but the sync key is cryptographically separate from the vault encryption key (compromise of one does not compromise the other)
3. THE derived key SHALL be cached in the browser's IndexedDB as a non-extractable CryptoKey so the user only enters the passphrase once per browser; THE salt used for key derivation SHALL be fixed and synchronized between client and server (not random per-session) so that the same passphrase always yields the same key
4. THE Mac Mini sync service SHALL hold the corresponding key (loaded from the unlocked Vault) and decrypt incoming uploads, encrypt outgoing downloads
5. THE encrypted payloads SHALL be wrapped in a multipart/form-data request with a benign-looking Content-Type (e.g., `image/png`) and filename pattern (e.g., `asset-{uuid}.png`) to avoid triggering DLP "unidentified binary blob" alerts
6. THE encrypted payloads SHALL include a magic number prefix matching common file types so DLP heuristic scanners classify them as normal media uploads
7. THE sync protocol SHALL use authenticated encryption (AEAD) so that any tampering by an intermediary (including a TLS MITM proxy) is detected and rejected by the receiving end
8. IF the receiving end fails to decrypt or authenticate a payload (corruption or tampering detected), THEN THE transfer SHALL be aborted and an error SHALL be logged
9. THE filename and folder structure SHALL also be encrypted in transit (not just file content), so that the MITM proxy cannot infer what files are being transferred
10. WHEN the Vault is locked, THE sync service SHALL refuse all sync operations (the encryption key is unavailable), and THE Sync Page SHALL display the appropriate state indicator (one of `vault-not-initialized`, `vault-locked`, `sync-key-not-set`, `auth-expired`, or `error`) per R19.21


### Requirement 21: Health Check and Diagnostics

**User Story:** As a developer, I want a single command to diagnose the state of all components, so that when something breaks I can quickly identify the failure point.

#### Acceptance Criteria

1. A `doctor.sh` script (also accessible as a command inside the container) SHALL perform a comprehensive health check covering: Clash_Proxy connectivity, gocryptfs vault mount status, code-server process status, cloudflared tunnel connectivity, vault sync last-success timestamp, sync service status, and disk usage of vault volume
2. THE doctor output SHALL use clear visual indicators (✓ / ✗ / ⚠) for each check and provide actionable suggestions for any failure
3. THE doctor script SHALL be runnable both from the container terminal and from the host (via `docker exec`)
4. THE Sync Page and code-server SHALL provide a "Run Diagnostics" UI button that displays the doctor output in a readable format

### Requirement 22: Vault Password Rotation

**User Story:** As a developer, I want to change my vault password if it is suspected to be compromised, without re-encrypting all data.

#### Acceptance Criteria

1. A `change-vault-password.sh` script SHALL invoke `gocryptfs -passwd` to rotate the password (which re-wraps the master key, not re-encrypting all blocks)
2. THE script SHALL prompt for the current password and the new password (with confirmation)
3. AFTER successful password change, THE script SHALL print clear instructions: "Update your password in the browser Sync Page on all devices, and update any password manager entries"
4. THE script SHALL NOT proceed if the vault is currently mounted (must be locked first to ensure consistency)

### Requirement 23: Host Power Management

**User Story:** As a developer, I want the Mac Mini to remain accessible 24/7, so that I can connect from anywhere at any time.

#### Acceptance Criteria

1. THE `setup-docker.sh` script SHALL configure macOS power management via `pmset` to: prevent system sleep when on AC power, disable display sleep enforcement, and ensure wake-on-network is enabled
2. THE setup script SHALL inform the user of these power management changes and the rationale
3. THE setup script SHALL set the Mac Mini to automatically restart after a power outage

### Requirement 24: Image Distribution and Build Resilience

**User Story:** As a developer, I want the Docker image build to succeed even on networks with restricted access to Docker Hub or other registries, so that initial deployment is not blocked.

#### Acceptance Criteria

1. THE Dockerfile SHALL use base images from registries that are commonly accessible in restricted networks (e.g., Docker Hub mirrors, specifying explicit registry URLs that work)
2. THE Dockerfile SHALL document alternative base image sources (e.g., aliyun mirror) for users behind restrictive networks
3. THE build process SHALL support being run with `docker build` after pre-pulling base images via the Clash_Proxy or directly when network allows
4. THE setup documentation SHALL include a fallback workflow: if `docker build` cannot reach the base image registry, the user MAY pre-build the image on a machine with unrestricted network and import it via `docker save` / `docker load` over GitHub release assets

### Requirement 25: Vault Hygiene and Sync Exclusions

**User Story:** As a developer, I want the vault Git repository to remain reasonably sized and version history to remain meaningful, so that sync stays fast and storage costs stay low.

#### Acceptance Criteria

1. WHEN the Vault is initialized (init-vault.sh), THE script SHALL place a comprehensive default `.gitignore` inside the vault repository excluding: `node_modules/`, `target/`, `build/`, `dist/`, `.cache/`, `**/*.log`, `.idea/`, `.vscode/`, `__pycache__/`, `.DS_Store`, and other common build/cache artifacts
2. THE doctor script (R21) SHALL warn if the vault size exceeds a threshold (e.g., 1GB) suggesting investigation of large files
3. A `vault-prune.sh` script SHALL be provided to identify and remove (with confirmation) accidentally committed large files from vault history using `git filter-repo` or equivalent

### Requirement 26: Clash Proxy High-Availability and Load Resilience

**User Story:** As a developer, I want the Clash proxy to handle high-concurrency scenarios (npm install, maven build) without dropping connections or degrading, and to automatically switch to healthy nodes when the current one fails.

#### Acceptance Criteria

1. THE Clash_Proxy configuration SHALL use a `url-test` or `fallback` proxy-group that automatically selects the lowest-latency node and fails over to alternatives when the active node becomes unreachable
2. THE Clash_Proxy configuration SHALL include periodic health-check probes (every 300s) for all proxy nodes, marking unhealthy nodes as unavailable
3. THE Clash_Proxy SHALL handle at least 200 concurrent connections without connection drops (sufficient for `npm install` / `maven build` parallel downloads)
4. THE doctor script SHALL report the currently active proxy node and its latency, and warn if all nodes are unhealthy

### Requirement 27: Long-Running Stability and Resource Management

**User Story:** As a developer, I want the system to remain stable over weeks of continuous operation without memory leaks, file handle exhaustion, or performance degradation.

#### Acceptance Criteria

1. THE sync-service SHALL implement a periodic graceful self-restart (configurable, default every 24h) to prevent memory fragmentation and resource accumulation; supervisord SHALL manage the restart transparently
2. THE sync-service ReplayWindow SHALL use a bounded data structure with O(1) eviction (e.g., ring buffer or LRU) rather than unbounded Map growth
3. THE doctor script SHALL report memory usage of key processes (clash, code-server, sync-service) and warn if any exceeds a configurable threshold (default: 2GB per process)
4. THE container SHALL configure file descriptor limits (`ulimit -n 65536`) to prevent exhaustion during high-concurrency operations
5. THE Workspace_Container SHALL include a periodic cleanup cron (every 6h) that removes: stale `.uploading-*` files older than 1h, empty notification files, and runs `sync-service` health probe

### Requirement 28: Chunked Upload Resilience (Resumable Transfers)

**User Story:** As a developer, I want large file uploads to be resumable after network interruptions, so that a 500MB file doesn't need to restart from zero if the connection drops mid-transfer.

#### Acceptance Criteria

1. THE sync-service SHALL track received chunks per `file_id` and expose a `GET /sync/api/upload-status?file_id=xxx` endpoint that returns the list of already-received chunk indices
2. THE Sync Page SHALL query upload-status before starting a new upload for a file that was previously interrupted, and skip already-received chunks (resume from the last successful chunk)
3. THE sync-service SHALL persist chunk receipt state in a lightweight file (`/workspace/shared/.uploads/{file_id}.json`) so that resumability survives sync-service restarts
4. THE Sync Page SHALL display upload progress (chunks completed / total) and estimated time remaining for large files
5. IF a `put-finalize` arrives but some chunks are missing, THE sync-service SHALL respond with a `missing-chunks` error listing the missing indices, allowing the client to retry only those chunks

# Requirements Document

## Introduction

This document specifies the requirements for adding **near-real-time backup** of the user's working files in the single-container secure dev workspace to an S3-compatible object store. The new chain uses **restic** for client-side encryption, deduplication, and snapshots. Backups are **event-driven**: a filesystem watcher on `/workspace` debounces change bursts and triggers `restic backup` once activity settles.

This feature is deliberately **additive and opt-in**. It **coexists** with the existing git-based `vault-sync` chain (cron → `git push` of the gocryptfs ciphertext to GitHub) rather than replacing it. restic is the fast/primary backup (seconds-to-minutes freshness); git-sync remains the slower secondary off-site copy. When the feature is not configured, the container behaves exactly as it does today.

The design honors the workspace's existing invariants:
- A single container where `/workspace` is a gocryptfs FUSE mount (plaintext view) and the ciphertext lives at `/vault/cipher`.
- The **FUSE-unmount safety invariant**: any process holding open file descriptors under `/workspace` must be stopped before `fusermount -u`, or the unmount fails with EBUSY. The backup watcher holds inotify watches and read fds on `/workspace`, so it must be stopped before unmount — the single most important new constraint.
- All container egress flows through the Clash proxy with fail-closed semantics (no Clash = no internet).
- Secrets are injected via `.env` (same pattern as `CLASH_SUBSCRIPTION_URL` and `PASSWORD`).
- Pure decision logic lives in `packages/core` (no IO); IO execution lives in `packages/cli`.

Two locked decisions frame these requirements and are encoded directly rather than re-evaluated:
1. **Coexistence**: restic-based realtime backup runs alongside `vault-sync`; both persistence chains operate independently.
2. **Plaintext source + restic-owned encryption**: restic backs up the plaintext `/workspace` mount directly and performs its own client-side encryption, so the S3 backend only ever stores restic-encrypted data. restic does NOT back up the gocryptfs ciphertext layer (dedup works poorly on already-encrypted data).
3. **Debounce window**: 10 seconds of filesystem quiet triggers exactly one `restic backup`; events during an in-flight backup accumulate for the next run.

Each requirement carries a **Traceability** line mapping it back to the locked decisions (LD1-LD3), the feature summary points, and the honored codebase invariants.

## Glossary

- **Workspace**: The single secure dev container that hosts the desktop, code-server, vault-sync, and the new backup chain.
- **Workspace_Mount**: The gocryptfs FUSE mount at `/workspace` that presents the plaintext view of the user's files. Backup source.
- **Vault_Cipher**: The gocryptfs ciphertext directory at `/vault/cipher` (docker volume `vault-data`). This is what `vault-sync` pushes to git; the restic chain does NOT read it.
- **Vault_Sync**: The existing periodic chain (`packages/cli/src/vault-sync.ts`, cron `image/etc/cron.d/vault-sync`) that does `git add/commit/push` of Vault_Cipher to GitHub. The secondary, slower off-site copy.
- **Backup_Watcher**: The new long-running supervised program that watches Workspace_Mount for filesystem changes via inotify, applies the debounce, and invokes Backup_Runner. Holds inotify watches and read fds under `/workspace`.
- **Backup_Runner**: The component that executes `restic backup` of Workspace_Mount into the Restic_Repo.
- **Restic_Repo**: The restic repository hosted on the S3-compatible backend. restic encrypts all data client-side before upload; the backend stores ciphertext only.
- **S3_Backend**: The S3-compatible object store (AWS S3, Cloudflare R2, or MinIO) configured via environment variables, that holds the Restic_Repo.
- **Restic_Password**: The restic repository password (`RESTIC_PASSWORD`) supplied via `.env`/process environment. Encrypts/decrypts the Restic_Repo. Loss makes the repo unrecoverable.
- **Backup_Config**: The set of environment variables that configure and gate the feature, supplied through `.env`: `RESTIC_REPOSITORY` (the gate variable), the S3 endpoint, bucket, region, access key, secret key, and Restic_Password.
- **Debounce_Window**: The fixed 10-second period of filesystem quiet that must elapse after the most recent change event before Backup_Runner is triggered.
- **Debounce_Decision**: The pure (no-IO) decision logic in Core_Package that, given the time of the last event and the current time, decides whether to trigger a backup, keep waiting, or coalesce with an in-flight run.
- **Retention_Plan**: The pure (no-IO) decision logic in Core_Package that decides, given a schedule cadence and the time of the last prune, whether a `restic forget --prune` cycle is due.
- **Retention_Policy**: The concrete keep-hourly/keep-daily/keep-weekly rules passed to `restic forget --prune`.
- **Exclusion_Set**: The set of paths excluded from backup at both the watcher level (ignore, to prevent watcher storms) and the restic level (`--exclude`): `node_modules`, `.git`, `.cache`, `.desktop`, `.desktop-vnc`, build/dist artifacts, and IDE index directories.
- **Clash_Proxy**: The in-container Clash HTTP/SOCKS proxy at `127.0.0.1:7890` through which ALL container egress flows, with fail-closed semantics (no Clash = no internet).
- **Backup_State_Dir**: The runtime state directory at `/var/run/backup-state` (a non-FUSE, container-local path — NOT under `/workspace`) holding `last-success`, `consecutive-failures`, and `last-prune` markers for the backup chain. Placed outside `/workspace` to avoid triggering inotify events and to avoid FUSE-unmount ordering issues. Loss on container restart is non-fatal (next successful backup rebuilds it).
- **Backup_Failure_Threshold**: The number of consecutive backup failures after which a failure notification is written (mirroring `SYNC_FAILURE_THRESHOLD`).
- **Doctor**: The health-check command `packages/cli/src/doctor.ts`.
- **Notification_Dir**: `/workspace/.notifications/` — where Vault_Sync writes `sync-failure.md`; the backup chain mirrors this with its own failure notification.
- **Vault_Lifecycle**: The callers `lock-vault`, `destroy`, and `desktop-stop` that must stop all `/workspace` fd-holders before a gocryptfs unmount.
- **Core_Package**: The pure (no-IO) package `packages/core`, including `constants.ts`, where Debounce_Decision and Retention_Plan logic and their constants live.
- **RESTIC_REPOSITORY**: The restic repository URL for the S3_Backend (e.g. `s3:https://<endpoint>/<bucket>/<prefix>`). This is the **opt-in gate variable**: its presence enables the feature, its absence keeps the feature dormant — mirroring how `VAULT_GIT_REPO` gates Vault_Sync.
- **Backup_Opt_In_Gate**: The environment condition that enables the feature, defined as **the presence of a non-empty `RESTIC_REPOSITORY`**. When `RESTIC_REPOSITORY` is absent or empty, the Backup_Watcher stays dormant and the container behaves exactly as before this feature. No separate boolean flag (such as `BACKUP_S3=1`) is used, consistent with the existing `VAULT_GIT_REPO` convention.

## Requirements

### Requirement 1: Event-Driven Watcher with 10-Second Debounce

**User Story:** As a developer, I want my working files backed up shortly after I stop editing, so that recent work is protected without waiting for a 30-minute cron cycle.

#### Acceptance Criteria

1. WHEN a filesystem change occurs under Workspace_Mount, THE Backup_Watcher SHALL reset the Debounce_Window to 10 seconds.
2. WHEN 10 seconds elapse with no further filesystem change under Workspace_Mount, THE Backup_Watcher SHALL trigger exactly one Backup_Runner invocation.
3. THE Backup_Watcher SHALL detect changes using inotify on Workspace_Mount rather than time-based polling.
4. WHILE a Backup_Runner invocation is in flight, THE Backup_Watcher SHALL accumulate further filesystem change events without starting a second concurrent Backup_Runner invocation.
5. WHEN a Backup_Runner invocation completes AND filesystem change events accumulated during that invocation, THE Backup_Watcher SHALL schedule one subsequent Backup_Runner invocation after the next Debounce_Window elapses.
6. THE Debounce_Decision SHALL be implemented as pure logic in the Core_Package that, given the time of the last event, the current time, and whether a backup is in flight, returns one of the decisions trigger, wait, or coalesce.
7. THE Backup_Watcher SHALL trigger a Backup_Runner invocation unconditionally at a periodic fallback interval (default 5 minutes) regardless of whether any inotify events were received, as a safety net against inotify event loss on the gocryptfs FUSE mount. restic's incremental scan is near-zero-cost when nothing changed.

**Traceability:** LD3; feature summary "Event-driven watcher on /workspace with 10s debounce; accumulate events during an in-flight backup"; invariant "pure decision logic belongs in core".

### Requirement 2: Plaintext Backup Source with restic Client-Side Encryption

**User Story:** As a developer, I want restic to back up my readable files and encrypt them itself, so that deduplication works well and the object store never sees plaintext.

#### Acceptance Criteria

1. THE Backup_Runner SHALL back up the plaintext Workspace_Mount (`/workspace`) as its source.
2. THE Backup_Runner SHALL NOT read or back up the Vault_Cipher directory (`/vault/cipher`).
3. THE Backup_Runner SHALL perform restic's client-side encryption using the Restic_Password before any data is transmitted to the S3_Backend.
4. THE S3_Backend SHALL receive only restic-encrypted data, never plaintext Workspace_Mount contents.

**Traceability:** LD2; feature summary "BACKUP TARGET"; invariant "S3 only ever stores restic-encrypted data".

### Requirement 3: Exclusion Rules at Watcher and restic Levels

**User Story:** As a developer, I want transient and regenerable directories excluded from backup, so that the watcher does not storm on churn and snapshots stay small.

#### Acceptance Criteria

1. THE Backup_Watcher SHALL ignore filesystem events originating under the Exclusion_Set paths so that churn in those directories does not reset the Debounce_Window.
2. THE Backup_Runner SHALL pass each Exclusion_Set path to restic via `--exclude` so excluded paths are absent from snapshots.
3. THE Exclusion_Set SHALL include `node_modules`, `.git`, `.cache`, `.desktop`, `.desktop-vnc`, `.notifications`, build and distribution artifact directories, and IDE index directories.
4. THE Exclusion_Set SHALL be defined once in the Core_Package and consumed by both the watcher-level ignore and the restic `--exclude` arguments.

**Traceability:** feature summary "Exclusion rules ... to prevent watcher storms and bloated snapshots"; invariant "core is the single source of truth".

### Requirement 4: restic Repository on the S3-Compatible Backend

**User Story:** As an operator, I want the restic repository hosted on my chosen S3-compatible store, so that backups live off the container's local volume.

#### Acceptance Criteria

1. THE Backup_Runner SHALL target a Restic_Repo on the S3_Backend addressed by RESTIC_REPOSITORY.
2. THE Workspace SHALL accept S3_Backend configuration for AWS S3, Cloudflare R2, or MinIO using the S3 endpoint, bucket, region, access key, and secret key from Backup_Config.
3. WHEN the feature is enabled AND the Restic_Repo does not yet exist on the S3_Backend, THE Workspace SHALL initialize the Restic_Repo as a one-time setup action (in the entrypoint or via a dedicated `backup-init` CLI command), NOT during the Backup_Watcher's runtime loop. This prevents a transient network error from being misinterpreted as "repo does not exist" and accidentally creating a new empty repo that shadows the real one.
4. WHEN the Restic_Repo already exists on the S3_Backend, THE Backup_Runner SHALL reuse the existing repository rather than reinitializing it.
5. IF the Backup_Runner cannot reach the Restic_Repo at runtime, THE Backup_Runner SHALL report the failure and retry (per Requirement 7) rather than attempting to initialize a new repository.

**Traceability:** feature summary "restic repo on S3-compatible backend; client-side encryption with a repo password from .env".

### Requirement 5: All restic Egress Through the Clash Proxy

**User Story:** As a security-conscious operator, I want restic-to-S3 traffic to use the same proxy as all other egress, so that the fail-closed network posture is preserved.

#### Acceptance Criteria

1. THE Backup_Runner SHALL route all S3_Backend network traffic through the Clash_Proxy at `127.0.0.1:7890` using the container's `HTTPS_PROXY` and `http_proxy` configuration.
2. IF the Clash_Proxy is unavailable, THEN THE Backup_Runner SHALL fail the backup attempt rather than transmitting S3_Backend traffic outside the Clash_Proxy.

**Traceability:** invariant "All container egress goes through Clash proxy; fail-closed egress"; feature summary "restic→S3 traffic must go through this proxy".

### Requirement 6: Scheduled Retention and Prune

**User Story:** As an operator, I want old snapshots pruned on a schedule, so that storage cost stays bounded without slowing down each backup.

#### Acceptance Criteria

1. THE Backup_Runner SHALL apply the Retention_Policy using `restic forget --prune` with keep-hourly, keep-daily, and keep-weekly rules.
2. THE Backup_Runner SHALL run the prune cycle on a schedule rather than after every backup. The prune check SHALL occur immediately after a successful backup completes; backup and prune SHALL be serialized (never concurrent) because restic's repository lock does not permit parallel operations.
3. THE Retention_Plan SHALL be implemented as pure logic in the Core_Package that, given the configured prune cadence and the time of the last prune, returns whether a prune cycle is due.
4. WHEN a prune cycle completes successfully, THE Backup_Runner SHALL record the prune completion time in the Backup_State_Dir.

**Traceability:** feature summary "Retention/prune policy ... run on a schedule (not every backup)"; invariant "retention planning belongs in core with unit tests".

### Requirement 7: Failure Handling and Event Preservation

**User Story:** As a developer, I want backups to retry and never silently drop pending work, so that a temporary proxy or S3 outage does not lose my recent changes.

#### Acceptance Criteria

1. IF a Backup_Runner invocation fails, THEN THE Backup_Runner SHALL retry using exponential backoff between attempts.
2. WHILE the Clash_Proxy or S3_Backend is unavailable, THE Backup_Watcher SHALL retain the accumulated set of pending filesystem changes so the next successful backup captures them.
3. WHEN a Backup_Runner invocation fails, THE Backup_Runner SHALL increment the consecutive-failure counter in the Backup_State_Dir.
4. WHEN a Backup_Runner invocation succeeds, THE Backup_Runner SHALL record the success time in the Backup_State_Dir and reset the consecutive-failure counter to zero.
5. WHEN the consecutive-failure counter reaches the Backup_Failure_Threshold, THE Backup_Runner SHALL write a backup-failure notification to the Notification_Dir (`/workspace/.notifications/`).

**Traceability:** feature summary "Failure handling: exponential backoff, do NOT lose accumulated events ...; surface health via doctor + a notification after a failure threshold (mirror vault-sync)".

### Requirement 8: FUSE-Unmount Safety (Stop Before Unmount)

**User Story:** As an operator unlocking or destroying the vault, I want the backup watcher stopped before the gocryptfs unmount, so that `fusermount -u` never fails with EBUSY.

#### Acceptance Criteria

1. THE Backup_Watcher SHALL be a supervised program that holds inotify watches and read file descriptors under Workspace_Mount.
2. WHEN Workspace_Mount is about to be unmounted, THE Vault_Lifecycle SHALL stop the Backup_Watcher before the unmount proceeds.
3. WHEN `lock-vault` or `destroy` runs, THE Vault_Lifecycle SHALL include the Backup_Watcher in the set of `/workspace` fd-holders stopped before unmount.
4. WHEN the Backup_Watcher receives a stop signal, THE Backup_Watcher SHALL release all inotify watches and open file descriptors under Workspace_Mount before reporting stopped.
5. WHILE a Backup_Runner invocation is in flight at stop time, THE Backup_Watcher SHALL terminate the restic subprocess (SIGTERM, then SIGKILL after a grace period) and release its Workspace_Mount file descriptors so the unmount can proceed. restic will redo the incomplete work on the next run without data loss.

**Traceability:** invariant "CRITICAL FUSE-unmount safety invariant ... the new backup watcher WILL hold inotify watches + read fds on /workspace, so it MUST be stopped before unmount — add it to the stop sequence. This is the single most important new constraint."

### Requirement 9: Coexistence with vault-sync

**User Story:** As an operator, I want the restic backup and git-based vault-sync to run independently, so that adding restic does not destabilize the existing off-site copy.

#### Acceptance Criteria

1. THE Backup_Watcher and Vault_Sync SHALL operate as independent persistence chains, neither blocking nor disabling the other.
2. THE Backup_Watcher SHALL NOT acquire or wait on any lock held by Vault_Sync, and Vault_Sync SHALL NOT acquire or wait on any lock held by the Backup_Watcher.
3. THE Backup_Watcher SHALL read Workspace_Mount (plaintext) while Vault_Sync continues to push Vault_Cipher to git, with neither chain modifying the other's source.
4. THE Workspace documentation SHALL state the division of responsibility: restic is the fast/primary backup and Vault_Sync is the slower secondary off-site copy.

**Traceability:** LD1; feature summary "Coexistence with vault-sync: both run independently; no shared lock contention; document the division of responsibility".

### Requirement 10: On-Demand Lifecycle (Start on Mount, Stop on Unmount)

**User Story:** As a developer, I want the backup watcher to start automatically when my vault is unlocked and stop when it is locked, so that it only runs when there is plaintext to protect.

#### Acceptance Criteria

1. THE Backup_Watcher SHALL be configured as a supervisord program with autostart disabled (on-demand), following the desktop-stack model.
2. WHEN the vault is mounted at Workspace_Mount AND `RESTIC_REPOSITORY` is present and non-empty (the Backup_Opt_In_Gate is enabled), THE Workspace SHALL start the Backup_Watcher.
3. IF the Backup_Watcher is requested to start while Workspace_Mount is not mounted, THEN THE Backup_Watcher SHALL exit with a non-zero status.
4. WHEN the vault is locked or destroyed, THE Vault_Lifecycle SHALL stop the Backup_Watcher as part of the unmount sequence.

**Traceability:** invariant "On-demand programs use autostart=false ... The desktop stack is the model to follow"; feature summary "On-demand / lifecycle: how the watcher starts (on vault mount, like the desktop autostart flip) and stops".

### Requirement 11: Restore Path

**User Story:** As a developer recovering from data loss, I want a documented command to restore a snapshot, so that I can get my files back from the Restic_Repo.

#### Acceptance Criteria

1. THE Workspace SHALL provide a documented command-line procedure to restore a snapshot from the Restic_Repo using `restic restore`.
2. THE documented restore procedure SHALL route S3_Backend traffic through the Clash_Proxy and use the Restic_Password from Backup_Config.
3. THE Workspace SHALL provide a documented command-line procedure to list available snapshots in the Restic_Repo.

**Traceability:** feature summary "Restore path: a documented way to restore a snapshot (restic restore) — at least a CLI command requirement".

### Requirement 12: Secret Handling and Encrypted-Only Backend

**User Story:** As a security auditor, I want backup secrets confined to the environment and never logged, so that credentials and the repo password are not leaked.

#### Acceptance Criteria

1. THE Workspace SHALL source the S3_Backend access key, secret key, and Restic_Password only from `.env` / process environment, following the existing `CLASH_SUBSCRIPTION_URL` and `PASSWORD` pattern.
2. THE Backup_Runner SHALL NOT write the S3_Backend secret key or the Restic_Password to any log file or notification.
3. THE S3_Backend SHALL store only restic-encrypted data, as established in Requirement 2.
4. THE Backup_Runner SHALL transmit S3_Backend traffic only through the Clash_Proxy, as established in Requirement 5.

**Traceability:** feature summary "Security: S3 stores only restic-encrypted data; secrets only in .env / process env, never logged; egress only via Clash".

### Requirement 13: Opt-In Feature Gate

**User Story:** As an operator who has not configured backup, I want the feature to stay completely dormant, so that the container behaves exactly as it does today.

#### Acceptance Criteria

1. WHERE `RESTIC_REPOSITORY` is absent or empty in the environment, THE Backup_Watcher SHALL remain dormant and SHALL NOT watch Workspace_Mount or contact the S3_Backend.
2. WHILE the feature is dormant, THE Workspace SHALL behave identically to its pre-feature behavior, with Vault_Sync unaffected.
3. WHEN `RESTIC_REPOSITORY` is present and non-empty, THE Backup_Opt_In_Gate SHALL evaluate to enabled and the on-demand lifecycle in Requirement 10 SHALL apply.
4. IF `RESTIC_REPOSITORY` is present but a required credential in Backup_Config (the S3 access key, S3 secret key, or Restic_Password) is missing, THEN THE Backup_Runner SHALL report the misconfiguration through the Doctor and SHALL NOT transmit data to the S3_Backend.

**Traceability:** feature summary "Optionality: the feature is opt-in ... when not configured, the watcher stays dormant"; mirrors the existing `VAULT_GIT_REPO` gating convention (presence implies enabled).

### Requirement 14: Doctor Health Reporting for Backup

**User Story:** As an operator, I want `doctor` to report the backup chain's health, so that I can tell whether recent backups are succeeding.

#### Acceptance Criteria

1. WHERE the feature is enabled, THE Doctor SHALL report a backup component reflecting the recorded last-success time from the Backup_State_Dir.
2. IF the consecutive-failure counter in the Backup_State_Dir is at or above the Backup_Failure_Threshold, THEN THE Doctor SHALL report the backup component with status fail.
3. WHERE the feature is dormant (not configured), THE Doctor SHALL report the backup component as not configured (optional) without status fail.
4. WHEN the most recent recorded backup success is older than a configured staleness bound, THE Doctor SHALL report the backup component with status warn.

**Traceability:** invariant "doctor.ts is the health-check surface"; feature summary "surface health via doctor + a notification after a failure threshold (mirror vault-sync)".

### Requirement 15: Architectural Invariants (Core Purity, Single Source of Truth)

**User Story:** As a maintainer, I want the backup feature to respect the layered architecture, so that the codebase stays clean and the decision logic is testable without IO.

#### Acceptance Criteria

1. THE Core_Package SHALL contain the Debounce_Decision logic, the Retention_Plan logic, the Exclusion_Set, and the backup-related constants with no IO.
2. THE Backup_Watcher and Backup_Runner (in `packages/cli`) SHALL execute the plans returned by the Core_Package logic without duplicating that branching logic.
3. THE Core_Package SHALL NOT introduce any upward dependency on the cli or server packages for the backup feature.
4. THE backup-related constants (Debounce_Window duration, Backup_Failure_Threshold, prune cadence, Backup_State_Dir path, Retention_Policy values) SHALL be defined once in the Core_Package as the single source of truth.

**Traceability:** invariant "Core package (packages/core) is pure (no IO) — pure decision logic ... belongs there with unit tests; IO execution belongs in cli"; mirrors selkies-desktop-migration Requirement 13.

# Implementation Plan: Realtime Backup to S3

## Overview

Incremental build order: (1) pure core logic + property tests → (2) CLI watcher + init →
(3) container artifacts (Dockerfile restic, supervisor, entrypoint) → (4) lifecycle integration
(lock-vault, destroy, unlock-vault, doctor) → (5) integration verification.

**Every task is MANDATORY. No optional tasks, no skippable tasks.**

Key design decisions baked in:
- `autorestart=false` (watcher does NOT auto-restart on crash; doctor surfaces the problem).
- `isExcludedPath` checks ALL path segments (not just first).
- `planDebounce`: `lastTriggerMs === null` → immediate baseline trigger on first start.
- `lastTriggerMs` = time of last backup START (success or failure).
- Entrypoint auto-inits repo (no manual `backup-init` step required for normal flow).
- `backup-init` CLI exists as manual fallback/diagnostic tool.
- `fs.watch` recursive with callback-level exclusion (trade-off documented in design).

## Tasks

- [x] 1. Core pure logic (`packages/core/src/backup.ts`)
  - [x] 1.1 Create `packages/core/src/backup.ts` with all constants and pure functions
    - Constants: `BACKUP_DEBOUNCE_MS`, `BACKUP_FALLBACK_INTERVAL_MS`, `BACKUP_STATE_DIR`, `BACKUP_FAILURE_THRESHOLD`, `BACKUP_STALENESS_MS`, `BACKUP_MAX_RETRIES`, `BACKUP_BASE_BACKOFF_MS`, `BACKUP_KEEP_HOURLY`, `BACKUP_KEEP_DAILY`, `BACKUP_KEEP_WEEKLY`, `BACKUP_PRUNE_CADENCE_MS`, `BACKUP_EXCLUSION_SET`
    - Types: `DebounceDecision`, `DebounceState` (includes `consecutiveFailures` + `lastFailureMs` for backoff), `PruneDecision`, `PruneState`, `BackupGateEnv`, `GateResult`, `DoctorBackupStatus`, `DoctorBackupState`
    - Functions: `planDebounce`, `planPrune`, `backoffDelayMs`, `evaluateBackupGate`, `shouldNotify`, `planDoctorBackupStatus`, `isExcludedPath`
    - `planDebounce` logic: `backupInFlight` → `"coalesce"`; `lastTriggerMs === null` → `"trigger"` (immediate baseline); within backoff window (`consecutiveFailures > 0` AND `nowMs - lastFailureMs < backoffDelayMs(consecutiveFailures)`) → `"wait"`; fallback ≥ 5min → `"trigger"`; `lastEventMs === null` → `"wait"`; quiet ≥ 10s → `"trigger"`; else `"wait"`
    - `isExcludedPath` splits on `/` and checks ALL segments against `BACKUP_EXCLUSION_SET`
    - _Requirements: 1.6, 3.4, 6.3, 7.1, 13.4, 14.1, 15.1, 15.2, 15.3, 15.4_
  - [x] 1.2 Export from barrel (`packages/core/src/index.ts`)
    - Add `export * from "./backup.ts"`
    - _Requirements: 15.3_

- [x] 2. Property-based tests (`packages/core/tests/backup.property.test.ts`)
  - [x] 2.1 Property 1: Debounce trigger correctness
    - Generate random `DebounceState` with `backupInFlight=false`; assert trigger iff (`lastTriggerMs === null` OR (quiet ≥ 10s AND not in backoff) OR (fallback ≥ 5min AND not in backoff)); assert `"wait"` when within backoff window
    - **Property 1: Debounce trigger correctness**
    - **Validates: Requirements 1.1, 1.2, 1.7, 7.1**
  - [x] 2.2 Property 2: In-flight coalesce invariant
    - Generate random states with `backupInFlight=true`; assert always `"coalesce"`
    - **Property 2: In-flight coalesce invariant**
    - **Validates: Requirements 1.4, 1.5**
  - [x] 2.3 Property 3: Exclusion path filtering
    - Generate random paths with/without exclusion segments; assert correct classification; test nested paths (e.g. `project/node_modules/pkg/index.js`)
    - **Property 3: Exclusion path filtering**
    - **Validates: Requirements 3.1, 3.3**
  - [x] 2.4 Property 4: Retention plan schedule
    - Generate random `PruneState`; assert `"prune-due"` iff `lastPruneMs === null` OR elapsed ≥ cadence
    - **Property 4: Retention plan schedule**
    - **Validates: Requirements 6.2**
  - [x] 2.5 Property 5: Exponential backoff computation
    - Generate random failure counts; assert `BACKUP_BASE_BACKOFF_MS * 2^min(n, 4)`
    - **Property 5: Exponential backoff computation**
    - **Validates: Requirements 7.1**
  - [x] 2.6 Property 6: Failure notification threshold
    - Generate random failure counts; assert `shouldNotify` iff `n >= BACKUP_FAILURE_THRESHOLD`
    - **Property 6: Failure notification threshold**
    - **Validates: Requirements 7.5**
  - [x] 2.7 Property 7: Opt-in gate evaluation
    - Generate random env objects; assert dormant/misconfigured/enabled per spec
    - **Property 7: Opt-in gate evaluation**
    - **Validates: Requirements 10.2, 13.1, 13.3, 13.4**
  - [x] 2.8 Property 8: Doctor backup status
    - Generate random `DoctorBackupState`; assert status matches decision table
    - **Property 8: Doctor backup status**
    - **Validates: Requirements 14.1, 14.2, 14.3, 14.4**
  - [x] 2.9 Property 9: Backoff monotonically non-decreasing
    - Generate pairs `a < b` (both ≤ 4); assert `backoffDelayMs(a) <= backoffDelayMs(b)`
    - **Property 9: Backoff is monotonically non-decreasing**
    - **Validates: Requirements 7.1**

- [x] 3. Unit tests (`packages/core/tests/backup.test.ts`)
  - [x] 3.1 Write unit tests for all pure functions
    - `planDebounce`: trigger after 10s quiet, wait during quiet, coalesce when in-flight, fallback after 5min, immediate trigger when `lastTriggerMs === null`, **wait when within backoff window** (consecutiveFailures > 0 AND nowMs - lastFailureMs < backoffDelay)
    - `planPrune`: prune-due when never pruned, prune-due after cadence, skip before cadence
    - `backoffDelayMs`: correct exponential values (5s, 10s, 20s, 40s, 80s), cap at failure count 4+
    - `evaluateBackupGate`: dormant/enabled/misconfigured for various env combinations
    - `planDoctorBackupStatus`: all status paths (ok/warn/fail/not-configured)
    - `isExcludedPath`: first segment match, nested segment match, non-matching paths, empty path
    - `shouldNotify`: threshold boundary (2 → false, 3 → true)
    - _Requirements: 1.6, 3.4, 6.3, 7.1, 13.4, 14.1, 15.1, 15.4_

- [x] 4. Checkpoint — `bun test` green for core backup logic
  - Ensure all tests pass, ask the user if questions arise.

- [x] 5. CLI: Backup watcher (`packages/cli/src/backup-watcher.ts`)
  - [x] 5.1 Create `packages/cli/src/backup-watcher.ts`
    - Pre-flight: evaluate gate (exit 0 if dormant, exit 1 if misconfigured), verify mount (`mountpoint -q /workspace`)
    - Create state dir (`/var/run/backup-state`)
    - State variables: `lastEventMs`, `lastTriggerMs` (null initially), `backupInFlight`, `pendingDuringFlight`, `consecutiveFailures`, `lastFailureMs` (null initially), `stopping`, `resticProc`
    - Recursive `fs.watch` on `/workspace` with callback-level exclusion via `isExcludedPath` (NOTE: requires Bun ≥ 1.0.15 for recursive support on Linux; add a startup smoke test or version check)
    - 1-second tick loop calling `planDebounce` (passing `consecutiveFailures` + `lastFailureMs` for backoff); on `"trigger"` → `runBackup()`
    - `runBackup()`: set `lastTriggerMs = Date.now()` (time of backup START), spawn `restic backup /workspace` with `--exclude` args from `BACKUP_EXCLUSION_SET` (pass as bare names without `/` prefix — restic matches any path segment), proxy env (`http_proxy`, `https_proxy`, `HTTPS_PROXY` = `http://127.0.0.1:7890`), restic env vars from process.env
    - On success: (1) check `pendingDuringFlight` → if true, set `lastEventMs = Date.now()` (re-enter debounce for accumulated events), (2) clear `pendingDuringFlight = false`, (3) record `last-success` ISO timestamp, (4) reset `consecutiveFailures` to 0, clear `lastFailureMs`, (5) call `planPrune` → optionally spawn `restic forget --prune`
    - On failure: set `lastFailureMs = Date.now()`, increment `consecutiveFailures`, write state, check `shouldNotify` → write `/workspace/.notifications/backup-failure.md` (never log secrets)
    - _Requirements: 1.1-1.7, 2.1-2.4, 3.1-3.2, 4.1-4.2, 5.1-5.2, 6.1-6.4, 7.1-7.5, 8.1, 10.3, 12.1-12.2_
  - [x] 5.2 Graceful shutdown handler
    - On SIGTERM/SIGINT: set `stopping=true`, clear interval, SIGTERM restic subprocess (5s grace → SIGKILL), close watcher, exit 0
    - Ensures all inotify watches and read fds released before exit
    - _Requirements: 8.2-8.5_

- [x] 6. CLI: Backup init (`packages/cli/src/backup-init.ts`)
  - [x] 6.1 Create `packages/cli/src/backup-init.ts`
    - Evaluate gate (exit if dormant/misconfigured)
    - Spawn `restic init` with proxy env and restic env vars
    - Handle "already initialized" (exit 0) vs real failure (exit 1)
    - This is a manual fallback/diagnostic tool; entrypoint handles auto-init
    - _Requirements: 4.3, 4.4_

- [x] 7. Checkpoint — CLI compiles, diagnostics clean
  - Ensure all tests pass, ask the user if questions arise.

- [x] 8. Container: Dockerfile restic installation (`image/Dockerfile`)
  - [x] 8.1 Add restic binary installation layer
    - `ARG RESTIC_VERSION=0.17.3`
    - Download from GitHub releases, bunzip2 to `/usr/local/bin/restic`, chmod +x
    - Architecture-aware: `dpkg --print-architecture`
    - Place immediately after the Caddy install layer (both are "download single static binary"; after selkies/kasmvnc layers for cache efficiency)
    - _Requirements: 4.1 (dependency)_

- [x] 9. Container: Supervisor configuration
  - [x] 9.1 Create `image/etc/supervisor/backup-watcher.conf`
    - `[program:backup-watcher]`
    - `command=/root/.bun/bin/bun run /opt/workspace/packages/cli/src/backup-watcher.ts`
    - `autostart=false` (on-demand, started by entrypoint/unlock-vault)
    - `autorestart=false` (NOT unexpected — no auto-restart on crash; doctor surfaces the problem)
    - `startsecs=5`, `stopwaitsecs=10`, `stopsignal=TERM`
    - stdout/stderr to fd/1 and fd/2
    - _Requirements: 10.1, 10.2_

- [x] 10. Container: Entrypoint auto-init and watcher start
  - [x] 10.1 Modify `packages/cli/src/entrypoint-main.ts`
    - After vault auto-unlock succeeds, if `RESTIC_REPOSITORY` is set:
      1. Check if repo exists: `restic snapshots --no-lock` (quiet, nothrow)
      2. If exit ≠ 0: inspect stderr — if "wrong password"/"unable to open config" → log "WARNING: Backup repo exists but RESTIC_PASSWORD does not match"; otherwise → run `restic init` (quiet, nothrow); log success or warning
      3. Flip `autostart=false` → `autostart=true` in `/etc/supervisor/conf.d/backup-watcher.conf` via sed
    - This eliminates the manual `backup-init` step for normal flow; distinguishes "repo not found" from "password mismatch" to avoid misleading logs
    - _Requirements: 4.3, 10.2, 13.1, 13.3_
  - [x] 10.2 Modify `packages/cli/src/unlock-vault.ts`
    - After successful mount, if `RESTIC_REPOSITORY` is set: `supervisorctl start backup-watcher`
    - _Requirements: 10.2, 10.4_

- [x] 11. Lifecycle: Stop before unmount
  - [x] 11.1 Modify `packages/cli/src/lock-vault.ts`
    - Add `await $\`supervisorctl stop backup-watcher\`.quiet().nothrow()` before the FUSE unmount step
    - Place after desktop stop, before `fusermount -u`
    - _Requirements: 8.2, 8.3, 10.4_
  - [x] 11.2 Modify `packages/cli/src/lib/destroy.ts`
    - Same addition: stop backup-watcher before unmount in the destroy sequence
    - _Requirements: 8.2, 8.3_

- [x] 12. Doctor health reporting
  - [x] 12.1 Modify `packages/cli/src/doctor.ts`
    - Import `evaluateBackupGate`, `planDoctorBackupStatus`, `BACKUP_STATE_DIR` from `@sdw/core`
    - Read state files: `last-success`, `consecutive-failures`
    - Call `planDoctorBackupStatus` with gate result, last success, failures, now
    - Report: "Backup (S3/restic)" component with status and detail string
    - When dormant: report "not configured (optional)" without fail status
    - _Requirements: 14.1-14.4_

- [x] 13. Configuration surface
  - [x] 13.1 Update `.env.example`
    - Add backup configuration section with `RESTIC_REPOSITORY`, `RESTIC_PASSWORD`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, optional `AWS_DEFAULT_REGION`
    - Comment explaining opt-in gate (presence of `RESTIC_REPOSITORY` enables feature)
    - _Requirements: 12.1, 13.1, 13.3_
  - [x] 13.2 Update `packages/cli/package.json`
    - Add `backup-watcher` and `backup-init` to bin/scripts if applicable
    - _Requirements: (housekeeping)_

- [x] 14. Checkpoint — full build + `bun test` green
  - Ensure all tests pass, ask the user if questions arise.

- [x] 15. Integration verification
  - [x] 15.1 Opt-in gate: container without `RESTIC_REPOSITORY` → backup-watcher not running, no errors
    - _Requirements: 13.1, 13.2_
  - [x] 15.2 Start on mount: unlock vault with `RESTIC_REPOSITORY` set → backup-watcher starts, repo auto-initialized
    - _Requirements: 4.3, 10.2_
  - [x] 15.3 FUSE safety: start backup-watcher → `lock-vault` → watcher stopped, unmount succeeds (no EBUSY)
    - _Requirements: 8.2, 8.3_
  - [x] 15.4 Coexistence: both backup-watcher and vault-sync-cron running simultaneously without interference
    - _Requirements: 9.1, 9.2, 9.3_
  - [x] 15.5 Doctor reporting: verify doctor output includes backup component in all states (ok, warn, fail, not-configured)
    - _Requirements: 14.1-14.4_

- [x] 16. Final checkpoint — all tests pass, image builds, end-to-end confirmed
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Every task is mandatory. No optional tasks.
- Property tests use `fast-check` (already in the project's test stack).
- The watcher uses `fs.watch({ recursive: true })` with callback-level exclusion — inotify watches are registered on ALL subdirectories (including excluded ones), but events from excluded paths are discarded in the callback. This is a documented trade-off (design §Performance Considerations).
- `autorestart=false` means the watcher does NOT auto-restart on crash. If it exits unexpectedly, doctor surfaces the problem; user fixes `.env` and manually restarts. This avoids infinite restart loops on misconfiguration.
- Restore path is documentation-only (design §Restore Path) — no code task needed; the documented `restic restore` commands work with the configured env vars.

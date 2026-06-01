# KasmVNC Framebuffer Fix — Bugfix Design

## Overview

KasmVNC 1.4.0 on arm64 Docker Desktop (macOS/Apple Silicon) completes the RFB handshake but never produces framebuffer updates, leaving the browser at "Connecting..." indefinitely. The fix investigation systematically isolates the root cause across a matrix of configurations (version × distro × runtime) using a dedicated test Dockerfile and automated verification script. The goal is to determine whether a viable fix exists or whether KasmVNC should be abandoned on this platform in favor of the working selkies stack.

## Glossary

- **Bug_Condition (C)**: KasmVNC 1.4.0 running on arm64 Docker Desktop macOS with trixie/kali libraries — produces zero framebuffer updates
- **Property (P)**: After RFB handshake, the encoder SHALL produce framebuffer updates and render the desktop in the browser within 5 seconds
- **Preservation**: The selkies desktop stack, KasmVNC 1.3.3 on bookworm, Caddy proxy, and XFCE sessions must remain unaffected
- **EncodeManager**: KasmVNC's internal component responsible for capturing X11 framebuffer changes and encoding them for RFB transmission
- **RFB**: Remote Framebuffer protocol — the wire protocol used by VNC implementations
- **Xkasmvnc**: The X server binary bundled with KasmVNC that attaches to an existing X display or runs as its own

## Bug Details

### Bug Condition

The bug manifests when KasmVNC 1.4.0 (built for trixie/arm64) runs inside Docker Desktop's virtualization layer on macOS Apple Silicon. The EncodeManager completes initialization but never transitions to active frame encoding. Threads remain in idle wait states rather than deadlocking — suggesting the encoder is waiting for a trigger (FramebufferUpdateRequest from client, or X11 damage events) that never arrives or is not processed correctly.

**Formal Specification:**
```
FUNCTION isBugCondition(input)
  INPUT: input of type KasmVNCDeployment
  OUTPUT: boolean
  
  RETURN input.kasmvnc_version = "1.4.0"
         AND input.architecture = "arm64"
         AND input.runtime = "docker-desktop-macos"
         AND input.base_distro IN {"kali-rolling", "debian-trixie"}
         AND input.framebuffer_updates = 0
END FUNCTION
```

### Examples

- KasmVNC 1.4.0 trixie .deb on `kalilinux/kali-rolling` arm64 Docker Desktop → 0 framebuffer updates, browser stuck (BUG)
- KasmVNC 1.4.0 trixie .deb on `debian:trixie-slim` arm64 Docker Desktop → expected to reproduce bug (VERIFY)
- KasmVNC 1.3.3 bookworm .deb on `debian:bookworm-slim` arm64 Docker Desktop → renders correctly (WORKING)
- KasmVNC 1.3.3 bookworm .deb on `kalilinux/kali-rolling` arm64 Docker Desktop → unknown, needs testing (VERIFY)

## Expected Behavior

### Preservation Requirements

**Unchanged Behaviors:**
- Selkies desktop stack on Kali arm64 Docker Desktop must continue streaming H.264/WebCodecs video
- KasmVNC 1.3.3 on bookworm must continue rendering frames (known-working baseline)
- Caddy reverse proxy must continue upgrading WebSocket connections correctly
- XFCE4 sessions must continue initializing on any X display regardless of VNC attachment
- The production `image/Dockerfile` must not be modified by this investigation

**Scope:**
All inputs that do NOT match the bug condition (non-1.4.0 versions, non-arm64-docker-desktop runtimes) should be completely unaffected. This includes:
- The production selkies-based desktop stack
- Any KasmVNC 1.3.3 deployments
- Native Linux arm64 hosts (non-Docker Desktop)
- x86_64 Docker Desktop environments

## Hypothesized Root Cause

Based on the bug description and known-working baseline, the most likely issues are:

1. **X11 DAMAGE Extension Incompatibility**: KasmVNC 1.4.0 may rely on X11 DAMAGE extension notifications to detect framebuffer changes. Docker Desktop's Rosetta/QEMU virtualization layer on arm64 may not correctly propagate DAMAGE events from Xvfb, causing the encoder to never see "dirty" regions.

2. **Shared Memory (SHM) Transport Failure**: KasmVNC 1.4.0 may use MIT-SHM for zero-copy framebuffer access. Docker Desktop's Linux VM may not support SHM between Xvfb and Xkasmvnc on arm64, causing silent fallback to a broken code path.

3. **Library ABI Mismatch (trixie libs)**: The trixie .deb links against newer versions of libX11/libXext/libpixman. Kali-rolling may ship slightly different versions causing symbol resolution issues in the encoder pipeline (not a crash, but a silent no-op in the capture loop).

4. **RFB Protocol Regression in 1.4.0**: KasmVNC 1.4.0 may have changed the FramebufferUpdateRequest handling or client capability negotiation, causing the encoder to wait for a message format that the bundled web client sends differently.

5. **arm64-Specific SIMD/NEON Code Path**: The 1.4.0 encoder may use arm64 NEON intrinsics for pixel comparison/encoding that behave incorrectly under Docker Desktop's virtualization (Rosetta 2 or QEMU user-mode).

## Correctness Properties

Property 1: Bug Condition - Framebuffer Encoding Engages After Handshake

_For any_ KasmVNC deployment where the bug condition holds (version 1.4.0, arm64, Docker Desktop macOS, trixie/kali base), the fixed configuration SHALL produce framebuffer updates (EncodeManager: Framebuffer updates > 0) and render the desktop in the browser within 5 seconds of RFB ClientInit completion.

**Validates: Requirements 2.1, 2.2, 2.3, 2.4**

Property 2: Preservation - Non-Buggy Configurations Unchanged

_For any_ deployment where the bug condition does NOT hold (KasmVNC 1.3.3 on bookworm, selkies stack, non-Docker-Desktop runtimes), the system SHALL produce the same behavior as before the investigation, preserving all existing desktop streaming, proxy, and session initialization functionality.

**Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5**

## Fix Implementation

### Changes Required

This is an investigation — the "fix" is a systematic test harness that isolates the root cause.

**File**: `image/Dockerfile.kasmvnc-test`

**Purpose**: Multi-configuration test Dockerfile supporting the verification matrix.

**Specific Changes**:
1. **Parameterized base image**: `ARG BASE_IMAGE=kalilinux/kali-rolling` (same as production)
2. **Parameterized KasmVNC version**: `ARG KASMVNC_VERSION=1.4.0` with `ARG KASMVNC_DISTRO=trixie`
3. **Minimal desktop layer**: Xvfb + XFCE (matching production) without selkies/caddy/workspace overhead
4. **Diagnostic tooling**: strace, ltrace, x11-utils, procps for runtime inspection
5. **Startup script**: Launches Xvfb → XFCE → KasmVNC with logging to stdout for easy capture

**File**: `docker-compose.kasmvnc-test.yml`

**Purpose**: Compose override for running test configurations.

**Specific Changes**:
1. **Service definition**: `kasmvnc-test` service with build args for matrix configurations
2. **Port mapping**: Direct 6080 exposure (no Caddy) for isolated testing
3. **Environment profiles**: Compose profiles for each verification matrix entry

**File**: `scripts/kasmvnc-test.sh`

**Purpose**: Automated verification script that runs the test matrix.

**Specific Changes**:
1. **Build matrix**: Iterates over (version × distro × base_image) combinations
2. **Health check**: Connects via WebSocket, completes RFB handshake, waits for framebuffer update
3. **Diagnostic capture**: Collects strace output, /proc/pid/status, KasmVNC logs per configuration
4. **Results output**: Markdown table summarizing pass/fail per matrix entry
5. **Timeout handling**: 30-second timeout per configuration to avoid hanging

**File**: `.kiro/specs/kasmvnc-framebuffer-fix/findings.md`

**Purpose**: Living document updated as investigation progresses with root cause analysis.

## Testing Strategy

### Validation Approach

The testing strategy follows a two-phase approach: first, reproduce the bug systematically across configurations to isolate the variable that causes it, then (if a fix is found) verify it works and preserves existing behavior.

### Exploratory Bug Condition Checking

**Goal**: Surface counterexamples that demonstrate the bug BEFORE attempting any fix. Confirm or refute each hypothesized root cause by varying one dimension at a time.

**Test Plan**: Build the test Dockerfile with each verification matrix configuration. Connect a WebSocket client, complete the RFB handshake, and check whether framebuffer updates arrive within 30 seconds. Capture diagnostic data (strace, X11 extension list, library versions) for each run.

**Test Cases**:
1. **V1 - Reproduce on Kali 1.4.0**: KasmVNC 1.4.0 trixie .deb on kali-rolling arm64 (will fail — confirms bug)
2. **V2 - Version Downgrade**: KasmVNC 1.3.3 bookworm .deb on kali-rolling arm64 (tests if version is the variable)
3. **V3 - Distro Isolation**: KasmVNC 1.4.0 trixie .deb on debian:trixie-slim arm64 (tests if Kali is the variable)
4. **V4 - LSIO Comparison**: linuxserver/kasmvnc image on same host (tests if packaging is the variable)

**Expected Counterexamples**:
- V1 produces 0 framebuffer updates (confirms bug)
- V2 may produce frames (implicating 1.4.0 encoder regression)
- V3 result isolates whether Kali's library versions are the cause
- Possible causes: X11 DAMAGE not firing, SHM failure, ABI mismatch, arm64 NEON bug

### Fix Checking

**Goal**: If a working configuration is found, verify that for all inputs where the bug condition holds, the fix produces framebuffer updates.

**Pseudocode:**
```
FOR ALL config WHERE isBugCondition(config) DO
  result := startKasmVNC(config, with_fix=true)
  ASSERT result.framebuffer_updates > 0
  ASSERT result.time_to_first_frame <= 5 seconds
  ASSERT result.browser_renders_desktop = true
END FOR
```

### Preservation Checking

**Goal**: Verify that for all inputs where the bug condition does NOT hold, the system produces the same result as before.

**Pseudocode:**
```
FOR ALL config WHERE NOT isBugCondition(config) DO
  ASSERT startKasmVNC(config, with_fix=false) = startKasmVNC(config, with_fix=true)
END FOR
```

**Testing Approach**: Since this is an investigation (not a code change to the production image), preservation checking is implicit — the production Dockerfile is never modified. The test harness is entirely isolated in `Dockerfile.kasmvnc-test` and `docker-compose.kasmvnc-test.yml`.

**Test Cases**:
1. **Selkies Preservation**: Run production image, verify desktop streams via selkies (unchanged)
2. **Bookworm Baseline**: Run KasmVNC 1.3.3 on bookworm, verify frames render (unchanged)
3. **XFCE Session**: Verify XFCE starts on :1 regardless of VNC attachment (unchanged)

### Unit Tests

- WebSocket RFB handshake validation (connect, version exchange, security type, ClientInit)
- Framebuffer update detection (parse RFB ServerMessage type 0 = FramebufferUpdate)
- Timeout and error handling in the test script

### Property-Based Tests

- Generate random (version × distro × runtime) tuples and verify the bug condition function correctly classifies them
- For working configurations, verify framebuffer updates arrive within bounded time across multiple connection attempts
- Verify that repeated connect/disconnect cycles don't degrade encoder state

### Integration Tests

- Full matrix run: build all configurations, start each, connect, verify frame delivery or timeout
- Diagnostic data collection: verify strace captures syscalls, /proc/pid/status is readable
- Results aggregation: verify markdown table output is generated correctly

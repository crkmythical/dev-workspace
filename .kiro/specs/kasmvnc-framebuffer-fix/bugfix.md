# Bugfix Requirements Document

## Introduction

KasmVNC 1.4.0 on Kali Linux arm64 (Docker Desktop for macOS/Apple Silicon) fails to render frames in the browser. The WebSocket upgrade and RFB handshake complete successfully, but the EncodeManager never produces framebuffer updates (`Framebuffer updates: 0`), leaving the browser stuck at "Connecting..." indefinitely. This blocks the dual-desktop architecture (Task 15) which requires a working KasmVNC baseline.

The known-working configuration is KasmVNC 1.3.3 on `debian:bookworm-slim` (arm64, same Docker Desktop host). The regression is isolated to the KasmVNC encoder pipeline — all surrounding infrastructure (WebSocket, Caddy, X11 display, XFCE session) is confirmed functional.

## Bug Analysis

### Current Behavior (Defect)

1.1 WHEN KasmVNC 1.4.0 (trixie .deb) runs on Kali arm64 under Docker Desktop for macOS THEN the system completes the RFB handshake but produces zero framebuffer updates (`EncodeManager: Framebuffer updates: 0`), leaving the browser at "Connecting..." indefinitely

1.2 WHEN KasmVNC 1.4.0 is exposed directly on a host port (bypassing Caddy reverse proxy) on the same arm64 Docker Desktop environment THEN the system exhibits the identical zero-framebuffer-update failure, confirming the issue is not proxy-related

1.3 WHEN KasmVNC 1.4.0 runs in a fresh container with no prior client connections and a private browser window THEN the system still fails to encode frames, ruling out client-side state poisoning

1.4 WHEN KasmVNC 1.4.0's Xkasmvnc threads are inspected during the failure THEN the system shows threads in normal idle wait states (pipe_read, do_epoll_wait, do_select) rather than deadlocked — the encoder appears to be waiting for client messages that never arrive or are not processed

### Expected Behavior (Correct)

2.1 WHEN KasmVNC runs on Kali arm64 under Docker Desktop for macOS THEN the system SHALL produce framebuffer updates after RFB ClientInit completes and render the XFCE desktop in the browser within 5 seconds of connection

2.2 WHEN KasmVNC is exposed directly on a host port (bypassing Caddy) THEN the system SHALL render frames identically to the proxied path, confirming encoder independence from network topology

2.3 WHEN a fresh container starts with no prior client state THEN the system SHALL begin encoding frames on first client connection without requiring any warm-up or retry

2.4 WHEN the RFB handshake completes successfully THEN the system SHALL transition from handshake to active encoding (EncodeManager: Framebuffer updates > 0) and deliver pixel data to the connected client

### Unchanged Behavior (Regression Prevention)

3.1 WHEN KasmVNC 1.3.3 (bookworm .deb) runs on debian:bookworm-slim arm64 under the same Docker Desktop host THEN the system SHALL CONTINUE TO render frames successfully in the browser (known-working baseline)

3.2 WHEN the selkies desktop stack runs on the same Kali arm64 Docker Desktop environment THEN the system SHALL CONTINUE TO stream H.264/WebCodecs video without any degradation

3.3 WHEN KasmVNC's WebSocket upgrade and RFB handshake are tested via programmatic clients (Python WS) THEN the system SHALL CONTINUE TO return valid RFB version bytes (`RFB 003.008\n`) and complete the security handshake

3.4 WHEN Caddy reverse-proxies WebSocket connections to any backend (selkies or KasmVNC) THEN the system SHALL CONTINUE TO correctly upgrade connections and pass frames bidirectionally

3.5 WHEN XFCE4 sessions start on X displays (:1 or :2) THEN the system SHALL CONTINUE TO initialize the desktop environment and respond to X11 client queries regardless of which VNC server (if any) is attached

---

## Bug Condition Derivation

### Bug Condition Function

```pascal
FUNCTION isBugCondition(X)
  INPUT: X of type KasmVNCDeployment
  OUTPUT: boolean
  
  // The bug triggers when KasmVNC 1.4.0 runs on arm64 Docker Desktop (macOS)
  // with trixie/kali-based libraries
  RETURN X.kasmvnc_version = "1.4.0"
     AND X.architecture = "arm64"
     AND X.runtime = "docker-desktop-macos"
     AND X.base_distro IN {"kali-rolling", "debian-trixie"}
END FUNCTION
```

### Fix Checking Property

```pascal
// Property: Fix Checking — Framebuffer encoding must engage after handshake
FOR ALL X WHERE isBugCondition(X) DO
  result ← connectAndAwaitFrames(X)
  ASSERT result.framebuffer_updates > 0
     AND result.browser_renders_desktop = true
     AND result.time_to_first_frame <= 5 seconds
END FOR
```

### Preservation Checking Property

```pascal
// Property: Preservation Checking — Non-buggy configurations unchanged
FOR ALL X WHERE NOT isBugCondition(X) DO
  ASSERT F(X) = F'(X)
  // Specifically:
  // - KasmVNC 1.3.3 on bookworm continues working
  // - Selkies stack continues working
  // - RFB handshake protocol unchanged
  // - Caddy proxy behavior unchanged
  // - XFCE session initialization unchanged
END FOR
```

---

## Investigation Verification Matrix

The fix must be validated through these verification directions:

| # | Configuration | Purpose |
|---|--------------|---------|
| V1 | KasmVNC 1.4.0 trixie .deb on Kali arm64 Docker Desktop | Confirm bug reproduces (baseline) |
| V2 | KasmVNC 1.3.3 bookworm .deb on Kali arm64 Docker Desktop | Test version downgrade as fix |
| V3 | KasmVNC 1.4.0 on native Linux arm64 (non-Docker Desktop) | Isolate Docker Desktop virtualization |
| V4 | KasmVNC 1.4.0 on x86_64 Docker Desktop | Isolate arm64/emulation layer |
| V5 | linuxserver/kasmvnc upstream image on same host | Compare with known-working packaging |

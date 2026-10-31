# Implementation Plan

- [ ] 1. Write bug condition exploration test
  - **Property 1: Bug Condition** - KasmVNC 1.4.0 Framebuffer Zero-Update on arm64 Docker Desktop
  - **CRITICAL**: This test MUST FAIL on unfixed code - failure confirms the bug exists
  - **DO NOT attempt to fix the test or the code when it fails**
  - **NOTE**: This test encodes the expected behavior - it will validate the fix when it passes after implementation
  - **GOAL**: Surface counterexamples that demonstrate the bug exists
  - **Scoped PBT Approach**: Scope the property to the concrete failing case: KasmVNC 1.4.0 trixie .deb on kali-rolling arm64 Docker Desktop macOS
  - Create `image/Dockerfile.kasmvnc-test` with parameterized build args (`BASE_IMAGE=kalilinux/kali-rolling`, `KASMVNC_VERSION=1.4.0`, `KASMVNC_DISTRO=trixie`)
  - Install minimal desktop layer: Xvfb + XFCE + dbus-x11 + x11-utils (no selkies/caddy/workspace)
  - Install diagnostic tools: strace, ltrace, procps, x11-utils, netcat-openbsd
  - Create startup script that launches Xvfb → XFCE → KasmVNC with stdout logging
  - Create `docker-compose.kasmvnc-test.yml` with `kasmvnc-test` service, port 6080 exposed directly
  - Create `scripts/kasmvnc-test.sh` that builds V1 config, starts container, connects via WebSocket, completes RFB handshake, waits 30s for framebuffer updates
  - Test asserts: `framebuffer_updates > 0` AND `time_to_first_frame <= 5s` (from Expected Behavior in design)
  - Run test on UNFIXED code (V1: KasmVNC 1.4.0 on Kali arm64 Docker Desktop)
  - **EXPECTED OUTCOME**: Test FAILS (0 framebuffer updates — this confirms the bug exists)
  - Document counterexamples: "KasmVNC 1.4.0 trixie on kali-rolling arm64 Docker Desktop produces EncodeManager: Framebuffer updates: 0"
  - Capture diagnostic data: KasmVNC logs, thread states, X11 extension list
  - Mark task complete when test harness is written, V1 is run, and failure is documented
  - _Requirements: 1.1, 1.2, 1.3, 1.4_

- [ ] 2. Write preservation property tests (BEFORE implementing fix)
  - **Property 2: Preservation** - Non-Buggy Configurations Unchanged
  - **IMPORTANT**: Follow observation-first methodology
  - Observe: KasmVNC 1.3.3 bookworm .deb on debian:bookworm-slim arm64 Docker Desktop renders frames (known-working baseline)
  - Observe: Selkies desktop stack on kali-rolling arm64 Docker Desktop streams H.264/WebCodecs video
  - Observe: XFCE4 session starts on :1 regardless of VNC attachment
  - Add V2 profile to `docker-compose.kasmvnc-test.yml`: KasmVNC 1.3.3 bookworm .deb on kali-rolling arm64
  - Extend `scripts/kasmvnc-test.sh` to run V2 (version downgrade) and verify framebuffer updates arrive
  - Write property-based test: for all configs where `NOT isBugCondition(config)` (non-1.4.0 versions, non-Docker-Desktop runtimes), framebuffer updates > 0
  - Verify preservation test passes on UNFIXED code (V2 should produce frames, confirming 1.3.3 works)
  - Verify production `image/Dockerfile` is NOT modified (git diff check)
  - **EXPECTED OUTCOME**: Tests PASS (confirms baseline behavior to preserve)
  - Mark task complete when preservation tests pass on unfixed code
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [ ] 3. Systematic investigation and fix

  - [ ] 3.1 V3: Isolate Docker Desktop virtualization layer
    - Add V3 profile to compose: KasmVNC 1.4.0 trixie .deb on `debian:trixie-slim` arm64
    - Run V3 on Docker Desktop — if it also fails, Kali is NOT the variable
    - Compare with linuxserver/kasmvnc upstream image (LSIO) on same host
    - If LSIO works: diff packaging (startup scripts, X11 config, library versions)
    - If LSIO also fails: confirms Docker Desktop arm64 + 1.4.0 is the root cause
    - _Bug_Condition: isBugCondition(input) where version=1.4.0 AND runtime=docker-desktop-macos AND arch=arm64_
    - _Expected_Behavior: framebuffer_updates > 0 within 5s of RFB ClientInit_
    - _Preservation: Non-buggy configs (1.3.3, selkies, native Linux) unchanged_
    - _Requirements: 1.1, 1.4, 2.1, 2.4_

  - [ ] 3.2 Diagnostic deep-dive
    - Run strace on Xkasmvnc process: trace poll/epoll/select syscalls to identify what it's waiting on
    - Check X11 extensions: `xdpyinfo -display :1 | grep -i damage` — verify DAMAGE extension is loaded
    - Library audit: `ldd /usr/bin/Xkasmvnc` — compare linked libs between 1.4.0 (failing) and 1.3.3 (working)
    - Check `/proc/<pid>/status` for thread states during failure
    - Compare RFB message exchange (tcpdump on loopback) between working and failing configs
    - Document findings in `.kiro/specs/kasmvnc-framebuffer-fix/findings.md`
    - _Bug_Condition: isBugCondition(input) — encoder idle despite valid handshake_
    - _Requirements: 1.4, 2.4_

  - [ ] 3.3 Apply fix or document abandon decision
    - IF root cause found AND fix is viable: implement fix in test Dockerfile, verify V1 passes
    - IF root cause is Docker Desktop arm64 virtualization (unfixable): document as known limitation
    - IF version downgrade (1.3.3) works on Kali: document as workaround with trade-offs
    - IF no viable fix: recommend abandoning KasmVNC on this platform, confirm selkies is the path forward
    - Update `findings.md` with final recommendation
    - _Bug_Condition: isBugCondition(input) from design_
    - _Expected_Behavior: expectedBehavior(result) — framebuffer_updates > 0, renders within 5s_
    - _Preservation: Production Dockerfile unchanged, selkies stack unaffected_
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 2.1, 2.2, 2.3, 2.4, 3.1, 3.2, 3.3, 3.4, 3.5_

  - [ ] 3.4 Verify bug condition exploration test now passes (if fix applied)
    - **Property 1: Expected Behavior** - Framebuffer Encoding Engages After Handshake
    - **IMPORTANT**: Re-run the SAME test from task 1 - do NOT write a new test
    - The test from task 1 encodes the expected behavior (framebuffer_updates > 0 within 5s)
    - When this test passes, it confirms the expected behavior is satisfied
    - Run bug condition exploration test from step 1 against fixed configuration
    - **EXPECTED OUTCOME**: Test PASSES (confirms bug is fixed) OR test is skipped if fix is "abandon KasmVNC"
    - _Requirements: 2.1, 2.2, 2.3, 2.4_

  - [ ] 3.5 Verify preservation tests still pass
    - **Property 2: Preservation** - Non-Buggy Configurations Unchanged
    - **IMPORTANT**: Re-run the SAME tests from task 2 - do NOT write new tests
    - Run preservation property tests from step 2
    - **EXPECTED OUTCOME**: Tests PASS (confirms no regressions)
    - Confirm production Dockerfile is unmodified (`git diff image/Dockerfile` shows no changes)
    - Confirm selkies stack still streams correctly

- [ ] 4. Checkpoint - Ensure all tests pass and findings documented
  - All test matrix results documented in `findings.md`
  - Root cause identified or investigation exhausted
  - Clear recommendation: fix, workaround, or abandon
  - Production `image/Dockerfile` confirmed unmodified
  - All test artifacts isolated in: `image/Dockerfile.kasmvnc-test`, `docker-compose.kasmvnc-test.yml`, `scripts/kasmvnc-test.sh`
  - Ask user if questions arise about findings or next steps

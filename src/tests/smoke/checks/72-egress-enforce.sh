#!/usr/bin/env bash
# The fail-closed egress firewall: an nftables default-DROP OUTPUT filter that lets only loopback,
# the clash user and the operator's EGRESS_ALLOW_CIDRS out. It is ON BY DEFAULT, so this check
#   * starts an instance with the flag left at its default and proves what the filter does and
#     what it still allows (Clash, the allowlist, ssh through Clash, names resolved by Clash);
#   * proves that EGRESS_ENFORCE=0 really turns it off, and says so (the main instance runs so);
#   * proves that a malformed or too wide allowlist stops the boot instead of being skipped;
#   * checks that the default compose file no longer carries the pentest plugin's capabilities.
# Nothing else in the suite boots with the default, so a change to how Clash is started or which
# uid it runs as could break the enforced mode unnoticed.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

# What set-clash-dns says about where its DNS choice came from. With nothing configured it probes the
# network for global DoH (as the clash user, the one identity the firewall lets out), so the verdict
# must be the same with the firewall off and on.
dns_verdict() { dx set-clash-dns 2>&1 | sed -n 's/^ *source: //p'; }

# "nc -z -v -w 3 <gateway> 1" is the probe used below. The gateway is the host side of the
# instance's own network and nothing listens on port 1, so an open path ends in "refused" at once
# and a filtered one in "timed out". Unlike a request to the internet, it needs no internet.

compose_files() {
  step "compose: the default grants no pentest capabilities; the overlay adds them"
  local base pentest
  base="$(cat "$ROOT/docker-compose.yml")"
  pentest="$(cat "$ROOT/docker-compose.pentest.yml")"
  expect_no_match "docker-compose.yml has no SYS_CHROOT, MKNOD or NET_RAW in cap_add" \
    '^[[:space:]]*-[[:space:]]*(SYS_CHROOT|MKNOD|NET_RAW)([[:space:]]|$)' "$base"
  expect_match "… and keeps NET_ADMIN, which the firewall needs" '^[[:space:]]*-[[:space:]]*NET_ADMIN([[:space:]]|$)' "$base"
  for cap in SYS_CHROOT MKNOD NET_RAW; do
    expect_match "docker-compose.pentest.yml adds $cap" "^[[:space:]]*-[[:space:]]*${cap}([[:space:]]|\$)" "$pentest"
  done
  expect_no_match "… and leaves the firewall alone" 'EGRESS_ENFORCE' "$(grep -v '^#' "$ROOT/docker-compose.pentest.yml")"
}

disabled() {
  step "egress: EGRESS_ENFORCE=0 (the main instance) turns the firewall off, and says so"
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the boot warns that the firewall is off" 'egress firewall is OFF' "$LOG"
  expect_match "the banner shows it" 'Firewall: +OFF' "$LOG"
  expect_no_match "no firewall table is loaded" 'sdw_egress' "$(dx nft list ruleset 2>&1)"
  expect_match "a direct connection to the network's gateway is answered (refused)" 'refused' "$(dx sh -c "nc -z -v -w 3 ${SMOKE_NET}.1 1 2>&1")"
  expect_false "ssh is not sent through Clash" dx test -e /etc/ssh/ssh_config.d/10-sdw-egress.conf
  DNS_VERDICT_OFF="$(dns_verdict)"
  expect_match "set-clash-dns probes the network for its DNS choice" 'auto-detect' "$DNS_VERDICT_OFF"
}

enforced() {
  step "egress: the firewall at its default (EGRESS_ENFORCE unset)"
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 20 "$WS" 2>&1)"
    stack_down
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the boot says the firewall is enforced" 'Egress firewall enforced' "$LOG"
  expect_match "… with one allowed network" '1 allowed network' "$LOG"
  expect_match "the banner shows it" 'Firewall: +on' "$LOG"
  expect_no_match "no warning that it is off" 'egress firewall is OFF' "$LOG"
  RULES="$(dx nft list ruleset 2>&1)"
  expect_match "the ruleset is loaded" 'table inet sdw_egress' "$RULES"
  expect_match "… with the allowlist as an accept rule" 'ip daddr 172\.30\.86\.10 accept' "$RULES"
  expect_match "… and drop as the policy" 'policy drop' "$RULES"
  expect_eq "Clash runs as the clash user" "clash" "$(dx ps -o user= -C clash | head -1 | tr -d ' ')"
  expect_match "the vault was unlocked all the same" 'Vault: +unlocked' "$LOG"

  step "egress: what the filter stops"
  expect_match "root cannot reach a network that is not allowed (it times out)" 'timed out' "$(dx sh -c "nc -z -v -w 3 ${SMOKE_NET}.1 1 2>&1")"
  expect_match "… nor can the workspace user" 'timed out' "$(docker exec -u dev "$WS" sh -c "nc -z -v -w 3 ${SMOKE_NET}.1 1 2>&1")"
  expect_false "root cannot open a direct connection to the internet" \
    dx sh -c 'nc -z -w 4 1.1.1.1 443'
  expect_false "… nor an HTTP request that skips the proxy" \
    dx curl -s --noproxy '*' --max-time 6 -o /dev/null http://example.com

  step "egress: what it still allows"
  # Inbound traffic is not what the filter is about: the published port (what the tunnel connects
  # to) answers, because replies on an established connection are let out.
  expect_eq "the published port still answers (the gate sends the browser to the login page)" 302 "$(http_code "$BASE/")"
  expect_eq "… and so does the login itself" 200 "$(http_code "$BASE/auth/login")"
  # Through Clash it still goes out: the fake upstream node logs the CONNECT.
  dx curl -s -x http://127.0.0.1:7890 --max-time 20 -o /dev/null https://example.com || true
  expect_match "traffic through Clash still reaches the node" 'CONNECT example.com:443' "$(docker logs "$UP" 2>&1)"
  expect_match "names resolve through Clash (a fake address), not past it" '198\.18\.' "$(dx getent hosts example.com 2>&1)"
  expect_eq "the allowed network is reachable directly, as root" "proxies: []" \
    "$(dx curl -s --noproxy '*' --max-time 8 http://172.30.86.10:8000/sub-empty.yaml | tr -d '\n')"
  expect_eq "… and as the workspace user" "proxies: []" \
    "$(docker exec -u dev "$WS" curl -s --noproxy '*' --max-time 8 http://172.30.86.10:8000/sub-empty.yaml | tr -d '\n')"
  expect_match "… the upstream saw those requests itself" 'PROVIDER served /sub-empty.yaml' "$(docker logs "$UP" 2>&1)"
  expect_match "the workspace user's tools are told to go there directly (no_proxy)" \
    '^no_proxy=.*172\.30\.86\.10' "$(dx sdw workload -- /usr/bin/env)"
  expect_match "… next to the ranges the image already skips the proxy for" \
    '^no_proxy=localhost,127\.0\.0\.0/8,' "$(dx sdw workload -- /usr/bin/env)"

  step "egress: ssh goes through Clash"
  expect_match "ssh is configured to use Clash's SOCKS5 listener" \
    'proxycommand nc -X 5 -x 127\.0\.0\.1:7890 (%h|example\.invalid) (%p|22)' "$(dx ssh -G example.invalid 2>&1)"
  # `ProxyCommand none` is not printed by `ssh -G`: no proxycommand line at all means none applies.
  expect_no_match "… except for loopback" 'proxycommand' "$(dx ssh -G localhost 2>&1)"
  # The user's own ~/.ssh/config is read before the system one and ssh keeps the first value it
  # finds, so `ProxyCommand none` for a host on an allowed network takes that host off Clash (this
  # is what the documentation tells people to do for EGRESS_ALLOW_CIDRS).
  dx sh -c 'mkdir -p /root/.ssh && printf "Host 10.9.9.9\n  ProxyCommand none\n" > /root/.ssh/config && chmod 600 /root/.ssh/config'
  expect_no_match "a host given ProxyCommand none in the user's ssh config goes direct" 'proxycommand' "$(dx ssh -G 10.9.9.9 2>&1)"
  expect_match "while another host still goes through Clash" 'proxycommand nc -X 5' "$(dx ssh -G 10.9.9.8 2>&1)"
  dx rm -f /root/.ssh/config
  dx ssh -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    git@example.invalid true >/dev/null 2>&1 || true
  expect_match "an ssh connection reaches the node through Clash, with the host name" \
    'CONNECT example\.invalid:22' "$(docker logs "$UP" 2>&1)"

  step "egress: the rest of the workspace is unharmed"
  expect_match "the status file records an outcome" '"status":"(confirmed|degraded)"' "$(dx cat /var/run/egress-status 2>&1)"
  expect_no_match "doctor has no failed row" '✗' "$(dx doctor 2>&1 || true)"
  expect_eq "set-clash-dns reaches the same verdict on the network as without the firewall" \
    "$DNS_VERDICT_OFF" "$(dns_verdict)"
  stack_down
}

bad_allowlist() {
  step "egress: an allowlist that would open the internet stops the boot"
  compose up -d --quiet-pull >/dev/null 2>&1 || compose up -d >/dev/null 2>&1
  refused() { docker logs "$WS" 2>&1 | grep -q 'EGRESS_ALLOW_CIDRS'; }
  if wait_until 120 refused; then pass "the boot reports the allowlist"; else fail "no word about EGRESS_ALLOW_CIDRS in the log" "$(docker logs --tail 20 "$WS" 2>&1)"; fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "… names the entry that is too wide" "0\.0\.0\.0/0.*too wide" "$LOG"
  expect_no_match "the workspace did not start" 'Starting supervisord' "$LOG"
  stack_down
}

compose_files
disabled
instance "${SMOKE_PROJECT}-enf" "$((SMOKE_PORT + 9))" 172.30.86 sub.yaml src/tests/smoke/enforce.env enforced
instance "${SMOKE_PROJECT}-bad" "$((SMOKE_PORT + 10))" 172.30.87 sub.yaml src/tests/smoke/enforce-bad-allowlist.env bad_allowlist
finish

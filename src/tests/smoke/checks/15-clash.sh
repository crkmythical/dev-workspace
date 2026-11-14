#!/usr/bin/env bash
# Clash: the config the start-up renders from the image's template, the controller that sits
# behind a secret, and the watcher that starts Clash again when it dies.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

secret_digest() { dx sed -n 's/^secret: "\(.*\)"$/\1/p' /etc/clash/config.yaml | sha256sum | cut -c1-16; }
status_of() { dx curl -s -o /dev/null -w '%{http_code}' "$@"; }

step "clash: the rendered config"
expect_eq "the config is readable by the clash user and nobody else" "600 clash:clash" \
  "$(dx stat -c '%a %U:%G' /etc/clash/config.yaml)"
expect_true "the template belongs to the image" dx test -r /usr/share/sdw/clash/config.yaml.template
expect_false "and no copy of it sits in the /etc/clash volume, where it would never be updated" \
  dx test -e /etc/clash/config.yaml.template
expect_no_match "no placeholder is left in the rendered config" '[{][{]' "$(dx cat /etc/clash/config.yaml)"
expect_eq "Clash runs as the clash user and group" "clash clash" \
  "$(dx sh -c 'ps -C clash -o user=,group= | head -1 | xargs')"
# shellcheck disable=SC2016 # expanded in the container
expect_match "with no supplementary groups" 'Groups:[[:space:]]*$' \
  "$(dx sh -c 'grep ^Groups: /proc/$(pgrep -x clash | head -1)/status')"

step "clash: the controller demands its secret"
expect_eq "a request without it is refused" 401 "$(status_of http://127.0.0.1:9090/configs)"
expect_eq "so is one with a wrong secret" 401 \
  "$(status_of -H 'Authorization: Bearer not-the-secret' http://127.0.0.1:9090/configs)"
expect_eq "the right secret is accepted" 200 "$(clash_api /configs -o /dev/null -w '%{http_code}')"
expect_match "and the subscription provider is behind it" '"name":"subscription"' \
  "$(clash_api /providers/proxies/subscription)"

step "clash: changing the DNS reloads the running Clash"
BEFORE="$(secret_digest)"
OUT="$(dx set-clash-dns 223.5.5.5,119.29.29.29 2>&1)"
expect_match "set-clash-dns reloads Clash through the controller" 'Clash reloaded' "$OUT"
expect_eq "the secret did not change under the running Clash" "$BEFORE" "$(secret_digest)"
expect_eq "the controller still accepts it" 200 "$(clash_api /configs -o /dev/null -w '%{http_code}')"
expect_match "the new resolvers are in the config" '"119.29.29.29"' "$(dx cat /etc/clash/config.yaml)"
expect_eq "and the file is still private" "600 clash:clash" "$(dx stat -c '%a %U:%G' /etc/clash/config.yaml)"
expect_match "clearing the override reloads again" 'Clash reloaded' "$(dx set-clash-dns --clear 2>&1)"

step "clash: the watcher starts it again"
OLD="$(dx pgrep -x clash | head -1)"
dx kill -9 "$OLD"
restarted() {
  local now
  now="$(dx pgrep -x clash 2>/dev/null | head -1)"
  [ -n "$now" ] && [ "$now" != "$OLD" ]
}
if wait_until 60 restarted; then
  pass "Clash was started again after being killed"
else
  fail "Clash was started again after being killed" "$(docker logs --tail 5 "$WS" 2>&1)"
fi
expect_eq "as the clash user" clash "$(dx sh -c 'ps -C clash -o user= | head -1 | xargs')"
expect_true "and the dead one is not left as a zombie" dx sh -c '! ps -eo stat=,comm= | grep -q "^Z.*clash"'
proxy_up() { dx nc -z 127.0.0.1 7890; }
if wait_until 30 proxy_up; then pass "the proxy port answers again"; else fail "the proxy port answers again"; fi
expect_eq "the controller takes the same secret" 200 "$(clash_api /configs -o /dev/null -w '%{http_code}')"
expect_eq "which was not rotated by the restart" "$BEFORE" "$(secret_digest)"

finish

#!/usr/bin/env bash
# Privilege separation. Everything the user runs (code-server and its terminals, the desktops, build
# tools) runs as the unprivileged user `dev`; the secrets, the gate, Clash, Caddy and supervisord
# stay out of its reach, and the few things that need root are reached through the doas bridge.
# `dx` is root (the operator's `docker exec`); `as_dev` is the workload's identity, which is how a
# malicious package or a stolen terminal would see the container.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

as_dev() { docker exec -u dev "$WS" "$@"; }
user_of() { dx sh -c "ps -o user= -p \$(pgrep -f '$1' | head -1) | tr -d ' '"; }

step "who runs what"
expect_eq "code-server runs as dev" dev "$(user_of 'bind-addr 127.0.0.1:8082')"
expect_eq "… so does the desktop's X server" dev "$(user_of 'Xkasmvnc')"
expect_eq "… and its session" dev "$(user_of 'xfce4-session')"
# The Selkies stack (images built with DESKTOP_STACK=selkies|both) has its own X server and streamer.
case "$(dx cat /etc/sdw-desktop-stack 2>/dev/null)" in
  selkies | both)
    expect_eq "(selkies) its X server runs as dev" dev "$(user_of 'Xvfb :1')"
    expect_eq "(selkies) so does the streamer" dev "$(user_of 'selkies --addr')"
    ;;
esac
# Where a process keeps its configuration, caches and shell history is its HOME. Docker puts HOME=/root
# in the environment of everything the container starts, and a desktop program that kept it would write
# its profile into the container's plaintext layer instead of the vault (threat-model invariant 1).
# Every process of dev's must have been given a HOME inside the vault. Read as dev: root has no
# CAP_SYS_PTRACE here, so another uid's /proc/<pid>/environ is closed to it.
# shellcheck disable=SC2016
homes_of_dev() { as_dev sh -c 'for p in $(pgrep -u dev); do tr "\0" "\n" < /proc/$p/environ 2>/dev/null | sed -n "s/^HOME=//p"; done | sort -u'; }
DEV_HOMES="$(homes_of_dev)"
expect_match "dev's processes have a HOME (the check saw some)" '^/workspace/' "$DEV_HOMES"
expect_eq "… and none of them outside the vault" "" "$(printf '%s\n' "$DEV_HOMES" | grep -v '^/workspace/' | grep -v '^$')"
expect_eq "the gate (sync-service) is root's" root "$(user_of 'sdw sync-service')"
expect_eq "Caddy is its own user" caddy "$(user_of 'caddy run')"
expect_eq "Clash is its own user" clash "$(user_of '/usr/bin/clash')"
expect_eq "the vault's files belong to dev, whoever made them" "dev dev" \
  "$(dx stat -c '%U %G' /workspace/home /workspace/shared | sort -u)"
expect_eq "the mount point admits no one else (0700)" 700 "$(dx stat -c %a /workspace)"

step "what the workload holds"
# A process that is not root and was given no ambient capability has an empty effective set, however
# many the container itself holds. (This is what makes `doas` the only way up.)
expect_eq "the workspace user has no capability" 0000000000000000 \
  "$(docker exec -u dev "$WS" sed -n 's/^CapEff:[[:space:]]*//p' /proc/self/status)"

# A vault made by an older image holds what root wrote, with root's modes: gocryptfs creates the
# underlying files as itself, so it is the same state that root writing through the mount makes now.
# The mount presents everything as dev's, so even a 0600 file or a 0700 directory of root's is dev's.
dx sh -c 'umask 077 && mkdir -p /workspace/root-made && echo from-root > /workspace/root-made/f && chmod 700 /workspace/root-made && sync'
expect_eq "dev reads what root made (a 0600 file in a 0700 directory)" from-root "$(docker exec -u dev "$WS" cat /workspace/root-made/f)"
expect_true "… and writes into that directory" docker exec -u dev "$WS" sh -c 'echo from-dev > /workspace/root-made/g'
dx rm -rf /workspace/root-made

step "what the workload can reach"
expect_match "it can work in its home, inside the vault" 'ok' "$(as_dev sh -c 'echo ok > /workspace/home/probe && cat /workspace/home/probe && rm /workspace/home/probe')"
expect_false "it cannot list the ciphertext" as_dev ls /vault/cipher
expect_false "… or read the key it is wrapped with" as_dev cat /vault/cipher/gocryptfs.conf
expect_false "… or the root-only runtime directory (the sync key source)" as_dev ls /run/sdw
expect_false "… or the saved secrets" as_dev ls /var/lib/sdw
expect_eq "the remote-destruct state is root's alone (0700)" "700 root" "$(dx stat -c '%a %U' /var/run/self-destruct)"
expect_false "… so the hash of the destruct key, a guessing oracle for the vault passphrase, is out of reach" \
  as_dev cat /var/run/self-destruct/key-hash
expect_false "… and the completion marker cannot be forged from the workspace" \
  as_dev sh -c 'echo x > /var/run/self-destruct/completed'
expect_false "… or the audit trail" as_dev cat /var/log/sdw/audit.jsonl
expect_false "… or Clash's configuration (subscription URL, controller secret)" as_dev cat /etc/clash/config.yaml
expect_false "… or the container's start-up environment" as_dev cat /proc/1/environ
# shellcheck disable=SC2016
expect_false "… or the gate's environment, which holds the password hash" \
  dx sh -c 'setpriv --reuid=1000 --regid=1000 --clear-groups cat /proc/$(pgrep -f "sdw sync-service" | head -1)/environ'
expect_false "it cannot change the gate" as_dev sh -c 'echo x >> /etc/caddy/Caddyfile'
# shellcheck disable=SC2016 # expanded in the container
expect_false "… or stop Caddy" as_dev sh -c 'kill -9 $(pgrep -x caddy | head -1)'
# shellcheck disable=SC2016
expect_false "… or the supervisor" as_dev sh -c 'kill -9 $(pgrep -x supervisord | head -1)'
expect_false "… or open the supervisor's socket" as_dev supervisorctl status
expect_false "… or read the bridge's rules" as_dev cat /etc/doas.conf
expect_eq "Clash's controller refuses it too (it has no secret)" 401 \
  "$(as_dev curl -s --noproxy '*' -o /dev/null -w '%{http_code}' http://127.0.0.1:9090/configs)"

step "no environment secrets in the workload"
WORKLOAD_ENVIRON="$(dx sdw workload -- /usr/bin/env)"
expect_no_match "a workload process gets none of the workspace's secrets" \
  '^(PASSWORD|PASSWORD_FILE|RESTIC_PASSWORD|AWS_SECRET_ACCESS_KEY|AWS_ACCESS_KEY_ID|CLASH_SUBSCRIPTION_URL|VAULT_GIT_REPO|DESKTOP_BCRYPT_HASH|PENTEST_VAULT_PASSWORD)=' \
  "$WORKLOAD_ENVIRON"
expect_match "… but its home, user and proxy" '^HOME=/workspace/home$' "$WORKLOAD_ENVIRON"
expect_match "… the proxy" '^http_proxy=http://127.0.0.1:7890$' "$WORKLOAD_ENVIRON"
expect_match "… and identity" '^USER=dev$' "$WORKLOAD_ENVIRON"
# Read as the workload user, who owns the process: root without CAP_SYS_PTRACE cannot read another
# user's environment, so reading it as root showed nothing and "passed" for the wrong reason.
# shellcheck disable=SC2016
CODE_SERVER_ENVIRON="$(docker exec -u dev "$WS" sh -c 'cat /proc/$(pgrep -f "bind-addr 127.0.0.1:8082" | head -1)/environ' | tr '\0' '\n')"
expect_match "the running code-server's environment could be read (it is the workload's)" '^HOME=/workspace/home$' "$CODE_SERVER_ENVIRON"
expect_no_match "the running code-server holds none of the secrets either" \
  '^(PASSWORD|RESTIC_PASSWORD|AWS_SECRET_ACCESS_KEY|CLASH_SUBSCRIPTION_URL|DESKTOP_BCRYPT_HASH)=' "$CODE_SERVER_ENVIRON"
expect_match "the preinstalled runtimes are there for it" '^v20\.' "$(dx sdw workload -- node -v)"
expect_match "… and so is the JDK" 'openjdk version "17' "$(dx sdw workload -- java -version 2>&1)"

step "the bridge to root"
expect_true "the rules are exactly the generated ones" dx sh -c 'sdw doas-rules | diff - /etc/doas.conf'
expect_eq "the only setuid or setgid file is doas" "/usr/bin/doas" \
  "$(dx sh -c 'find / -xdev -type f \( -perm -4000 -o -perm -2000 \) 2>/dev/null')"
expect_false "dev cannot run a shell as root" as_dev doas /bin/sh -c id
expect_false "… nor a command that merely shares a name with a bridge entry" \
  as_dev sh -c 'mkdir -p /tmp/x && cp /bin/id /tmp/x/doctor && doas /tmp/x/doctor'
DOCTOR="$(as_dev doctor 2>&1)"
expect_match "doctor, which needs root, works from dev's terminal" 'All checks passed' "$DOCTOR"
expect_match "… and sees the container's configuration, not dev's" 'Vault mount — /workspace mounted' "$DOCTOR"
OPERATOR="$(as_dev self-destruct 2>&1 < /dev/null)"
expect_match "an operator's command is refused for dev, with where to run it" 'docker exec' "$OPERATOR"
expect_true "the workload can still lock and unlock the vault through the bridge" as_dev sh -c 'command -v lock-vault && command -v unlock-vault'

step "the dev user's tools"
expect_match "it installs its own runtime into its home, not into the image's" 'v22\.' \
  "$(dx sdw workload -- sh -c 'mise use -g node@22 >/dev/null 2>&1; node -v')"
expect_true "… and the image's own are untouched" dx test -d /opt/mise/installs/node/20.20.2
expect_false "… the system tools are read-only to it" as_dev touch /opt/mise/probe
dx rm -rf /workspace/home/.local/share/mise /workspace/home/.config/mise 2>/dev/null

finish

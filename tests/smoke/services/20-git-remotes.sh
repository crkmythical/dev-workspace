#!/usr/bin/env bash
# The two ways VAULT_GIT_REPO is meant to be written, tried for real by the unattended job
# (cron's environment: nothing from the container, no terminal):
#   ssh://git@host/...         a key in the vault, a real sshd, a host key nobody has accepted yet
#   http://<token>@host/...    the token as the user name, like https://<token>@github.com/...
# Both run on the main instance: its origin is pointed at the servers below and put back after.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

NET="${SMOKE_PROJECT}_default"
CIPHER=/vault/cipher
ORIGIN_BEFORE="$(dx git -C "$CIPHER" remote get-url origin)"
SSHD="${SMOKE_PROJECT}-sshd"
HTTPD="${SMOKE_PROJECT}-githttp"
TOKEN=smoke-token-abc

dx supervisorctl stop vault-sync-cron >/dev/null
cleanup() {
  dx git -C "$CIPHER" remote set-url origin "$ORIGIN_BEFORE" >/dev/null 2>&1
  dx rm -rf /workspace/.credentials/ssh /root/.ssh >/dev/null 2>&1
  dx supervisorctl start vault-sync-cron >/dev/null 2>&1
  docker rm -f "$SSHD" "$HTTPD" >/dev/null 2>&1
}
trap cleanup EXIT

job() { dx env -i HOME=/root PATH=/usr/bin:/bin /root/.bun/bin/bun run /opt/workspace/packages/cli/src/vault-sync.ts; }
as_cron() { dx env -i HOME=/root PATH=/usr/bin:/bin "$@"; }
local_head() { dx git -C "$CIPHER" rev-parse HEAD; }
# What the start-up's prepare-replication step does with VAULT_GIT_REPO: the same function.
prepare() {
  dx env REMOTE="$1" sh -c 'cd /opt/workspace/packages/cli && bun -e "
    import { ensureCipherRepo } from \"@sdw/infra\";
    import { VAULT_CIPHER_DIR, VAULT_GITATTRIBUTES, VAULT_GITIGNORE } from \"@sdw/core/vault\";
    console.log(await ensureCipherRepo({ dir: VAULT_CIPHER_DIR, remote: process.env.REMOTE,
      gitignore: VAULT_GITIGNORE, gitattributes: VAULT_GITATTRIBUTES }));"'
}

# ------------------------------------------------------------------------------------------------
step "SSH remote"
docker build -q -t sdw-smoke-sshd - >/dev/null <<'EOF'
FROM alpine:latest
RUN apk add --no-cache openssh git \
 && ssh-keygen -A \
 && adduser -D -s /bin/sh git \
 && echo "git:$(head -c 24 /dev/urandom | base64)" | chpasswd \
 && mkdir -p /home/git/.ssh /srv \
 && git init -q --bare -b main /srv/vault.git \
 && chown -R git:git /home/git /srv/vault.git
CMD ["/usr/sbin/sshd", "-D", "-e"]
EOF
dx rm -rf /workspace/.credentials/ssh /root/.ssh
dx mkdir -p /workspace/.credentials/ssh
dx sh -c 'ssh-keygen -q -t ed25519 -N "" -C smoke -f /workspace/.credentials/ssh/id_ed25519'
# What the unlock hook does with credentials found in the vault.
dx sh -c 'rm -rf ~/.ssh && ln -s /workspace/.credentials/ssh ~/.ssh'
PUB="$(dx cat /workspace/.credentials/ssh/id_ed25519.pub)"
docker rm -f "$SSHD" >/dev/null 2>&1
docker run -d --name "$SSHD" --network "$NET" --ip "${SMOKE_NET}.31" sdw-smoke-sshd >/dev/null
docker exec "$SSHD" sh -c "printf '%s\n' '$PUB' > /home/git/.ssh/authorized_keys && chown -R git:git /home/git/.ssh && chmod 700 /home/git/.ssh && chmod 600 /home/git/.ssh/authorized_keys"
sleep 2

OUT="$(prepare "ssh://git@${SMOKE_NET}.31/srv/vault.git" 2>&1)"
expect_match "the start-up step points origin at it" 'remote-updated|unchanged' "$OUT"
OUT="$(job 2>&1)"
CODE=$?
expect_eq "the unattended job pushes over SSH (exit 0)" 0 "$CODE"
expect_eq "the SSH server has exactly the vault's latest commit" "$(local_head)" \
  "$(docker exec -u git "$SSHD" git -C /srv/vault.git rev-parse main 2>/dev/null)"
# known_hosts is hashed (HashKnownHosts), so the host is looked up rather than grepped for.
expect_true "the server's host key is now pinned in the vault's known_hosts" \
  dx ssh-keygen -F "${SMOKE_NET}.31" -f /workspace/.credentials/ssh/known_hosts

# A key that changes afterwards is refused: trusting a host once is not trusting it forever.
HEAD_BEFORE="$(docker exec -u git "$SSHD" git -C /srv/vault.git rev-parse main)"
docker exec "$SSHD" sh -c 'rm -f /etc/ssh/ssh_host_* && ssh-keygen -A >/dev/null && kill -HUP 1'
sleep 3
dx sh -c 'echo changed > /workspace/ssh-change.txt && sync'
OUT="$(job 2>&1)"
CODE=$?
expect_eq "with a different host key the job fails (exit 1)" 1 "$CODE"
expect_eq "and nothing reached the server" "$HEAD_BEFORE" "$(docker exec -u git "$SSHD" git -C /srv/vault.git rev-parse main)"
expect_match "git says why" 'REMOTE HOST IDENTIFICATION HAS CHANGED|Host key verification failed' \
  "$(as_cron git -C "$CIPHER" push origin main 2>&1)"
docker rm -f "$SSHD" >/dev/null 2>&1

# ------------------------------------------------------------------------------------------------
step "HTTP remote with the token in the URL"
docker build -q -t sdw-smoke-githttp - >/dev/null <<'EOF'
FROM alpine:latest
RUN apk add --no-cache git git-daemon python3 \
 && git init -q --bare -b main /srv/git/vault.git
EOF
docker rm -f "$HTTPD" >/dev/null 2>&1
docker run -d --name "$HTTPD" --network "$NET" --ip "${SMOKE_NET}.32" -e GIT_TOKEN="$TOKEN" \
  -v "$SMOKE_DIR/fixtures/git_http_server.py:/srv/server.py:ro" sdw-smoke-githttp python3 -u /srv/server.py >/dev/null
sleep 3
dx sh -c 'echo over-http > /workspace/http-change.txt && sync'
prepare "http://${TOKEN}@${SMOKE_NET}.32:8080/vault.git" >/dev/null 2>&1
OUT="$(job 2>&1)"
CODE=$?
expect_eq "the unattended job pushes with the token from the URL (exit 0)" 0 "$CODE"
expect_eq "the server has exactly the vault's latest commit" "$(local_head)" \
  "$(docker exec "$HTTPD" git -C /srv/git/vault.git rev-parse main 2>/dev/null)"
LOG="$(docker logs "$HTTPD" 2>&1)"
expect_match "the server saw the token" 'auth=token' "$LOG"
expect_no_match "and nobody was asked for a password it did not have" 'terminal prompts disabled|could not read' "$OUT"
dx git -C "$CIPHER" remote set-url origin "http://wrong-token@${SMOKE_NET}.32:8080/vault.git"
expect_false "a wrong token gets nowhere" as_cron git -C "$CIPHER" ls-remote origin

finish

#!/usr/bin/env bash
# Secrets as files (what docker-compose.secrets.yml does): the master password and the Clash
# subscription URL come from /run/secrets/*, so only their PATHS are in the container's environment;
# and a secret file that cannot be read stops the boot instead of being skipped (a missing PASSWORD
# would otherwise turn into a generated one). Each in an instance of its own.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

FILE_PASSWORD='file-secret-pass-456'
# Under the repository, not /var/folders: the Docker daemon runs in a VM that shares only the
# home directory (Colima, Docker Desktop), and a compose secret is a bind mount of a host file.
mkdir -p "$ROOT/.poc-tmp"
SECRET_DIR="$(mktemp -d "$ROOT/.poc-tmp/smoke-secrets.XXXXXX")"
cleanup_secrets() {
  rm -f "$SECRET_DIR/password" "$SECRET_DIR/clash_url" "$SECRET_DIR/override.yml"
  rmdir "$SECRET_DIR" 2>/dev/null || true
}
trap cleanup_secrets EXIT

# write_override <PASSWORD_FILE value>: the compose override with the two file secrets.
write_override() {
  printf '%s\n' "$FILE_PASSWORD" >"$SECRET_DIR/password" # with the newline `echo secret > file` leaves
  printf '%s\n' "http://${SMOKE_UPSTREAM_IP}:8000/${SMOKE_SUB}" >"$SECRET_DIR/clash_url"
  cat >"$SECRET_DIR/override.yml" <<EOF
secrets:
  password:
    file: ${SECRET_DIR}/password
  clash_url:
    file: ${SECRET_DIR}/clash_url
services:
  workspace:
    secrets: [password, clash_url]
    environment:
      PASSWORD: ""
      CLASH_SUBSCRIPTION_URL: ""
      PASSWORD_FILE: $1
      CLASH_SUBSCRIPTION_URL_FILE: /run/secrets/clash_url
EOF
  # shellcheck disable=SC2034 # read by compose() in stack.sh
  SMOKE_EXTRA_COMPOSE="$SECRET_DIR/override.yml"
}

has_log() { docker logs "$WS" 2>&1 | grep -q -- "$1"; }

file_secrets() {
  step "secrets as files: the password and the subscription URL come from /run/secrets"
  write_override /run/secrets/password
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    stack_down
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  ENVTXT="$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$WS")"
  expect_no_match "the password is not in the container's environment" "$FILE_PASSWORD" "$ENVTXT"
  expect_match "only the path to it is" '^PASSWORD_FILE=/run/secrets/password$' "$ENVTXT"
  expect_no_match "nor the subscription URL" 'sub\.yaml' "$ENVTXT"
  expect_match "the Clash config was rendered from the subscription file" 'Clash config generated' "$LOG"
  expect_match "the vault was created and unlocked with the password from the file" 'Vault: +unlocked' "$LOG"
  expect_no_match "no password was generated: the file's was used" 'Generated password' "$LOG"
  expect_match "the gate accepts the password from the file" '^sdw_session=' "$(gate_login "$FILE_PASSWORD")"
  expect_eq "… and no longer the one .env would have set (it was blanked)" "" "$(gate_login "$SMOKE_PASSWORD")"
  expect_match "doctor sees the vault mounted" 'Vault mount — /workspace mounted' "$(dx doctor 2>&1)"
  stack_down
}

unreadable_secret() {
  step "secrets as files: an unreadable file stops the boot"
  write_override /run/secrets/not-there
  compose up -d >/dev/null 2>&1
  if ! wait_until 120 has_log 'cannot read the secret file'; then
    fail "the boot named the secret file it could not read" "$(docker logs --tail 15 "$WS" 2>&1)"
    stack_down
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the boot names the variable and the path" 'PASSWORD_FILE=/run/secrets/not-there: cannot read the secret file' "$LOG"
  expect_no_match "it did not carry on with a generated password" 'Generated password' "$LOG"
  expect_no_match "and never started the services" 'Starting supervisord' "$LOG"
  stack_down
}

instance "${SMOKE_PROJECT}-sec" "$((SMOKE_PORT + 8))" 172.30.85 sub.yaml src/tests/smoke/empty.env file_secrets
instance "${SMOKE_PROJECT}-sec" "$((SMOKE_PORT + 8))" 172.30.85 sub.yaml src/tests/smoke/empty.env unreadable_secret
finish

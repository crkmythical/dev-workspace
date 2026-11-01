#!/usr/bin/env bash
# Does a TPM2 seal actually make the vault key memory-only and measured-boot-bound?
#
# This is the verification for the "memory-only secrets" design
# (deploy/docs/design-memory-only-secrets.md, Track B1). It can ONLY run where a TPM2 is
# reachable — a host with /dev/tpmrm0, or swtpm via TPM2TOOLS_TCTI (how CI would run it).
# The project's dev box (macOS + colima) has no container-visible TPM2, so there it SKIPS.
# Nothing here is a hollow PASS: each check below really seals/unseals against the TPM and
# fails loudly if the property does not hold.
#
#   src/tests/smoke/manual/tpm-seal-verify.sh
#
# Needs: tpm2-tools (tpm2_createprimary, tpm2_create, tpm2_load, tpm2_unseal, tpm2_pcr*).
#        A TPM2 at /dev/tpmrm0, OR export TPM2TOOLS_TCTI=... for a swtpm (e.g.
#        "swtpm socket --tpm2 --tpmstate dir=... --ctrl type=tcp,port=2322 --server type=tcp,port=2321"
#        then TPM2TOOLS_TCTI="swtpm:port=2321").
#        Optional check 4 also needs gocryptfs + /dev/fuse.
#
# Uses PCR 23 (the resettable debug PCR) so the test is self-contained and never perturbs the
# host's real measured-boot PCRs (0-7). It resets PCR 23 on exit.
set -u

# A public, non-secret test value. This is NOT a real key; it only has to be distinctive so the
# "the sealed blob does not contain the plaintext" check means something.
SECRET="vault-key-test-$(date +%s)-adef0123456789"
readonly SECRET
readonly PCR=23
readonly ALG=sha256

pass=0
fail=0
ok()   { echo "  PASS  $1"; pass=$((pass + 1)); }
bad()  { echo "  FAIL  $1"; fail=$((fail + 1)); }
skip() { echo "SKIP: $1"; exit 0; }

# --- Preconditions ---------------------------------------------------------
command -v tpm2_createprimary >/dev/null 2>&1 || skip "tpm2-tools not installed."
if [ -z "${TPM2TOOLS_TCTI:-}" ] && [ ! -e /dev/tpmrm0 ] && [ ! -e /dev/tpm0 ]; then
  skip "no TPM2 (no /dev/tpmrm0 and no TPM2TOOLS_TCTI). This is expected on the dev box."
fi
if ! tpm2_pcrread "$ALG:$PCR" >/dev/null 2>&1; then
  skip "cannot talk to the TPM (tpm2_pcrread failed). Check permissions / TPM2TOOLS_TCTI."
fi

WORK="$(mktemp -d)"
cleanup() {
  tpm2_pcrreset "$PCR" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

cd "$WORK" || exit 1
tpm2_pcrreset "$PCR" >/dev/null 2>&1 || true

echo "== seal a key under a PCR $PCR policy"
tpm2_createprimary -Q -C o -g "$ALG" -G ecc -c primary.ctx || { bad "createprimary"; exit 1; }

# Build the policy digest that binds the sealed object to the current PCR $PCR value.
tpm2_startauthsession -Q -S trial.session
tpm2_policypcr -Q -S trial.session -l "$ALG:$PCR" -L policy.dat
tpm2_flushcontext trial.session

if printf '%s' "$SECRET" | tpm2_create -Q -C primary.ctx -g "$ALG" \
  -u seal.pub -r seal.priv -L policy.dat -i-; then
  ok "the key sealed under the PCR policy"
else
  bad "sealing failed"
fi
tpm2_load -Q -C primary.ctx -u seal.pub -r seal.priv -c seal.ctx \
  || bad "loading the sealed object failed"

# --- Check 1: unseal succeeds while PCR $PCR still matches ------------------
echo "== check 1: unseal succeeds under the matching PCR state"
unseal_now() {
  tpm2_startauthsession -Q --policy-session -S policy.session
  tpm2_policypcr -Q -S policy.session -l "$ALG:$PCR"
  local out rc
  out="$(tpm2_unseal -c seal.ctx -p session:policy.session 2>/dev/null)"; rc=$?
  tpm2_flushcontext policy.session >/dev/null 2>&1 || true
  printf '%s' "$out"
  return $rc
}
got="$(unseal_now)"
if [ "$got" = "$SECRET" ]; then
  ok "unsealed value matches what was sealed"
else
  bad "unseal did not return the original key (got ${#got} bytes)"
fi

# --- Check 2: unseal FAILS after the PCR changes ---------------------------
echo "== check 2: unseal fails once the measured state changes"
tpm2_pcrextend "$PCR:$ALG=0000000000000000000000000000000000000000000000000000000000000001" \
  >/dev/null 2>&1 || bad "could not extend PCR $PCR to simulate a changed boot state"
if got2="$(unseal_now)" && [ "$got2" = "$SECRET" ]; then
  bad "unseal STILL succeeded after the PCR changed — the seal is not measured-boot-bound"
else
  ok "unseal refused after the PCR changed (measured-boot binding holds)"
fi
tpm2_pcrreset "$PCR" >/dev/null 2>&1 || true

# --- Check 3: the plaintext key never sits in the sealed blob --------------
echo "== check 3: the key at rest is wrapped, not plaintext"
if grep -a -q -- "$SECRET" seal.pub seal.priv 2>/dev/null; then
  bad "the plaintext key is present in the sealed blob on disk"
else
  ok "the sealed blob on disk does not contain the plaintext key"
fi

# --- Check 4 (optional): gocryptfs mounts from the fd-fed key ---------------
echo "== check 4: gocryptfs accepts the unsealed key over a pipe (optional)"
if ! command -v gocryptfs >/dev/null 2>&1; then
  echo "  (skipped: gocryptfs not installed)"
elif [ ! -e /dev/fuse ]; then
  echo "  (skipped: no /dev/fuse)"
else
  cipher="$WORK/cipher"; plain="$WORK/plain"
  mkdir -p "$cipher" "$plain"
  if unseal_now | gocryptfs -q -init -- "$cipher" >/dev/null 2>&1; then
    ok "gocryptfs -init took the key on stdin"
  else
    bad "gocryptfs -init refused the piped key"
  fi
  if unseal_now | gocryptfs -q -- "$cipher" "$plain" >/dev/null 2>&1; then
    echo "canary" > "$plain/canary.txt"
    if [ "$(cat "$plain/canary.txt" 2>/dev/null)" = "canary" ]; then
      ok "wrote and read back through the mount"
    else
      bad "the mount did not round-trip data"
    fi
    fusermount -u "$plain" 2>/dev/null || fusermount3 -u "$plain" 2>/dev/null || true
  else
    bad "gocryptfs could not mount with the piped key"
  fi
fi

echo "== summary: $pass passed, $fail failed"
[ "$fail" -eq 0 ]

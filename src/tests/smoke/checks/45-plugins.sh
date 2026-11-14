#!/usr/bin/env bash
# The optional parts that normally need the internet and gigabytes: desktop-install and the
# pentest plugin (init, unlock, pentest-bootstrap, pentest, lock). debootstrap, apt-get and the two
# big downloads are replaced by the stand-ins in fixtures/stubs, which also record what they were
# asked, so the commands' own logic runs for real: arguments, fallbacks, files, mounts, namespaces.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

STUBS="$SMOKE_DIR/fixtures/stubs"
cleanup() {
  dx rm -f /usr/local/bin/debootstrap /usr/local/bin/apt-get /usr/local/bin/curl \
    /etc/fake-debootstrap-fail /etc/fake-apt-fail /etc/fake-curl-fail \
    /var/log/fake-debootstrap.log /var/log/fake-apt-get.log /var/log/fake-curl.log 2>/dev/null || true
  dx rm -rf /var/lib/fake-downloads /tmp/fake-tools 2>/dev/null || true
}
trap cleanup EXIT

for stub in debootstrap apt-get curl; do
  docker cp "$STUBS/$stub" "$WS:/usr/local/bin/$stub" >/dev/null
  dx chmod 0755 "/usr/local/bin/$stub"
done

# exit_code <command...>: the exit status, with the output thrown away.
exit_code() {
  "$@" >/dev/null 2>&1
  echo $?
}
# By exact name: the shell that started the session also has "sleep 31" in its command line.
inside_visible() { dx sh -c "pgrep -a -x sleep | grep -q ' 31\$'"; }
inside_gone() { ! inside_visible; }

# ------------------------------------------------------------------------------------------------
step "desktop-install"
OUT="$(dx desktop-install 2>&1)"
expect_match "no argument prints the usage" 'Usage: desktop-install' "$OUT"
expect_match "and lists what can be installed" 'Available: firefox, chromium, wireshark, idea, burpsuite' "$OUT"
expect_eq "an unknown application is refused (exit 1)" 1 "$(exit_code dx desktop-install nonsense)"

OUT="$(dx desktop-install firefox 2>&1)"
expect_match "firefox installs" 'Installed Firefox' "$OUT"
expect_no_match "with Clash up, there is no proxy warning" 'Clash proxy not available' "$OUT"
expect_match "apt got the right package" 'install -y --no-install-recommends firefox-esr' \
  "$(dx cat /var/log/fake-apt-get.log)"
expect_match "and there is an entry the desktop menu can show" 'Exec=firefox-esr %u' \
  "$(dx cat /usr/share/applications/firefox.desktop)"
dx desktop-install firefox >/dev/null 2>&1
expect_eq "installing twice does not register it twice" 1 \
  "$(dx sh -c 'grep -o firefox /opt/desktop-apps/.registry.json | wc -l' | tr -d ' ')"

# The applications that are downloaded are pinned (src/image/tools/*.env) and fetched by the image's
# own fetch-tool, which refuses bytes that do not match. The pins that ship are checked for shape;
# the install is exercised against fake pins (SDW_TOOLS_DIR) for files the curl shim serves.
for tool in idea burpsuite; do
  expect_eq "the shipped $tool pin has a sha256 for both architectures" 2 \
    "$(dx grep -Ec '^SHA256_(amd64|arm64)=[0-9a-f]{64}$' /usr/local/lib/sdw/tools/$tool.env)"
done
dx sh -c 'mkdir -p /var/lib/fake-downloads /tmp/fake-tools && printf "PK fake burp jar\n" >/var/lib/fake-downloads/burp.jar'
dx sh -c 'd=$(mktemp -d) && mkdir -p "$d/idea-fake/bin" && printf "#!/bin/sh\necho fake idea\n" >"$d/idea-fake/bin/idea.sh" \
  && chmod +x "$d/idea-fake/bin/idea.sh" \
  && tar --mtime=@0 --owner=0 --group=0 --numeric-owner -cf - -C "$d" idea-fake | gzip -n >/var/lib/fake-downloads/idea.tar.gz \
  && rm -rf "$d"'
# write_pin <tool> <file> [sha256]: a pin for the file the shim serves (its real sha256 unless given).
write_pin() {
  local sha="${3:-$(dx sha256sum "/var/lib/fake-downloads/$2" | cut -d' ' -f1)}"
  dx sh -c "printf '%s\n' NAME=$1 VERSION=fake \"URL_amd64='http://smoke.invalid/$2'\" SHA256_amd64=$sha \"URL_arm64='http://smoke.invalid/$2'\" SHA256_arm64=$sha >/tmp/fake-tools/$1.env"
}
write_pin burpsuite burp.jar
write_pin idea idea.tar.gz
OUT="$(dx env SDW_TOOLS_DIR=/tmp/fake-tools desktop-install burpsuite 2>&1)"
expect_match "a downloaded application installs" 'Installed Burp Suite' "$OUT"
expect_match "after fetch-tool checked its sha256" 'fetch-tool: burpsuite verified from' "$OUT"
expect_true "its jar is where its launcher expects it" dx test -s /opt/desktop-apps/burpsuite/burpsuite.jar
expect_match "and it has a menu entry" 'Exec=java -jar /opt/desktop-apps/burpsuite/burpsuite.jar' \
  "$(dx cat /usr/share/applications/burpsuite.desktop)"
OUT="$(dx env SDW_TOOLS_DIR=/tmp/fake-tools desktop-install idea 2>&1)"
expect_match "an archive is unpacked into place" 'Installed IntelliJ IDEA' "$OUT"
expect_true "with the leading directory stripped" dx test -x /opt/desktop-apps/idea/bin/idea.sh
expect_false "and the downloaded archive is not left behind" dx test -e /opt/desktop-apps/idea/.idea.download
# A file whose hash is not the pinned one is refused, and nothing is installed from it.
dx rm -rf /opt/desktop-apps/burpsuite /usr/share/applications/burpsuite.desktop
write_pin burpsuite burp.jar 0000000000000000000000000000000000000000000000000000000000000000
OUT="$(dx env SDW_TOOLS_DIR=/tmp/fake-tools desktop-install burpsuite 2>&1)"
expect_match "a download that does not match its pin is refused" 'sha256 mismatch.*rejected' "$OUT"
expect_match "and reported" 'ERROR: Failed to download Burp Suite' "$OUT"
expect_false "no jar is left" dx test -e /opt/desktop-apps/burpsuite/burpsuite.jar
expect_false "and no menu entry" dx test -e /usr/share/applications/burpsuite.desktop
write_pin burpsuite burp.jar
dx touch /etc/fake-apt-fail
OUT="$(dx desktop-install chromium 2>&1)"
expect_match "a failing apt is reported" 'ERROR: Failed to install chromium' "$OUT"
expect_eq "with exit status 1" 1 "$(exit_code dx desktop-install chromium)"
expect_false "and no menu entry is left behind" dx test -e /usr/share/applications/chromium.desktop
expect_no_match "nor a registry entry" 'chromium' "$(dx cat /opt/desktop-apps/.registry.json)"
dx rm -f /etc/fake-apt-fail
dx touch /etc/fake-curl-fail
OUT="$(dx env SDW_TOOLS_DIR=/tmp/fake-tools desktop-install burpsuite 2>&1)"
expect_match "a failing download is reported" 'ERROR: Failed to download Burp Suite' "$OUT"
dx rm -f /etc/fake-curl-fail

# ------------------------------------------------------------------------------------------------
step "pentest plugin: before anything exists"
expect_match "doctor shows the vault as not initialized" 'Pentest vault — not initialized' "$(dx doctor 2>&1 || true)"
OUT="$(dx pentest 2>&1 || true)"
expect_match "pentest refuses while the vault is not mounted" 'Pentest vault not mounted' "$OUT"
OUT="$(dx unlock-pentest 2>&1 || true)"
expect_match "unlock-pentest says to initialize first" "Run 'init-pentest' first" "$OUT"

step "pentest plugin: init and unlock"
OUT="$(dx init-pentest 2>&1)"
expect_match "init-pentest creates the vault (password from PENTEST_VAULT_PASSWORD)" 'Pentest vault initialized' "$OUT"
expect_match "doctor shows it locked" 'Pentest vault — locked \(initialized\)' "$(dx doctor 2>&1 || true)"
OUT="$(dx init-pentest 2>&1)"
expect_match "init-pentest twice is harmless" 'already initialized' "$OUT"
OUT="$(dx unlock-pentest 2>&1)"
expect_match "unlock-pentest mounts it" 'Pentest vault unlocked at /pentest/rootfs' "$OUT"
expect_true "/pentest/rootfs is a mount" dx mountpoint -q /pentest/rootfs
# A root file system runs programs as their own users (tor as debian-tor, apt downloads as _apt):
# FUSE refuses every uid but the mounting one unless the mount allows others. Found by running the
# real pentest-bootstrap: `service tor start` died with "Permission denied" on /var/lib/tor.
expect_match "the rootfs mount lets other users in" 'allow_other' "$(dx grep ' /pentest/rootfs ' /proc/mounts)"
expect_false "while the workspace vault stays private to its owner" \
  dx setpriv --reuid=100 --regid=101 --clear-groups ls /workspace
expect_true "the workspace vault is still mounted beside it" dx mountpoint -q /workspace
# The mount's root directory is the ciphertext directory itself, so ITS mode decides who can reach
# the rootfs: others may pass through (0711) but not list it, and the key file is 0400. (0700, which
# the workspace's ciphertext has, made `cat /pentest/rootfs/etc/hostname` fail for uid 100.)
expect_eq "the pentest ciphertext directory can be passed through but not listed" 711 "$(dx stat -c %a /pentest/cipher)"
expect_eq "the workspace's ciphertext directory is closed to everyone but root" 700 "$(dx stat -c %a /vault/cipher)"
expect_false "the workspace user cannot list the pentest ciphertext" docker exec -u dev "$WS" ls /pentest/cipher
expect_false "… nor read its key file" docker exec -u dev "$WS" cat /pentest/cipher/gocryptfs.conf
OUT="$(dx pentest 2>&1 || true)"
expect_match "pentest refuses while there is no rootfs yet" 'rootfs not bootstrapped' "$OUT"

step "pentest-bootstrap: Kali unreachable, Debian fallback"
dx sh -c 'echo kali-rolling > /etc/fake-debootstrap-fail'
OUT="$(dx pentest-bootstrap 2>&1)"
expect_match "the fallback is announced" 'Falling back to Debian bookworm' "$OUT"
expect_match "and it succeeds" 'Debian rootfs created' "$OUT"
LOG="$(dx cat /var/log/fake-debootstrap.log)"
expect_match "Kali was tried first" 'suite=kali-rolling target=/pentest/rootfs mirror=http://http.kali.org/kali' "$LOG"
expect_match "Debian second" 'suite=bookworm target=/pentest/rootfs mirror=http://deb.debian.org/debian' "$LOG"
expect_match "debootstrap downloads through Clash" 'proxy=http://127.0.0.1:7890' "$LOG"
expect_match "with the pentest tool list" 'include=.*nmap.*proxychains4.*tor' "$LOG"
expect_eq "the host name inside is pentest" pentest "$(dx cat /pentest/rootfs/etc/hostname)"
expect_match "proxychains goes to Tor, not to the host proxy" 'socks5 127.0.0.1 9050' \
  "$(dx cat /pentest/rootfs/etc/proxychains4.conf)"
expect_match "Tor itself bootstraps through Clash" 'HTTPSProxy 127.0.0.1:7890' "$(dx cat /pentest/rootfs/etc/tor/torrc)"
expect_match "apt inside uses Clash" 'Acquire::https::Proxy "http://127.0.0.1:7890"' \
  "$(dx cat /pentest/rootfs/etc/apt/apt.conf.d/90proxy)"
expect_true "the Kali tools installer is executable" dx test -x /pentest/rootfs/opt/tools/install-kali-tools.sh
expect_eq "another user (uid 100, like debian-tor) can read the rootfs" pentest \
  "$(dx setpriv --reuid=100 --regid=101 --clear-groups cat /pentest/rootfs/etc/hostname 2>&1)"

step "pentest-bootstrap: running it again"
OUT="$(printf 'n\n' | dxi pentest-bootstrap 2>&1)"
expect_match "an existing rootfs is kept unless the answer is y" 'Skipped' "$OUT"
dx rm -f /etc/fake-debootstrap-fail /var/log/fake-debootstrap.log
OUT="$(printf 'y\n' | dxi pentest-bootstrap 2>&1)"
expect_match "answering y rebuilds it" 'Removing existing rootfs' "$OUT"
expect_match "this time from Kali" 'Kali rootfs created' "$OUT"
expect_no_match "without the fallback" 'Falling back' "$OUT"

step "pentest: entering the environment"
dx mkdir -p /workspace/projects
dx sh -c 'echo shared > /workspace/projects/smoke-project.txt'
# shellcheck disable=SC2016 # expanded by the shell inside the pentest root
SCRIPT='echo HOSTNAME=$(cat /etc/hostname)
echo PID1=$(tr "\0" " " </proc/1/cmdline)
echo PROJECTS=$(ls /root/projects)
echo PROXYVARS=$(env | grep -ciE "^(http|https|all)_proxy=")
echo UID0=$(id -u)
echo PROCS=$(ls /proc | grep -c "^[0-9]")
exit'
OUT="$(printf '%s\n' "$SCRIPT" | dxi pentest 2>&1)"
expect_match "pentest announces the environment" 'Entering pentest environment' "$OUT"
expect_match "the shell inside runs in the pentest root" 'HOSTNAME=pentest' "$OUT"
expect_match "and is PID 1 of its own PID namespace" 'PID1=/bin/bash --login' "$OUT"
expect_match "the workspace projects directory is shared in" 'PROJECTS=smoke-project.txt' "$OUT"
expect_match "the host proxy variables do not leak into it" 'PROXYVARS=0' "$OUT"
PROCS="$(printf '%s' "$OUT" | sed -n 's/.*PROCS=\([0-9]*\).*/\1/p' | head -1)"
expect_true "the container's own processes are not visible from inside (saw $PROCS)" test "${PROCS:-999}" -le 12
expect_match "pentest says it left" 'Exited pentest environment' "$OUT"
expect_eq "none of its mounts leaked into the container" 0 \
  "$(dx sh -c 'grep -c " /pentest/rootfs/" /proc/mounts || true' | tr -d ' ')"
expect_false "the temporary enter script is gone" dx test -e /tmp/.pentest-enter.sh

# What a PID namespace hides is the HOST's processes from the inside; the host still sees the
# inside. (enter.ts used to claim the opposite.)
docker exec -d "$WS" sh -c "printf 'sleep 31 &\nsleep 15\nexit\n' | pentest >/tmp/pentest-bg.log 2>&1"
expect_true "a process started inside is visible from the host" wait_until 30 inside_visible
expect_true "and is gone again when the session ends" wait_until 60 inside_gone

step "lock-pentest"
OUT="$(dx lock-pentest 2>&1)"
expect_match "lock-pentest locks it" 'Pentest vault locked' "$OUT"
expect_false "/pentest/rootfs is no longer a mount" dx mountpoint -q /pentest/rootfs
expect_eq "and is empty" 0 "$(dx sh -c 'ls -A /pentest/rootfs | wc -l' | tr -d ' ')"
expect_true "the workspace vault was not touched" dx mountpoint -q /workspace
OUT="$(dx lock-pentest 2>&1)"
expect_match "locking twice is a no-op" 'not currently mounted' "$OUT"
OUT="$(dx unlock-pentest 2>&1)"
expect_match "it unlocks again" 'Pentest vault unlocked' "$OUT"
expect_true "with the rootfs intact" dx test -x /pentest/rootfs/bin/bash
dx lock-pentest >/dev/null 2>&1

finish

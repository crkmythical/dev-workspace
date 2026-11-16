//! The user-facing environment as DATA — a partial port of
//! packages/core/src/workspace/env-schema.ts.
//!
//! Only the *types* are ported here. The big `CORE_ENV_GROUPS` data table is consumed solely by
//! the `.env.example` generator and the drift test, both of which stay in TypeScript until stage
//! 3; the plugin contract (crate::plugins) carries env groups, so it needs these shapes now.

/// One environment variable: its name, the comment written above it in the example file, and the
/// placeholder shown for it. Secrets are left empty; a test rejects anything that looks like a
/// real token. The three optional TS fields collapse to a `bool`/`bool`/`Option` with the same
/// "absent means off" meaning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvVarSpec {
    pub name: String,
    /// Comment lines written above the variable.
    pub doc: Vec<String>,
    /// Value shown in the example file. A placeholder, never a credential.
    pub example: String,
    /// Written as `# NAME=value`: the feature stays off until the user opts in.
    pub opt_in: bool,
    /// The container refuses to start without it.
    pub required: bool,
    /// Read by a third-party tool rather than by this repo (name it); exempt from the
    /// dead-variable check.
    pub external: Option<String>,
}

/// A titled group of variables, written as one section of `.env.example`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvGroup {
    pub title: String,
    pub vars: Vec<EnvVarSpec>,
}

/// A variable with only a name and example (the common case), no docs/flags.
fn var(name: &str, doc: &[&str], example: &str) -> EnvVarSpec {
    EnvVarSpec {
        name: name.to_string(),
        doc: doc.iter().map(|s| s.to_string()).collect(),
        example: example.to_string(),
        ..Default::default()
    }
}

/// The core's user-facing environment as DATA — a port of CORE_ENV_GROUPS in the TS env-schema.
/// `.env.example` is generated from this (plus each plugin's groups); `sdw env-example --check`
/// fails when the file drifts.
pub fn core_env_groups() -> Vec<EnvGroup> {
    vec![
        EnvGroup {
            title: "REQUIRED — container won't start without this".to_string(),
            vars: vec![EnvVarSpec {
                required: true,
                ..var(
                    "CLASH_SUBSCRIPTION_URL",
                    &["Clash-format proxy subscription URL."],
                    "https://sub.example.com/your-subscription?target=clash",
                )
            }],
        },
        EnvGroup {
            title: "EGRESS — Clash proxy and the fail-closed firewall (optional)".to_string(),
            vars: vec![
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "CLASH_DNS",
                        &[
                            "DNS resolvers Clash itself uses to resolve proxy-node and subscription",
                            "hostnames, comma-separated. LEAVE UNSET to auto-detect at boot: the workspace",
                            "probes whether global DoH (1.1.1.1 / 8.8.8.8) is reachable and, on a censored",
                            "network such as mainland China — where those are blocked and Clash would load",
                            "ZERO nodes — switches to domestic resolvers (223.5.5.5,119.29.29.29) by itself.",
                            "Set this to pin a specific list, e.g. CLASH_DNS=223.5.5.5,119.29.29.29.",
                            "You can also change it at runtime from the code-server / VNC terminal with",
                            "  set-clash-dns 223.5.5.5,119.29.29.29   (persists, no rebuild/restart)",
                            "Plain IPs (UDP), tls:// (DoT) and https:// (DoH) entries are all accepted.",
                        ],
                        "https://1.1.1.1/dns-query,https://8.8.8.8/dns-query",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "EGRESS_ENFORCE",
                        &[
                            "The fail-closed egress firewall. ON BY DEFAULT: an nftables default-DROP filter lets",
                            "ONLY loopback, Clash's own uid and the networks in EGRESS_ALLOW_CIDRS leave the box.",
                            "A process that ignores the proxy, speaks UDP/QUIC, runs under `noproxy` or resolves",
                            "names itself is blocked instead of connecting with this machine's real address, and",
                            "DNS cannot leak past Clash. ssh is sent through Clash as well. The filter needs",
                            "CAP_NET_ADMIN (docker-compose.yml grants it); if it cannot be loaded the boot stops.",
                            "Set 0 (or false / no / off) to run without it. Any value that is not a clear 'no'",
                            "keeps the firewall on. To reach one network directly, list it in EGRESS_ALLOW_CIDRS",
                            "rather than turning the firewall off.",
                        ],
                        "0",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "EGRESS_ALLOW_CIDRS",
                        &[
                            "Networks the workspace may reach DIRECTLY, not through Clash, while the firewall is",
                            "on: IPv4/IPv6 addresses or CIDR blocks, separated by commas or spaces, for example a",
                            "LAN host or the Docker network of a database (192.168.1.0/24, 10.0.0.5). Traffic to",
                            "them leaves with this machine's real address, so list only what must stay local.",
                            "Blocks wider than /8 (IPv6: /16), names and malformed entries are refused, and the",
                            "boot stops. Tools learn of the networks through no_proxy; names are not resolved for",
                            "you (use extra_hosts in the compose file); for ssh put `ProxyCommand none` for the",
                            "host in ~/.ssh/config. Ignored when the firewall is off.",
                        ],
                        "192.168.1.0/24,10.0.0.5",
                    )
                },
            ],
        },
        EnvGroup {
            title: "IMAGE BUILD — used by `docker compose build`".to_string(),
            vars: vec![
                var(
                    "BASE_PROFILE",
                    &[
                        "Tested package adapter for the image userland: kali, debian or alpine.",
                        "BASE_PROFILE must match BASE_IMAGE; Alpine currently supports only none suites.",
                    ],
                    "kali",
                ),
                var(
                    "BASE_IMAGE",
                    &[
                        "Base image selected by BASE_PROFILE:",
                        "  kalilinux/kali-rolling  — Kali apt repositories and tools (default)",
                        "  debian:trixie-slim      — lean Debian base",
                        "  alpine:3.22              — minimal apk base; optional suites must be none",
                    ],
                    "kalilinux/kali-rolling",
                ),
                var(
                    "WORKSPACE_VARIANT",
                    &[
                        "Local image tag suffix. Name it after the selected profile, for example:",
                        "  kali-kasmvnc, debian-selkies or alpine-minimal",
                    ],
                    "kali-kasmvnc",
                ),
                var(
                    "APT_MIRROR",
                    &[
                        "Apt mirror host for Debian bases (a Kali base uses KALI_MIRROR instead).",
                        "  mirrors.aliyun.com  fast inside mainland China, very slow elsewhere (default)",
                        "  deb.debian.org      the official CDN, for everywhere else",
                    ],
                    "mirrors.aliyun.com",
                ),
                var(
                    "KALI_MIRROR",
                    &[
                        "Apt mirror host for a Kali base (the same repositories, a different host).",
                        "  http.kali.org                    the official redirector, picks one by location (default)",
                        "  mirrors.aliyun.com               a fast mirror inside mainland China",
                        "  mirrors.ustc.edu.cn              another one (mirrors.tuna.tsinghua.edu.cn answers 403)",
                        "The redirector can pick a very slow mirror from some networks (~14 KB/s was seen).",
                    ],
                    "http.kali.org",
                ),
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "BUILD_PROXY",
                        &[
                            "HTTP(S) proxy for the image BUILD only (empty = none). Use it where GitHub, Docker",
                            "Hub, crates.io or npm are blocked or flaky. The build runs inside the Docker VM, so",
                            "127.0.0.1 is the VM itself: use the address under which the VM reaches your host,",
                            "e.g. http://192.168.5.2:1080 on Colima (the proxy must accept connections from it).",
                            "It reaches every build step but is not stored in the image or its history. Image",
                            "PULLS are made by the Docker daemon and need the proxy on the daemon itself.",
                        ],
                        "http://192.168.5.2:1080",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "BUILD_NO_PROXY",
                        &[
                            "Hosts the build reaches directly, comma-separated, used with BUILD_PROXY. Put the",
                            "apt mirror here when it is faster direct (default: localhost,127.0.0.0/8).",
                        ],
                        "localhost,127.0.0.0/8,mirrors.aliyun.com",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "SDW_GH_MIRROR",
                        &[
                            "GitHub download mirror for the pinned third-party tools (Caddy, restic, croc, mise,",
                            "code-server, KasmVNC, Selkies, mihomo), used only by `docker compose build`. GitHub",
                            "release downloads are the ones that get throttled or blocked; set this to a caching",
                            "reverse proxy and the build fetches <mirror><github-url> first, then falls back to",
                            "GitHub itself. The pinned sha256 is still verified, so a mirror can only serve the",
                            "exact pinned bytes — a tampered or wrong mirror is rejected, never trusted. Empty =",
                            "straight from GitHub (default). Note the trailing slash.",
                        ],
                        "https://ghfast.top/",
                    )
                },
                var(
                    "DESKTOP_STACK",
                    &[
                        "Remote desktop(s) to install:",
                        "  kasmvnc  KasmVNC only, at /vnc/ (default; small, builds in seconds)",
                        "  selkies  Selkies only, at /desktop/ (H.264; a ~160 MB Python environment)",
                        "  both     Selkies and KasmVNC, in parallel",
                        "  none     no desktop (required by the Alpine profile)",
                    ],
                    "kasmvnc",
                ),
                var(
                    "EDITOR_SUITE",
                    &["Browser editor to install: code-server (default) or none."],
                    "code-server",
                ),
                var(
                    "DEV_RUNTIMES",
                    &["Preinstalled language suite: standard (Java, Node, pnpm, Python) or none."],
                    "standard",
                ),
                var(
                    "DESKTOP_APPS",
                    &["Preinstalled desktop applications: firefox (default) or none."],
                    "firefox",
                ),
                var(
                    "PENTEST_SUITE",
                    &["Encrypted pentest-rootfs bootstrap support: bootstrap (default) or none."],
                    "bootstrap",
                ),
                var(
                    "BACKUP_TRANSFER",
                    &["Backup and transfer tools: standard (restic + croc, default) or none."],
                    "standard",
                ),
            ],
        },
        EnvGroup {
            title: "ACCESS".to_string(),
            vars: vec![
                var(
                    "PASSWORD",
                    &[
                        "Master password: the web gate, installed editor/desktops and vault share this secret.",
                        "If empty, a random one is generated at first start and printed in the start-up banner",
                        "(docker logs dev-workspace). That one lives only inside the container: a restart keeps",
                        "it, but recreating the container (docker compose up -d after a rebuild or pull, docker",
                        "rm) generates a NEW one, and the existing vault then stays locked until you give it the",
                        "old one. Save the printed password, or set it here (it then sits in .env and in the",
                        "container environment).",
                    ],
                    "",
                ),
                var(
                    "TUNNEL_HOST_PORT",
                    &[
                        "Host port (127.0.0.1) the web UI is published on: the Cloudflare Tunnel connects",
                        "to it and the start-up banner prints it.",
                    ],
                    "18080",
                ),
                var(
                    "TUNNEL_NAME",
                    &["Cloudflare tunnel name (host script: scripts/setup-cloudflared.sh)."],
                    "dev-workspace",
                ),
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "TUNNEL_HOSTNAME",
                        &[
                            "Public hostname routed to the tunnel (host script: scripts/setup-cloudflared.sh).",
                            "Required by that script; there is no default.",
                        ],
                        "workspace.example.com",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "AUTH_SESSION_PERSIST",
                        &[
                            "Keep browser logins across a container restart. The ingress auth gateway signs",
                            "session cookies with a key chosen at boot: OFF (default) uses a random per-boot",
                            "key, so a restart logs everyone out and they re-enter the master password; ON (1)",
                            "keeps a random key in the container (/var/lib/sdw/session.key, root-only), so",
                            "sessions survive a restart. The key is never derived from the password.",
                        ],
                        "1",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "ACCESS_TEAM_DOMAIN",
                        &[
                            "Cloudflare Access unification (optional). When the workspace sits behind Access,",
                            "set this to your team domain so the auth gate ALSO accepts a valid Access session",
                            "(the Cf-Access-Jwt-Assertion the edge injects) — no second login after Access.",
                            "The gate verifies the JWT against the team's public keys; unset = cookie-only, and",
                            "the header is never trusted. Set together with ACCESS_AUD.",
                        ],
                        "your-team.cloudflareaccess.com",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "ACCESS_AUD",
                        &[
                            "The Access application's AUD tag (Access app → Overview). Required claim: the",
                            "assertion is accepted only if its audience matches. Set with ACCESS_TEAM_DOMAIN.",
                        ],
                        "",
                    )
                },
            ],
        },
        EnvGroup {
            title: "VAULT REPLICATION AND GIT IDENTITY (optional)".to_string(),
            vars: vec![
                var(
                    "VAULT_GIT_REPO",
                    &[
                        "Git remote the vault's CIPHERTEXT is replicated to every 30 minutes.",
                        "  https://<token>@github.com/user/dev-vault.git  (HTTPS + token, simplest)",
                        "  git@github.com:user/dev-vault.git              (SSH, needs a key in the vault)",
                        "An SSH server's host key is trusted the first time it is seen and pinned in the vault's",
                        "~/.ssh/known_hosts; a key that changes later is refused. The key must have no passphrase.",
                        "Leave empty to disable replication. At start-up the cipher directory is made a git",
                        "repository with this as its origin; the URL (token included) is kept in its .git/config",
                        "on the cipher volume, outside the vault.",
                    ],
                    "",
                ),
                var(
                    "GIT_USER_NAME",
                    &["Git identity for commits made inside the container."],
                    "",
                ),
                var("GIT_USER_EMAIL", &[], ""),
            ],
        },
        EnvGroup {
            title: "RUNTIME".to_string(),
            vars: vec![
                EnvVarSpec {
                    external: Some("tzdata".to_string()),
                    ..var("TZ", &["Timezone of the container."], "Asia/Shanghai")
                },
                var(
                    "MEMORY_RESERVATION",
                    &["Docker memory reservation for the container."],
                    "8g",
                ),
                EnvVarSpec {
                    opt_in: true,
                    external: Some("sdw-host".to_string()),
                    ..var(
                        "SDW_DOCKER_CONTEXT",
                        &[
                            "Optional host safety lock for destructive and multi-instance commands.",
                            "When set (for example colima-dev-workspace), sdw-host refuses up/down/destroy",
                            "unless `docker context show` matches, preventing cleanup in the wrong daemon.",
                        ],
                        "colima-dev-workspace",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    external: Some("docker compose".to_string()),
                    ..var(
                        "COMPOSE_FILE",
                        &[
                            "Linux hosts with AppArmor (Ubuntu, Debian): Docker's default profile forbids the FUSE",
                            "mount, so the vault stays locked (\"mount failed: Permission denied\"). This adds the",
                            "opt-in override docker-compose.apparmor.yml, which lifts the container's AppArmor",
                            "confinement, for every `docker compose` command run here. Not needed on Docker Desktop.",
                        ],
                        "docker-compose.yml:docker-compose.apparmor.yml",
                    )
                },
            ],
        },
        EnvGroup {
            title: "WORKSPACE USER (optional)".to_string(),
            vars: vec![EnvVarSpec {
                opt_in: true,
                ..var(
                    "WORKLOAD_ENV",
                    &[
                        "Everything you run in the workspace (the code-server terminal, the desktops) runs as an",
                        "unprivileged user that sees only a short list of environment variables, NOT this file's",
                        "secrets (PASSWORD, RESTIC_*, AWS_*, CLASH_SUBSCRIPTION_URL, ...). Name here, comma",
                        "separated, any other variable from this file that your tools should see, for example",
                        "WORKLOAD_ENV=GITHUB_TOKEN,NPM_TOKEN. A name that is one of the workspace's own secrets",
                        "is refused with a warning.",
                    ],
                    "GITHUB_TOKEN,NPM_TOKEN",
                )
            },
            EnvVarSpec {
                opt_in: true,
                ..var(
                    "WORKLOAD_ROOT",
                    &[
                        "Let the workspace user become root in its own terminal with a single `root` (or any",
                        "`doas <cmd>`), no password. OFF by default: normally the workload user is confined and",
                        "cannot escalate, so code you run (dependencies, tools, pentest targets) cannot turn off",
                        "the egress firewall, read the unlocked vault or kill self-destruct. Set 1/true/yes/on",
                        "only if you alone use this box and trust what you run. The confinement-preserving way to",
                        "get root is `docker exec -u root` from the host instead.",
                    ],
                    "1",
                )
            },
            ],
        },
        EnvGroup {
            title: "REMOTE DESKTOP (optional)".to_string(),
            vars: vec![
                var(
                    "DESKTOP_RESOLUTION",
                    &[
                        "Resolution of the KasmVNC desktop (/vnc/). Selkies (/desktop/) fits the browser",
                        "window itself and ignores this.",
                    ],
                    "1920x1080",
                ),
                var(
                    "DESKTOP_AUDIO",
                    &["Selkies desktop audio (PulseAudio null sink): 1 to enable, 0 to disable."],
                    "0",
                ),
            ],
        },
        EnvGroup {
            title: "REALTIME BACKUP (optional — S3-compatible store via restic)".to_string(),
            vars: vec![
                var(
                    "RESTIC_REPOSITORY",
                    &[
                        "Setting this turns the feature on; empty keeps the backup watcher dormant.",
                        "  s3:https://s3.us-east-1.amazonaws.com/my-bucket/workspace-backup",
                        "  s3:https://<account-id>.r2.cloudflarestorage.com/<bucket>/prefix   (Cloudflare R2)",
                        "A repository that does not exist yet is created at start-up.",
                        "The backup holds the PLAINTEXT workspace, protected only by RESTIC_PASSWORD.",
                        "It excludes .credentials/ (SSH keys and tokens), VCS data, dependencies, caches and",
                        "build outputs; use VAULT_GIT_REPO as well if those credentials must be recoverable.",
                        "Self-destruct does not delete the restic repository: treat it as part of the vault.",
                    ],
                    "",
                ),
                var(
                    "RESTIC_PASSWORD",
                    &["Repository encryption password (loss = unrecoverable repository)."],
                    "",
                ),
                var(
                    "AWS_ACCESS_KEY_ID",
                    &[
                        "S3 credentials, as restic reads them. These are the standard AWS variables: if you",
                        "use other AWS services with different credentials, isolate them (AWS_PROFILE).",
                    ],
                    "",
                ),
                var("AWS_SECRET_ACCESS_KEY", &[], ""),
                EnvVarSpec {
                    opt_in: true,
                    external: Some("restic".to_string()),
                    ..var(
                        "AWS_DEFAULT_REGION",
                        &["Custom region (restic default: us-east-1)."],
                        "us-east-1",
                    )
                },
            ],
        },
        EnvGroup {
            title: "FILE TRANSFER (optional — croc, end-to-end encrypted)".to_string(),
            vars: vec![
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "CROC_RELAY",
                        &[
                            "Self-hosted croc relay (host:port) for the `push` / `pull` commands. croc is",
                            "end-to-end encrypted (a code phrase, then AES-256-GCM); the relay only sees",
                            "ciphertext. Transfers go through Clash by default (`push --direct` for a relay on",
                            "the LAN). Leave empty to keep push/pull off. Do not use croc's public default relay.",
                        ],
                        "relay.example.com:9009",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    external: Some("croc".to_string()),
                    ..var(
                        "CROC_PASS",
                        &["Password of the relay above (its `croc relay` CROC_PASS). Empty if it has none."],
                        "",
                    )
                },
            ],
        },
        EnvGroup {
            title: "OPTIONAL FEATURES — all OFF unless enabled here".to_string(),
            vars: vec![
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "SYNC_ENABLED",
                        &[
                            "Browser file-sync page (/sync/) and its upload/list API.",
                            "Enabling writes the vault passphrase to /run/sdw/sync-passphrase on every unlock",
                            "(the sync key source). That directory is root-only and outside the vault, so code",
                            "running in the workspace cannot read it; locking the vault removes the file.",
                        ],
                        "1",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "REMOTE_DESTRUCT_ENABLED",
                        &[
                            "Remote self-destruct endpoint (POST /sync/api/destruct).",
                            "Enabling stores a bcrypt hash of a key derived from the vault passphrase; the key is",
                            "printed once and `destruct-passphrase` re-derives it at any time. Disabling removes the",
                            "hash on the next unlock.",
                        ],
                        "1",
                    )
                },
                EnvVarSpec {
                    opt_in: true,
                    ..var(
                        "METRICS_ENABLED",
                        &[
                            "Prometheus text metrics at GET /sync/api/metrics (logins, gate decisions, self-destruct",
                            "requests). Behind the same login gate as the rest of /sync/api/*: a scraper needs a session",
                            "cookie or a Cloudflare Access service token. Off = 404. Audit events (who logged in or",
                            "tried to, self-destruct calls, egress changes) are always recorded: run `audit-log`.",
                        ],
                        "1",
                    )
                },
            ],
        },
        EnvGroup {
            title: "SECRETS AS FILES — keep them out of the environment (optional)".to_string(),
            vars: vec![EnvVarSpec {
                opt_in: true,
                ..var(
                    "PASSWORD_FILE",
                    &[
                        "Every secret can be given as a file instead of a value: set <NAME>_FILE to the path of a",
                        "file holding it (Docker/Compose secrets are mounted at /run/secrets/<name>). Names:",
                        "PASSWORD, CLASH_SUBSCRIPTION_URL, VAULT_GIT_REPO, RESTIC_PASSWORD, AWS_ACCESS_KEY_ID,",
                        "AWS_SECRET_ACCESS_KEY, PENTEST_VAULT_PASSWORD, CROC_PASS. Only the path is then in the",
                        "environment (docker inspect, /proc/<pid>/environ); the value is read from the file when",
                        "needed. A non-empty <NAME> still wins, so remove it from this file; an unreadable",
                        "<NAME>_FILE stops the boot. Ready-made: docker-compose.secrets.yml and `make secrets-init`.",
                    ],
                    "/run/secrets/password",
                )
            }],
        },
    ]
}

const RULE: &str = "# ----------------------------------------------------------";
const HEAVY: &str = "# ============================================================";

/// The full text of `.env.example` for a set of groups — a port of renderEnvExample. Required
/// variables are named in the header; opt-in ones are written commented-out (`# NAME=value`).
pub fn render_env_example(groups: &[EnvGroup]) -> String {
    let required: Vec<&str> = groups
        .iter()
        .flat_map(|g| &g.vars)
        .filter(|v| v.required)
        .map(|v| v.name.as_str())
        .collect();
    let copy_line = if required.is_empty() {
        "# Create `.env` with mode 0600 and fill it in. Nothing is required.".to_string()
    } else {
        let verb = if required.len() > 1 { "are" } else { "is" };
        format!(
            "# Create `.env` with mode 0600 and fill it in. Only {} {verb} required.",
            required.join(", ")
        )
    };
    let mut out: Vec<String> = vec![
        HEAVY.to_string(),
        "# Secure Dev Workspace — Environment Variables".to_string(),
        HEAVY.to_string(),
        copy_line,
        "# Everything else is optional and can be configured later.".to_string(),
        "# Generated from rust/core/src/workspace/env_schema.rs (and plugins):".to_string(),
        "# edit the schema, then run `make env-example`.".to_string(),
        HEAVY.to_string(),
    ];
    for group in groups {
        out.push(String::new());
        out.push(RULE.to_string());
        out.push(format!("# {}", group.title));
        out.push(RULE.to_string());
        for v in &group.vars {
            out.push(String::new());
            for line in &v.doc {
                out.push(if line.is_empty() {
                    "#".to_string()
                } else {
                    format!("# {line}")
                });
            }
            let prefix = if v.opt_in { "# " } else { "" };
            out.push(format!("{prefix}{}={}", v.name, v.example));
        }
    }
    format!("{}\n", out.join("\n"))
}

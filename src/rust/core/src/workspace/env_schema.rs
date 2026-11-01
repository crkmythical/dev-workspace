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
            title: "IMAGE BUILD — used by `docker compose build`".to_string(),
            vars: vec![
                var(
                    "BASE_IMAGE",
                    &[
                        "Base image:",
                        "  kalilinux/kali-rolling  — Kali apt repositories and tools (larger)",
                        "  debian:trixie-slim      — lean base",
                    ],
                    "debian:trixie-slim",
                ),
                var(
                    "WORKSPACE_VARIANT",
                    &[
                        "Image tag suffix, to tell builds apart in the local registry:",
                        "  latest  → dev-workspace:latest (Kali builds)",
                        "  trixie  → dev-workspace:trixie (Debian builds)",
                    ],
                    "trixie",
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
                        "  mirrors.tuna.tsinghua.edu.cn     a fast mirror inside mainland China",
                        "The redirector can pick a very slow mirror from some networks (~14 KB/s was seen).",
                    ],
                    "http.kali.org",
                ),
                var(
                    "DESKTOP_STACK",
                    &[
                        "Remote desktop(s) to install:",
                        "  kasmvnc  KasmVNC only, at /vnc/ (default; small, builds in seconds)",
                        "  selkies  Selkies only, at /desktop/ (H.264; ~420 MB venv, slow build layer)",
                        "  both     Selkies and KasmVNC, in parallel",
                    ],
                    "kasmvnc",
                ),
            ],
        },
        EnvGroup {
            title: "ACCESS".to_string(),
            vars: vec![
                var(
                    "PASSWORD",
                    &[
                        "Master password: code-server, the desktops AND the vault share this one secret.",
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
                        "The backup holds the PLAINTEXT workspace (.credentials/ included), protected only by",
                        "RESTIC_PASSWORD, and self-destruct does not delete it: treat the repository as part of",
                        "the vault.",
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
                            "Enabling writes the vault passphrase to /workspace/.credentials/sync-passphrase on",
                            "every unlock (that file is the sync key source). Disabling removes it on the next unlock.",
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
                        "EGRESS_ENFORCE",
                        &[
                            "Fail-closed egress. Installs an nftables default-DROP filter so ONLY loopback and",
                            "Clash's own uid may leave the box: a process that ignores the proxy, speaks UDP/QUIC,",
                            "or runs under `noproxy` is blocked, and DNS cannot leak past Clash. Off (default) is",
                            "the best-effort proxy env vars only. Requires the container to hold CAP_NET_ADMIN",
                            "(docker-compose.yml grants it); if asked for but unavailable, the boot fails closed.",
                        ],
                        "1",
                    )
                },
            ],
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
        "# Copy to `.env` and fill in. Nothing is required.".to_string()
    } else {
        let verb = if required.len() > 1 { "are" } else { "is" };
        format!(
            "# Copy to `.env` and fill in. Only {} {verb} required.",
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

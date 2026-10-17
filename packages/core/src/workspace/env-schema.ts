/**
 * The user-facing environment as DATA: every variable, what it does, and what the
 * example file shows for it.
 *
 * `.env.example` is generated from this (`make env-example`) and a test fails when the
 * file, the code that reads the environment, or docker-compose.yml disagree with it, so
 * the documented variables cannot drift from the real ones the way a hand-written
 * example did (a documented `VAULT_SYNC_INTERVAL` that nothing read; `DESKTOP_STACK`
 * and `DESKTOP_AUDIO` that nothing documented).
 *
 * Plugins add their own groups (see ../plugins).
 */

export interface EnvVarSpec {
  name: string;
  /** Comment lines written above the variable. */
  doc: readonly string[];
  /**
   * Value shown in the example file. A placeholder, never a credential: secrets are
   * left empty, and a test rejects anything that looks like a real token.
   */
  example: string;
  /** Written as `# NAME=value`: the feature stays off until the user opts in. */
  optIn?: boolean;
  /** The container refuses to start without it. */
  required?: boolean;
  /** Read by a third-party tool rather than by this repo (name it); exempt from the dead-variable check. */
  external?: string;
}

export interface EnvGroup {
  title: string;
  vars: readonly EnvVarSpec[];
}

export const CORE_ENV_GROUPS: readonly EnvGroup[] = [
  {
    title: "REQUIRED — container won't start without this",
    vars: [
      {
        name: "CLASH_SUBSCRIPTION_URL",
        doc: ["Clash-format proxy subscription URL."],
        example: "https://sub.example.com/your-subscription?target=clash",
        required: true,
      },
    ],
  },
  {
    title: "IMAGE BUILD — used by `docker compose build`",
    vars: [
      {
        name: "BASE_IMAGE",
        doc: [
          "Base image:",
          "  kalilinux/kali-rolling  — Kali apt repositories and tools (larger)",
          "  debian:trixie-slim      — lean base",
        ],
        example: "debian:trixie-slim",
      },
      {
        name: "WORKSPACE_VARIANT",
        doc: [
          "Image tag suffix, to tell builds apart in the local registry:",
          "  latest  → dev-workspace:latest (Kali builds)",
          "  trixie  → dev-workspace:trixie (Debian builds)",
        ],
        example: "trixie",
      },
      {
        name: "APT_MIRROR",
        doc: [
          "Apt mirror host for Debian bases (Kali keeps its own repositories).",
          "  mirrors.aliyun.com  fast inside mainland China, very slow elsewhere (default)",
          "  deb.debian.org      the official CDN, for everywhere else",
        ],
        example: "mirrors.aliyun.com",
      },
      {
        name: "DESKTOP_STACK",
        doc: [
          "Remote desktop(s) to install:",
          "  kasmvnc  KasmVNC only, at /vnc/ (default; small, builds in seconds)",
          "  selkies  Selkies only, at /desktop/ (H.264; ~420 MB venv, slow build layer)",
          "  both     Selkies and KasmVNC, in parallel",
        ],
        example: "kasmvnc",
      },
    ],
  },
  {
    title: "ACCESS",
    vars: [
      {
        name: "PASSWORD",
        doc: [
          "Master password: code-server, the desktops AND the vault share this one secret.",
          "If empty, a random one is generated at first start and printed in the start-up banner",
          "(docker logs dev-workspace). That one lives only inside the container: a restart keeps",
          "it, but recreating the container (docker compose up -d after a rebuild or pull, docker",
          "rm) generates a NEW one, and the existing vault then stays locked until you give it the",
          "old one. Save the printed password, or set it here (it then sits in .env and in the",
          "container environment).",
        ],
        example: "",
      },
      {
        name: "TUNNEL_HOST_PORT",
        doc: [
          "Host port (127.0.0.1) the web UI is published on: the Cloudflare Tunnel connects",
          "to it and the start-up banner prints it.",
        ],
        example: "18080",
      },
      {
        name: "TUNNEL_NAME",
        doc: ["Cloudflare tunnel name (host script: scripts/setup-cloudflared.sh)."],
        example: "dev-workspace",
      },
      {
        name: "TUNNEL_HOSTNAME",
        doc: [
          "Public hostname routed to the tunnel (host script: scripts/setup-cloudflared.sh).",
          "Required by that script; there is no default.",
        ],
        example: "workspace.example.com",
        optIn: true,
      },
    ],
  },
  {
    title: "VAULT REPLICATION AND GIT IDENTITY (optional)",
    vars: [
      {
        name: "VAULT_GIT_REPO",
        doc: [
          "Git remote the vault's CIPHERTEXT is replicated to every 30 minutes.",
          "  https://<token>@github.com/user/dev-vault.git  (HTTPS + token, simplest)",
          "  git@github.com:user/dev-vault.git              (SSH, needs a key in the vault)",
          "Leave empty to disable replication. At start-up the cipher directory is made a git",
          "repository with this as its origin; the URL (token included) is kept in its .git/config",
          "on the cipher volume, outside the vault.",
        ],
        example: "",
      },
      {
        name: "GIT_USER_NAME",
        doc: ["Git identity for commits made inside the container."],
        example: "",
      },
      { name: "GIT_USER_EMAIL", doc: [], example: "" },
    ],
  },
  {
    title: "RUNTIME",
    vars: [
      {
        name: "TZ",
        doc: ["Timezone of the container."],
        example: "Asia/Shanghai",
        external: "tzdata",
      },
      {
        name: "MEMORY_RESERVATION",
        doc: ["Docker memory reservation for the container."],
        example: "8g",
      },
      {
        name: "COMPOSE_FILE",
        doc: [
          "Linux hosts with AppArmor (Ubuntu, Debian): Docker's default profile forbids the FUSE",
          'mount, so the vault stays locked ("mount failed: Permission denied"). This adds the',
          "opt-in override docker-compose.apparmor.yml, which lifts the container's AppArmor",
          "confinement, for every `docker compose` command run here. Not needed on Docker Desktop.",
        ],
        example: "docker-compose.yml:docker-compose.apparmor.yml",
        optIn: true,
        external: "docker compose",
      },
    ],
  },
  {
    title: "REMOTE DESKTOP (optional)",
    vars: [
      {
        name: "DESKTOP_RESOLUTION",
        doc: [
          "Resolution of the KasmVNC desktop (/vnc/). Selkies (/desktop/) fits the browser",
          "window itself and ignores this.",
        ],
        example: "1920x1080",
      },
      {
        name: "DESKTOP_AUDIO",
        doc: ["Selkies desktop audio (PulseAudio null sink): 1 to enable, 0 to disable."],
        example: "0",
      },
    ],
  },
  {
    title: "REALTIME BACKUP (optional — S3-compatible store via restic)",
    vars: [
      {
        name: "RESTIC_REPOSITORY",
        doc: [
          "Setting this turns the feature on; empty keeps the backup watcher dormant.",
          "  s3:https://s3.us-east-1.amazonaws.com/my-bucket/workspace-backup",
          "  s3:https://<account-id>.r2.cloudflarestorage.com/<bucket>/prefix   (Cloudflare R2)",
          "A repository that does not exist yet is created at start-up.",
          "The backup holds the PLAINTEXT workspace (.credentials/ included), protected only by",
          "RESTIC_PASSWORD, and self-destruct does not delete it: treat the repository as part of",
          "the vault.",
        ],
        example: "",
      },
      {
        name: "RESTIC_PASSWORD",
        doc: ["Repository encryption password (loss = unrecoverable repository)."],
        example: "",
      },
      {
        name: "AWS_ACCESS_KEY_ID",
        doc: [
          "S3 credentials, as restic reads them. These are the standard AWS variables: if you",
          "use other AWS services with different credentials, isolate them (AWS_PROFILE).",
        ],
        example: "",
      },
      { name: "AWS_SECRET_ACCESS_KEY", doc: [], example: "" },
      {
        name: "AWS_DEFAULT_REGION",
        doc: ["Custom region (restic default: us-east-1)."],
        example: "us-east-1",
        optIn: true,
        external: "restic",
      },
    ],
  },
  {
    title: "OPTIONAL FEATURES — all OFF unless enabled here",
    vars: [
      {
        name: "SYNC_ENABLED",
        doc: [
          "Browser file-sync page (/sync/) and its upload/list API.",
          "Enabling writes the vault passphrase to /workspace/.credentials/sync-passphrase on",
          "every unlock (that file is the sync key source). Disabling removes it on the next unlock.",
        ],
        example: "1",
        optIn: true,
      },
      {
        name: "REMOTE_DESTRUCT_ENABLED",
        doc: [
          "Remote self-destruct endpoint (POST /sync/api/destruct).",
          "Enabling stores a bcrypt hash of a key derived from the vault passphrase; the key is",
          "printed once and `destruct-passphrase` re-derives it at any time. Disabling removes the",
          "hash on the next unlock.",
        ],
        example: "1",
        optIn: true,
      },
    ],
  },
];

/** Names the code sets or reads that are not meant to be configured by the user. */
export const INTERNAL_ENV: readonly string[] = [
  // Set by the start-up for Caddy's basic_auth; derived from the master password.
  "DESKTOP_BCRYPT_HASH",
];

export const flattenEnv = (groups: readonly EnvGroup[]): EnvVarSpec[] =>
  groups.flatMap((g) => [...g.vars]);

const RULE = "# ----------------------------------------------------------";
const HEAVY = "# ============================================================";

/** The text of `.env.example`. */
export function renderEnvExample(groups: readonly EnvGroup[]): string {
  const required = flattenEnv(groups)
    .filter((v) => v.required)
    .map((v) => v.name);
  const out: string[] = [
    HEAVY,
    "# Secure Dev Workspace — Environment Variables",
    HEAVY,
    `# Copy to \`.env\` and fill in. ${
      required.length > 0
        ? `Only ${required.join(", ")} ${required.length > 1 ? "are" : "is"} required.`
        : "Nothing is required."
    }`,
    "# Everything else is optional and can be configured later.",
    "# Generated from packages/core/src/workspace/env-schema.ts (and plugins):",
    "# edit the schema, then run `make env-example`.",
    HEAVY,
  ];

  for (const group of groups) {
    out.push("", RULE, `# ${group.title}`, RULE);
    for (const v of group.vars) {
      out.push("");
      for (const line of v.doc) out.push(line === "" ? "#" : `# ${line}`);
      out.push(`${v.optIn ? "# " : ""}${v.name}=${v.example}`);
    }
  }
  return `${out.join("\n")}\n`;
}

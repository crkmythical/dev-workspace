//! The unprivileged side of the container (pure policy; the process plumbing is in infra).
//!
//! Everything the user runs (code-server and its terminals, the desktops and their applications,
//! build tools, whatever a package's install script does) runs as [`WORKLOAD_USER`], not root. The
//! services that hold the secrets and guard the perimeter (the gate, Clash, Caddy, the backup and
//! sync jobs, supervisord) keep their own identities. So code in the workspace cannot read the
//! master password, the backup and subscription credentials, the session key or the audit log, and
//! cannot reconfigure the gate or the egress firewall.
//!
//! Two pieces of policy live here:
//!
//! * which environment variables a workload process inherits. The container's environment holds the
//!   secrets (`PASSWORD`, `RESTIC_PASSWORD`, `CLASH_SUBSCRIPTION_URL`, ...), and a child inherits it
//!   by default, so the set is an ALLOWLIST: a variable is passed only if it is named here, or the
//!   user asked for it by name in `WORKLOAD_ENV`;
//! * which `sdw` commands need root, and so are reached from the workload through the bridge.

use crate::workspace::secrets::is_secret_var;

pub const WORKLOAD_USER: &str = "dev";
pub const WORKLOAD_UID: u32 = 1000;
pub const WORKLOAD_GID: u32 = 1000;
/// The workload's home: inside the vault, so its shell history, tool caches and configuration are
/// encrypted at rest, backed up and gone when the vault is locked.
pub const WORKLOAD_HOME: &str = "/workspace/home";

/// Where tools preinstalled in the image live (mise "system" installs): read-only to the workload,
/// which installs its own into its home.
pub const SYSTEM_TOOLS_DIR: &str = "/opt/mise";

/// `PATH` of a workload process: its own tools, the image's system tools, then the system.
pub const WORKLOAD_PATH: &str = "/workspace/home/.local/bin:/workspace/home/.local/share/mise/shims:/opt/mise/shims:/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin";

/// Container-local directory of root-only runtime state (the sync key, the saved password, ...).
/// 0700, root: the workload cannot even list it.
pub const PRIVATE_RUN_DIR: &str = "/run/sdw";
/// Where a password the start-up generated is kept so a restart finds the vault key again.
pub const GENERATED_PASSWORD_PATH: &str = "/var/lib/sdw/master-password";

/// The container's environment as of its start, which root-only tools read through the bridge: the
/// bridge starts them with a clean environment, and they need the configuration (`RESTIC_*`,
/// `CLASH_SUBSCRIPTION_URL`, feature flags) the container was given.
pub const CONTAINER_ENV_SOURCE: &str = "/proc/1/environ";

/// Variables a workload keeps from its parent as they are.
const KEEP: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "TZ",
    "TERM",
    "COLORTERM",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "JAVA_HOME",
    "DISPLAY",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "PULSE_SERVER",
    "PULSE_LOG",
    "DESKTOP_RESOLUTION",
    "DESKTOP_AUDIO",
];

/// ... and any variable that starts with one of these (locale, the Selkies settings).
const KEEP_PREFIXES: &[&str] = &["LC_", "SELKIES_"];

/// `base` (a `no_proxy` list) with `extra` appended, each entry once.
fn merge_no_proxy(base: &str, extra: &[String]) -> String {
    let mut entries: Vec<&str> = base
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    for item in extra {
        if !entries.contains(&item.as_str()) {
            entries.push(item);
        }
    }
    entries.join(",")
}

/// The environment a workload process starts with.
///
/// `parent` is the environment of the process that launches it; `requested` the names the user put
/// in `WORKLOAD_ENV`; `direct_networks` the networks the egress firewall lets it reach without the
/// proxy (`EGRESS_ALLOW_CIDRS`), which are added to `no_proxy` so its tools go there directly
/// instead of handing them to Clash, whose proxy node is on some other network. Returns the
/// environment, and the requested names that were refused because they are workspace secrets (the
/// caller says so; a secret is never passed on request).
pub fn workload_env(
    parent: impl IntoIterator<Item = (String, String)>,
    requested: &[String],
    home: &str,
    direct_networks: &[String],
) -> (Vec<(String, String)>, Vec<String>) {
    let mut refused: Vec<String> = Vec::new();
    for name in requested {
        if refuses(name) && !refused.contains(name) {
            refused.push(name.clone());
        }
    }
    let mut env: Vec<(String, String)> = parent
        .into_iter()
        .filter(|(name, _)| {
            if refuses(name) {
                return false;
            }
            KEEP.contains(&name.as_str())
                || KEEP_PREFIXES.iter().any(|p| name.starts_with(p))
                || requested.iter().any(|r| r == name)
        })
        .collect();
    env.retain(|(name, _)| !is_set_by_us(name));
    if !direct_networks.is_empty() {
        let current = ["no_proxy", "NO_PROXY"]
            .iter()
            .find_map(|n| env.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()))
            .unwrap_or_else(|| "localhost,127.0.0.0/8".to_string());
        let merged = merge_no_proxy(&current, direct_networks);
        env.retain(|(name, _)| name != "no_proxy" && name != "NO_PROXY");
        env.push(("no_proxy".to_string(), merged.clone()));
        env.push(("NO_PROXY".to_string(), merged));
    }
    for (name, value) in [
        ("HOME", home.to_string()),
        ("USER", WORKLOAD_USER.to_string()),
        ("LOGNAME", WORKLOAD_USER.to_string()),
        ("SHELL", "/bin/bash".to_string()),
        ("PATH", WORKLOAD_PATH.to_string()),
        ("MISE_DATA_DIR", format!("{home}/.local/share/mise")),
        ("MISE_SYSTEM_DATA_DIR", SYSTEM_TOOLS_DIR.to_string()),
    ] {
        env.push((name.to_string(), value));
    }
    env.sort();
    (env, refused)
}

/// A variable the workload may never get, whatever is asked: the workspace's own secrets and the
/// `<NAME>_FILE` variables that name the files holding them.
fn refuses(name: &str) -> bool {
    is_secret_var(name)
        || name.strip_suffix("_FILE").is_some_and(is_secret_var)
        || name == "DESKTOP_BCRYPT_HASH"
}

/// Variables the launcher sets itself, so a parent's value never wins.
fn is_set_by_us(name: &str) -> bool {
    matches!(
        name,
        "HOME" | "USER" | "LOGNAME" | "SHELL" | "PATH" | "MISE_DATA_DIR" | "MISE_SYSTEM_DATA_DIR"
    )
}

/// The `WORKLOAD_ENV` value as names: comma or space separated, blanks dropped.
pub fn parse_requested_names(raw: &str) -> Vec<String> {
    raw.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Commands that need root, so a workload reaches them through the bridge: `doas` runs a symlink of
/// the same name under [`PRIV_BRIDGE_DIR`] as root, with a clean environment, and nothing else
/// (`/etc/doas.conf` lists exactly these paths). Everything that mounts, kills, installs packages,
/// talks to supervisord or reads the secrets is here. Deliberately NOT here: `backup-init`,
/// `destruct-passphrase` and `self-destruct`, which are for the operator (`docker exec`), not for
/// code running in the workspace.
pub const PRIVILEGED_VERBS: &[&str] = &[
    "audit-log",
    "desktop-install",
    "desktop-start",
    "desktop-stop",
    "doctor",
    "init-pentest",
    "init-vault",
    "lock-pentest",
    "lock-vault",
    "pentest",
    "pentest-bootstrap",
    "pull",
    "push",
    "set-clash-dns",
    "setup",
    "unlock-pentest",
    "unlock-vault",
];

/// The symlinks `doas` is allowed to run, each pointing at `sdw`.
pub const PRIV_BRIDGE_DIR: &str = "/usr/local/libexec/sdw-priv";

/// The line-per-verb rules for /etc/doas.conf: no password (the caller is already inside the
/// container; this is a bridge, not a login), only these exact paths, any arguments.
pub fn doas_rules() -> String {
    let mut out = String::from(
        "# Generated from PRIVILEGED_VERBS (core/src/workspace/workload.rs); `sdw doas-rules` prints it.\n",
    );
    for verb in PRIVILEGED_VERBS {
        out.push_str(&format!(
            "permit nopass {WORKLOAD_USER} as root cmd {PRIV_BRIDGE_DIR}/{verb}\n"
        ));
    }
    out
}

/// The opt-in `WORKLOAD_ROOT` rule, appended to /etc/doas.conf at boot ONLY when the operator sets
/// WORKLOAD_ROOT. It lets the workload user run anything as root with no password (`keepenv`, so the
/// proxy vars and PATH carry into the root shell — `apt` etc. still go through Clash). This
/// deliberately drops the dev/root confinement, so it is OFF by default; the confinement-preserving
/// way to get root is `docker exec -u root` from the trusted host.
pub fn workload_root_rule() -> String {
    format!("permit nopass keepenv {WORKLOAD_USER} as root\n")
}

/// Whether `argv0` is a bridge entry (`/usr/local/libexec/sdw-priv/<verb>`), which is how a root
/// process knows it was started on behalf of a workload.
pub fn is_bridge_invocation(argv0: &str) -> bool {
    argv0
        .strip_prefix(PRIV_BRIDGE_DIR)
        .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn get<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_workload_gets_what_it_needs_and_none_of_the_secrets() {
        let parent = env(&[
            ("PASSWORD", "master"),
            ("PASSWORD_FILE", "/run/secrets/password"),
            ("RESTIC_PASSWORD", "r"),
            ("AWS_SECRET_ACCESS_KEY", "a"),
            ("CLASH_SUBSCRIPTION_URL", "https://sub"),
            ("DESKTOP_BCRYPT_HASH", "$2b$"),
            ("PENTEST_VAULT_PASSWORD", "p"),
            ("http_proxy", "http://127.0.0.1:7890"),
            ("LANG", "en_US.UTF-8"),
            ("LC_ALL", "C"),
            ("SELKIES_ENCODER", "x264enc"),
            ("DISPLAY", ":2"),
            ("HOME", "/root"),
            ("PATH", "/root/.local/bin:/usr/bin"),
            ("SOMETHING_ELSE", "x"),
        ]);
        let (out, refused) = workload_env(parent, &[], "/workspace/home", &[]);
        for secret in [
            "PASSWORD",
            "PASSWORD_FILE",
            "RESTIC_PASSWORD",
            "AWS_SECRET_ACCESS_KEY",
            "CLASH_SUBSCRIPTION_URL",
            "DESKTOP_BCRYPT_HASH",
            "PENTEST_VAULT_PASSWORD",
        ] {
            assert_eq!(
                get(&out, secret),
                None,
                "{secret} must not reach a workload"
            );
        }
        assert_eq!(get(&out, "http_proxy"), Some("http://127.0.0.1:7890"));
        assert_eq!(get(&out, "LANG"), Some("en_US.UTF-8"));
        assert_eq!(get(&out, "LC_ALL"), Some("C"));
        assert_eq!(get(&out, "SELKIES_ENCODER"), Some("x264enc"));
        assert_eq!(get(&out, "DISPLAY"), Some(":2"));
        // Not named, so not passed.
        assert_eq!(get(&out, "SOMETHING_ELSE"), None);
        // The launcher sets identity itself, whatever the parent had.
        assert_eq!(get(&out, "HOME"), Some("/workspace/home"));
        assert_eq!(get(&out, "USER"), Some("dev"));
        assert_eq!(get(&out, "PATH"), Some(WORKLOAD_PATH));
        assert_eq!(
            get(&out, "MISE_DATA_DIR"),
            Some("/workspace/home/.local/share/mise")
        );
        assert!(refused.is_empty());
    }

    #[test]
    fn a_variable_can_be_requested_by_name_but_a_secret_never_is() {
        let parent = env(&[
            ("GITHUB_TOKEN", "ghp_x"),
            ("OPENAI_API_KEY", "sk-x"),
            ("PASSWORD", "master"),
            ("RESTIC_PASSWORD_FILE", "/run/secrets/restic"),
            ("UNREQUESTED", "x"),
        ]);
        let requested =
            parse_requested_names("GITHUB_TOKEN, OPENAI_API_KEY PASSWORD,RESTIC_PASSWORD_FILE,,");
        let (out, refused) = workload_env(parent, &requested, "/workspace/home", &[]);
        assert_eq!(get(&out, "GITHUB_TOKEN"), Some("ghp_x"));
        assert_eq!(get(&out, "OPENAI_API_KEY"), Some("sk-x"));
        assert_eq!(get(&out, "UNREQUESTED"), None);
        assert_eq!(get(&out, "PASSWORD"), None);
        assert_eq!(get(&out, "RESTIC_PASSWORD_FILE"), None);
        assert_eq!(refused, ["PASSWORD", "RESTIC_PASSWORD_FILE"]);
    }

    #[test]
    fn requested_names_parse_whatever_the_separator() {
        assert_eq!(
            parse_requested_names("A,B C\tD\nE"),
            ["A", "B", "C", "D", "E"]
        );
        assert!(parse_requested_names("  , ,").is_empty());
        assert!(parse_requested_names("").is_empty());
    }

    #[test]
    fn the_environment_is_sorted_and_has_no_duplicates() {
        let (out, _) = workload_env(env(&[("TERM", "xterm"), ("LANG", "C")]), &[], "/h", &[]);
        let names: Vec<_> = out.iter().map(|(k, _)| k.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);

        // ... also when no_proxy is rewritten for the allowlist.
        let direct = vec!["10.0.0.5".to_string()];
        let (out, _) = workload_env(
            env(&[("no_proxy", "localhost"), ("NO_PROXY", "localhost")]),
            &[],
            "/h",
            &direct,
        );
        let names: Vec<_> = out.iter().map(|(k, _)| k.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
    }

    #[test]
    fn allowed_networks_are_added_to_no_proxy_without_dropping_what_was_there() {
        let direct = vec!["10.0.0.5".to_string(), "203.0.113.0/24".to_string()];
        let parent = env(&[("no_proxy", "localhost,127.0.0.0/8,10.0.0.0/8")]);
        let (out, _) = workload_env(parent, &[], "/h", &direct);
        let expected = "localhost,127.0.0.0/8,10.0.0.0/8,10.0.0.5,203.0.113.0/24";
        assert_eq!(get(&out, "no_proxy"), Some(expected));
        assert_eq!(get(&out, "NO_PROXY"), Some(expected));

        // An entry already there is not repeated.
        let parent = env(&[("NO_PROXY", "localhost,10.0.0.5")]);
        let (out, _) = workload_env(parent, &[], "/h", &direct);
        assert_eq!(
            get(&out, "no_proxy"),
            Some("localhost,10.0.0.5,203.0.113.0/24")
        );

        // No parent list: start from loopback.
        let (out, _) = workload_env(env(&[]), &[], "/h", &direct);
        assert_eq!(
            get(&out, "no_proxy"),
            Some("localhost,127.0.0.0/8,10.0.0.5,203.0.113.0/24")
        );
    }

    #[test]
    fn without_an_allowlist_no_proxy_is_left_as_it_was() {
        let (out, _) = workload_env(env(&[("no_proxy", "a,b")]), &[], "/h", &[]);
        assert_eq!(get(&out, "no_proxy"), Some("a,b"));
        assert_eq!(get(&out, "NO_PROXY"), None);
        let (out, _) = workload_env(env(&[]), &[], "/h", &[]);
        assert_eq!(get(&out, "no_proxy"), None);
    }

    #[test]
    fn the_doas_rules_name_each_verb_exactly_once_and_nothing_else() {
        let rules = doas_rules();
        let permits: Vec<&str> = rules.lines().filter(|l| l.starts_with("permit")).collect();
        assert_eq!(permits.len(), PRIVILEGED_VERBS.len());
        for verb in PRIVILEGED_VERBS {
            assert!(
                rules.contains(&format!(
                    "permit nopass dev as root cmd /usr/local/libexec/sdw-priv/{verb}\n"
                )),
                "{verb}"
            );
        }
        // No wildcard, no shell, no `keepenv`: the environment stays reset.
        assert!(!rules.contains("keepenv"));
        assert!(!rules.contains("setenv"));
        assert!(!rules
            .lines()
            .any(|l| l.contains(" cmd ") && !l.contains("/sdw-priv/")));
    }

    #[test]
    fn operator_commands_are_not_reachable_from_the_workload() {
        for verb in [
            "self-destruct",
            "destruct-passphrase",
            "backup-init",
            "entrypoint",
            "sync-service",
        ] {
            assert!(!PRIVILEGED_VERBS.contains(&verb), "{verb}");
        }
    }

    #[test]
    fn a_bridge_invocation_is_recognised_by_its_path() {
        assert!(is_bridge_invocation("/usr/local/libexec/sdw-priv/doctor"));
        assert!(!is_bridge_invocation("/usr/local/bin/doctor"));
        assert!(!is_bridge_invocation("/usr/local/libexec/sdw-priv"));
        assert!(!is_bridge_invocation("/usr/local/libexec/sdw-privileged/x"));
        assert!(!is_bridge_invocation("doctor"));
    }
}

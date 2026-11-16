//! entrypoint — the container boot orchestrator. A port of packages/cli/src/entrypoint-main.ts.
//!
//! Binds each core BOOT_PLAN step to its infra effects and runs run_boot, then (the final step)
//! spawns supervisord and forwards SIGTERM/SIGINT to it so a `docker stop` or self-destruct stops
//! every program in order. The pure orchestration (order, abort/degrade, vault gating) is in core.

use crate::commands::system_env;
use sdw_core::audit::{event, outcome, AuditEvent};
use sdw_core::backup::domain::RepoProbe;
use sdw_core::desktop::domain::DesktopTarget;
use sdw_core::desktop::layout::{DESKTOP_CACHE_DIR, SELKIES_HOME, VNC_HOME};
use sdw_core::egress::allowlist::AllowList;
use sdw_core::egress::domain::{EgressStatus, DEFAULT_EGRESS_TIMING};
use sdw_core::egress::layout::{CLASH_HTTP_PORT, CLASH_USER};
use sdw_core::egress::usecases::bring_up_egress;
use sdw_core::plugins::all_vaults;
use sdw_core::sync::layout::SYNC_PASSPHRASE_PATH;
use sdw_core::vault::destruct::{DESTRUCT_COMPLETED_PATH, DESTRUCT_STATE_DIR};
use sdw_core::vault::domain::cipher_dir_mode;
use sdw_core::vault::domain::{explain_unlock_failure, VAULT_GITATTRIBUTES, VAULT_GITIGNORE};
use sdw_core::vault::layout::VAULT_CIPHER_DIR;
use sdw_core::vault::ports::UnlockHook;
use sdw_core::vault::specs::workspace_vault;
use sdw_core::vault::usecases::{unlock_vault, UnlockDeps, UnlockRejectReason, UnlockResult};
use sdw_core::workspace::auth::{resolve_master_password, PasswordSource};
use sdw_core::workspace::banner::{render_banner, BannerInput};
use sdw_core::workspace::boot::{run_boot, BootState, BOOT_PLAN};
use sdw_core::workspace::config::{
    env_validation_error, find_missing_required_env, load_config, WorkspaceConfig,
    REQUIRED_ENV_VARS,
};
use sdw_core::workspace::features::is_recognised_switch;
use sdw_core::workspace::inotify::{inotify_advice, WATCHES_PATH, WATCHES_WANTED};
use sdw_core::workspace::layout::{
    code_server_selected, supervisor_conf, EDITOR_SUITE_PATH, SUPERVISORD_BIN, SUPERVISORD_CONF,
};
use sdw_core::workspace::workload::{
    workload_root_rule, GENERATED_PASSWORD_PATH, PRIVATE_RUN_DIR, WORKLOAD_GID, WORKLOAD_UID,
};
use sdw_infra::audit::record_default;
use sdw_infra::caddy::caddy_password_hash;
use sdw_infra::clash::{write_config, ConfigOptions, SecretPolicy};
use sdw_infra::cmd;
use sdw_infra::desktop::installed_stacks;
use sdw_infra::egress::{
    apply_egress_firewall, clash_egress_ports, prepare_clash_dir, read_egress_status,
    remove_ssh_proxy_config, resolve_uid, sync_egress_notice, write_egress_status,
    write_ssh_proxy_config, NoticeResult,
};
use sdw_infra::gocryptfs::{cleanup_stale_mount, init_vault, is_initialized, GocryptfsVolume};
use sdw_infra::replication::{ensure_cipher_repo, EnsureResult};
use sdw_infra::restic::{init_repository, probe_repository};
use sdw_infra::secrets::random_hex;
use sdw_infra::supervisor::enable_autostart;
use sdw_infra::time::now_ms;
use sdw_infra::unlock_hooks::workspace_unlock_hooks;
use std::fs;
use std::path::Path;
use std::process::Command;

/// What resolve-auth figures out; the later steps read it.
struct Auth {
    master_password: String,
    password_source: Option<PasswordSource>,
}

fn announce(message: &str) {
    println!("{message}");
}

/// 32 hex chars = 16 bytes from /dev/urandom, used as the master password when PASSWORD is unset.
/// This string becomes the gocryptfs vault passphrase, so there is NO low-entropy fallback: if
/// /dev/urandom cannot be read we abort the boot rather than mint a guessable vault key.
fn generate_password() -> String {
    match random_hex(16) {
        Ok(password) => password,
        Err(e) => {
            eprintln!("FATAL: cannot generate a password: {e}");
            eprintln!(
                "Refusing to create a vault with a guessable master key; set PASSWORD in .env."
            );
            std::process::exit(1);
        }
    }
}

/// Root-only runtime state, and the ciphertext kept from the unprivileged workload.
fn prepare_run_dirs() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let private = |dir: &str| -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{dir}: {e}"))
    };
    private(PRIVATE_RUN_DIR)?;
    private("/var/lib/sdw")?;
    // The remote-destruct key hash and the completion marker: a volume, so it outlives the
    // container, and root's alone, so code in the workspace can neither read the hash nor forge
    // the marker.
    private(DESTRUCT_STATE_DIR)?;
    // A key source an earlier run left behind means nothing until THIS start has unlocked the vault.
    let _ = fs::remove_file(SYNC_PASSPHRASE_PATH);
    // The ciphertext holds the passphrase-wrapped master key (gocryptfs.conf), which a guesser could
    // work on offline, so the directory is closed to everyone but root. (A vault that other users
    // must be able to enter, the pentest rootfs, can only be opened as far as 'traverse'.)
    for vault in all_vaults() {
        if Path::new(&vault.cipher_dir).is_dir() {
            let mode = cipher_dir_mode(&vault);
            fs::set_permissions(&vault.cipher_dir, fs::Permissions::from_mode(mode))
                .map_err(|e| format!("{}: {e}", vault.cipher_dir))?;
        }
    }
    Ok(())
}

fn git_config_system(key: &str, value: &str) {
    if !cmd::ok("git", &["config", "--system", key, value]) {
        eprintln!("WARNING: git config {key} failed.");
    }
}

/// Opt-in via WORKLOAD_ROOT (1/true/yes/on): let the workload user become root in its own terminal
/// (`root`, or `doas <cmd>`) with no password. OFF by default, so the dev/root confinement — a
/// compromise of the workload cannot escalate — holds unless the operator asks for this.
fn workload_root_enabled() -> bool {
    match std::env::var("WORKLOAD_ROOT") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// When WORKLOAD_ROOT is on, append its one doas rule to /etc/doas.conf (idempotent). The `root`
/// helper (baked into the image) then gives the workload user a root shell; without the rule, doas
/// denies it and the confinement is the default.
fn apply_workload_root() {
    if !workload_root_enabled() {
        return;
    }
    let rule = workload_root_rule();
    let path = "/etc/doas.conf";
    if fs::read_to_string(path)
        .unwrap_or_default()
        .contains(rule.trim())
    {
        return;
    }
    use std::io::Write;
    let result = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(rule.as_bytes()));
    match result {
        Ok(()) => println!(
            "WORKLOAD_ROOT: 'dev' can become root in its terminal (type `root`); dev/root \
             confinement relaxed by request."
        ),
        Err(e) => eprintln!("WARNING: WORKLOAD_ROOT set but could not update {path}: {e}"),
    }
}

/// The fail-closed egress firewall: ON unless `EGRESS_ENFORCE` says no.
///
/// When on, a failure to load it aborts the boot (the caller's step is Abort): booting while
/// believing egress is enforced is worse than not booting. The error says how to run without it.
fn enforce_egress(config: &WorkspaceConfig) -> Result<(), String> {
    if let Ok(raw) = std::env::var("EGRESS_ENFORCE") {
        if !is_recognised_switch(&raw) {
            eprintln!(
                "WARNING: EGRESS_ENFORCE='{raw}' is not 1/true/yes/on or 0/false/no/off; \
                 treating it as ON (only a clear 'no' turns the egress firewall off)."
            );
        }
    }

    if !config.features.egress_enforce {
        // ssh would otherwise keep the proxy settings of an earlier, enforced start.
        remove_ssh_proxy_config();
        eprintln!(
            "WARNING: the egress firewall is OFF (EGRESS_ENFORCE=0). Only the proxy variables \
             point processes at Clash: anything that ignores them, speaks UDP/QUIC or resolves \
             names itself connects directly and shows this machine's real address."
        );
        return Ok(());
    }

    // An invalid allowlist stops the boot: a typo must not silently open the wrong block.
    let allow = AllowList::parse(config.egress_allow_cidrs.as_deref().unwrap_or(""))
        .map_err(|e| format!("EGRESS_ALLOW_CIDRS: {e}"))?;

    // Clash must run as its own uid for the firewall to single out its traffic. (It always does;
    // bring-up-egress gives it its config dir and starts it.)
    let uid = resolve_uid(CLASH_USER)
        .ok_or_else(|| format!("egress enforce: user '{CLASH_USER}' not found"))?;

    // Force the system resolver through Clash so no lookup leaks past it. Best-effort: the
    // firewall is the security boundary; this only keeps hostname DNS working under it.
    if fs::write("/etc/resolv.conf", "nameserver 127.0.0.1\n").is_err() {
        eprintln!(
            "WARNING: could not point /etc/resolv.conf at Clash; hostname DNS may be blocked by \
             the egress filter until fixed."
        );
    }

    // Load the ruleset. On failure (no NET_ADMIN / no nft) this Errs and, as an Abort step, fails
    // the boot closed rather than booting while believing egress is enforced.
    let backend = apply_egress_firewall(uid, &allow).map_err(|e| {
        format!(
            "{e}. The egress firewall is on by default and needs the NET_ADMIN capability and a \
             working nftables or iptables-legacy; to run without it set EGRESS_ENFORCE=0 (see the \
             README)."
        )
    })?;

    // ssh ignores the proxy variables: send it through Clash too. A usability matter, not a leak
    // (without it ssh is dropped), so a failure to write the file only warns.
    if let Err(e) = write_ssh_proxy_config() {
        eprintln!("WARNING: ssh will not reach its remote through Clash: {e}");
    }

    println!(
        "Egress firewall enforced via {} (fail-closed; direct egress only for Clash uid {uid}{}).",
        backend.label(),
        if allow.is_empty() {
            String::new()
        } else {
            format!(" and {} allowed network(s)", allow.len())
        }
    );
    Ok(())
}

/// The desktop sessions' runtime and cache directory: the workload's, and nobody else's.
fn prepare_desktop_cache() -> Result<(), String> {
    use std::os::unix::fs::{chown, PermissionsExt};
    fs::create_dir_all(DESKTOP_CACHE_DIR).map_err(|e| e.to_string())?;
    fs::set_permissions(DESKTOP_CACHE_DIR, fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    chown(DESKTOP_CACHE_DIR, Some(WORKLOAD_UID), Some(WORKLOAD_GID)).map_err(|e| e.to_string())
}

fn run_step(
    id: &str,
    st: &mut BootState,
    config: &WorkspaceConfig,
    auth: &mut Auth,
) -> Result<(), String> {
    match id {
        "preflight" => {
            // A secret given as <NAME>_FILE must be readable: refuse to boot rather than carry on
            // without it (a missing PASSWORD would turn into a generated one).
            let unreadable = sdw_infra::secrets::unreadable_secret_files();
            if !unreadable.is_empty() {
                for e in &unreadable {
                    eprintln!("{e}");
                }
                return Err("a secret file cannot be read".to_string());
            }
            // Defense in depth behind scripts/entrypoint.sh.
            if let Some(err) =
                env_validation_error(&find_missing_required_env(system_env, REQUIRED_ENV_VARS))
            {
                eprintln!("{err}");
                return Err("missing required environment".to_string());
            }
            // supervisord expands %(ENV_DESKTOP_RESOLUTION)s and fails to start the desktop if it
            // is unset; set the safety net here (supervisord inherits this env).
            if std::env::var("DESKTOP_RESOLUTION").is_err() {
                std::env::set_var("DESKTOP_RESOLUTION", &config.desktop_resolution);
            }
            Ok(())
        }
        "prepare-run-dirs" => {
            prepare_run_dirs()?;
            apply_workload_root();
            Ok(())
        }
        "cleanup-stale-mounts" => {
            for vault in all_vaults() {
                cleanup_stale_mount(&vault.mount_point);
            }
            Ok(())
        }
        "render-clash-config" => {
            // The template belongs to the image and preflight has required the subscription URL,
            // so there is nothing legitimate to skip: a failure is reported (a degraded step)
            // rather than quietly starting Clash on whatever config an older run left behind.
            if config.clash_subscription_url.is_empty() {
                return Err("CLASH_SUBSCRIPTION_URL is empty".to_string());
            }
            let (dns, source) = crate::commands::clash_dns::resolve_dns();
            write_config(&ConfigOptions {
                subscription_url: &config.clash_subscription_url,
                dns_nameservers: &dns,
                allow_lan: false,
                // A new start, so no running Clash holds the old secret.
                secret: SecretPolicy::Rotate,
            })?;
            println!("Clash config generated from template (DNS: {dns} [{source}]).");
            Ok(())
        }
        "git-globals" => {
            // System-wide (/etc/gitconfig), so root's jobs and the workload's own git agree; the
            // workload's ~/.gitconfig can still override what it wants.
            if let Some(name) = &config.git.user_name {
                git_config_system("user.name", name);
            }
            if let Some(email) = &config.git.user_email {
                git_config_system("user.email", email);
            }
            let proxy = format!("http://127.0.0.1:{CLASH_HTTP_PORT}");
            git_config_system("http.proxy", &proxy);
            git_config_system("https.proxy", &proxy);
            Ok(())
        }
        "tune-inotify" => {
            // One try: a container rarely may write /proc/sys. Then say what is actually in force
            // rather than pretend the limit was raised.
            cmd::ok(
                "sysctl",
                &[
                    "-w",
                    &format!("fs.inotify.max_user_watches={WATCHES_WANTED}"),
                ],
            );
            let current = fs::read_to_string(WATCHES_PATH)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok());
            if let Some(advice) = inotify_advice(current) {
                eprintln!("WARNING: {advice}");
            }
            Ok(())
        }
        "enforce-egress" => enforce_egress(config),
        "bring-up-egress" => {
            // Clash always runs as its own unprivileged user, not only under EGRESS_ENFORCE: a
            // proxy that parses subscriptions and talks to the whole internet should not be root.
            // (Under enforcement it is also the one uid the firewall lets out.) It needs to own its
            // config dir to write the provider cache there.
            let clash_uid = prepare_clash_dir();
            let previous = read_egress_status().map(|r| r.status);
            let report = bring_up_egress(
                &clash_egress_ports(announce, clash_uid),
                DEFAULT_EGRESS_TIMING,
            );
            if let Err(e) = write_egress_status(&report) {
                eprintln!("WARNING: could not record the egress status: {e}");
            }
            // The audit trail: how this boot's egress compares with the last one seen.
            if previous != Some(report.status) {
                record_default(
                    &AuditEvent::new(now_ms(), event::EGRESS, outcome::CHANGED)
                        .with("from", previous.map_or("none", EgressStatus::as_str))
                        .with("to", report.status.as_str())
                        .with("source", "boot"),
                );
            }
            st.egress = Some(report);
            Ok(())
        }
        "resolve-auth" => {
            // A password an earlier start generated, kept in a root-only file so a restart opens
            // the same vault. There is no copy for code-server or the desktops: the gate is the
            // only login, and nothing the workload can read holds the password.
            let saved = fs::read_to_string(GENERATED_PASSWORD_PATH).ok();
            let password = resolve_master_password(
                config.password.as_deref(),
                saved.as_deref(),
                generate_password,
            )?;
            if password.save {
                sdw_infra::secrets::write_private_file(
                    Path::new(GENERATED_PASSWORD_PATH),
                    &format!("{}\n", password.value),
                )?;
            }
            if password.source == PasswordSource::Generated {
                println!("Generated password: {}", password.value);
                println!("  (set PASSWORD in .env to use a fixed password)");
            }
            auth.master_password = password.value;
            auth.password_source = Some(password.source);

            // bcrypt hash of the master password, exported so the auth gateway (sync-service)
            // can verify logins against it. Set in this process's env so supervisord and its
            // children (sync-service) inherit it. (DESKTOP_BCRYPT_HASH is the historical name.)
            let hash = caddy_password_hash(&auth.master_password);
            if hash.is_empty() {
                eprintln!(
                    "WARNING: could not hash the password for Caddy; desktop auth may not start."
                );
            } else {
                std::env::set_var("DESKTOP_BCRYPT_HASH", hash);
                println!("Auth configured (user: user).");
            }
            Ok(())
        }
        "init-vault" => {
            if is_initialized(VAULT_CIPHER_DIR) {
                return Ok(());
            }
            if init_vault(VAULT_CIPHER_DIR, Some(&auth.master_password)) {
                println!("Vault initialized (password = same as code-server).");
                Ok(())
            } else {
                // A real failure: mark the step degraded (run_boot logs it, the banner lists it)
                // instead of warning and returning Ok, which left the boot looking healthy.
                Err("vault initialization failed".to_string())
            }
        }
        "prepare-replication" => {
            let remote = match &config.git.vault_repo {
                Some(r) if !r.is_empty() => r,
                _ => return Ok(()),
            };
            if !is_initialized(VAULT_CIPHER_DIR) {
                return Ok(());
            }
            let change = ensure_cipher_repo(
                VAULT_CIPHER_DIR,
                remote,
                VAULT_GITIGNORE,
                VAULT_GITATTRIBUTES,
            )?;
            match change {
                EnsureResult::Created => {
                    println!("Vault replication: ciphertext repository created.")
                }
                EnsureResult::RemoteUpdated => {
                    println!("Vault replication: ciphertext repository remote-updated.")
                }
                EnsureResult::Unchanged => {}
            }
            Ok(())
        }
        "unlock-vault" => {
            let pw = auth.master_password.clone();
            let hooks = workspace_unlock_hooks(config.features, announce);
            let hook_refs: Vec<&dyn UnlockHook> = hooks.iter().map(|h| h.as_ref()).collect();
            let result = unlock_vault(
                &workspace_vault(),
                || pw.clone(),
                &UnlockDeps {
                    volume: &GocryptfsVolume,
                    hooks: &hook_refs,
                },
            );
            match result {
                UnlockResult::Unlocked { hook_failures, .. } => {
                    st.vault_mounted = true;
                    println!("Vault unlocked.");
                    for failure in hook_failures {
                        eprintln!("WARNING: post-unlock step failed: {failure}");
                    }
                    Ok(())
                }
                UnlockResult::Noop => {
                    st.vault_mounted = true;
                    println!("Vault already unlocked.");
                    Ok(())
                }
                UnlockResult::Failed { detail } => {
                    // Surface it as a degraded boot step (run_boot logs it, the banner lists it)
                    // rather than warning and returning Ok. The vault stays locked either way, and
                    // the vault-gated steps below are skipped because vault_mounted is still false.
                    let joined = detail.trim().split('\n').collect::<Vec<_>>().join(" | ");
                    let generated = auth.password_source == Some(PasswordSource::Generated);
                    for line in explain_unlock_failure(&detail, generated) {
                        eprintln!("  {line}");
                    }
                    eprintln!("  Run 'unlock-vault' once that is fixed.");
                    Err(if joined.is_empty() {
                        "vault unlock failed".to_string()
                    } else {
                        format!("vault unlock failed: {joined}")
                    })
                }
                UnlockResult::Rejected { reason } => {
                    let r = match reason {
                        UnlockRejectReason::NotInitialized => "not-initialized",
                        UnlockRejectReason::EmptyPassphrase => "empty-passphrase",
                        UnlockRejectReason::Destroyed => "destroyed",
                    };
                    Err(format!("vault not unlocked ({r})"))
                }
            }
        }
        "notify-egress" => {
            if let Some(report) = &st.egress {
                if sync_egress_notice(report) == NoticeResult::Written {
                    eprintln!(
                        "Egress is not confirmed: see .notifications/egress-degraded.md in the vault."
                    );
                }
            }
            Ok(())
        }
        "prepare-desktops" => {
            // The desktops run as the workload user: the session's runtime directory is theirs
            // alone, and their homes (inside the vault) are theirs by the mount's forced owner.
            prepare_desktop_cache()?;
            let stacks = installed_stacks();
            if stacks.contains(&DesktopTarget::Selkies) {
                let _ = fs::create_dir_all(format!("{SELKIES_HOME}/.local/share/applications"));
            }
            if stacks.contains(&DesktopTarget::Vnc) {
                let _ = fs::create_dir_all(format!("{VNC_HOME}/.local/share/applications"));
            }
            enable_autostart(&supervisor_conf("desktop"));
            enable_autostart(&supervisor_conf("vnc"));
            Ok(())
        }
        "prepare-backup" => {
            if config.backup.restic_repository.is_none() {
                return Ok(());
            }
            match probe_repository().verdict {
                RepoProbe::Ready => {}
                RepoProbe::WrongPassword => eprintln!(
                    "WARNING: Backup repo exists but RESTIC_PASSWORD does not match. Check .env."
                ),
                RepoProbe::Unreachable => {
                    eprintln!("WARNING: Backup repo not reachable (will retry on next start).")
                }
                RepoProbe::Missing => {
                    if init_repository() {
                        println!("Backup repo initialized.");
                    } else {
                        eprintln!("WARNING: Backup repo init failed (will retry on next start).");
                    }
                }
            }
            enable_autostart(&supervisor_conf("backup-watcher"));
            Ok(())
        }
        "banner" => {
            for line in render_banner(&BannerInput {
                password: auth.master_password.clone(),
                password_generated: auth.password_source == Some(PasswordSource::Generated),
                host_port: config.host_port.clone(),
                editor: code_server_selected(fs::read_to_string(EDITOR_SUITE_PATH).ok().as_deref()),
                stacks: installed_stacks(),
                vault_mounted: st.vault_mounted,
                egress: st.egress.clone(),
                egress_enforced: config.features.egress_enforce,
                degraded: st.degraded.clone(),
            }) {
                println!("{line}");
            }
            println!("Starting supervisord...");
            Ok(())
        }
        "supervisord" => run_supervisord(),
        // A BOOT_PLAN id with no arm here is a programmer error, not a silent success: report it
        // (Degrade steps are recorded degraded; an Abort step would stop the boot visibly).
        other => Err(format!("unhandled boot step '{other}' (no effect ran)")),
    }
}

/// Spawn supervisord and hand SIGTERM/SIGINT on to it, then wait. Err on a non-zero exit (the
/// boot step is Abort, so run_boot reports the failure).
fn run_supervisord() -> Result<(), String> {
    // supervisord inherits this process's env (so Caddy sees DESKTOP_BCRYPT_HASH) and stdio.
    let mut child = Command::new(SUPERVISORD_BIN)
        .args(["-c", SUPERVISORD_CONF])
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id();

    if let Ok(mut signals) = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
    ]) {
        std::thread::spawn(move || {
            // `docker stop` / self-destruct send SIGTERM to PID 1 (tini) → this process. Pass it on
            // so supervisord stops every program in order; left alone, a restic backup mid-run
            // leaves its lock in the repository.
            for _ in signals.forever() {
                cmd::ok("kill", &["-TERM", &pid.to_string()]);
            }
        });
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "supervisord exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

pub fn run(_args: &[String]) -> i32 {
    // Inert mode: a completed self-destruct leaves this marker. Do not boot — stay alive so the
    // container does not crash-loop. (Previously the shell entrypoint did this before delegating.)
    if Path::new(DESTRUCT_COMPLETED_PATH).exists() {
        println!("Environment destroyed. Container is inert. Clean up: docker compose down -v");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    let config = load_config(system_env);
    let mut auth = Auth {
        master_password: String::new(),
        password_source: None,
    };
    let mut state = BootState::default();
    let report = run_boot(
        BOOT_PLAN,
        |id, st| run_step(id, st, &config, &mut auth),
        &mut state,
        |m| eprintln!("{m}"),
    );
    i32::from(report.aborted)
}

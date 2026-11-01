//! entrypoint — the container boot orchestrator. A port of packages/cli/src/entrypoint-main.ts.
//!
//! Binds each core BOOT_PLAN step to its infra effects and runs run_boot, then (the final step)
//! spawns supervisord and forwards SIGTERM/SIGINT to it so a `docker stop` or self-destruct stops
//! every program in order. The pure orchestration (order, abort/degrade, vault gating) is in core.

use crate::commands::system_env;
use sdw_core::backup::domain::RepoProbe;
use sdw_core::desktop::domain::DesktopTarget;
use sdw_core::desktop::layout::{DESKTOP_CACHE_DIR, SELKIES_HOME, VNC_HOME};
use sdw_core::egress::domain::{render_clash_config, DEFAULT_EGRESS_TIMING};
use sdw_core::egress::layout::{
    CLASH_CONFIG_PATH, CLASH_DIR, CLASH_HTTP_PORT, CLASH_PROVIDERS_DIR, CLASH_TEMPLATE_PATH,
    CLASH_USER,
};
use sdw_core::egress::usecases::bring_up_egress;
use sdw_core::plugins::all_vaults;
use sdw_core::vault::destruct::DESTRUCT_COMPLETED_PATH;
use sdw_core::vault::domain::{explain_unlock_failure, VAULT_GITATTRIBUTES, VAULT_GITIGNORE};
use sdw_core::vault::layout::VAULT_CIPHER_DIR;
use sdw_core::vault::ports::UnlockHook;
use sdw_core::vault::specs::workspace_vault;
use sdw_core::vault::usecases::{unlock_vault, UnlockDeps, UnlockRejectReason, UnlockResult};
use sdw_core::workspace::auth::{
    render_code_server_config, resolve_master_password, PasswordSource,
};
use sdw_core::workspace::banner::{render_banner, BannerInput};
use sdw_core::workspace::boot::{run_boot, BootState, BOOT_PLAN};
use sdw_core::workspace::config::{
    env_validation_error, find_missing_required_env, load_config, WorkspaceConfig,
    REQUIRED_ENV_VARS,
};
use sdw_core::workspace::layout::{
    supervisor_conf, CODE_SERVER_CONFIG_DIR, CODE_SERVER_CONFIG_PATH, CODE_SERVER_PORT,
    SUPERVISORD_BIN, SUPERVISORD_CONF,
};
use sdw_infra::caddy::caddy_password_hash;
use sdw_infra::cmd;
use sdw_infra::desktop::{ensure_kasm_passwd, installed_stacks};
use sdw_infra::egress::{
    apply_egress_firewall, clash_egress_ports, resolve_uid, sync_egress_notice,
    write_egress_status, NoticeResult,
};
use sdw_infra::gocryptfs::{cleanup_stale_mount, init_vault, is_initialized, GocryptfsVolume};
use sdw_infra::replication::{ensure_cipher_repo, EnsureResult};
use sdw_infra::restic::{init_repository, probe_repository};
use sdw_infra::supervisor::enable_autostart;
use sdw_infra::unlock_hooks::workspace_unlock_hooks;
use std::fs;
use std::io::Read;
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
    let mut buf = [0u8; 16];
    match fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)) {
        Ok(()) => buf.iter().map(|b| format!("{b:02x}")).collect(),
        Err(e) => {
            eprintln!("FATAL: cannot read /dev/urandom to generate a password: {e}");
            eprintln!(
                "Refusing to create a vault with a guessable master key; set PASSWORD in .env."
            );
            std::process::exit(1);
        }
    }
}

fn git_config_global(key: &str, value: &str) {
    if !cmd::ok("git", &["config", "--global", key, value]) {
        eprintln!("WARNING: git config {key} failed.");
    }
}

fn run_step(
    id: &str,
    st: &mut BootState,
    config: &WorkspaceConfig,
    auth: &mut Auth,
) -> Result<(), String> {
    match id {
        "preflight" => {
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
        "cleanup-stale-mounts" => {
            for vault in all_vaults() {
                cleanup_stale_mount(&vault.mount_point);
            }
            Ok(())
        }
        "render-clash-config" => {
            if config.clash_subscription_url.is_empty() || !Path::new(CLASH_TEMPLATE_PATH).exists()
            {
                return Ok(());
            }
            let template = fs::read_to_string(CLASH_TEMPLATE_PATH).map_err(|e| e.to_string())?;
            let _ = fs::create_dir_all(CLASH_PROVIDERS_DIR);
            fs::write(
                CLASH_CONFIG_PATH,
                render_clash_config(&template, &config.clash_subscription_url, &config.clash_dns),
            )
            .map_err(|e| e.to_string())?;
            println!("Clash config generated from template.");
            Ok(())
        }
        "git-globals" => {
            if let Some(name) = &config.git.user_name {
                git_config_global("user.name", name);
            }
            if let Some(email) = &config.git.user_email {
                git_config_global("user.email", email);
            }
            let proxy = format!("http://127.0.0.1:{CLASH_HTTP_PORT}");
            git_config_global("http.proxy", &proxy);
            git_config_global("https.proxy", &proxy);
            Ok(())
        }
        "tune-inotify" => {
            cmd::ok("sysctl", &["-w", "fs.inotify.max_user_watches=524288"]);
            Ok(())
        }
        "enforce-egress" => {
            if !config.features.egress_enforce {
                return Ok(()); // opt-in; default is the best-effort proxy env only
            }
            // Clash must run as its own uid for the firewall to single out its traffic.
            let uid = resolve_uid(CLASH_USER)
                .ok_or_else(|| format!("egress enforce: user '{CLASH_USER}' not found"))?;
            // Clash is non-root under enforcement, so it must own its config dir (providers/cache).
            cmd::ok(
                "chown",
                &["-R", &format!("{CLASH_USER}:{CLASH_USER}"), CLASH_DIR],
            );
            // Force the system resolver through Clash so no lookup leaks past it. Best-effort: the
            // firewall is the security boundary; this only keeps hostname DNS working under it.
            if fs::write("/etc/resolv.conf", "nameserver 127.0.0.1\n").is_err() {
                eprintln!(
                    "WARNING: could not point /etc/resolv.conf at Clash; hostname DNS may be \
                     blocked by the egress filter until fixed."
                );
            }
            // Load the ruleset. On failure (no NET_ADMIN / no nft) this Errs and, as an Abort step,
            // fails the boot closed rather than booting while believing egress is enforced.
            apply_egress_firewall(uid)?;
            println!(
                "Egress firewall enforced (fail-closed; direct egress only for Clash uid {uid})."
            );
            Ok(())
        }
        "bring-up-egress" => {
            let clash_uid = if config.features.egress_enforce {
                resolve_uid(CLASH_USER)
            } else {
                None
            };
            let report = bring_up_egress(
                &clash_egress_ports(announce, clash_uid),
                DEFAULT_EGRESS_TIMING,
            );
            if let Err(e) = write_egress_status(&report) {
                eprintln!("WARNING: could not record the egress status: {e}");
            }
            st.egress = Some(report);
            Ok(())
        }
        "resolve-auth" => {
            let existing = fs::read_to_string(CODE_SERVER_CONFIG_PATH).ok();
            let password = resolve_master_password(
                config.password.as_deref(),
                existing.as_deref(),
                generate_password,
            )?;
            if password.write_config {
                let _ = fs::create_dir_all(CODE_SERVER_CONFIG_DIR);
                fs::write(
                    CODE_SERVER_CONFIG_PATH,
                    render_code_server_config(&password.value, CODE_SERVER_PORT),
                )
                .map_err(|e| e.to_string())?;
            }
            if password.source == PasswordSource::Generated {
                println!("Generated password: {}", password.value);
                println!("  (set PASSWORD in .env to use a fixed password)");
            }
            auth.master_password = password.value;
            auth.password_source = Some(password.source);

            // bcrypt hash for Caddy's basic_auth (supervisord/Caddy inherit this env).
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
            let _ = fs::create_dir_all(DESKTOP_CACHE_DIR);
            let stacks = installed_stacks();
            if stacks.contains(&DesktopTarget::Selkies) {
                let _ = fs::create_dir_all(format!("{SELKIES_HOME}/.local/share/applications"));
            }
            if stacks.contains(&DesktopTarget::Vnc) {
                let _ = fs::create_dir_all(format!("{VNC_HOME}/.local/share/applications"));
                ensure_kasm_passwd(&auth.master_password);
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
                stacks: installed_stacks(),
                vault_mounted: st.vault_mounted,
                egress: st.egress.clone(),
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

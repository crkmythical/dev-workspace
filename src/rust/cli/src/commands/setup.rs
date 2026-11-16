//! setup — interactive or automated workspace configuration. A port of
//! packages/cli/src/setup.ts.
//!
//!   setup                  interactive (prompts for each setting)
//!   setup --auto           reads env vars, no prompts
//!   setup --clash-url URL  set specific values non-interactively

use crate::commands::system_env;
use crate::prompt::prompt_line;
use sdw_core::egress::domain::{describe_egress, EgressStatus, DEFAULT_EGRESS_TIMING};
use sdw_core::egress::usecases::bring_up_egress;
use sdw_core::vault::domain::{VAULT_GITATTRIBUTES, VAULT_GITIGNORE};
use sdw_core::vault::layout::VAULT_CIPHER_DIR;
use sdw_core::workspace::config::load_config;
use sdw_infra::clash::{write_config, ConfigOptions, SecretPolicy};
use sdw_infra::cmd;
use sdw_infra::egress::{clash_egress_ports, prepare_clash_dir};
use sdw_infra::gocryptfs::init_vault;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// The value after `flag`, or None when the flag is absent or its value is empty.
fn get_arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
}

/// Prompt with a default (interactive); in auto mode just return the default.
fn ask(is_auto: bool, question: &str, default: &str) -> String {
    if is_auto {
        return default.to_string();
    }
    let p = if default.is_empty() {
        format!("{question}: ")
    } else {
        format!("{question} [{default}]: ")
    };
    let answer = prompt_line(&p).unwrap_or_default();
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn vault_initialized() -> bool {
    Path::new(&format!("{VAULT_CIPHER_DIR}/gocryptfs.conf")).exists()
}

pub fn run(args: &[String]) -> i32 {
    let config = load_config(system_env);
    let is_auto = args.iter().any(|a| a == "--auto");

    println!("\n=== Secure Dev Workspace Setup ===\n");

    // Step 1: Clash subscription
    println!("[1/4] Clash 代理订阅");
    let mut clash_url = get_arg(args, "--clash-url")
        .map(str::to_string)
        .unwrap_or_else(|| config.clash_subscription_url.clone());
    if clash_url.is_empty() && !is_auto {
        clash_url = ask(is_auto, "  Clash 订阅链接 (回车跳过)", "");
    }
    if clash_url.is_empty() {
        println!("  跳过 — 设置 CLASH_SUBSCRIPTION_URL，或使用 --clash-url");
    } else {
        let (dns, _source) = crate::commands::clash_dns::resolve_dns();
        match write_config(&ConfigOptions {
            subscription_url: &clash_url,
            dns_nameservers: &dns,
            allow_lan: false,
            // Clash is restarted just below, but keeping the secret costs nothing and leaves the
            // controller and the file agreeing if that restart fails.
            secret: SecretPolicy::Keep,
        }) {
            Err(e) => println!("  ✗ {e}"),
            Ok(()) => {
                println!("  ✓ 订阅配置已更新");
                cmd::ok("pkill", &["-x", "clash"]);
                std::thread::sleep(Duration::from_millis(1000));
                // Clash runs as its own user (as at boot): restarting it as root here would leave
                // root-owned files in its directory that the unprivileged watcher could not replace.
                let clash_uid = prepare_clash_dir();
                let report = bring_up_egress(
                    &clash_egress_ports(|_: &str| {}, clash_uid),
                    DEFAULT_EGRESS_TIMING,
                );
                if report.status == EgressStatus::Confirmed {
                    println!("  ✓ 代理连接正常");
                } else {
                    println!("  ⚠ 代理已启动，出口未确认: {}", describe_egress(&report));
                }
            }
        }
    }

    // Step 2: Vault
    println!("\n[2/4] Vault 加密存储");
    if vault_initialized() {
        println!("  ✓ Vault 已初始化");
    } else {
        let init = if is_auto {
            "y".to_string()
        } else {
            ask(is_auto, "  初始化加密 vault？(y/n)", "y")
        };
        if init.to_lowercase() == "y" {
            println!("  正在初始化...");
            // The vault passphrase is the master password: PASSWORD in auto mode; gocryptfs
            // prompts otherwise.
            match (is_auto, config.password.as_deref()) {
                (true, Some(pw)) => {
                    let ok = init_vault(VAULT_CIPHER_DIR, Some(pw));
                    println!(
                        "{}",
                        if ok {
                            "  ✓ Vault 初始化完成"
                        } else {
                            "  ✗ 初始化失败"
                        }
                    );
                }
                _ => {
                    if init_vault(VAULT_CIPHER_DIR, None) {
                        println!("  ✓ Vault 初始化完成");
                    }
                }
            }
            let email = config.git.user_email.as_deref().unwrap_or("vault@local");
            let name = config.git.user_name.as_deref().unwrap_or("Vault");
            cmd::ok("git", &["config", "--global", "user.email", email]);
            cmd::ok("git", &["config", "--global", "user.name", name]);
            cmd::ok("git", &["-C", VAULT_CIPHER_DIR, "init", "-b", "main"]);
            let _ = fs::write(format!("{VAULT_CIPHER_DIR}/.gitignore"), VAULT_GITIGNORE);
            let _ = fs::write(
                format!("{VAULT_CIPHER_DIR}/.gitattributes"),
                VAULT_GITATTRIBUTES,
            );
            cmd::ok("git", &["-C", VAULT_CIPHER_DIR, "add", "-A"]);
            cmd::ok(
                "git",
                &["-C", VAULT_CIPHER_DIR, "commit", "-q", "-m", "init vault"],
            );
        }
    }

    // Step 3: GitHub backup
    println!("\n[3/4] GitHub 备份 (可选)");
    let mut vault_repo = get_arg(args, "--vault-repo")
        .map(str::to_string)
        .or_else(|| config.git.vault_repo.clone())
        .unwrap_or_default();
    if vault_repo.is_empty() && !is_auto {
        println!("  备份 vault 密文到 GitHub，丢失数据可恢复。");
        println!("  格式: https://ghp_TOKEN@github.com/user/repo.git");
        vault_repo = ask(is_auto, "  GitHub repo URL (回车跳过)", "");
    }
    if vault_repo.is_empty() {
        println!("  跳过 — 后续可在 .env 中配置 VAULT_GIT_REPO");
    } else {
        if !cmd::ok(
            "git",
            &["-C", VAULT_CIPHER_DIR, "remote", "get-url", "origin"],
        ) {
            cmd::ok(
                "git",
                &[
                    "-C",
                    VAULT_CIPHER_DIR,
                    "remote",
                    "add",
                    "origin",
                    &vault_repo,
                ],
            );
        }
        if cmd::ok(
            "git",
            &["-C", VAULT_CIPHER_DIR, "push", "-u", "origin", "main"],
        ) {
            println!("  ✓ 已推送到 GitHub");
        } else {
            println!("  ⚠ 推送失败（检查 token 权限或仓库是否存在）");
        }
    }

    // Step 4: Git identity
    println!("\n[4/4] Git 身份");
    let mut git_name = get_arg(args, "--git-name")
        .map(str::to_string)
        .or_else(|| config.git.user_name.clone())
        .unwrap_or_default();
    let mut git_email = get_arg(args, "--git-email")
        .map(str::to_string)
        .or_else(|| config.git.user_email.clone())
        .unwrap_or_default();
    if git_name.is_empty() && !is_auto {
        git_name = ask(is_auto, "  你的名字", "Developer");
    }
    if git_email.is_empty() && !is_auto {
        git_email = ask(is_auto, "  你的邮箱", "dev@local");
    }
    if !git_name.is_empty() {
        cmd::ok("git", &["config", "--global", "user.name", &git_name]);
    }
    if !git_email.is_empty() {
        cmd::ok("git", &["config", "--global", "user.email", &git_email]);
    }
    let shown_name = if git_name.is_empty() {
        "Developer"
    } else {
        &git_name
    };
    let shown_email = if git_email.is_empty() {
        "dev@local"
    } else {
        &git_email
    };
    println!("  ✓ {shown_name} <{shown_email}>");

    // Done
    println!("\n=== 配置完成 ===");
    println!("  Web 入口:    http://localhost:{}", config.host_port);
    println!("  远程访问:    经 Cloudflare Tunnel 暴露的域名（宿主机 scripts/setup-cloudflared.sh 配置）");
    println!("  健康检查:    docker exec dev-workspace doctor");
    if vault_initialized() {
        println!("  解锁 vault:  docker exec -it dev-workspace unlock-vault");
    }
    println!();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_arg_reads_the_value_after_a_flag() {
        let args: Vec<String> = ["--clash-url", "https://x/sub", "--auto"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(get_arg(&args, "--clash-url"), Some("https://x/sub"));
        assert_eq!(get_arg(&args, "--vault-repo"), None);
    }

    #[test]
    fn a_flag_with_no_value_is_none() {
        let args = vec!["--clash-url".to_string()];
        assert_eq!(get_arg(&args, "--clash-url"), None);
    }
}

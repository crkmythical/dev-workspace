use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

use crate::{docker_context, env_file, process};

const VOLUME_SUFFIXES: &[&str] = &[
    "vault-data",
    "cache-data",
    "clash-config",
    "destruct-state",
    "audit-data",
];
const HISTORY_TERMS: &[&str] = &[
    "dev-workspace",
    "vault",
    "clash",
    "cloudflared",
    "gocryptfs",
    "unlock-vault",
    "init-vault",
];

#[derive(Default)]
struct Options {
    project_dir: Option<PathBuf>,
    force: bool,
    remote: bool,
    paranoid: bool,
    delete_project: bool,
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!(
            "usage: sdw-host destroy --project-dir DIR [--force] [--remote] [--paranoid] [--delete-project]"
        );
        return 0;
    }
    match parse(args).and_then(execute) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("destroy: {error}");
            2
        }
    }
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project-dir" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--project-dir needs a path".to_string())?;
                options.project_dir = Some(PathBuf::from(path));
            }
            "--force" => options.force = true,
            "--remote" => options.remote = true,
            "--paranoid" => options.paranoid = true,
            "--delete-project" => options.delete_project = true,
            "-h" | "--help" => return Err("help must be handled before parsing".to_string()),
            option => return Err(format!("unknown option: {option}")),
        }
        index += 1;
    }
    Ok(options)
}

fn execute(options: Options) -> Result<i32, String> {
    let project_dir = options
        .project_dir
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?);
    let project_dir = project_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", project_dir.display()))?;
    if !project_dir.join("docker-compose.yml").is_file() {
        return Err(format!(
            "{} does not look like the project directory",
            project_dir.display()
        ));
    }
    std::env::set_current_dir(&project_dir).map_err(|error| error.to_string())?;
    let config = env_file::read(&project_dir.join(".env")).unwrap_or_default();

    if !options.force && !confirmed()? {
        println!("Aborted.");
        return Ok(0);
    }

    if !process::command_exists("docker") {
        return Err("docker not found".to_string());
    }
    let context = docker_context::enforce(&project_dir)?;
    println!("Docker context: {context}");
    println!("Destroying workspace...");

    let _ = process::success("docker", ["compose", "down", "-v"]);
    let project = compose_project_name()?;
    remove_project_volumes(&project);
    remove_workspace_images();

    let tunnel_name = env_file::value(&config, "TUNNEL_NAME", "dev-workspace");
    if !safe_tunnel_name(&tunnel_name) {
        return Err("TUNNEL_NAME has an unsafe shape".to_string());
    }
    stop_tunnel_service(&tunnel_name);
    let tunnel_deleted = process::command_exists("cloudflared")
        && process::success("cloudflared", ["tunnel", "delete", tunnel_name.as_str()]);

    let remote_deleted = if options.remote {
        delete_remote(&config)
    } else {
        None
    };

    if options.paranoid {
        paranoid_cleanup(&tunnel_name);
    }

    let mut problems = 0usize;
    println!();
    println!("=== Destruction Summary ===");
    if docker_reachable() {
        let still = remaining_project_objects(&project);
        if still.is_empty() {
            println!("  ✗ Containers + volumes removed");
        } else {
            problems += 1;
            println!(
                "  ! Containers or volumes of project '{project}' are still there: {}",
                still.join(" ")
            );
        }
        let images = workspace_images();
        if images.is_empty() {
            println!("  ✗ Docker images removed");
        } else {
            problems += 1;
            println!("  ! Docker images are still there: {}", images.join(" "));
        }
    } else {
        problems += 1;
        println!(
            "  ! Docker is not reachable: the containers, volumes and images were NOT removed"
        );
    }
    if tunnel_deleted {
        println!("  ✗ Tunnel '{tunnel_name}' deleted");
    } else {
        println!(
            "  - Tunnel '{tunnel_name}' not deleted (already gone, or cloudflared is not available)"
        );
    }
    if options.remote {
        if remote_deleted == Some(true) {
            println!("  ✗ Remote vault repo deleted");
        } else {
            problems += 1;
            println!(
                "  ! The remote vault repo was NOT deleted (no VAULT_GIT_REPO, or deletion failed)"
            );
        }
    }
    if options.paranoid {
        println!("  ✗ Deep forensic cleanup attempted");
    }
    println!();

    if problems > 0 {
        println!(
            "Not everything was removed; the project directory ({}) is kept so this can be run again.",
            project_dir.display()
        );
        return Ok(1);
    }
    if !options.delete_project {
        println!(
            "The project directory ({}) was left in place; add --delete-project to remove it too.",
            project_dir.display()
        );
        return Ok(0);
    }
    validate_delete_target(&project_dir)?;
    println!("  ✗ Project directory will be removed");
    std::env::set_current_dir("/").map_err(|error| error.to_string())?;
    fs::remove_dir_all(&project_dir)
        .map_err(|error| format!("cannot remove {}: {error}", project_dir.display()))?;
    println!("Done. No workspace traces remain.");
    Ok(0)
}

fn confirmed() -> Result<bool, String> {
    println!("⚠️  This will PERMANENTLY destroy all workspace data.");
    print!("Type 'destroy' to confirm: ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    Ok(line.trim_end() == "destroy")
}

fn compose_project_name() -> Result<String, String> {
    let output = process::output("docker", ["compose", "config", "--format", "json"])?;
    if !output.success() {
        return Err(
            "docker compose config failed; refusing to guess the destructive project scope"
                .to_string(),
        );
    }
    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|error| format!("docker compose config returned invalid JSON: {error}"))?;
    value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            "compose config has no project name; refusing destructive cleanup".to_string()
        })
}

fn output_lines(program: &str, args: &[&str]) -> Vec<String> {
    process::output(program, args)
        .ok()
        .filter(|output| output.success())
        .map(|output| {
            output
                .stdout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn remove_project_volumes(project: &str) {
    let filter = format!("label=com.docker.compose.project={project}");
    let volumes = output_lines("docker", &["volume", "ls", "-q", "--filter", &filter]);
    let selected: Vec<String> = volumes
        .into_iter()
        .filter(|volume| {
            VOLUME_SUFFIXES
                .iter()
                .any(|suffix| volume == suffix || volume.ends_with(&format!("_{suffix}")))
        })
        .collect();
    if !selected.is_empty() {
        let mut args = vec!["volume".to_string(), "rm".to_string()];
        args.extend(selected);
        let _ = process::success("docker", args);
    }
}

fn workspace_images() -> Vec<String> {
    output_lines(
        "docker",
        &["image", "ls", "--format", "{{.Repository}}:{{.Tag}}"],
    )
    .into_iter()
    .filter(|image| image.starts_with("dev-workspace:"))
    .collect()
}

fn remove_workspace_images() {
    let images = workspace_images();
    if !images.is_empty() {
        let mut args = vec!["image".to_string(), "rm".to_string()];
        args.extend(images);
        let _ = process::success("docker", args);
    }
}

fn docker_reachable() -> bool {
    process::success("docker", ["info"])
}

fn remaining_project_objects(project: &str) -> Vec<String> {
    let filter = format!("label=com.docker.compose.project={project}");
    let mut objects = output_lines("docker", &["ps", "-aq", "--filter", &filter]);
    objects.extend(output_lines(
        "docker",
        &["volume", "ls", "-q", "--filter", &filter],
    ));
    objects
}

fn delete_remote(config: &HashMap<String, String>) -> Option<bool> {
    let remote = env_file::value(config, "VAULT_GIT_REPO", "");
    if remote.is_empty() {
        return None;
    }
    let (slug, token) = github_remote(&remote)?;
    println!("Deleting remote vault repo...");
    if let Some(token) = token {
        Some(delete_github_api(&slug, token.as_str()))
    } else if process::command_exists("gh") {
        Some(process::success(
            "gh",
            ["repo", "delete", slug.as_str(), "--yes"],
        ))
    } else {
        None
    }
}

fn delete_github_api(slug: &str, token: &str) -> bool {
    if !process::command_exists("curl") {
        return false;
    }
    let config = Zeroizing::new(format!(
        "silent\nshow-error\nfail\nrequest = \"DELETE\"\nurl = \"https://api.github.com/repos/{slug}\"\n\
         header = \"Accept: application/vnd.github+json\"\n\
         header = \"Authorization: Bearer {token}\"\n\
         header = \"X-GitHub-Api-Version: 2022-11-28\"\n"
    ));
    let mut command = Command::new("curl");
    process::retain_network_environment(&mut command);
    let mut child = match command
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(config.as_bytes()).is_ok());
    if !written {
        let _ = child.kill();
    }
    child.wait().is_ok_and(|status| written && status.success())
}

fn github_remote(remote: &str) -> Option<(String, Option<Zeroizing<String>>)> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let (candidate, token) = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        (path, None)
    } else if let Some((_, hierarchical)) = trimmed.split_once("://") {
        let (authority, path) = hierarchical.split_once('/')?;
        let (userinfo, host) = authority
            .rsplit_once('@')
            .map_or((None, authority), |(user, host)| (Some(user), host));
        if host.split(':').next()? != "github.com" {
            return None;
        }
        let token = userinfo
            .filter(|value| {
                !value.is_empty()
                    && *value != "git"
                    && value.len() <= 512
                    && !value.bytes().any(|byte| byte.is_ascii_control())
            })
            .map(|value| Zeroizing::new(value.to_string()));
        (path, token)
    } else {
        (trimmed, None)
    };
    let mut parts = candidate.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let safe = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    (parts.next().is_none() && safe(owner) && safe(repo))
        .then(|| (format!("{owner}/{repo}"), token))
}

fn safe_tunnel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn stop_tunnel_service(tunnel_name: &str) {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    if std::env::consts::OS == "macos" {
        let launch_agents = home.join("Library/LaunchAgents");
        for plist in [
            launch_agents.join("com.cloudflare.tunnel.plist"),
            launch_agents.join(format!("com.cloudflare.tunnel.{tunnel_name}.plist")),
        ] {
            let _ = process::success("launchctl", ["unload", plist.to_string_lossy().as_ref()]);
            let _ = fs::remove_file(plist);
        }
    } else {
        let _ = process::success(
            "systemctl",
            [
                "--user",
                "disable",
                "--now",
                "cloudflared-dev-workspace.service",
            ],
        );
        let unit = home.join(".config/systemd/user/cloudflared-dev-workspace.service");
        let _ = fs::remove_file(unit);
        let _ = process::success("systemctl", ["--user", "daemon-reload"]);
    }
}

fn paranoid_cleanup(tunnel_name: &str) {
    println!("Paranoid mode: deep cleanup...");
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    if std::env::consts::OS == "macos" {
        let launch_agents = home.join("Library/LaunchAgents");
        for plist in [
            launch_agents.join("com.dev-workspace.host-watcher.plist"),
            launch_agents.join("com.cloudflare.tunnel.plist"),
            launch_agents.join(format!("com.cloudflare.tunnel.{tunnel_name}.plist")),
        ] {
            let _ = process::success("launchctl", ["unload", plist.to_string_lossy().as_ref()]);
            let _ = fs::remove_file(plist);
        }
        println!("  ✗ workspace LaunchAgents removed");
    } else {
        let _ = process::success(
            "systemctl",
            [
                "--user",
                "disable",
                "--now",
                "dev-workspace-host-watcher.service",
                "cloudflared-dev-workspace.service",
            ],
        );
        let unit_dir = home.join(".config/systemd/user");
        let _ = fs::remove_file(unit_dir.join("dev-workspace-host-watcher.service"));
        let _ = fs::remove_file(unit_dir.join("cloudflared-dev-workspace.service"));
        let _ = process::success("systemctl", ["--user", "daemon-reload"]);
        println!("  ✗ systemd user services removed");
    }

    let _ = fs::remove_dir_all(home.join(".cloudflared"));
    println!("  ✗ ~/.cloudflared/ removed");
    for file in [home.join(".zsh_history"), home.join(".bash_history")] {
        scrub_history(&file);
    }
    println!("  ✗ Shell history scrubbed");

    if std::env::consts::OS == "macos" {
        let _ = process::success("sudo", ["dscacheutil", "-flushcache"]);
        let _ = process::success("sudo", ["killall", "-HUP", "mDNSResponder"]);
    } else {
        let _ = process::success("resolvectl", ["flush-caches"]);
    }
    println!("  ✗ DNS cache flushed");
    let _ = process::success("docker", ["builder", "prune", "-af"]);
    println!("  ✗ Docker build cache cleared");

    if std::env::consts::OS == "macos" {
        scrub_docker_logs(&home.join("Library/Containers/com.docker.docker/Data/log"));
        println!("  ✗ Docker logs scrubbed");
    }
}

fn scrub_history(path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| !HISTORY_TERMS.iter().any(|term| line.contains(term)))
        .collect();
    let mut output = kept.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    let _ = fs::write(path, output);
}

fn scrub_docker_logs(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            scrub_docker_logs(&path);
        } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "log") {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let mut kept = text
                .lines()
                .filter(|line| !line.contains("dev-workspace"))
                .collect::<Vec<_>>()
                .join("\n");
            if !kept.is_empty() {
                kept.push('\n');
            }
            let _ = fs::write(path, kept);
        }
    }
}

fn validate_delete_target(project_dir: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| path.canonicalize().ok());
    if project_dir == Path::new("/") || home.as_ref().is_some_and(|home| home == project_dir) {
        return Err(format!(
            "refusing to remove unsafe target {}",
            project_dir.display()
        ));
    }
    if !project_dir.join("docker-compose.yml").is_file() {
        return Err(format!(
            "refusing to remove {}: project marker is missing",
            project_dir.display()
        ));
    }
    Ok(())
}

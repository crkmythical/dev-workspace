use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{docker_context, env_file, process};

const API: &str = "https://api.cloudflare.com/client/v4";

#[derive(Zeroize, ZeroizeOnDrop)]
struct Secrets {
    api_token: String,
    account_id: String,
    zone_id: String,
    zone_name: String,
    subdomain: String,
    allow_email: String,
    team_domain: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct InstanceState {
    version: u32,
    id: String,
    fqdn: String,
    origin_port: u16,
    team_domain: Option<String>,
    tunnel_id: Option<String>,
    tunnel_token: Option<String>,
    dns_record_id: Option<String>,
    service_token_id: Option<String>,
    service_client_id: Option<String>,
    service_client_secret: Option<String>,
    app_id: Option<String>,
    app_aud: Option<String>,
    policy_service_id: Option<String>,
    policy_human_id: Option<String>,
    connector_pid: Option<u32>,
}

struct ApiClient {
    auth_file: PathBuf,
    temp_dir: PathBuf,
}

impl Drop for ApiClient {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.auth_file);
        let _ = fs::remove_dir(&self.temp_dir);
    }
}

impl ApiClient {
    fn new(token: &mut String) -> Result<Self, String> {
        if token.is_empty()
            || token.len() > 512
            || token
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b'"' || byte == b'\\')
        {
            return Err("CF_API_TOKEN has an invalid shape".to_string());
        }
        let base = std::env::var_os("ORCH_TMPDIR")
            .or_else(|| std::env::var_os("TMPDIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let temp_dir = base.join(format!("sdw-host-{}", std::process::id()));
        fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
        fs::set_permissions(&temp_dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        let auth_file = temp_dir.join("cloudflare-auth.conf");
        let auth =
            zeroize::Zeroizing::new(format!("header = \"Authorization: Bearer {}\"\n", token));
        env_file::write_private(&auth_file, auth.as_bytes())?;
        token.zeroize();
        Ok(Self {
            auth_file,
            temp_dir,
        })
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let mut command = Command::new("curl");
        process::retain_network_environment(&mut command);
        command.args([
            "-sS",
            "--retry",
            "4",
            "--retry-delay",
            "2",
            "--max-time",
            "30",
            "-X",
            method,
            "-K",
            self.auth_file.to_string_lossy().as_ref(),
            "-H",
            "Content-Type: application/json",
        ]);
        if body.is_some() {
            command.args(["--data-binary", "@-"]);
        }
        command
            .arg(format!("{API}{path}"))
            .stdin(if body.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot run curl: {error}"))?;
        if let (Some(body), Some(mut stdin)) = (body, child.stdin.take()) {
            if let Err(error) = stdin.write_all(body.to_string().as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot send Cloudflare request body: {error}"));
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "Cloudflare {method} {path}: curl failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let response: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("Cloudflare {method} {path}: invalid JSON ({error})"))?;
        Ok(response)
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let response = self.request(method, path, body)?;
        if response["success"].as_bool() == Some(true) {
            return Ok(response["result"].clone());
        }
        let messages = response["errors"]
            .as_array()
            .map(|errors| {
                errors
                    .iter()
                    .filter_map(|error| error["message"].as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_else(|| "unknown API error".to_string());
        Err(format!("Cloudflare {method} {path}: {messages}"))
    }

    fn delete(&self, label: &str, path: &str) -> bool {
        match self.call("DELETE", path, None) {
            Ok(_) => {
                println!("  deleted {label}");
                true
            }
            Err(error) if missing_resource(&error) => {
                println!("  {label} already gone");
                true
            }
            Err(error) => {
                eprintln!("  WARN could not delete {label}: {error}");
                false
            }
        }
    }
}

fn show_help(args: &[String], usage: &str) -> bool {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: sdw-host {usage}");
        true
    } else {
        false
    }
}

pub fn setup_tunnel(args: &[String]) -> i32 {
    if show_help(
        args,
        "setup-tunnel --project-dir DIR [--name NAME] --hostname HOST [--port PORT]",
    ) {
        return 0;
    }
    match parse_tunnel_setup(args).and_then(|(project, name, hostname, port)| {
        setup_named_tunnel(&project, &name, &hostname, port)
    }) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("setup-tunnel: {error}");
            1
        }
    }
}

pub fn provision(args: &[String]) -> i32 {
    if show_help(args, "provision --project-dir DIR ID [PORT]") {
        return 0;
    }
    match parse_id_port(args).and_then(|(project, id, port)| provision_for(&project, &id, port)) {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("provision: {error}");
            1
        }
    }
}

pub fn teardown(args: &[String]) -> i32 {
    if show_help(args, "teardown --project-dir DIR ID") {
        return 0;
    }
    match parse_project_id(args).and_then(|(project, id)| teardown_for(&project, &id)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("teardown: {error}");
            1
        }
    }
}

pub fn up(args: &[String]) -> i32 {
    if show_help(args, "up --project-dir DIR ID [PORT]") {
        return 0;
    }
    match parse_project_id_optional_port(args).and_then(|(project, id, requested)| {
        docker_context::enforce(&project)?;
        let port = requested.map_or_else(choose_port, Ok)?;
        let state = provision_for(&project, &id, port)?;
        if start_workspace(&project, &state) {
            println!();
            println!(
                "== {} up: https://{}/   (origin 127.0.0.1:{}, container ws-{})",
                state.id, state.fqdn, state.origin_port, state.id
            );
            println!(
                "   gate single-sign-on behind Access: {}",
                if state.app_aud.is_some() && state.team_domain.is_some() {
                    "ON"
                } else {
                    "off (set CF_TEAM_DOMAIN in secrets.env to enable)"
                }
            );
            println!("   down: deploy/orchestration/down.sh {}", state.id);
            Ok(())
        } else {
            eprintln!("up: container start failed; rolling back edge and local volumes");
            let _ = stop_workspace(&project, &id);
            let _ = teardown_for(&project, &id);
            Err("container did not start".to_string())
        }
    }) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("up: {error}");
            1
        }
    }
}

pub fn down(args: &[String]) -> i32 {
    if show_help(args, "down --project-dir DIR ID") {
        return 0;
    }
    match parse_project_id(args).and_then(|(project, id)| {
        docker_context::enforce(&project)?;
        let local = stop_workspace(&project, &id);
        let edge = teardown_for(&project, &id);
        match (local, edge) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(local), Ok(())) => Err(local),
            (Ok(()), Err(edge)) => Err(edge),
            (Err(local), Err(edge)) => Err(format!("{local}; {edge}")),
        }
    }) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("down: {error}");
            1
        }
    }
}

pub fn run_tunnel(args: &[String]) -> i32 {
    if show_help(args, "run-tunnel --project-dir DIR ID") {
        return 0;
    }
    match parse_project_id(args).and_then(|(project, id)| {
        let state = load_state(&project, &id)?;
        let token = state
            .tunnel_token
            .as_deref()
            .ok_or_else(|| "state has no tunnel token".to_string())?;
        let mut command = Command::new("cloudflared");
        command
            .args(["tunnel", "--no-autoupdate", "run"])
            .env_clear()
            .env(
                "PATH",
                std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
            )
            .env("TUNNEL_TOKEN", token);
        let error = command.exec();
        Err(format!("cannot exec cloudflared: {error}"))
    }) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("run-tunnel: {error}");
            1
        }
    }
}

fn setup_named_tunnel(
    _project: &Path,
    name: &str,
    hostname: &str,
    port: u16,
) -> Result<(), String> {
    validate_tunnel_name(name)?;
    validate_hostname(hostname)?;
    let cloudflared = process::find_command("cloudflared").ok_or_else(|| {
        if std::env::consts::OS == "macos" {
            "cloudflared not found; install it with `brew install cloudflared`".to_string()
        } else {
            "cloudflared not found; install it from the official Cloudflare package repository"
                .to_string()
        }
    })?;
    println!("=== Cloudflare Tunnel Setup ===");
    println!("  Tunnel:   {name}");
    println!("  Hostname: {hostname}");
    println!("  Target:   http://localhost:{port}");

    let mut tunnels = list_named_tunnels()?;
    if !tunnels.iter().any(|(_, existing)| existing == name) {
        println!("Opening browser for Cloudflare login if required...");
        let list_ok = process::success("cloudflared", ["tunnel", "list"]);
        if !list_ok
            && !Command::new("cloudflared")
                .args(["tunnel", "login"])
                .status()
                .is_ok_and(|status| status.success())
        {
            return Err("Cloudflare login failed".to_string());
        }
        if !Command::new("cloudflared")
            .args(["tunnel", "create", name])
            .status()
            .is_ok_and(|status| status.success())
        {
            return Err(format!("could not create tunnel '{name}'"));
        }
        tunnels = list_named_tunnels()?;
    }
    let tunnel_id = tunnels
        .iter()
        .find(|(_, existing)| existing == name)
        .map(|(id, _)| id.clone())
        .ok_or_else(|| format!("tunnel '{name}' is not present after create"))?;

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())?;
    let cloudflared_dir = home.join(".cloudflared");
    fs::create_dir_all(&cloudflared_dir).map_err(|error| error.to_string())?;
    fs::set_permissions(&cloudflared_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let credentials = cloudflared_dir.join(format!("{tunnel_id}.json"));
    if !credentials.is_file() {
        return Err(format!(
            "tunnel credential file is missing: {}",
            credentials.display()
        ));
    }
    env_file::ensure_private(&credentials)?;
    let config = cloudflared_dir.join(format!("sdw-{name}.yml"));
    let yaml = format!(
        "tunnel: {}\ncredentials-file: {}\n\ningress:\n  - hostname: {}\n    service: {}\n  - service: http_status:404\n",
        serde_json::to_string(&tunnel_id).map_err(|error| error.to_string())?,
        serde_json::to_string(&credentials.to_string_lossy()).map_err(|error| error.to_string())?,
        serde_json::to_string(hostname).map_err(|error| error.to_string())?,
        serde_json::to_string(&format!("http://localhost:{port}"))
            .map_err(|error| error.to_string())?,
    );
    env_file::write_private(&config, yaml.as_bytes())?;
    println!("✓ Config written: {}", config.display());

    if process::success("cloudflared", ["tunnel", "route", "dns", name, hostname]) {
        println!("✓ DNS: {hostname} → tunnel");
    } else {
        println!("- DNS route was not created (it may already exist)");
    }

    if std::env::consts::OS == "macos" {
        install_tunnel_launch_agent(&home, &cloudflared, &config, name)?;
    } else {
        println!(
            "Auto-start: deploy/linux-systemd/install.sh --cloudflared (uses {})",
            config.display()
        );
    }
    println!("=== Done: https://{hostname}");
    Ok(())
}

fn list_named_tunnels() -> Result<Vec<(String, String)>, String> {
    let output = process::output("cloudflared", ["tunnel", "list", "--output", "json"])?;
    if !output.success() {
        return Ok(Vec::new());
    }
    let values: Value = serde_json::from_str(&output.stdout)
        .map_err(|error| format!("cloudflared tunnel list returned invalid JSON: {error}"))?;
    Ok(values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            Some((
                value["id"].as_str()?.to_string(),
                value["name"].as_str()?.to_string(),
            ))
        })
        .collect())
}

fn install_tunnel_launch_agent(
    home: &Path,
    cloudflared: &Path,
    config: &Path,
    name: &str,
) -> Result<(), String> {
    let directory = home.join("Library/LaunchAgents");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let plist = directory.join(format!("com.cloudflare.tunnel.{name}.plist"));
    let legacy = directory.join("com.cloudflare.tunnel.plist");
    if fs::read_to_string(&legacy)
        .ok()
        .is_some_and(|contents| contents.contains(&format!("<string>{name}</string>")))
    {
        let _ = process::success("launchctl", ["unload", legacy.to_string_lossy().as_ref()]);
        let _ = fs::remove_file(&legacy);
    }
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.cloudflare.tunnel.{}</string>
  <key>ProgramArguments</key><array>
    <string>{}</string><string>--config</string><string>{}</string>
    <string>tunnel</string><string>run</string><string>{}</string>
  </array>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/cloudflared-{}.log</string>
  <key>StandardErrorPath</key><string>/tmp/cloudflared-{}.err</string>
</dict></plist>
"#,
        xml_escape(name),
        xml_escape(&cloudflared.to_string_lossy()),
        xml_escape(&config.to_string_lossy()),
        xml_escape(name),
        xml_escape(name),
        xml_escape(name),
    );
    fs::write(&plist, xml).map_err(|error| error.to_string())?;
    fs::set_permissions(&plist, fs::Permissions::from_mode(0o644))
        .map_err(|error| error.to_string())?;
    let _ = process::success("launchctl", ["unload", plist.to_string_lossy().as_ref()]);
    if !process::success("launchctl", ["load", plist.to_string_lossy().as_ref()]) {
        return Err(format!("launchctl could not load {}", plist.display()));
    }
    println!("✓ LaunchAgent installed: {}", plist.display());
    Ok(())
}

fn parse_tunnel_setup(args: &[String]) -> Result<(PathBuf, String, String, u16), String> {
    let mut project = None;
    let mut name = None;
    let mut hostname = None;
    let mut port = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        if !matches!(option, "--project-dir" | "--name" | "--hostname" | "--port") {
            return Err(format!("unknown option: {option}"));
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} needs a value"))?;
        match option {
            "--project-dir" => project = Some(PathBuf::from(value)),
            "--name" => name = Some(value.clone()),
            "--hostname" => hostname = Some(value.clone()),
            "--port" => port = Some(parse_port(value)?),
            _ => return Err(format!("unknown option: {option}")),
        }
        index += 1;
    }
    let project = project
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !project.join("docker-compose.yml").is_file() {
        return Err(format!(
            "{} does not look like the project directory",
            project.display()
        ));
    }
    let config = env_file::read(&project.join(".env")).unwrap_or_default();
    let name = name.unwrap_or_else(|| env_file::value(&config, "TUNNEL_NAME", "dev-workspace"));
    let hostname = hostname
        .or_else(|| {
            let value = env_file::value(&config, "TUNNEL_HOSTNAME", "");
            (!value.is_empty()).then_some(value)
        })
        .ok_or_else(|| "no hostname; pass --hostname or set TUNNEL_HOSTNAME".to_string())?;
    let port = match port {
        Some(port) => port,
        None => parse_port(&env_file::value(&config, "TUNNEL_HOST_PORT", "18080"))?,
    };
    Ok((project, name, hostname, port))
}

fn parse_port(value: &str) -> Result<u16, String> {
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(format!("invalid port: {value}")),
    }
}

fn validate_tunnel_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        Err("tunnel name must use 1-64 letters, digits, hyphens or underscores".to_string())
    } else {
        Ok(())
    }
}

fn cloudflare_identifier(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_dns_label(label: &str) -> Result<(), String> {
    let valid = !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!("invalid DNS label: {label}"))
    }
}

fn validate_hostname(hostname: &str) -> Result<(), String> {
    let valid = hostname.len() <= 253
        && hostname.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(format!("invalid hostname: {hostname}"))
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn provision_for(project: &Path, id: &str, port: u16) -> Result<InstanceState, String> {
    validate_id(id)?;
    require_command("curl")?;
    require_command("cloudflared")?;
    let mut secrets = load_secrets(project)?;
    let client = ApiClient::new(&mut secrets.api_token)?;
    let instances = instances_dir(project);
    fs::create_dir_all(&instances).map_err(|error| error.to_string())?;
    fs::set_permissions(&instances, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let path = state_path(project, id);
    if path.exists() || legacy_state_path(project, id).exists() {
        return Err(format!("instance '{id}' already exists; teardown first"));
    }

    let label = format!("{}-{id}", secrets.subdomain);
    validate_dns_label(&label)?;
    let mut state = InstanceState::default();
    state.version = 1;
    state.id = id.to_string();
    state.fqdn = format!("{label}.{}", secrets.zone_name);
    state.origin_port = port;
    state.team_domain = secrets.team_domain.clone();
    save_state(project, &state)?;
    println!("== provisioning {} (origin 127.0.0.1:{port})", state.fqdn);

    let result = provision_resources(&client, &secrets, project, &mut state);
    if let Err(error) = result {
        eprintln!("provision failed — rolling back {id}: {error}");
        if teardown_resources(&client, &secrets, &state) {
            remove_state_files(project, &state);
        } else {
            eprintln!(
                "rollback incomplete; private state retained at {}",
                path.display()
            );
        }
        return Err(error);
    }

    println!();
    println!("== {id} ready: https://{}/", state.fqdn);
    println!("   human : browser -> Access login -> email OTP");
    println!(
        "   machine: CF-Access-Client-Id={}  (secret remains in private state)",
        state.service_client_id.as_deref().unwrap_or("unavailable")
    );
    println!("   state: {}", path.display());
    Ok(state)
}

fn provision_resources(
    client: &ApiClient,
    secrets: &Secrets,
    project: &Path,
    state: &mut InstanceState,
) -> Result<(), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let tunnel = client.call(
        "POST",
        &format!("/accounts/{}/cfd_tunnel", secrets.account_id),
        Some(&json!({ "name": format!("ws-{}-{stamp}", state.id), "config_src": "cloudflare" })),
    )?;
    state.tunnel_id = Some(field(&tunnel, "id")?);
    state.tunnel_token = Some(field(&tunnel, "token")?);
    save_state(project, state)?;
    println!("  tunnel ok");

    let tunnel_id = state.tunnel_id.as_deref().unwrap_or_default();
    let dns = client.call(
        "POST",
        &format!("/zones/{}/dns_records", secrets.zone_id),
        Some(&json!({
            "type": "CNAME",
            "name": state.fqdn,
            "content": format!("{tunnel_id}.cfargotunnel.com"),
            "proxied": true
        })),
    )?;
    state.dns_record_id = Some(field(&dns, "id")?);
    save_state(project, state)?;
    println!("  dns ok");

    client.call(
        "PUT",
        &format!(
            "/accounts/{}/cfd_tunnel/{tunnel_id}/configurations",
            secrets.account_id
        ),
        Some(&json!({ "config": { "ingress": [
            { "hostname": state.fqdn, "service": format!("http://127.0.0.1:{}", state.origin_port) },
            { "service": "http_status:404" }
        ]}})),
    )?;
    println!("  ingress ok");

    let service = client.call(
        "POST",
        &format!("/accounts/{}/access/service_tokens", secrets.account_id),
        Some(&json!({ "name": format!("ws-{}", state.id) })),
    )?;
    state.service_token_id = Some(field(&service, "id")?);
    state.service_client_id = Some(field(&service, "client_id")?);
    state.service_client_secret = Some(field(&service, "client_secret")?);
    save_state(project, state)?;
    println!("  service-token ok");

    let app = client.call(
        "POST",
        &format!("/accounts/{}/access/apps", secrets.account_id),
        Some(&json!({
            "name": format!("ws-{}", state.id),
            "domain": state.fqdn,
            "type": "self_hosted",
            "session_duration": "30m"
        })),
    )?;
    state.app_id = Some(field(&app, "id")?);
    state.app_aud = Some(field(&app, "aud")?);
    save_state(project, state)?;
    println!("  access-app ok");

    let app_id = state.app_id.as_deref().unwrap_or_default();
    let service_id = state.service_token_id.as_deref().unwrap_or_default();
    let policy = client.call(
        "POST",
        &format!(
            "/accounts/{}/access/apps/{app_id}/policies",
            secrets.account_id
        ),
        Some(&json!({
            "name": "machine",
            "decision": "non_identity",
            "include": [{ "service_token": { "token_id": service_id }}]
        })),
    )?;
    state.policy_service_id = Some(field(&policy, "id")?);
    save_state(project, state)?;
    println!("  policy-machine ok");

    let policy = client.call(
        "POST",
        &format!(
            "/accounts/{}/access/apps/{app_id}/policies",
            secrets.account_id
        ),
        Some(&json!({
            "name": "human",
            "decision": "allow",
            "include": [{ "email": { "email": secrets.allow_email }}]
        })),
    )?;
    state.policy_human_id = Some(field(&policy, "id")?);
    save_state(project, state)?;
    println!("  policy-human ok");

    state.connector_pid = Some(start_connector(project, state)?);
    save_state(project, state)?;
    Ok(())
}

fn teardown_for(project: &Path, id: &str) -> Result<(), String> {
    validate_id(id)?;
    let mut secrets = load_secrets(project)?;
    let client = ApiClient::new(&mut secrets.api_token)?;
    let state = load_state(project, id)?;
    if teardown_resources(&client, &secrets, &state) {
        remove_state_files(project, &state);
        println!("== {id} torn down (state removed)");
        Ok(())
    } else {
        Err(format!(
            "teardown incomplete; state retained at {}",
            state_path(project, id).display()
        ))
    }
}

fn teardown_resources(client: &ApiClient, secrets: &Secrets, state: &InstanceState) -> bool {
    stop_connector(state.connector_pid);
    let mut ok = true;
    if let Some(app) = &state.app_id {
        ok &= client.delete(
            "access app",
            &format!("/accounts/{}/access/apps/{app}", secrets.account_id),
        );
    }
    if let Some(token) = &state.service_token_id {
        ok &= client.delete(
            "service token",
            &format!(
                "/accounts/{}/access/service_tokens/{token}",
                secrets.account_id
            ),
        );
    }
    if let Some(record) = &state.dns_record_id {
        ok &= client.delete(
            "dns record",
            &format!("/zones/{}/dns_records/{record}", secrets.zone_id),
        );
    }
    if let Some(tunnel) = &state.tunnel_id {
        let _ = client.call(
            "DELETE",
            &format!(
                "/accounts/{}/cfd_tunnel/{tunnel}/connections",
                secrets.account_id
            ),
            None,
        );
        let path = format!("/accounts/{}/cfd_tunnel/{tunnel}", secrets.account_id);
        let mut removed = false;
        for _ in 0..5 {
            if client.delete("tunnel", &path) {
                removed = true;
                break;
            }
            thread::sleep(Duration::from_secs(3));
        }
        ok &= removed;
    }
    ok
}

fn start_connector(project: &Path, state: &InstanceState) -> Result<u32, String> {
    let token = state
        .tunnel_token
        .as_deref()
        .ok_or_else(|| "state has no tunnel token".to_string())?;
    let log_path = connector_log_path(project, &state.id);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
        .map_err(|error| error.to_string())?;
    let stderr = log.try_clone().map_err(|error| error.to_string())?;
    let mut child = Command::new("nohup")
        .arg("cloudflared")
        .args(["tunnel", "--no-autoupdate", "run"])
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        )
        .env("TUNNEL_TOKEN", token)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .process_group(0)
        .spawn()
        .map_err(|error| format!("cannot start cloudflared: {error}"))?;
    thread::sleep(Duration::from_secs(1));
    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
        return Err(format!("cloudflared exited immediately: {status}"));
    }
    println!("  connector started (pid {})", child.id());
    Ok(child.id())
}

fn stop_connector(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let pid_text = pid.to_string();
    let command = process::output("ps", ["-p", pid_text.as_str(), "-o", "args="])
        .ok()
        .filter(|output| output.success())
        .map(|output| output.stdout);
    if command
        .as_deref()
        .is_some_and(|name| name.trim().contains("cloudflared"))
    {
        let group = format!("-{pid}");
        if process::success("kill", ["-TERM", "--", group.as_str()]) {
            println!("  stopped cloudflared (pid {pid})");
            thread::sleep(Duration::from_secs(2));
        }
    } else if command.is_some() {
        eprintln!("  WARN refusing to kill reused pid {pid}: it is not cloudflared");
    }
}

fn start_workspace(project: &Path, state: &InstanceState) -> bool {
    let root = project.to_string_lossy();
    Command::new("docker")
        .args([
            "compose",
            "-p",
            &format!("ws-{}", state.id),
            "-f",
            &format!("{root}/docker-compose.yml"),
            "-f",
            &format!("{root}/docker-compose.apparmor.yml"),
            "up",
            "-d",
        ])
        .env("ACCESS_AUD", state.app_aud.as_deref().unwrap_or(""))
        .env(
            "ACCESS_TEAM_DOMAIN",
            state.team_domain.as_deref().unwrap_or(""),
        )
        .env("TUNNEL_HOST_PORT", state.origin_port.to_string())
        .env("WORKSPACE_CONTAINER_NAME", format!("ws-{}", state.id))
        .status()
        .is_ok_and(|status| status.success())
}

fn stop_workspace(project: &Path, id: &str) -> Result<(), String> {
    let root = project.to_string_lossy();
    let status = Command::new("docker")
        .args([
            "compose",
            "-p",
            &format!("ws-{id}"),
            "-f",
            &format!("{root}/docker-compose.yml"),
            "-f",
            &format!("{root}/docker-compose.apparmor.yml"),
            "down",
            "-v",
        ])
        .env("WORKSPACE_CONTAINER_NAME", format!("ws-{id}"))
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("docker compose down exited {status}"))
    }
}

fn load_secrets(project: &Path) -> Result<Secrets, String> {
    let path = project.join("deploy/orchestration/secrets.env");
    env_file::ensure_private(&path)?;
    let mut values = env_file::read(&path)?;
    let required = |values: &mut HashMap<String, String>, name: &str| {
        values
            .remove(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing {name} in {}", path.display()))
    };
    let email = required(&mut values, "ALLOW_EMAIL")?;
    if !email.contains('@')
        || !email
            .rsplit_once('.')
            .is_some_and(|(left, right)| left.contains('@') && !right.is_empty())
    {
        return Err("ALLOW_EMAIL is not a full address".to_string());
    }
    let account_id = required(&mut values, "CF_ACCOUNT_ID")?;
    let zone_id = required(&mut values, "CF_ZONE_ID")?;
    if !cloudflare_identifier(&account_id) || !cloudflare_identifier(&zone_id) {
        return Err("CF_ACCOUNT_ID and CF_ZONE_ID must be 32 hexadecimal characters".to_string());
    }
    let zone_name = required(&mut values, "CF_ZONE_NAME")?;
    validate_hostname(&zone_name)?;
    let subdomain = values
        .remove("WS_SUBDOMAIN")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ws".to_string());
    validate_dns_label(&subdomain)?;
    let team_domain = values
        .remove("CF_TEAM_DOMAIN")
        .filter(|value| !value.is_empty());
    if let Some(domain) = &team_domain {
        validate_hostname(domain)?;
    }
    let api_token = required(&mut values, "CF_API_TOKEN")?;
    for value in values.values_mut() {
        value.zeroize();
    }
    Ok(Secrets {
        api_token,
        account_id,
        zone_id,
        zone_name,
        subdomain,
        allow_email: email,
        team_domain,
    })
}

fn save_state(project: &Path, state: &InstanceState) -> Result<(), String> {
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?,
    );
    env_file::write_private(&state_path(project, &state.id), bytes.as_slice())
}

fn load_state(project: &Path, id: &str) -> Result<InstanceState, String> {
    let path = state_path(project, id);
    if path.exists() {
        env_file::ensure_private(&path)?;
        let bytes = zeroize::Zeroizing::new(fs::read(&path).map_err(|error| error.to_string())?);
        let state: InstanceState = serde_json::from_slice(bytes.as_slice())
            .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
        if state.version != 1 || state.id != id {
            return Err(format!(
                "{} has incompatible or mismatched state",
                path.display()
            ));
        }
        return Ok(state);
    }
    let legacy = legacy_state_path(project, id);
    env_file::ensure_private(&legacy)?;
    let values = env_file::read(&legacy)?;
    Ok(state_from_legacy(id, values))
}

fn state_from_legacy(id: &str, values: HashMap<String, String>) -> InstanceState {
    let get = |name: &str| values.get(name).cloned().filter(|value| !value.is_empty());
    InstanceState {
        version: 1,
        id: id.to_string(),
        fqdn: get("FQDN").unwrap_or_default(),
        origin_port: get("ORIGIN_PORT")
            .and_then(|port| port.parse().ok())
            .unwrap_or(0),
        team_domain: get("TEAM_DOMAIN"),
        tunnel_id: get("TUNNEL_ID"),
        tunnel_token: get("TUNNEL_TOKEN"),
        dns_record_id: get("DNS_RECORD_ID"),
        service_token_id: get("ST_ID"),
        service_client_id: get("ST_CLIENT_ID"),
        service_client_secret: get("ST_CLIENT_SECRET"),
        app_id: get("APP_ID"),
        app_aud: get("APP_AUD"),
        policy_service_id: get("POLICY_SVC_ID"),
        policy_human_id: get("POLICY_HUMAN_ID"),
        connector_pid: get("CFD_PID").and_then(|pid| pid.parse().ok()),
    }
}

fn remove_state_files(project: &Path, state: &InstanceState) {
    let _ = fs::remove_file(connector_log_path(project, &state.id));
    let _ = fs::remove_file(state_path(project, &state.id));
    let _ = fs::remove_file(legacy_state_path(project, &state.id));
}

fn parse_id_port(args: &[String]) -> Result<(PathBuf, String, u16), String> {
    let (project, id, port) = parse_common(args)?;
    Ok((project, id, port.unwrap_or(18091)))
}

fn parse_project_id_optional_port(
    args: &[String],
) -> Result<(PathBuf, String, Option<u16>), String> {
    parse_common(args)
}

fn parse_project_id(args: &[String]) -> Result<(PathBuf, String), String> {
    let (project, id, port) = parse_common(args)?;
    if port.is_some() {
        return Err("this command does not accept an origin port".to_string());
    }
    Ok((project, id))
}

fn parse_common(args: &[String]) -> Result<(PathBuf, String, Option<u16>), String> {
    let mut project = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--project-dir" {
            index += 1;
            project = Some(PathBuf::from(
                args.get(index)
                    .ok_or_else(|| "--project-dir needs a path".to_string())?,
            ));
        } else if args[index].starts_with('-') {
            return Err(format!("unknown option: {}", args[index]));
        } else {
            positional.push(args[index].clone());
        }
        index += 1;
    }
    let project = project
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !project.join("docker-compose.yml").is_file() {
        return Err(format!(
            "{} does not look like the project directory",
            project.display()
        ));
    }
    let id = positional
        .first()
        .cloned()
        .ok_or_else(|| "instance id is required".to_string())?;
    let port = positional.get(1).map(|port| parse_port(port)).transpose()?;
    if positional.len() > 2 {
        return Err("too many arguments".to_string());
    }
    validate_id(&id)?;
    Ok((project, id, port))
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 40
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || id.starts_with('-')
        || id.ends_with('-')
    {
        Err(format!(
            "invalid instance id '{id}'; use 1-40 letters, digits or internal hyphens"
        ))
    } else {
        Ok(())
    }
}

fn choose_port() -> Result<u16, String> {
    for port in 18090..=18189 {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        if TcpListener::bind(address).is_ok() {
            return Ok(port);
        }
    }
    Err("no free loopback port in 18090-18189".to_string())
}

fn require_command(command: &str) -> Result<(), String> {
    if process::command_exists(command) {
        Ok(())
    } else {
        Err(format!("{command} not found"))
    }
}

fn field(value: &Value, name: &str) -> Result<String, String> {
    value[name]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Cloudflare response has no result.{name}"))
}

fn missing_resource(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("already deleted")
}

fn instances_dir(project: &Path) -> PathBuf {
    project.join("deploy/orchestration/instances")
}

fn state_path(project: &Path, id: &str) -> PathBuf {
    instances_dir(project).join(format!("{id}.json"))
}

fn legacy_state_path(project: &Path, id: &str) -> PathBuf {
    instances_dir(project).join(format!("{id}.env"))
}

fn connector_log_path(project: &Path, id: &str) -> PathBuf {
    instances_dir(project).join(format!("{id}.cloudflared.log"))
}

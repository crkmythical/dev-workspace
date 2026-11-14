use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::{env_file, process};

const CLOUDFLARED_UNIT: &str = "cloudflared-dev-workspace.service";
const WATCHER_UNIT: &str = "dev-workspace-host-watcher.service";

#[derive(Default)]
struct Options {
    project_dir: Option<PathBuf>,
    cloudflared: bool,
    watcher: bool,
    uninstall: bool,
    dry_run: bool,
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!(
            "usage: sdw-host services --project-dir DIR [--cloudflared] [--host-watcher] [--uninstall] [--dry-run]"
        );
        return 0;
    }
    match parse(args).and_then(execute) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("services: {error}");
            1
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
                options.project_dir = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--project-dir needs a path".to_string())?,
                ));
            }
            "--cloudflared" => options.cloudflared = true,
            "--host-watcher" => options.watcher = true,
            "--uninstall" => options.uninstall = true,
            "--dry-run" => options.dry_run = true,
            option => return Err(format!("unknown option: {option}")),
        }
        index += 1;
    }
    Ok(options)
}

fn execute(options: Options) -> Result<(), String> {
    if std::env::consts::OS != "linux" && !options.dry_run {
        return Err("systemd user services are supported on Linux only".to_string());
    }
    let project = options
        .project_dir
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !project.join("docker-compose.yml").is_file() {
        return Err(format!(
            "{} does not look like the project directory",
            project.display()
        ));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())?;
    let unit_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("systemd/user");

    if options.uninstall {
        for name in [CLOUDFLARED_UNIT, WATCHER_UNIT] {
            if options.dry_run {
                println!("would remove {}", unit_dir.join(name).display());
            } else {
                let _ = process::success("systemctl", ["--user", "disable", "--now", name]);
                remove_if_present(&unit_dir.join(name))?;
            }
        }
        if !options.dry_run {
            reload_systemd()?;
        }
        return Ok(());
    }
    if !options.cloudflared && !options.watcher {
        return Err("pass --cloudflared and/or --host-watcher (or --uninstall)".to_string());
    }

    let config = env_file::read(&project.join(".env")).unwrap_or_default();
    if options.cloudflared {
        let name = env_file::value(&config, "TUNNEL_NAME", "dev-workspace");
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        {
            return Err("TUNNEL_NAME has an unsafe shape".to_string());
        }
        let binary = process::find_command("cloudflared")
            .unwrap_or_else(|| PathBuf::from("/usr/local/bin/cloudflared"));
        let tunnel_config = home.join(format!(".cloudflared/sdw-{name}.yml"));
        if !options.dry_run && !tunnel_config.is_file() {
            return Err(format!(
                "{} is missing; run setup-cloudflared.sh first",
                tunnel_config.display()
            ));
        }
        let unit = cloudflared_unit(&binary, &tunnel_config, &name);
        install_unit(&unit_dir, CLOUDFLARED_UNIT, &unit, options.dry_run)?;
    }
    if options.watcher {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let context = std::env::var("DOCKER_CONTEXT")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| env_file::value(&config, "SDW_DOCKER_CONTEXT", ""));
        let unit = watcher_unit(&executable, &project, &context);
        install_unit(&unit_dir, WATCHER_UNIT, &unit, options.dry_run)?;
    }
    if !options.dry_run {
        let user = std::env::var("USER").unwrap_or_default();
        if !user.is_empty() && !process::success("loginctl", ["enable-linger", user.as_str()]) {
            println!("Hint: sudo loginctl enable-linger {user}");
        }
    }
    Ok(())
}

fn cloudflared_unit(binary: &Path, config: &Path, tunnel: &str) -> String {
    format!(
        "[Unit]\nDescription=Cloudflare Tunnel for dev-workspace\nAfter=network-online.target\nWants=network-online.target\n\n\
         [Service]\nExecStart={} --config {} tunnel run {}\nRestart=always\nRestartSec=5\n\n\
         [Install]\nWantedBy=default.target\n",
        unit_quote(&binary.to_string_lossy()),
        unit_quote(&config.to_string_lossy()),
        unit_quote(tunnel),
    )
}

fn watcher_unit(executable: &Path, project: &Path, context: &str) -> String {
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    format!(
        "[Unit]\nDescription=dev-workspace host watcher\nAfter=network-online.target\n\n\
         [Service]\nWorkingDirectory={}\nExecStart={} watch --project-dir {}\nEnvironment={}\nEnvironment={}\nRestart=always\nRestartSec=10\n\n\
         [Install]\nWantedBy=default.target\n",
        unit_quote(&project.to_string_lossy()),
        unit_quote(&executable.to_string_lossy()),
        unit_quote(&project.to_string_lossy()),
        unit_quote(&format!("PATH={path}")),
        unit_quote(&format!("DOCKER_CONTEXT={context}")),
    )
}

fn install_unit(directory: &Path, name: &str, contents: &str, dry_run: bool) -> Result<(), String> {
    if dry_run {
        println!("# {}\n{contents}", directory.join(name).display());
        return Ok(());
    }
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let path = directory.join(name);
    fs::write(&path, contents).map_err(|error| error.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
        .map_err(|error| error.to_string())?;
    reload_systemd()?;
    if !process::success("systemctl", ["--user", "enable", "--now", name]) {
        return Err(format!("systemctl could not enable {name}"));
    }
    println!("✓ {name} installed and started");
    Ok(())
}

fn reload_systemd() -> Result<(), String> {
    if process::success("systemctl", ["--user", "daemon-reload"]) {
        Ok(())
    } else {
        Err("systemctl --user daemon-reload failed".to_string())
    }
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
    }
}

fn unit_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace(['\n', '\r'], "")
    )
}

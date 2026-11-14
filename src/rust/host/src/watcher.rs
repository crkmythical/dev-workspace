use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{docker_context, env_file, process};

#[derive(Default)]
struct Options {
    project_dir: Option<PathBuf>,
    mode: Mode,
}

#[derive(Default)]
enum Mode {
    #[default]
    Daemon,
    Once,
    Install,
    Uninstall,
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: sdw-host watch --project-dir DIR [--once | --install | --uninstall]");
        return 0;
    }
    match parse(args).and_then(execute) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("host-watcher: {error}");
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
            "--once" => options.mode = Mode::Once,
            "--install" => options.mode = Mode::Install,
            "--uninstall" => options.mode = Mode::Uninstall,
            option => return Err(format!("unknown option: {option}")),
        }
        index += 1;
    }
    Ok(options)
}

fn execute(options: Options) -> Result<i32, String> {
    let project_dir = options
        .project_dir
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| format!("cannot resolve project directory: {error}"))?;
    let config = env_file::read(&project_dir.join(".env")).unwrap_or_default();
    let container = env_file::value(&config, "WORKSPACE_CONTAINER_NAME", "dev-workspace");
    match options.mode {
        Mode::Daemon => daemon(&project_dir, &container),
        Mode::Once => once(&project_dir, &container),
        Mode::Install => install(&project_dir),
        Mode::Uninstall => uninstall(),
    }
}

fn inspect(container: &str) -> Option<serde_json::Value> {
    let output = process::output("docker", ["inspect", container]).ok()?;
    if !output.success() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&output.stdout)
        .ok()?
        .as_array()?
        .first()
        .cloned()
}

fn marker_present(container: &str) -> bool {
    let Some(info) = inspect(container) else {
        return false;
    };
    let volume = info["Mounts"].as_array().and_then(|mounts| {
        mounts.iter().find_map(|mount| {
            (mount["Destination"].as_str() == Some("/var/run/self-destruct"))
                .then(|| mount["Name"].as_str().map(str::to_string))
                .flatten()
        })
    });
    let image = info["Config"]["Image"].as_str();
    match (volume, image) {
        (Some(volume), Some(image)) => process::success(
            "docker",
            [
                "run",
                "--rm",
                "--entrypoint",
                "test",
                "-v",
                &format!("{volume}:/s:ro"),
                image,
                "-f",
                "/s/completed",
            ],
        ),
        _ => false,
    }
}

fn once(project_dir: &Path, container: &str) -> Result<i32, String> {
    require_docker()?;
    docker_context::enforce(project_dir)?;
    if marker_present(container) {
        cleanup_outside_service(project_dir)?;
    } else {
        println!("No self-destruct marker found.");
    }
    Ok(0)
}

fn daemon(project_dir: &Path, container: &str) -> Result<i32, String> {
    require_docker()?;
    let context = docker_context::enforce(project_dir)?;
    println!(
        "[{}] host-watcher started (container: {container}, context: {context})",
        timestamp()
    );
    if marker_present(container) {
        cleanup_outside_service(project_dir)?;
        return Ok(0);
    }
    let mut child = Command::new("docker")
        .args([
            "events",
            "--filter",
            &format!("container={container}"),
            "--filter",
            "event=die",
            "--format",
            "{{json .}}",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot start docker events: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "docker events has no stdout".to_string())?;
    for line in BufReader::new(stdout).lines() {
        if line.is_err() {
            break;
        }
        if marker_present(container) {
            let _ = child.kill();
            let _ = child.wait();
            cleanup_outside_service(project_dir)?;
            return Ok(0);
        }
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    Err(format!("docker events ended unexpectedly: {status}"))
}

fn cleanup_outside_service(project_dir: &Path) -> Result<(), String> {
    println!(
        "[{}] Self-destruct confirmed. Running host cleanup...",
        timestamp()
    );
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let project = project_dir.to_string_lossy().into_owned();
    let executable_text = executable.to_string_lossy().into_owned();
    let common = [
        "destroy",
        "--project-dir",
        project.as_str(),
        "--force",
        "--paranoid",
        "--delete-project",
    ];

    let status = if std::env::consts::OS == "macos" && process::command_exists("perl") {
        Command::new("perl")
            .args(["-MPOSIX", "-e", "POSIX::setsid(); exec @ARGV"])
            .arg(&executable)
            .args(common)
            .status()
    } else if std::env::consts::OS == "linux"
        && process::command_exists("systemd-run")
        && process::success("systemctl", ["--user", "show-environment"])
    {
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
        let context = std::env::var("DOCKER_CONTEXT").unwrap_or_default();
        Command::new("systemd-run")
            .args([
                "--user",
                "--quiet",
                "--collect",
                "--pipe",
                "--wait",
                &format!("--working-directory={project}"),
                &format!("--setenv=PATH={path}"),
                &format!("--setenv=DOCKER_CONTEXT={context}"),
                "--",
                executable_text.as_str(),
            ])
            .args(common)
            .status()
    } else {
        Command::new(&executable).args(common).status()
    }
    .map_err(|error| format!("cannot start detached cleanup: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("host cleanup exited {status}"))
    }
}

fn install(project_dir: &Path) -> Result<i32, String> {
    if std::env::consts::OS != "macos" {
        return Err(
            "LaunchAgents are macOS only; on Linux use deploy/linux-systemd/install.sh --host-watcher"
                .to_string(),
        );
    }
    let home = home()?;
    let launch_agents = home.join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents).map_err(|error| error.to_string())?;
    let plist = launch_agents.join("com.dev-workspace.host-watcher.plist");
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    let project_config = env_file::read(&project_dir.join(".env")).unwrap_or_default();
    let context = std::env::var("DOCKER_CONTEXT")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| env_file::value(&project_config, "SDW_DOCKER_CONTEXT", ""));
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.dev-workspace.host-watcher</string>
  <key>ProgramArguments</key><array>
    <string>{}</string><string>watch</string><string>--project-dir</string><string>{}</string>
  </array>
  <key>WorkingDirectory</key><string>{}</string>
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>{}</string>
    <key>DOCKER_CONTEXT</key><string>{}</string>
  </dict>
  <key>KeepAlive</key><true/><key>RunAtLoad</key><true/>
  <key>StandardOutPath</key><string>/tmp/dev-workspace-watcher.log</string>
  <key>StandardErrorPath</key><string>/tmp/dev-workspace-watcher.log</string>
</dict></plist>
"#,
        xml_escape(&executable.to_string_lossy()),
        xml_escape(&project_dir.to_string_lossy()),
        xml_escape(&project_dir.to_string_lossy()),
        xml_escape(&path),
        xml_escape(&context),
    );
    fs::write(&plist, xml).map_err(|error| error.to_string())?;
    fs::set_permissions(&plist, fs::Permissions::from_mode(0o644))
        .map_err(|error| error.to_string())?;
    let _ = process::success("launchctl", ["unload", plist.to_string_lossy().as_ref()]);
    if !process::success("launchctl", ["load", plist.to_string_lossy().as_ref()]) {
        return Err(format!("launchctl could not load {}", plist.display()));
    }
    println!("Installed and loaded: {}", plist.display());
    Ok(0)
}

fn uninstall() -> Result<i32, String> {
    let plist = home()?.join("Library/LaunchAgents/com.dev-workspace.host-watcher.plist");
    let _ = process::success("launchctl", ["unload", plist.to_string_lossy().as_ref()]);
    match fs::remove_file(&plist) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    println!("Uninstalled: {}", plist.display());
    Ok(0)
}

fn require_docker() -> Result<(), String> {
    if process::command_exists("docker") {
        Ok(())
    } else {
        Err("docker not found".to_string())
    }
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

fn timestamp() -> String {
    process::output("date", ["-Iseconds"])
        .ok()
        .filter(|output| output.success())
        .map(|output| output.stdout.trim().to_string())
        .unwrap_or_else(|| "unknown-time".to_string())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

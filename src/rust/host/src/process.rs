use std::ffi::OsStr;
use std::process::{Command, ExitStatus, Stdio};

pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

pub fn output<I, S>(program: &str, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    Ok(Output {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}

pub fn success<I, S>(program: &str, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn command_exists(name: &str) -> bool {
    find_command(name).is_some()
}

pub fn find_command(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Clear a network child's environment and restore only process lookup, TLS and proxy plumbing.
/// Callers pass credentials through a dedicated file/stdin or add one explicit variable afterwards.
pub fn retain_network_environment(command: &mut Command) {
    const KEEP: &[&str] = &[
        "PATH",
        "HOME",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
    ];
    command.env_clear();
    for name in KEEP {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

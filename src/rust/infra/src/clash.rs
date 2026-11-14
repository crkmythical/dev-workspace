//! Clash (mihomo) as the container drives it: the rendered config on disk, and the REST controller
//! on loopback.
//!
//! One place renders the config for every caller (the boot, `setup`, `set-clash-dns`, the
//! standalone client), so the template, the file's owner and mode and the controller secret cannot
//! drift apart between them.
//!
//! The controller demands a bearer secret that lives in the rendered config (mode 0600, owned by
//! the clash user). Requests go over a plain TCP socket rather than through curl, so the secret
//! never sits on a command line that every process in the container can read, and the wire format
//! is unit-tested against a local listener.

use crate::egress::{resolve_gid, resolve_uid};
use crate::secrets::random_hex;
use sdw_core::egress::domain::{parse_controller_secret, render_clash_config, ClashRender};
use sdw_core::egress::layout::{
    CLASH_CONFIG_PATH, CLASH_CONTROLLER_PORT, CLASH_PROVIDERS_DIR, CLASH_TEMPLATE_PATH, CLASH_USER,
};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

/// Bytes of randomness in a controller secret (hex-encoded in the file: twice as many characters).
const SECRET_BYTES: usize = 32;

/// What to do about the controller secret when the config is rendered again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretPolicy {
    /// Mint a new one: for a start, when no running Clash holds the old one.
    Rotate,
    /// Reuse the one already on disk (minting only if there is none): for a re-render while Clash
    /// runs, so the controller and the file keep agreeing.
    Keep,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigOptions<'a> {
    pub subscription_url: &'a str,
    pub dns_nameservers: &'a str,
    pub allow_lan: bool,
    pub secret: SecretPolicy,
}

/// The controller secret in the config on disk, if there is a readable one.
pub fn read_controller_secret() -> Option<String> {
    read_secret_from(Path::new(CLASH_CONFIG_PATH))
}

/// Whether there is a config we rendered for Clash to start from (one that carries the controller
/// secret). Clash must not start without it: given no config.yaml it writes a default of its own and
/// listens on 7890 as a plain DIRECT proxy, which would pass the egress probe while no subscription
/// is in use. Not starting is the fail-closed answer.
pub fn config_is_ready() -> bool {
    read_controller_secret().is_some()
}

fn read_secret_from(config: &Path) -> Option<String> {
    parse_controller_secret(&std::fs::read_to_string(config).ok()?)
}

/// Render the image's template into the live config: mode 0600, owned by the clash user (so the
/// unprivileged Clash can read it and nobody else can), replaced atomically so a reader never sees
/// half a file. Creates the provider cache directory too.
pub fn write_config(opts: &ConfigOptions) -> Result<(), String> {
    let template = std::fs::read_to_string(CLASH_TEMPLATE_PATH)
        .map_err(|e| format!("cannot read the Clash template {CLASH_TEMPLATE_PATH}: {e}"))?;
    let owner = match (resolve_uid(CLASH_USER), resolve_gid(CLASH_USER)) {
        (Some(uid), Some(gid)) => Some((uid, gid)),
        _ => None, // an image without the user: Clash runs as root, which reads any file
    };
    let _ = std::fs::create_dir_all(CLASH_PROVIDERS_DIR);
    write_config_at(&template, Path::new(CLASH_CONFIG_PATH), opts, owner)
}

fn write_config_at(
    template: &str,
    config: &Path,
    opts: &ConfigOptions,
    owner: Option<(u32, u32)>,
) -> Result<(), String> {
    let kept = match opts.secret {
        SecretPolicy::Keep => read_secret_from(config),
        SecretPolicy::Rotate => None,
    };
    let secret = match kept {
        Some(s) => s,
        None => random_hex(SECRET_BYTES)?,
    };
    let text = render_clash_config(
        template,
        &ClashRender {
            subscription_url: opts.subscription_url,
            dns_nameservers: opts.dns_nameservers,
            controller_secret: &secret,
            allow_lan: opts.allow_lan,
        },
    )?;
    write_private(config, &text, owner)
}

/// Write `text` to `path` through a sibling temp file created 0600, owned by `owner`, then rename.
fn write_private(path: &Path, text: &str, owner: Option<(u32, u32)>) -> Result<(), String> {
    let tmp = path.with_extension("yaml.tmp");
    let _ = std::fs::remove_file(&tmp);
    let fail = |what: &str, e: std::io::Error| format!("{what} {}: {e}", tmp.display());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| fail("cannot create", e))?;
    let result = (|| {
        file.write_all(text.as_bytes())
            .map_err(|e| fail("cannot write", e))?;
        if let Some((uid, gid)) = owner {
            std::os::unix::fs::fchown(&file, Some(uid), Some(gid))
                .map_err(|e| fail("cannot chown", e))?;
        }
        file.sync_all().map_err(|e| fail("cannot sync", e))
    })();
    drop(file);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

// --- the controller ---

/// One answer from the controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// How long a request may take to answer. Almost everything is instant; asking Clash to fetch the
/// subscription again waits for that fetch.
pub const QUICK: Duration = Duration::from_secs(5);
pub const REFRESH: Duration = Duration::from_secs(60);

/// A client for the controller on 127.0.0.1. Holds the secret read from the config at creation.
pub struct Controller {
    port: u16,
    secret: Option<String>,
}

impl Controller {
    /// The controller of the running Clash, authenticated with the secret in the live config.
    pub fn from_config() -> Self {
        Self::new(CLASH_CONTROLLER_PORT, read_controller_secret())
    }

    pub fn new(port: u16, secret: Option<String>) -> Self {
        Controller { port, secret }
    }

    pub fn get(&self, path: &str, timeout: Duration) -> Result<Reply, String> {
        self.request("GET", path, None, timeout)
    }

    /// PUT with an optional JSON body.
    pub fn put(&self, path: &str, json: Option<&str>, timeout: Duration) -> Result<Reply, String> {
        self.request("PUT", path, json, timeout)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        timeout: Duration,
    ) -> Result<Reply, String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let mut stream = TcpStream::connect_timeout(&addr, QUICK)
            .map_err(|e| format!("controller not reachable on {addr}: {e}"))?;
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(QUICK));
        stream
            .write_all(
                build_request(method, path, self.port, self.secret.as_deref(), body).as_bytes(),
            )
            .map_err(|e| format!("controller request failed: {e}"))?;
        // HTTP/1.0: the server answers and closes, so reading to the end is the whole response.
        // The cap keeps a misbehaving peer from filling memory.
        let mut raw = Vec::new();
        stream
            .take(MAX_REPLY)
            .read_to_end(&mut raw)
            .map_err(|e| format!("controller response failed: {e}"))?;
        parse_reply(&raw)
    }
}

const MAX_REPLY: u64 = 16 * 1024 * 1024;

/// The request as text. HTTP/1.0 so the reply is neither chunked nor kept alive.
fn build_request(
    method: &str,
    path: &str,
    port: u16,
    secret: Option<&str>,
    body: Option<&str>,
) -> String {
    let mut req = format!("{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n");
    if let Some(secret) = secret {
        req.push_str(&format!("Authorization: Bearer {secret}\r\n"));
    }
    match body {
        Some(body) => req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )),
        None => req.push_str("\r\n"),
    }
    req
}

fn parse_reply(raw: &[u8]) -> Result<Reply, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "controller sent no complete response".to_string())?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| "controller sent a malformed status line".to_string())?;
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = &raw[split + 4..];
    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };
    Ok(Reply {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Join the pieces of a `Transfer-Encoding: chunked` body.
fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "controller sent a malformed chunked body".to_string();
    let mut out = Vec::new();
    loop {
        let eol = rest.windows(2).position(|w| w == b"\r\n").ok_or_else(bad)?;
        let size_text = String::from_utf8_lossy(&rest[..eol]);
        // A chunk size may carry extensions after a semicolon.
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| bad())?;
        rest = &rest[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size + 2 {
            return Err(bad());
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    /// Permission bits of a file.
    fn file_mode(path: &Path) -> Option<u32> {
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    }

    const TEMPLATE: &str = "allow-lan: {{CLASH_ALLOW_LAN}}\nsecret: \"{{CLASH_CONTROLLER_SECRET}}\"\n\
                            dns:\n  nameserver:\n{{CLASH_DNS_NAMESERVER}}\nurl: \"{{CLASH_SUBSCRIPTION_URL}}\"\n";

    fn opts(policy: SecretPolicy) -> ConfigOptions<'static> {
        ConfigOptions {
            subscription_url: "https://sub.example/x",
            dns_nameservers: "223.5.5.5",
            allow_lan: false,
            secret: policy,
        }
    }

    #[test]
    fn the_config_is_private_and_carries_a_fresh_secret() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.yaml");
        write_config_at(TEMPLATE, &config, &opts(SecretPolicy::Rotate), None).unwrap();
        assert_eq!(file_mode(&config), Some(0o600));
        let secret = read_secret_from(&config).unwrap();
        assert_eq!(secret.len(), SECRET_BYTES * 2);
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.contains("url: \"https://sub.example/x\""));
        // No temp file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["config.yaml"]);
    }

    #[test]
    fn rotate_changes_the_secret_and_keep_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.yaml");
        write_config_at(TEMPLATE, &config, &opts(SecretPolicy::Rotate), None).unwrap();
        let first = read_secret_from(&config).unwrap();

        write_config_at(TEMPLATE, &config, &opts(SecretPolicy::Keep), None).unwrap();
        assert_eq!(read_secret_from(&config).unwrap(), first);

        write_config_at(TEMPLATE, &config, &opts(SecretPolicy::Rotate), None).unwrap();
        assert_ne!(read_secret_from(&config).unwrap(), first);
    }

    #[test]
    fn keep_mints_a_secret_when_there_is_none_to_keep() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.yaml");
        write_config_at(TEMPLATE, &config, &opts(SecretPolicy::Keep), None).unwrap();
        assert!(read_secret_from(&config).is_some());

        // A damaged file is not a secret to keep.
        std::fs::write(&config, "secret: not-quoted\n").unwrap();
        write_config_at(TEMPLATE, &config, &opts(SecretPolicy::Keep), None).unwrap();
        assert!(read_secret_from(&config).is_some());
    }

    #[test]
    fn a_broken_template_leaves_the_existing_config_alone() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.yaml");
        std::fs::write(&config, "old: config\n").unwrap();
        let err = write_config_at(
            "no placeholders here\n",
            &config,
            &opts(SecretPolicy::Rotate),
            None,
        )
        .unwrap_err();
        assert!(err.contains("template"), "{err}");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "old: config\n");
    }

    #[test]
    fn the_request_names_the_secret_only_in_a_header() {
        let req = build_request(
            "PUT",
            "/configs?force=true",
            9090,
            Some("tok"),
            Some("{\"a\":1}"),
        );
        assert!(req.starts_with("PUT /configs?force=true HTTP/1.0\r\n"));
        assert!(req.contains("\r\nAuthorization: Bearer tok\r\n"));
        assert!(req.contains("\r\nContent-Length: 7\r\n\r\n{\"a\":1}"));
        let bare = build_request("GET", "/x", 9090, None, None);
        assert!(!bare.contains("Authorization"));
        assert!(bare.ends_with("\r\n\r\n"));
    }

    #[test]
    fn replies_parse_plain_and_chunked() {
        let plain = parse_reply(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!(
            plain,
            Reply {
                status: 200,
                body: "{}".to_string()
            }
        );
        assert!(plain.ok());

        let chunked = parse_reply(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(chunked.body, "hello world");

        let denied =
            parse_reply(b"HTTP/1.1 401 Unauthorized\r\n\r\n{\"message\":\"Unauthorized\"}")
                .unwrap();
        assert!(!denied.ok());
        assert_eq!(denied.status, 401);

        let empty = parse_reply(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
        assert!(empty.ok());
        assert_eq!(empty.body, "");

        assert!(parse_reply(b"").is_err());
        assert!(parse_reply(b"garbage").is_err());
        assert!(parse_reply(b"HTTP/1.1 abc\r\n\r\n").is_err());
        assert!(
            parse_reply(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_err()
        );
        assert!(parse_reply(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nshort\r\n"
        )
        .is_err());
    }

    /// A one-shot HTTP server on a free port: reads one request, sends `response`, returns the
    /// request text it saw.
    fn serve_once(response: &'static [u8]) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            // Read until the headers (and a declared body) are in.
            loop {
                let n = conn.read(&mut buf).unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).into_owned();
                if let Some(end) = text.find("\r\n\r\n") {
                    let declared = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + declared {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            conn.write_all(response).unwrap();
            String::from_utf8_lossy(&seen).into_owned()
        });
        (port, handle)
    }

    #[test]
    fn a_request_reaches_the_server_with_its_secret_and_the_reply_comes_back() {
        let (port, server) = serve_once(b"HTTP/1.0 200 OK\r\n\r\n{\"proxies\":[]}");
        let reply = Controller::new(port, Some("tok".into()))
            .put(
                "/providers/proxies/subscription",
                Some("{\"x\":true}"),
                QUICK,
            )
            .unwrap();
        assert_eq!(reply.body, "{\"proxies\":[]}");
        let seen = server.join().unwrap();
        assert!(
            seen.starts_with("PUT /providers/proxies/subscription HTTP/1.0\r\n"),
            "{seen}"
        );
        assert!(seen.contains("Authorization: Bearer tok\r\n"), "{seen}");
        assert!(seen.ends_with("{\"x\":true}"), "{seen}");
    }

    #[test]
    fn a_closed_port_is_an_error_not_a_hang() {
        // Bind then drop: the port is free and nothing listens on it.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = Controller::new(port, None).get("/", QUICK).unwrap_err();
        assert!(err.contains("not reachable"), "{err}");
    }
}

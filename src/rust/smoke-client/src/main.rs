use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sdw_core::sync::client::{
    build_chunk_request, build_finalize_request, build_list_headers, SyncNonce, SyncRequest,
};
use sdw_core::sync::crypto::derive_key;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

struct HttpClient {
    host: String,
    port: u16,
    prefix: String,
    cookie: Option<String>,
}

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| json!({}))
    }
}

impl HttpClient {
    fn from_base(base: &str) -> Result<Self, String> {
        let rest = base
            .strip_prefix("http://")
            .ok_or_else(|| "sync smoke accepts an http:// loopback URL only".to_string())?;
        let (authority, suffix) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| format!("base URL needs an explicit port: {base}"))?;
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("invalid port in {base}"))?;
        Ok(Self {
            host: host.to_string(),
            port,
            prefix: if suffix.is_empty() {
                String::new()
            } else {
                format!("/{}", suffix.trim_end_matches('/'))
            },
            cookie: None,
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, String)],
        body: &[u8],
    ) -> Result<Reply, String> {
        let address = format!("{}:{}", self.host, self.port);
        let socket = address
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .next()
            .ok_or_else(|| format!("cannot resolve {address}"))?;
        let mut stream = TcpStream::connect_timeout(&socket, Duration::from_secs(10))
            .map_err(|e| format!("connect {address}: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| e.to_string())?;

        let target = format!("{}{}", self.prefix, path);
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.host,
            self.port,
            body.len()
        )
        .map_err(|e| e.to_string())?;
        if let Some(cookie) = &self.cookie {
            write!(stream, "Cookie: {cookie}\r\n").map_err(|e| e.to_string())?;
        }
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").map_err(|e| e.to_string())?;
        }
        stream.write_all(b"\r\n").map_err(|e| e.to_string())?;
        stream.write_all(body).map_err(|e| e.to_string())?;
        stream.flush().map_err(|e| e.to_string())?;

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("read response: {e}"))?;
        parse_response(&raw)
    }

    fn login(&mut self, password: &str) -> Result<(), String> {
        let body = format!("password={}&next=%2F", form_component(password));
        let reply = self.request(
            "POST",
            "/auth/login",
            &[(
                "content-type",
                "application/x-www-form-urlencoded".to_string(),
            )],
            body.as_bytes(),
        )?;
        if reply.status != 303 {
            return Err(format!("auth gateway returned HTTP {}", reply.status));
        }
        let cookie = reply
            .headers
            .get("set-cookie")
            .and_then(|value| value.split(';').next())
            .filter(|value| value.starts_with("sdw_session="))
            .ok_or_else(|| "auth gateway returned no session cookie".to_string())?;
        self.cookie = Some(cookie.to_string());
        Ok(())
    }

    fn sync_request(&self, request: &SyncRequest) -> Result<Reply, String> {
        let headers: Vec<(&str, String)> = request
            .headers
            .iter()
            .map(|(name, value)| (*name, value.clone()))
            .collect();
        self.request("POST", "/sync/api/upload", &headers, &request.body)
    }
}

fn parse_response(raw: &[u8]) -> Result<Reply, String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "HTTP response has no header terminator".to_string())?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|e| e.to_string())?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "HTTP response has no status".to_string())?;
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let encoded = &raw[split + 4..];
    let body = if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        decode_chunked(encoded)?
    } else if let Some(length) = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
    {
        encoded[..encoded.len().min(length)].to_vec()
    } else {
        encoded.to_vec()
    };
    Ok(Reply {
        status,
        headers,
        body,
    })
}

fn decode_chunked(mut encoded: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "malformed chunked response".to_string())?;
        let line = std::str::from_utf8(&encoded[..line_end]).map_err(|e| e.to_string())?;
        let size = usize::from_str_radix(line.split(';').next().unwrap_or(""), 16)
            .map_err(|_| "invalid chunk size".to_string())?;
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            break;
        }
        if encoded.len() < size + 2 || &encoded[size..size + 2] != b"\r\n" {
            return Err("truncated chunked response".to_string());
        }
        out.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
    }
    Ok(out)
}

fn form_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn nonce() -> Result<SyncNonce, String> {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
    Ok(nonce)
}

fn file_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn check(failed: &mut usize, name: &str, ok: bool, detail: impl AsRef<str>) {
    if !ok {
        *failed += 1;
    }
    let detail = detail.as_ref();
    println!(
        "{}  {}{}",
        if ok { "PASS" } else { "FAIL" },
        name,
        if detail.is_empty() {
            String::new()
        } else {
            format!("   [{detail}]")
        }
    );
}

fn run() -> Result<usize, String> {
    let args: Vec<String> = std::env::args().collect();
    let base = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("http://127.0.0.1:8080");
    let passphrase = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("smoke-master-pass-123");
    let key = derive_key(passphrase);
    let wrong = derive_key("not-the-passphrase");
    let mut client = HttpClient::from_base(base)?;
    client.login(passphrase)?;
    let mut failed = 0usize;

    let doctor = client.request("GET", "/sync/api/doctor", &[], &[])?.json();
    check(
        &mut failed,
        "doctor reports ready, with an available sync key",
        doctor["state"] == "ready" && doctor["sync_key_available"] == true,
        doctor.to_string(),
    );

    let id = file_id()?;
    let whole = b"hello from the smoke run\n";
    let first = build_chunk_request(&key, now_ms(), &id, 0, b"hello from the ", nonce()?)?;
    let first_reply = client.sync_request(&first)?;
    check(
        &mut failed,
        "chunk 0 accepted",
        first_reply.status == 200 && first_reply.json()["status"] == "chunk-received",
        first_reply.json().to_string(),
    );
    let second = build_chunk_request(&key, now_ms(), &id, 1, b"smoke run\n", nonce()?)?;
    let second_reply = client.sync_request(&second)?;
    check(
        &mut failed,
        "chunk 1 accepted",
        second_reply.status == 200 && second_reply.json()["status"] == "chunk-received",
        second_reply.json().to_string(),
    );
    let digest = format!("{:x}", Sha256::digest(whole));
    let finalize =
        build_finalize_request(&key, now_ms(), &id, "smoke-hello.txt", &digest, 2, nonce()?)?;
    let finalize_reply = client.sync_request(&finalize)?;
    check(
        &mut failed,
        "finalize commits the file",
        finalize_reply.status == 200 && finalize_reply.json()["status"] == "upload-complete",
        finalize_reply.json().to_string(),
    );

    let list_headers = build_list_headers(&key, now_ms(), nonce()?)?;
    let list_reply = client.request("GET", "/sync/api/download", &list_headers, &[])?;
    let list_json = list_reply.json();
    let listed = list_json["files"]
        .as_array()
        .is_some_and(|files| files.iter().any(|file| file["name"] == "smoke-hello.txt"));
    check(
        &mut failed,
        "authenticated listing shows the uploaded file",
        list_reply.status == 200 && listed,
        list_json.to_string(),
    );
    let anonymous = client.request("GET", "/sync/api/download", &[], &[])?;
    check(
        &mut failed,
        "listing without an authenticator is 401",
        anonymous.status == 401,
        anonymous.status.to_string(),
    );
    let wrong_headers = build_list_headers(&wrong, now_ms(), nonce()?)?;
    let wrong_list = client.request("GET", "/sync/api/download", &wrong_headers, &[])?;
    check(
        &mut failed,
        "listing with the wrong key is 401",
        wrong_list.status == 401,
        wrong_list.status.to_string(),
    );
    let content = client.request("GET", "/sync/api/download?path=smoke-hello.txt", &[], &[])?;
    check(
        &mut failed,
        "the old plaintext ?path= download is gone (404)",
        content.status == 404,
        content.status.to_string(),
    );

    let replay = build_chunk_request(&key, now_ms(), &file_id()?, 0, b"x", nonce()?)?;
    let accepted = client.sync_request(&replay)?;
    let replayed = client.sync_request(&replay)?;
    check(
        &mut failed,
        "an envelope is accepted once",
        accepted.status == 200,
        accepted.status.to_string(),
    );
    check(
        &mut failed,
        "the same envelope again is a replay (400)",
        replayed.status == 400 && replayed.json()["error"] == "replay-detected",
        replayed.json().to_string(),
    );
    let forged = build_chunk_request(&wrong, now_ms(), &file_id()?, 0, b"x", nonce()?)?;
    let forged_reply = client.sync_request(&forged)?;
    check(
        &mut failed,
        "an envelope sealed with another key fails authentication (400)",
        forged_reply.status == 400 && forged_reply.json()["error"] == "aead-failure",
        forged_reply.json().to_string(),
    );
    let stale = build_chunk_request(&key, now_ms() - 10 * 60_000, &file_id()?, 0, b"x", nonce()?)?;
    let stale_reply = client.sync_request(&stale)?;
    check(
        &mut failed,
        "a 10 minute old timestamp is refused (400)",
        stale_reply.status == 400,
        stale_reply.json().to_string(),
    );

    let evil_id = file_id()?;
    let evil_chunk = build_chunk_request(&key, now_ms(), &evil_id, 0, b"x", nonce()?)?;
    let _ = client.sync_request(&evil_chunk)?;
    let evil_finalize = build_finalize_request(
        &key,
        now_ms(),
        &evil_id,
        "../../etc/cron.d/evil",
        "",
        1,
        nonce()?,
    )?;
    let evil_reply = client.sync_request(&evil_finalize)?;
    check(
        &mut failed,
        "a traversing target path is rejected (400)",
        evil_reply.status == 400 && evil_reply.json()["error"] == "bad-file-path",
        evil_reply.json().to_string(),
    );
    let evil_file = build_chunk_request(&key, now_ms(), "../escape", 0, b"x", nonce()?)?;
    let evil_file_reply = client.sync_request(&evil_file)?;
    check(
        &mut failed,
        "a traversing file id is rejected (400)",
        evil_file_reply.status == 400 && evil_file_reply.json()["error"] == "bad-file-id",
        evil_file_reply.json().to_string(),
    );

    Ok(failed)
}

fn main() {
    match run() {
        Ok(0) => println!("\nALL SYNC CHECKS PASSED"),
        Ok(failed) => {
            println!("\n{failed} SYNC CHECK(S) FAILED");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("FAIL  sync smoke client could not run   [{error}]");
            std::process::exit(1);
        }
    }
}

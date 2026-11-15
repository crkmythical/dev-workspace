use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::{env_file, process};

const ALL_TOOLS: &[&str] = &[
    "caddy",
    "restic",
    "croc",
    "mihomo",
    "code-server",
    "kasmvnc",
    "selkies",
    "mise",
    "idea",
    "burpsuite",
];

#[derive(Default)]
struct Options {
    project_dir: Option<PathBuf>,
    tools_dir: Option<PathBuf>,
    check: bool,
    relock: bool,
    selected: Vec<String>,
}

#[derive(Clone)]
struct Artifact {
    url: String,
    sha256: String,
    sha512: Option<String>,
}

struct Pin {
    name: String,
    version: String,
    artifacts: BTreeMap<&'static str, Artifact>,
}

struct PlannedWrite {
    path: PathBuf,
    contents: Vec<u8>,
}

struct WriteBackup {
    path: PathBuf,
    contents: Option<Vec<u8>>,
}

struct CurlClient {
    auth_file: Option<PathBuf>,
    temp_dir: PathBuf,
    proxy: Option<Zeroizing<String>>,
}

impl Drop for CurlClient {
    fn drop(&mut self) {
        if let Some(path) = &self.auth_file {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

impl CurlClient {
    fn new() -> Result<Self, String> {
        if !process::command_exists("curl") {
            return Err("curl not found".to_string());
        }
        let temp_dir = temp_dir("sdw-tools")?;
        let mut token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
        std::env::remove_var("GITHUB_TOKEN");
        let auth_file = if token.is_empty() {
            None
        } else {
            if token.len() > 512
                || token
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte == b'"' || byte == b'\\')
            {
                token.zeroize();
                return Err("GITHUB_TOKEN has an invalid shape".to_string());
            }
            let path = temp_dir.join("github.conf");
            let header = Zeroizing::new(format!("header = \"Authorization: Bearer {token}\"\n"));
            env_file::write_private(&path, header.as_bytes())?;
            token.zeroize();
            Some(path)
        };
        let proxy = std::env::var("UPDATE_TOOLS_PROXY")
            .ok()
            .filter(|value| !value.is_empty())
            .map(Zeroizing::new);
        std::env::remove_var("UPDATE_TOOLS_PROXY");
        Ok(Self {
            auth_file,
            temp_dir,
            proxy,
        })
    }

    fn command(&self, url: &str) -> Command {
        let mut command = Command::new("curl");
        process::retain_network_environment(&mut command);
        if let Some(proxy) = &self.proxy {
            command.env("HTTPS_PROXY", proxy.as_str());
            command.env("HTTP_PROXY", proxy.as_str());
        }
        command.args(["-fsSL", "--retry", "3", "--retry-delay", "2"]);
        if url.starts_with("https://api.github.com/") {
            if let Some(config) = &self.auth_file {
                command.arg("-K").arg(config);
            }
        }
        command
    }

    fn text(&self, url: &str) -> Result<String, String> {
        validate_https_url(url)?;
        let output = self
            .command(url)
            .args(["--max-time", "120", url])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot run curl: {error}"))?;
        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|error| error.to_string())
        } else {
            Err(format!(
                "GET {url} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    fn download(&self, url: &str, path: &Path, timeout: u64) -> Result<(), String> {
        validate_https_url(url)?;
        let status = self
            .command(url)
            .args(["--max-time", &timeout.to_string(), "-o"])
            .arg(path)
            .arg(url)
            .stdin(Stdio::null())
            .status()
            .map_err(|error| format!("cannot run curl: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            let _ = fs::remove_file(path);
            Err(format!("download failed: {url}"))
        }
    }

    fn headers_for_first_byte(&self, url: &str) -> Result<String, String> {
        validate_https_url(url)?;
        let output = self
            .command(url)
            .args([
                "--max-time",
                "120",
                "--range",
                "0-0",
                "-D",
                "-",
                "-o",
                "/dev/null",
                url,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot run curl: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!(
                "could not inspect {url}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!(
            "usage: sdw-host update-tools --project-dir DIR [--tools-dir DIR] [--check] [--relock] [TOOL ...]"
        );
        println!("tools: {}", ALL_TOOLS.join(" "));
        return 0;
    }
    match parse(args).and_then(execute) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("update-tools: {error}");
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
            "--tools-dir" => {
                index += 1;
                options.tools_dir = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--tools-dir needs a path".to_string())?,
                ));
            }
            "--check" => options.check = true,
            "--relock" => options.relock = true,
            option if option.starts_with('-') => return Err(format!("unknown option: {option}")),
            tool => options.selected.push(tool.to_string()),
        }
        index += 1;
    }
    if options.selected.is_empty() {
        options.selected = ALL_TOOLS.iter().map(|tool| (*tool).to_string()).collect();
    }
    for tool in &options.selected {
        if !ALL_TOOLS.contains(&tool.as_str()) {
            return Err(format!("unknown tool '{tool}'"));
        }
    }
    Ok(options)
}

fn execute(options: Options) -> Result<i32, String> {
    let project = options
        .project_dir
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let tools_dir = options
        .tools_dir
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                project.join(path)
            }
        })
        .unwrap_or_else(|| project.join("src/image/tools"));
    if !tools_dir.is_dir() {
        return Err(format!(
            "tools directory does not exist: {}",
            tools_dir.display()
        ));
    }
    let client = CurlClient::new()?;
    let mut behind = false;
    let mut failures = Vec::new();
    let mut writes = Vec::new();

    for tool in &options.selected {
        match plan_tool(&client, &tools_dir, tool, options.check, options.relock) {
            Ok(Plan::Current) => {}
            Ok(Plan::Behind) => behind = true,
            Ok(Plan::Writes(mut planned)) => writes.append(&mut planned),
            Err(error) => failures.push(format!("{tool}: {error}")),
        }
    }
    if !failures.is_empty() {
        for failure in failures {
            eprintln!("{failure}");
        }
        return Ok(1);
    }
    if options.check {
        if behind {
            eprintln!("some pinned tools are behind; run deploy/scripts/update-tools.sh");
            return Ok(1);
        }
        return Ok(0);
    }
    commit_writes(writes)?;
    println!("done. Review src/image/tools, then build and run make smoke.");
    Ok(0)
}

enum Plan {
    Current,
    Behind,
    Writes(Vec<PlannedWrite>),
}

fn plan_tool(
    client: &CurlClient,
    tools_dir: &Path,
    tool: &str,
    check: bool,
    relock: bool,
) -> Result<Plan, String> {
    match tool {
        "idea" => plan_idea(client, tools_dir, check),
        "burpsuite" => plan_burpsuite(client, tools_dir, check),
        _ => plan_github(client, tools_dir, tool, check, relock),
    }
}

fn plan_github(
    client: &CurlClient,
    tools_dir: &Path,
    tool: &str,
    check: bool,
    relock: bool,
) -> Result<Plan, String> {
    let repo = repository(tool)?;
    let release_text = client.text(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    let release: Value = serde_json::from_str(&release_text)
        .map_err(|error| format!("invalid GitHub release JSON: {error}"))?;
    let latest_tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| "release has no tag_name".to_string())?;
    let latest = latest_tag.strip_prefix('v').unwrap_or(latest_tag);
    validate_version(latest)?;
    let current = current_version(tools_dir, tool)?;
    if current == latest {
        println!("{tool:<12} {current:<12} up to date");
        if tool == "selkies"
            && !check
            && (relock || !tools_dir.join("selkies-requirements.txt").is_file())
        {
            let pin = read_pin(tools_dir, tool)?;
            let lock = lock_selkies(client, &pin)?;
            return Ok(Plan::Writes(vec![PlannedWrite {
                path: tools_dir.join("selkies-requirements.txt"),
                contents: lock.into_bytes(),
            }]));
        }
        return Ok(Plan::Current);
    }
    println!("{tool:<12} {current:<12} -> {latest}");
    if check {
        return Ok(Plan::Behind);
    }

    let description = description(tool, latest)?;
    let sums = match &description.checksums {
        Some(checksums) => Some(client.text(&asset_url(&release, &checksums.asset)?)?),
        None => None,
    };
    let mut artifacts = BTreeMap::new();
    for arch in ["amd64", "arm64"] {
        let asset_name = description
            .assets
            .get(arch)
            .ok_or_else(|| format!("no asset template for {arch}"))?;
        let url = asset_url(&release, asset_name)?;
        let sha256 = asset_digest(&release, asset_name)?;
        let mut sha512 = None;
        if let Some(checksums) = &description.checksums {
            let published = published_hash(sums.as_deref().unwrap_or_default(), asset_name)
                .ok_or_else(|| format!("{} contains no hash for {asset_name}", checksums.asset))?;
            match checksums.algorithm {
                "sha256" if published == sha256 => {}
                "sha256" => {
                    return Err(format!(
                        "GitHub says {sha256}, published checksums say {published} for {asset_name}"
                    ))
                }
                "sha512" => sha512 = Some(validate_hash(&published, 128)?),
                algorithm => return Err(format!("unsupported checksum algorithm {algorithm}")),
            }
        }
        artifacts.insert(
            arch,
            Artifact {
                url,
                sha256: validate_hash(&sha256, 64)?,
                sha512,
            },
        );
    }
    let pin = Pin {
        name: tool.to_string(),
        version: latest.to_string(),
        artifacts,
    };
    let mut writes = vec![PlannedWrite {
        path: tools_dir.join(format!("{tool}.env")),
        contents: render_pin(&pin)?.into_bytes(),
    }];
    if tool == "selkies" {
        writes.push(PlannedWrite {
            path: tools_dir.join("selkies-requirements.txt"),
            contents: lock_selkies(client, &pin)?.into_bytes(),
        });
    }
    Ok(Plan::Writes(writes))
}

struct Description {
    assets: BTreeMap<&'static str, String>,
    checksums: Option<ChecksumFile>,
}

struct ChecksumFile {
    asset: String,
    algorithm: &'static str,
}

fn description(tool: &str, version: &str) -> Result<Description, String> {
    let pair = |amd64: String, arm64: String, checksums| Description {
        assets: BTreeMap::from([("amd64", amd64), ("arm64", arm64)]),
        checksums,
    };
    Ok(match tool {
        "caddy" => pair(
            format!("caddy_{version}_linux_amd64.tar.gz"),
            format!("caddy_{version}_linux_arm64.tar.gz"),
            Some(ChecksumFile {
                asset: format!("caddy_{version}_checksums.txt"),
                algorithm: "sha512",
            }),
        ),
        "restic" => pair(
            format!("restic_{version}_linux_amd64.bz2"),
            format!("restic_{version}_linux_arm64.bz2"),
            Some(ChecksumFile {
                asset: "SHA256SUMS".to_string(),
                algorithm: "sha256",
            }),
        ),
        "croc" => pair(
            format!("croc_v{version}_Linux-64bit.tar.gz"),
            format!("croc_v{version}_Linux-ARM64.tar.gz"),
            Some(ChecksumFile {
                asset: format!("croc_v{version}_checksums.txt"),
                algorithm: "sha256",
            }),
        ),
        "mihomo" => pair(
            format!("mihomo-linux-amd64-v{version}.gz"),
            format!("mihomo-linux-arm64-v{version}.gz"),
            None,
        ),
        "code-server" => pair(
            format!("code-server_{version}_amd64.deb"),
            format!("code-server_{version}_arm64.deb"),
            None,
        ),
        "kasmvnc" => pair(
            format!("kasmvncserver_debian_trixie_{version}_amd64.deb"),
            format!("kasmvncserver_debian_trixie_{version}_arm64.deb"),
            None,
        ),
        "selkies" => {
            let asset = format!("selkies-{version}-py3-none-any.whl");
            pair(
                asset.clone(),
                asset,
                Some(ChecksumFile {
                    asset: "SHA256SUMS".to_string(),
                    algorithm: "sha256",
                }),
            )
        }
        "mise" => pair(
            format!("mise-v{version}-linux-x64.tar.gz"),
            format!("mise-v{version}-linux-arm64.tar.gz"),
            Some(ChecksumFile {
                asset: "SHASUMS256.txt".to_string(),
                algorithm: "sha256",
            }),
        ),
        _ => return Err(format!("no GitHub description for {tool}")),
    })
}

fn repository(tool: &str) -> Result<&'static str, String> {
    match tool {
        "caddy" => Ok("caddyserver/caddy"),
        "restic" => Ok("restic/restic"),
        "croc" => Ok("schollz/croc"),
        "mihomo" => Ok("MetaCubeX/mihomo"),
        "code-server" => Ok("coder/code-server"),
        "kasmvnc" => Ok("kasmtech/KasmVNC"),
        "selkies" => Ok("selkies-project/selkies"),
        "mise" => Ok("jdx/mise"),
        _ => Err(format!("{tool} is not a GitHub release tool")),
    }
}

fn plan_idea(client: &CurlClient, tools_dir: &Path, check: bool) -> Result<Plan, String> {
    let text = client.text(
        "https://data.services.jetbrains.com/products/releases?code=IIC&latest=true&type=release",
    )?;
    let feed: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let release = feed["IIC"]
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| "JetBrains feed has no IIC release".to_string())?;
    let latest = release["version"]
        .as_str()
        .ok_or_else(|| "JetBrains feed has no version".to_string())?;
    validate_version(latest)?;
    let current = current_version(tools_dir, "idea")?;
    if current == latest {
        println!("{:<12} {current:<12} up to date", "idea");
        return Ok(Plan::Current);
    }
    println!("{:<12} {current:<12} -> {latest}", "idea");
    if check {
        return Ok(Plan::Behind);
    }
    let downloads = &release["downloads"];
    let mut artifacts = BTreeMap::new();
    for (arch, key) in [("amd64", "linux"), ("arm64", "linuxARM64")] {
        let item = &downloads[key];
        let url = item["link"]
            .as_str()
            .ok_or_else(|| format!("JetBrains feed has no {key}.link"))?;
        let checksum_url = item["checksumLink"]
            .as_str()
            .ok_or_else(|| format!("JetBrains feed has no {key}.checksumLink"))?;
        validate_https_url(url)?;
        let checksum = client
            .text(checksum_url)?
            .split_whitespace()
            .next()
            .ok_or_else(|| format!("empty checksum for {key}"))?
            .to_string();
        artifacts.insert(
            arch,
            Artifact {
                url: url.to_string(),
                sha256: validate_hash(&checksum, 64)?,
                sha512: None,
            },
        );
    }
    let pin = Pin {
        name: "idea".to_string(),
        version: latest.to_string(),
        artifacts,
    };
    Ok(Plan::Writes(vec![PlannedWrite {
        path: tools_dir.join("idea.env"),
        contents: render_pin(&pin)?.into_bytes(),
    }]))
}

fn plan_burpsuite(client: &CurlClient, tools_dir: &Path, check: bool) -> Result<Plan, String> {
    let latest_url =
        "https://portswigger-cdn.net/burp/releases/download?product=community&type=Jar";
    let headers = client.headers_for_first_byte(latest_url)?;
    let latest = extract_burp_version(&headers)
        .ok_or_else(|| "could not read latest Burp Community version".to_string())?;
    validate_version(&latest)?;
    let current = current_version(tools_dir, "burpsuite")?;
    if current == latest {
        println!("{:<12} {current:<12} up to date", "burpsuite");
        return Ok(Plan::Current);
    }
    println!("{:<12} {current:<12} -> {latest}", "burpsuite");
    if check {
        return Ok(Plan::Behind);
    }
    let url = format!(
        "https://portswigger.net/burp/releases/download?product=community&version={latest}&type=Jar"
    );
    let jar = client.temp_dir.join("burpsuite.jar");
    eprintln!("burpsuite: hashing the download (about 700 MB)...");
    client.download(&url, &jar, 3600)?;
    let checksum = sha256_file(&jar)?;
    let artifact = Artifact {
        url,
        sha256: checksum,
        sha512: None,
    };
    let pin = Pin {
        name: "burpsuite".to_string(),
        version: latest,
        artifacts: BTreeMap::from([("amd64", artifact.clone()), ("arm64", artifact)]),
    };
    Ok(Plan::Writes(vec![PlannedWrite {
        path: tools_dir.join("burpsuite.env"),
        contents: render_pin(&pin)?.into_bytes(),
    }]))
}

fn extract_burp_version(headers: &str) -> Option<String> {
    let marker = "burpsuite_community_v";
    let start = headers.find(marker)? + marker.len();
    let tail = &headers[start..];
    let version: String = tail
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect::<String>()
        .trim_end_matches('.')
        .to_string();
    (!version.is_empty()).then_some(version)
}

fn read_pin(tools_dir: &Path, tool: &str) -> Result<Pin, String> {
    let values = env_file::read(&tools_dir.join(format!("{tool}.env")))?;
    let version = values
        .get("VERSION")
        .cloned()
        .ok_or_else(|| format!("{tool}.env has no VERSION"))?;
    let mut artifacts = BTreeMap::new();
    for arch in ["amd64", "arm64"] {
        artifacts.insert(
            arch,
            Artifact {
                url: values
                    .get(&format!("URL_{arch}"))
                    .cloned()
                    .ok_or_else(|| format!("{tool}.env has no URL_{arch}"))?,
                sha256: values
                    .get(&format!("SHA256_{arch}"))
                    .cloned()
                    .ok_or_else(|| format!("{tool}.env has no SHA256_{arch}"))?,
                sha512: values.get(&format!("SHA512_{arch}")).cloned(),
            },
        );
    }
    Ok(Pin {
        name: tool.to_string(),
        version,
        artifacts,
    })
}

fn current_version(tools_dir: &Path, tool: &str) -> Result<String, String> {
    env_file::read(&tools_dir.join(format!("{tool}.env")))?
        .remove("VERSION")
        .filter(|version| !version.is_empty())
        .ok_or_else(|| format!("{tool}.env has no VERSION"))
}

fn render_pin(pin: &Pin) -> Result<String, String> {
    validate_shell_atom(&pin.name)?;
    validate_version(&pin.version)?;
    let mut output = String::from(
        "# Generated by sdw-host update-tools; update by running deploy/scripts/update-tools.sh.\n",
    );
    output.push_str(&format!("NAME={}\nVERSION={}\n", pin.name, pin.version));
    for arch in ["amd64", "arm64"] {
        let artifact = pin
            .artifacts
            .get(arch)
            .ok_or_else(|| format!("missing {arch} artifact"))?;
        validate_https_url(&artifact.url)?;
        output.push_str(&format!("URL_{arch}='{}'\n", artifact.url));
        output.push_str(&format!(
            "SHA256_{arch}={}\n",
            validate_hash(&artifact.sha256, 64)?
        ));
        if let Some(hash) = &artifact.sha512 {
            output.push_str(&format!("SHA512_{arch}={}\n", validate_hash(hash, 128)?));
        }
    }
    Ok(output)
}

fn commit_writes(writes: Vec<PlannedWrite>) -> Result<(), String> {
    commit_writes_with(writes, atomic_public_write)
}

fn commit_writes_with<F>(writes: Vec<PlannedWrite>, mut write_file: F) -> Result<(), String>
where
    F: FnMut(&Path, &[u8]) -> Result<(), String>,
{
    if writes.is_empty() {
        return Ok(());
    }

    let mut backups = Vec::with_capacity(writes.len());
    for write in &writes {
        let contents = match fs::read(&write.path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "cannot back up {} before updating: {error}",
                    write.path.display()
                ));
            }
        };
        backups.push(WriteBackup {
            path: write.path.clone(),
            contents,
        });
    }

    let recovery_dir = preserve_write_backups(&backups)?;
    for (index, write) in writes.iter().enumerate() {
        if let Err(error) = write_file(&write.path, &write.contents) {
            let mut rollback_errors = Vec::new();
            for backup in backups[..=index].iter().rev() {
                let result = match &backup.contents {
                    Some(contents) => write_file(&backup.path, contents).map_err(|error| {
                        format!("cannot restore {}: {error}", backup.path.display())
                    }),
                    None => match fs::remove_file(&backup.path) {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(format!(
                            "cannot remove newly created {}: {error}",
                            backup.path.display()
                        )),
                    },
                };
                if let Err(error) = result {
                    rollback_errors.push(error);
                }
            }
            if rollback_errors.is_empty() {
                discard_write_backups(&recovery_dir);
                return Err(error);
            }
            return Err(format!(
                "{error}; rollback failed: {}; recovery material retained at {}",
                rollback_errors.join("; "),
                recovery_dir.display()
            ));
        }
    }
    discard_write_backups(&recovery_dir);
    Ok(())
}

fn preserve_write_backups(backups: &[WriteBackup]) -> Result<PathBuf, String> {
    let directory = temp_dir("sdw-tools-recovery")?;
    let result = (|| {
        let mut instructions = String::from(
            "Original tool pin files saved before update-tools changed any target.\n\
             Copy each numbered .original file back to the listed path.\n\
             Remove paths marked MISSING because they did not exist before the update.\n",
        );
        for (index, backup) in backups.iter().enumerate() {
            if let Some(contents) = &backup.contents {
                let name = format!("{index}.original");
                env_file::write_private(&directory.join(&name), contents)?;
                instructions.push_str(&format!("{name}\t{:?}\n", backup.path));
            } else {
                instructions.push_str(&format!("MISSING\t{:?}\n", backup.path));
            }
        }
        env_file::write_private(&directory.join("RECOVERY.txt"), instructions.as_bytes())
    })();
    if let Err(error) = result {
        return match fs::remove_dir_all(&directory) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(format!(
                "{error}; cannot remove incomplete recovery directory {}: {cleanup_error}",
                directory.display()
            )),
        };
    }
    Ok(directory)
}

fn discard_write_backups(directory: &Path) {
    if let Err(error) = fs::remove_dir_all(directory) {
        eprintln!(
            "warning: cannot remove temporary recovery directory {}: {error}",
            directory.display()
        );
    }
}

fn atomic_public_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let _ = fs::remove_file(&temp);
    let result = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o644)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(contents)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temp, path));
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot write {}: {error}", path.display()));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
        .map_err(|error| format!("cannot set {} permissions: {error}", path.display()))?;
    Ok(())
}

fn lock_selkies(client: &CurlClient, pin: &Pin) -> Result<String, String> {
    for command in ["unzip", "uv"] {
        if !process::command_exists(command) {
            return Err(format!(
                "{command} is required to lock Selkies dependencies"
            ));
        }
    }
    let artifact = pin
        .artifacts
        .get("amd64")
        .ok_or_else(|| "Selkies pin has no artifact".to_string())?;
    let directory = temp_dir("sdw-selkies")?;
    let wheel = directory.join(format!("selkies-{}-py3-none-any.whl", pin.version));
    client.download(&artifact.url, &wheel, 600)?;
    if sha256_file(&wheel)? != artifact.sha256 {
        let _ = fs::remove_dir_all(&directory);
        return Err("Selkies wheel does not match its pinned sha256".to_string());
    }
    let metadata = process::output(
        "unzip",
        [
            "-p",
            wheel.to_string_lossy().as_ref(),
            "*.dist-info/METADATA",
        ],
    )?;
    if !metadata.success() {
        let _ = fs::remove_dir_all(&directory);
        return Err("could not read Selkies wheel metadata".to_string());
    }
    let requirements: Vec<&str> = metadata
        .stdout
        .lines()
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.strip_prefix("Requires-Dist: "))
        .filter(|line| !line.contains("extra =="))
        .collect();
    if requirements.is_empty() {
        let _ = fs::remove_dir_all(&directory);
        return Err("Selkies wheel lists no runtime requirements".to_string());
    }
    let input = directory.join("selkies.in");
    fs::write(&input, requirements.join("\n") + "\n").map_err(|error| error.to_string())?;
    let output = directory.join("selkies-requirements.txt");
    let mut command = Command::new("uv");
    process::retain_network_environment(&mut command);
    if let Some(proxy) = &client.proxy {
        command.env("HTTPS_PROXY", proxy.as_str());
        command.env("HTTP_PROXY", proxy.as_str());
    }
    let status = command
        .args([
            "pip",
            "compile",
            "--universal",
            "--python-version",
            "3.13",
            "--generate-hashes",
            "--no-header",
            "--no-annotate",
            "--quiet",
        ])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .status()
        .map_err(|error| format!("cannot run uv: {error}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&directory);
        return Err("uv could not resolve Selkies requirements".to_string());
    }
    let body = fs::read_to_string(&output).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&directory);
    let count = body.lines().filter(|line| {
        line.chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
    });
    println!(
        "{:<12} {:<12} locked {} requirements",
        "selkies",
        pin.version,
        count.count()
    );
    Ok(format!(
        "# Generated by sdw-host update-tools from selkies {}; never edit by hand.\n{}",
        pin.version, body
    ))
}

fn repository_asset<'a>(release: &'a Value, name: &str) -> Option<&'a Value> {
    release["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name))
}

fn asset_url(release: &Value, name: &str) -> Result<String, String> {
    let url = repository_asset(release, name)
        .and_then(|asset| asset["browser_download_url"].as_str())
        .ok_or_else(|| format!("release has no asset named {name}"))?;
    validate_https_url(url)?;
    Ok(url.to_string())
}

fn asset_digest(release: &Value, name: &str) -> Result<String, String> {
    repository_asset(release, name)
        .and_then(|asset| asset["digest"].as_str())
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .map(str::to_string)
        .ok_or_else(|| format!("release metadata has no sha256 digest for {name}"))
}

fn published_hash(contents: &str, asset: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let file = fields
            .next()?
            .trim_start_matches('*')
            .trim_start_matches("./");
        (file == asset).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_hash(value: &str, length: usize) -> Result<String, String> {
    let value = value.to_ascii_lowercase();
    if value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(format!("invalid {}-character hexadecimal hash", length))
    }
}

fn validate_https_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://")
        && !url
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\'')
    {
        Ok(())
    } else {
        Err(format!("unsafe or non-HTTPS URL: {url}"))
    }
}

fn validate_version(version: &str) -> Result<(), String> {
    if !version.is_empty()
        && version.len() <= 80
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
    {
        Ok(())
    } else {
        Err(format!("unsafe version value: {version}"))
    }
}

fn validate_shell_atom(value: &str) -> Result<(), String> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(format!("unsafe pin value: {value}"))
    }
}

fn temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    for attempt in 0..100u32 {
        let path = base.join(format!("{prefix}-{}-{attempt}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("cannot create temporary directory".to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{commit_writes, commit_writes_with, temp_dir, PlannedWrite};

    #[test]
    fn commit_writes_restores_an_earlier_file_after_a_later_failure() {
        let directory = temp_dir("sdw-commit-writes-test").expect("create test directory");
        let first = directory.join("first.env");
        let blocked = directory.join("missing/blocked.env");
        fs::write(&first, b"before").expect("write original file");

        let result = commit_writes(vec![
            PlannedWrite {
                path: first.clone(),
                contents: b"after".to_vec(),
            },
            PlannedWrite {
                path: blocked.clone(),
                contents: b"cannot be written".to_vec(),
            },
        ]);

        assert!(result.is_err());
        assert_eq!(
            fs::read(&first).expect("read restored file"),
            b"before".to_vec()
        );
        assert!(!blocked.exists());
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn commit_writes_aborts_before_changes_when_a_backup_cannot_be_read() {
        let directory = temp_dir("sdw-commit-backup-test").expect("create test directory");
        let first = directory.join("first.env");
        let unreadable = directory.join("unreadable.env");
        fs::write(&first, b"before").expect("write original file");
        fs::create_dir(&unreadable).expect("create unreadable target");

        let error = commit_writes(vec![
            PlannedWrite {
                path: first.clone(),
                contents: b"after".to_vec(),
            },
            PlannedWrite {
                path: unreadable,
                contents: b"cannot be written".to_vec(),
            },
        ])
        .expect_err("backup read must fail");

        assert!(error.contains("cannot back up"));
        assert_eq!(
            fs::read(&first).expect("read unchanged file"),
            b"before".to_vec()
        );
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn commit_writes_reports_rollback_failure_and_retains_backups() {
        let directory = temp_dir("sdw-commit-rollback-test").expect("create test directory");
        let first = directory.join("first.env");
        let second = directory.join("second.env");
        fs::write(&first, b"first before").expect("write first original");
        fs::write(&second, b"second before").expect("write second original");

        let result = commit_writes_with(
            vec![
                PlannedWrite {
                    path: first.clone(),
                    contents: b"first after".to_vec(),
                },
                PlannedWrite {
                    path: second.clone(),
                    contents: b"second after".to_vec(),
                },
            ],
            |path, contents| {
                if path == second && contents == b"second after" {
                    Err("injected write failure".to_string())
                } else if path == first && contents == b"first before" {
                    Err("injected restore failure".to_string())
                } else {
                    fs::write(path, contents).map_err(|error| error.to_string())
                }
            },
        );

        let error = result.expect_err("commit and rollback must fail");
        assert!(error.contains("injected write failure"));
        assert!(error.contains("injected restore failure"));
        let recovery_dir = PathBuf::from(
            error
                .rsplit_once("recovery material retained at ")
                .expect("error reports recovery directory")
                .1,
        );
        assert_eq!(
            fs::read(recovery_dir.join("0.original")).expect("read retained first backup"),
            b"first before".to_vec()
        );
        assert!(fs::read_to_string(recovery_dir.join("RECOVERY.txt"))
            .expect("read recovery instructions")
            .contains(&format!("{:?}", first)));
        assert_eq!(
            fs::read(&first).expect("read unrestored first file"),
            b"first after".to_vec()
        );
        assert_eq!(
            fs::read(&second).expect("read restored second file"),
            b"second before".to_vec()
        );
        fs::remove_dir_all(recovery_dir).expect("remove recovery directory");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}

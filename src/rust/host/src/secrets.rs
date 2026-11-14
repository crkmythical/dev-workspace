use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::env_file;
use zeroize::Zeroizing;

#[derive(Default)]
struct Options {
    project_dir: Option<PathBuf>,
    env_file: Option<PathBuf>,
    force: bool,
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: sdw-host secrets-init --project-dir DIR [--env-file FILE] [--force]");
        return 0;
    }
    match parse(args).and_then(execute) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("secrets-init: {error}");
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
            "--env-file" => {
                index += 1;
                options.env_file = Some(PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--env-file needs a path".to_string())?,
                ));
            }
            "--force" => options.force = true,
            option => return Err(format!("unknown option: {option}")),
        }
        index += 1;
    }
    Ok(options)
}

fn execute(options: Options) -> Result<(), String> {
    let project = options
        .project_dir
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let env_path = options
        .env_file
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                project.join(path)
            }
        })
        .unwrap_or_else(|| project.join(".env"));
    env_file::ensure_private(&env_path)?;
    let mut values = env_file::read(&env_path)?;
    let url = Zeroizing::new(
        values
            .remove("CLASH_SUBSCRIPTION_URL")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "{} has no CLASH_SUBSCRIPTION_URL; no secret file was changed",
                    env_path.display()
                )
            })?,
    );
    let generated = values.get("PASSWORD").is_none_or(String::is_empty);
    let password = Zeroizing::new(
        match values.remove("PASSWORD").filter(|value| !value.is_empty()) {
            Some(password) => password,
            None => random_hex(16)?,
        },
    );

    let directory = project.join("secrets");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    println!("Writing secret files from {}:", env_path.display());
    if generated {
        println!("  PASSWORD was empty: generated a persistent random value");
    }
    write_one(&directory, "password", password.as_bytes(), options.force)?;
    write_one(
        &directory,
        "clash_subscription_url",
        url.as_bytes(),
        options.force,
    )?;
    println!();
    println!("Next:");
    println!("  1. remove PASSWORD and CLASH_SUBSCRIPTION_URL values from .env");
    println!("  2. add docker-compose.secrets.yml to COMPOSE_FILE");
    println!("  3. docker compose up -d");
    Ok(())
}

fn write_one(directory: &Path, name: &str, value: &[u8], force: bool) -> Result<(), String> {
    let path = directory.join(name);
    if path.exists() && !force {
        env_file::ensure_private(&path)?;
        println!("  keep   {} (use --force to replace)", path.display());
        return Ok(());
    }
    env_file::write_private(&path, value)?;
    println!("  wrote  {}", path.display());
    Ok(())
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let mut random = vec![0u8; bytes];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|error| format!("cannot read OS randomness: {error}"))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

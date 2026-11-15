use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::process;

#[derive(Default)]
struct Options {
    project_dir: Option<PathBuf>,
    yes: bool,
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("usage: sdw-host purge-history --project-dir DIR [--yes]");
        return 0;
    }
    match parse(args).and_then(execute) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("purge-history: {error}");
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
            "--yes" => options.yes = true,
            option => return Err(format!("unknown option: {option}")),
        }
        index += 1;
    }
    Ok(options)
}

fn execute(options: Options) -> Result<i32, String> {
    require("git")?;
    let requested = options
        .project_dir
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let root_text = git_capture(&requested, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root_text.trim())
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if root != requested {
        return Err(format!(
            "{} resolves inside repository {}; pass the repository root explicitly",
            requested.display(),
            root.display()
        ));
    }
    if git_ok(&root, &["ls-files", "--error-unmatch", ".env"]) {
        return Err(
            ".env is still tracked; run `git rm --cached .env` and keep it ignored first"
                .to_string(),
        );
    }
    let status = git_capture(&root, &["status", "--porcelain", "--untracked-files=no"])?;
    if !status.trim().is_empty() {
        return Err(
            "there are uncommitted tracked changes; commit or stash them first".to_string(),
        );
    }
    let count = history_count(&root)?;
    println!("{count} commit(s) touch .env.");
    if count == 0 {
        println!("nothing to purge.");
        return Ok(0);
    }

    let backup = backup_path(&root)?;
    if !options.yes {
        println!("dry run. With --yes this would:");
        println!(
            "  1. write and verify a full backup bundle at {}",
            backup.display()
        );
        println!("  2. use git-filter-repo to remove .env from every branch and tag");
        println!("  3. verify the path is absent, then expire reflogs and prune old objects");
        return Ok(0);
    }
    if !process::command_exists("git-filter-repo") {
        return Err(
            "git-filter-repo is required; install it before --yes (no deprecated filter-branch fallback)"
                .to_string(),
        );
    }

    git_run(
        &root,
        &[
            "bundle",
            "create",
            backup.to_string_lossy().as_ref(),
            "--all",
        ],
    )?;
    git_run(
        &root,
        &["bundle", "verify", backup.to_string_lossy().as_ref()],
    )?;
    println!("backup written and verified: {}", backup.display());

    git_run(
        &root,
        &["filter-repo", "--path", ".env", "--invert-paths", "--force"],
    )?;
    let remaining = history_count(&root)?;
    if remaining != 0 {
        return Err(format!(
            "history rewrite returned success but {remaining} ref(s) still expose .env; backup is {}",
            backup.display()
        ));
    }
    git_run(&root, &["reflog", "expire", "--expire=now", "--all"])?;
    git_run(&root, &["gc", "--prune=now", "--aggressive", "-q"])?;

    println!();
    println!(
        "History rewritten and verified. The backup remains at {}.",
        backup.display()
    );
    println!(
        "Copies of old history still contain the removed values; rotate affected credentials."
    );
    println!(
        "git-filter-repo may remove origin as a safety measure; review remotes before pushing."
    );
    Ok(0)
}

fn history_count(root: &Path) -> Result<usize, String> {
    Ok(
        git_capture(root, &["log", "--all", "--format=%H", "--", ".env"])?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
    )
}

fn backup_path(root: &Path) -> Result<PathBuf, String> {
    let parent = root
        .parent()
        .ok_or_else(|| "repository root has no parent for the backup".to_string())?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "repository directory name is not UTF-8".to_string())?;
    let stamp = process::output("date", ["+%Y%m%d-%H%M%S"])
        .ok()
        .filter(|output| output.success())
        .map(|output| output.stdout.trim().to_string())
        .filter(|stamp| !stamp.is_empty())
        .unwrap_or_else(|| std::process::id().to_string());
    let preferred = parent.join(format!("{name}-before-purge-{stamp}.bundle"));
    if !preferred.exists() {
        return Ok(preferred);
    }
    Ok(parent.join(format!(
        "{name}-before-purge-{stamp}-{}.bundle",
        std::process::id()
    )))
}

fn git_capture(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn git_ok(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn git_run(root: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {} exited {status}", args.join(" ")))
    }
}

fn require(command: &str) -> Result<(), String> {
    if process::command_exists(command) {
        Ok(())
    } else {
        Err(format!("{command} not found"))
    }
}

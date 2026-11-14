use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub fn read(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut values = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, raw_value)) = line.split_once('=') else {
            continue;
        };
        if !valid_name(name) {
            continue;
        }
        let value = unquote(raw_value);
        values.insert(name.to_string(), value.to_string());
    }
    Ok(values)
}

pub fn value(values: &HashMap<String, String>, name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| values.get(name).filter(|value| !value.is_empty()).cloned())
        .unwrap_or_else(|| default.to_string())
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_'))
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

pub fn ensure_private(path: &Path) -> Result<(), String> {
    let mode = fs::metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "{} has mode {mode:03o}; secrets files must be 600 or stricter",
            path.display()
        ));
    }
    Ok(())
}

pub fn write_private(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let _ = fs::remove_file(&temp);
    let result = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
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
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

use std::path::Path;

use crate::{env_file, process};

/// Verify the optional context lock and return the context being used. Every host operation that
/// reads or mutates Docker calls this one function before touching resources.
pub fn enforce(project: &Path) -> Result<String, String> {
    let config = env_file::read(&project.join(".env")).unwrap_or_default();
    let expected = env_file::value(&config, "SDW_DOCKER_CONTEXT", "");
    let output = process::output("docker", ["context", "show"])?;
    if !output.success() {
        return Err("cannot determine current Docker context".to_string());
    }
    let current = output.stdout.trim();
    if !expected.is_empty() && current != expected {
        return Err(format!(
            "Docker context is '{current}', but SDW_DOCKER_CONTEXT requires '{expected}'"
        ));
    }
    Ok(if current.is_empty() {
        "unknown".to_string()
    } else {
        current.to_string()
    })
}

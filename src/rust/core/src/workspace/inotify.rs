//! The inotify watch limit. code-server (VS Code) and every other file watcher in the workspace
//! share one per-user limit, `fs.inotify.max_user_watches`. It is a kernel setting of the Docker
//! HOST: a container can write it only if `/proc/sys` is writable to it, which it normally is not.
//! The boot tries once and then says what is true, instead of claiming success.

pub const WATCHES_PATH: &str = "/proc/sys/fs/inotify/max_user_watches";
/// What a large project under VS Code needs (the value VS Code's own documentation recommends).
pub const WATCHES_WANTED: u64 = 524_288;

/// What to tell the user about the limit currently in force: None when it is enough, or when it
/// could not be read (nothing to say).
pub fn inotify_advice(current: Option<u64>) -> Option<String> {
    let current = current?;
    if current >= WATCHES_WANTED {
        return None;
    }
    Some(format!(
        "fs.inotify.max_user_watches is {current}; code-server and other file watchers want at \
         least {WATCHES_WANTED}, and a large project fails with \"ENOSPC: System limit for \
         number of file watchers reached\". A container cannot raise it: on the Docker host (on \
         Docker Desktop or Colima, inside its VM) run `sudo sysctl -w \
         fs.inotify.max_user_watches={WATCHES_WANTED}`, and put the same line in \
         /etc/sysctl.d/ to keep it."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enough_or_unknown_is_silent() {
        assert_eq!(inotify_advice(Some(WATCHES_WANTED)), None);
        assert_eq!(inotify_advice(Some(1_048_576)), None);
        assert_eq!(inotify_advice(None), None);
    }

    #[test]
    fn too_low_says_what_it_is_and_where_to_raise_it() {
        let advice = inotify_advice(Some(8192)).unwrap();
        assert!(advice.contains("is 8192"), "{advice}");
        assert!(advice.contains("524288"), "{advice}");
        assert!(advice.contains("Docker host"), "{advice}");
        assert!(
            advice.contains("sysctl -w fs.inotify.max_user_watches=524288"),
            "{advice}"
        );
    }
}

//! Shredder adapter — a port of packages/infra/src/shred.ts. shred(1) overwrites the file; if that
//! fails for any reason the file is still unlinked, so a key file is never left in place just
//! because the overwrite step errored. A missing file counts as already gone.

use crate::cmd;
use sdw_core::vault::ports::Shredder;
use std::path::Path;

pub struct FileShredder;

impl Shredder for FileShredder {
    fn shred(&self, path: &str, passes: u32) -> bool {
        if !Path::new(path).exists() {
            return true;
        }
        cmd::ok("shred", &[&format!("-n{passes}"), "-z", path]);
        cmd::ok("rm", &["-f", path]);
        !Path::new(path).exists()
    }
}

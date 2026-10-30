//! Sync types shared by the reconciler (SPA-side). Port of the relevant parts of
//! packages/core/src/sync/types.ts.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub path: String,
    pub mtime: i64,
    pub size: i64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    Upload {
        path: String,
    },
    Download {
        path: String,
    },
    DeleteRemote {
        path: String,
    },
    DeleteLocal {
        path: String,
    },
    Conflict {
        path: String,
        local_hash: String,
        remote_hash: String,
    },
}

impl SyncAction {
    pub fn path(&self) -> &str {
        match self {
            SyncAction::Upload { path }
            | SyncAction::Download { path }
            | SyncAction::DeleteRemote { path }
            | SyncAction::DeleteLocal { path }
            | SyncAction::Conflict { path, .. } => path,
        }
    }
}

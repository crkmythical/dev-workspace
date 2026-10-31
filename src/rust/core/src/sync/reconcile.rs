//! Pure three-way reconciliation — a port of packages/core/src/sync/reconcile.ts. Deterministic,
//! idempotent, no I/O. Tests mirror packages/core/tests/reconcile.test.ts.

use std::collections::{HashMap, HashSet};

use super::types::{FileState, SyncAction};

pub fn reconcile(
    local: &[FileState],
    remote: &[FileState],
    baseline: &[FileState],
) -> Vec<SyncAction> {
    let by_path = |files: &[FileState]| -> HashMap<String, FileState> {
        files.iter().map(|f| (f.path.clone(), f.clone())).collect()
    };
    let local_map = by_path(local);
    let remote_map = by_path(remote);
    let baseline_map = by_path(baseline);

    let mut paths: HashSet<&String> = HashSet::new();
    paths.extend(local_map.keys());
    paths.extend(remote_map.keys());
    paths.extend(baseline_map.keys());

    let mut actions = Vec::new();
    for path in paths {
        let l = local_map.get(path);
        let r = remote_map.get(path);
        let b = baseline_map.get(path);
        match (l, r, b) {
            (Some(_), None, None) => actions.push(SyncAction::Upload { path: path.clone() }),
            (None, Some(_), None) => actions.push(SyncAction::Download { path: path.clone() }),
            (None, Some(_), Some(_)) => {
                actions.push(SyncAction::DeleteRemote { path: path.clone() })
            }
            (Some(_), None, Some(_)) => {
                actions.push(SyncAction::DeleteLocal { path: path.clone() })
            }
            (Some(l), Some(r), b) if l.hash != r.hash => {
                let local_changed = b.is_none_or(|b| l.hash != b.hash);
                let remote_changed = b.is_none_or(|b| r.hash != b.hash);
                if local_changed && remote_changed {
                    actions.push(SyncAction::Conflict {
                        path: path.clone(),
                        local_hash: l.hash.clone(),
                        remote_hash: r.hash.clone(),
                    });
                } else if local_changed {
                    actions.push(SyncAction::Upload { path: path.clone() });
                } else if remote_changed {
                    actions.push(SyncAction::Download { path: path.clone() });
                }
            }
            _ => {}
        }
    }

    actions.sort_by(|a, b| a.path().cmp(b.path()));
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, hash: &str) -> FileState {
        FileState {
            path: path.to_string(),
            mtime: 1,
            size: 5,
            hash: hash.to_string(),
        }
    }

    #[test]
    fn identical_states_do_nothing() {
        let state = [f("a.txt", "abc")];
        assert_eq!(reconcile(&state, &state, &state), vec![]);
    }

    #[test]
    fn new_local_file_uploads() {
        let local = [f("new.txt", "x")];
        assert_eq!(
            reconcile(&local, &[], &[]),
            vec![SyncAction::Upload {
                path: "new.txt".to_string()
            }]
        );
    }

    #[test]
    fn new_remote_file_downloads() {
        let remote = [f("remote.txt", "y")];
        assert_eq!(
            reconcile(&[], &remote, &[]),
            vec![SyncAction::Download {
                path: "remote.txt".to_string()
            }]
        );
    }

    #[test]
    fn both_sides_changed_is_a_conflict() {
        let baseline = [f("f.txt", "base")];
        let local = [f("f.txt", "local")];
        let remote = [f("f.txt", "remote")];
        assert_eq!(
            reconcile(&local, &remote, &baseline),
            vec![SyncAction::Conflict {
                path: "f.txt".to_string(),
                local_hash: "local".to_string(),
                remote_hash: "remote".to_string(),
            }]
        );
    }

    #[test]
    fn actions_are_sorted_by_path() {
        let local = [f("z.txt", "z"), f("a.txt", "a")];
        let actions = reconcile(&local, &[], &[]);
        assert_eq!(actions[0].path(), "a.txt");
        assert_eq!(actions[1].path(), "z.txt");
    }

    #[test]
    fn deletes_and_one_sided_changes() {
        let baseline = [f("gone.txt", "h")];
        // Deleted locally, unchanged remotely since baseline → delete remote.
        assert_eq!(
            reconcile(&[], &[f("gone.txt", "h")], &baseline),
            vec![SyncAction::DeleteRemote {
                path: "gone.txt".to_string()
            }]
        );
        // Deleted remotely, unchanged locally → delete local.
        assert_eq!(
            reconcile(&[f("gone.txt", "h")], &[], &baseline),
            vec![SyncAction::DeleteLocal {
                path: "gone.txt".to_string()
            }]
        );
        // Only the local side changed since baseline → upload.
        assert_eq!(
            reconcile(
                &[f("f.txt", "new")],
                &[f("f.txt", "old")],
                &[f("f.txt", "old")]
            ),
            vec![SyncAction::Upload {
                path: "f.txt".to_string()
            }]
        );
    }
}

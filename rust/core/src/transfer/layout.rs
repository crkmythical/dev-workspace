//! File transfer (croc): fixed endpoints and defaults — a port of
//! packages/core/src/transfer/layout.ts.

use crate::shared::layout::WORKSPACE_MOUNT;

/// Clash's mixed port (HTTP + SOCKS5) on localhost — the endpoint `all_proxy` already advertises.
/// `push` / `pull` reach a remote relay through it by default.
pub const CLASH_SOCKS_ADDR: &str = "127.0.0.1:7890";

/// Where `pull` writes received files unless told otherwise: the vault mount.
pub const TRANSFER_DEFAULT_OUT: &str = WORKSPACE_MOUNT;

/** File transfer (croc): fixed endpoints and defaults. */
import { WORKSPACE_MOUNT } from "../shared/layout.ts";

/**
 * Clash's mixed port (HTTP + SOCKS5) on localhost — the endpoint `all_proxy`
 * already advertises. `push` / `pull` reach a remote relay through it by default.
 */
export const CLASH_SOCKS_ADDR = "127.0.0.1:7890";

/** Where `pull` writes received files unless told otherwise: the vault mount. */
export const TRANSFER_DEFAULT_OUT = WORKSPACE_MOUNT;

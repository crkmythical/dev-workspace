//! File transfer over croc — a port of packages/core/src/transfer. Pure argv builders; the cli
//! push/pull adapters read the environment (CROC_PASS, CROC_SECRET) and spawn croc with these.

pub mod croc;
pub mod layout;

pub use croc::{build_croc_receive, build_croc_send, ReceiveOptions, SendOptions, Socks};
pub use layout::{CLASH_SOCKS_ADDR, TRANSFER_DEFAULT_OUT};

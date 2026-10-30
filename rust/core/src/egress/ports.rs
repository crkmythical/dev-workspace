//! Egress ports: what bringing the proxy up needs from the outside world — a port of
//! packages/core/src/egress/ports.ts.
//!
//! Implemented by an adapter in the cli/infra layer; replaced by fakes in tests. The TS methods
//! are async because Bun I/O is; the bring-up is strictly sequential, so the trait is sync here
//! (a start-up doing one blocking step after another), which keeps the core crate runtime-free.
//! Where a TS method could throw and the use case swallowed it, the method returns `Result` and
//! the use case treats `Err` as the miss.

pub trait EgressPorts {
    /// Launch the proxy process. May fail (binary missing); the use case carries on.
    fn start_proxy(&self) -> Result<(), String>;
    /// Something accepts connections on the proxy port.
    fn proxy_listening(&self) -> Result<bool, String>;
    /// Nodes the subscription provider has loaded; None when the controller does not answer.
    fn provider_nodes(&self) -> Result<Option<i64>, String>;
    /// Fetch the subscription again now. Clash fetches it once at start-up and, if that fails
    /// (network not up yet after a reboot), tries again only after an hour.
    fn refresh_provider(&self) -> Result<(), String>;
    /// GLOBAL defaults to DIRECT, which bypasses the proxy: point it at the auto-select group.
    fn select_auto_node(&self) -> Result<(), String>;
    /// One request to the probe target THROUGH the proxy.
    fn probe(&self) -> Result<bool, String>;
    fn sleep(&self, ms: u64);
    /// ISO timestamp.
    fn now(&self) -> String;
    fn log(&self, message: &str);
}

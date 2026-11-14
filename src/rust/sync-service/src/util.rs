//! Small shared helpers.
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::HeaderMap;

/// Milliseconds since the Unix epoch, 0 if the clock is before it.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The caller's address as the proxies in front claim it. Prefer Cloudflare's edge-set
/// CF-Connecting-IP: when the only ingress is the CF tunnel, an external client cannot overwrite
/// it. Fall back to the first X-Forwarded-For hop, then a constant (which still shares the global
/// budget of the rate limiters). A claim, not a proof: a caller that reaches the origin directly
/// can set either header, which is why the limiters also have a global cap.
pub fn client_ip(headers: &HeaderMap) -> String {
    if let Some(cf) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
    {
        return cf.trim().to_string();
    }
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            return first.trim().to_string();
        }
    }
    "unknown".to_string()
}

/// The User-Agent, if the client sent one (for the audit trail; length and control characters are
/// handled when the event is built).
pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

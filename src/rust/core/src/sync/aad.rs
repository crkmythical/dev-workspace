//! AAD framing — must match packages/core/src/aad.ts.
//! Format: "op|timestamp_ms|extra1|extra2|...".

pub fn build_aad(op: &str, timestamp_ms: i64, extra: &[&str]) -> Vec<u8> {
    let mut parts: Vec<String> = Vec::with_capacity(2 + extra.len());
    parts.push(op.to_string());
    parts.push(timestamp_ms.to_string());
    for e in extra {
        parts.push((*e).to_string());
    }
    parts.join("|").into_bytes()
}

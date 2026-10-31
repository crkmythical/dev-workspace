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

pub fn build_chunk_aad(timestamp_ms: i64, file_id: &str, chunk_idx: i64) -> Vec<u8> {
    let ci = chunk_idx.to_string();
    build_aad("put-chunk", timestamp_ms, &[file_id, ci.as_str()])
}

pub fn build_finalize_aad(timestamp_ms: i64, file_id: &str, total_chunks: i64) -> Vec<u8> {
    let tc = total_chunks.to_string();
    build_aad("put-finalize", timestamp_ms, &[file_id, tc.as_str()])
}

//! Pure client-side construction of the sync wire protocol.
//!
//! Browser WebAssembly and the native smoke client share this module. Time and random nonces are
//! supplied by the driving adapter, so the protocol has one Rust implementation without depending
//! on a browser, an HTTP library or an OS random source.

use base64::{engine::general_purpose::STANDARD, Engine as _};

use super::aad::build_aad;
use super::crypto::encrypt;
use super::protocol::{
    HEADER_AUTH, HEADER_CHUNK_IDX, HEADER_FILE_ID, HEADER_OP, HEADER_TS, NONCE_LENGTH, PNG_HEADER,
};

pub type SyncNonce = [u8; NONCE_LENGTH];

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

fn seal(
    key: &[u8; 32],
    nonce: &SyncNonce,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    let ciphertext = encrypt(key, nonce, plaintext, aad)?;
    let mut body = Vec::with_capacity(PNG_HEADER.len() + nonce.len() + ciphertext.len());
    body.extend_from_slice(&PNG_HEADER);
    body.extend_from_slice(nonce);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

fn upload_headers(
    op: &'static str,
    timestamp_ms: i64,
    file_id: &str,
    chunk_index: u64,
) -> Vec<(&'static str, String)> {
    vec![
        ("content-type", "image/png".to_string()),
        (HEADER_OP, op.to_string()),
        (HEADER_TS, timestamp_ms.to_string()),
        (HEADER_FILE_ID, file_id.to_string()),
        (HEADER_CHUNK_IDX, chunk_index.to_string()),
    ]
}

pub fn build_chunk_request(
    key: &[u8; 32],
    timestamp_ms: i64,
    file_id: &str,
    chunk_index: u64,
    chunk: &[u8],
    nonce: SyncNonce,
) -> Result<SyncRequest, String> {
    let index = chunk_index.to_string();
    let aad = build_aad("put-chunk", timestamp_ms, &[file_id, &index]);
    Ok(SyncRequest {
        headers: upload_headers("put-chunk", timestamp_ms, file_id, chunk_index),
        body: seal(key, &nonce, chunk, &aad)?,
    })
}

pub fn build_finalize_request(
    key: &[u8; 32],
    timestamp_ms: i64,
    file_id: &str,
    file_path: &str,
    content_sha256: &str,
    total_chunks: u64,
    nonce: SyncNonce,
) -> Result<SyncRequest, String> {
    let total = total_chunks.to_string();
    let payload = serde_json::to_vec(&serde_json::json!({
        "file_id": file_id,
        "file_path": file_path,
        "content_sha256": content_sha256,
        "total_chunks": total_chunks,
    }))
    .map_err(|e| e.to_string())?;
    let aad = build_aad("put-finalize", timestamp_ms, &[file_id, &total]);
    Ok(SyncRequest {
        headers: upload_headers("put-finalize", timestamp_ms, file_id, total_chunks),
        body: seal(key, &nonce, &payload, &aad)?,
    })
}

pub fn build_list_headers(
    key: &[u8; 32],
    timestamp_ms: i64,
    nonce: SyncNonce,
) -> Result<Vec<(&'static str, String)>, String> {
    let tag = encrypt(key, &nonce, &[], &build_aad("list", timestamp_ms, &[]))?;
    let mut auth = Vec::with_capacity(nonce.len() + tag.len());
    auth.extend_from_slice(&nonce);
    auth.extend_from_slice(&tag);
    Ok(vec![
        (HEADER_TS, timestamp_ms.to_string()),
        (HEADER_AUTH, STANDARD.encode(auth)),
    ])
}

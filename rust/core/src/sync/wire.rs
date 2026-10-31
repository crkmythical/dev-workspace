//! Upload body framing: PNG_HEADER(33) || nonce(12) || ciphertext||tag.
//! Mirrors the parsing in packages/server/src/routes/upload.ts.

use super::protocol::{NONCE_LENGTH, PNG_HEADER_SIZE, TAG_LENGTH};

pub struct UploadFrame<'a> {
    pub nonce: &'a [u8],
    pub ct_with_tag: &'a [u8],
}

/// Strip the PNG header and split off the nonce. Returns None if the body is
/// too short to contain header + nonce + tag.
pub fn parse_upload_body(body: &[u8]) -> Option<UploadFrame<'_>> {
    if body.len() < PNG_HEADER_SIZE + NONCE_LENGTH + TAG_LENGTH {
        return None;
    }
    let raw = &body[PNG_HEADER_SIZE..];
    Some(UploadFrame {
        nonce: &raw[..NONCE_LENGTH],
        ct_with_tag: &raw[NONCE_LENGTH..],
    })
}

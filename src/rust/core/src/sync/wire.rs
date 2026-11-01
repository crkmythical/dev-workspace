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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MIN: usize = PNG_HEADER_SIZE + NONCE_LENGTH + TAG_LENGTH;

    proptest! {
        // This parser runs on UNAUTHENTICATED HTTP bodies (before decrypt), so it must be total.
        // Fuzz arbitrary bytes: it never panics; returns Some exactly when long enough; and the
        // framing is exact (nonce is the 12 bytes after the header, ct_with_tag is the rest and
        // always holds at least a tag). A slicing bug here would be a panic on hostile input.
        #[test]
        fn parse_upload_body_is_total_and_frames_exactly(
            body in proptest::collection::vec(any::<u8>(), 0..8192)
        ) {
            match parse_upload_body(&body) {
                None => prop_assert!(body.len() < MIN),
                Some(f) => {
                    prop_assert!(body.len() >= MIN);
                    prop_assert_eq!(f.nonce.len(), NONCE_LENGTH);
                    prop_assert!(f.ct_with_tag.len() >= TAG_LENGTH);
                    prop_assert_eq!(
                        f.nonce.len() + f.ct_with_tag.len(),
                        body.len() - PNG_HEADER_SIZE
                    );
                    prop_assert_eq!(
                        f.nonce,
                        &body[PNG_HEADER_SIZE..PNG_HEADER_SIZE + NONCE_LENGTH]
                    );
                }
            }
        }
    }
}

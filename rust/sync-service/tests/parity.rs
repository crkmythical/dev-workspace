//! Cross-language parity: the Rust implementation must reproduce the golden
//! wire vectors generated from the authoritative TypeScript implementation
//! (packages/server/scripts/gen-wire-vectors.ts).
//!
//! Regenerate vectors with:
//!   bun run packages/server/scripts/gen-wire-vectors.ts

use serde_json::Value;
use sync_service::aad::build_aad;
use sync_service::constants::PNG_HEADER;
use sync_service::crypto::{decrypt, derive_key, encrypt};
use sync_service::path_safety::{check_relative_path, PathCheck};
use sync_service::wire::parse_upload_body;

fn vectors() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/wire-vectors.json");
    let raw = std::fs::read_to_string(path).expect("read wire-vectors.json");
    serde_json::from_str(&raw).expect("parse wire-vectors.json")
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s).expect("valid hex")
}

#[test]
fn png_header_matches_ts() {
    let v = vectors();
    assert_eq!(hex::encode(PNG_HEADER), v["png_header_hex"].as_str().unwrap());
}

#[test]
fn hkdf_matches_ts() {
    let v = vectors();
    for item in v["hkdf"].as_array().unwrap() {
        let passphrase = item["passphrase"].as_str().unwrap();
        let want = item["key_hex"].as_str().unwrap();
        assert_eq!(hex::encode(derive_key(passphrase)), want, "hkdf: {passphrase:?}");
    }
}

#[test]
fn aad_matches_ts() {
    let v = vectors();
    for item in v["aad"].as_array().unwrap() {
        let op = item["op"].as_str().unwrap();
        let ts = item["ts"].as_i64().unwrap();
        let extra: Vec<String> = item["extra"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_string())
            .collect();
        let extra_refs: Vec<&str> = extra.iter().map(String::as_str).collect();
        let got = build_aad(op, ts, &extra_refs);
        assert_eq!(hex::encode(got), item["aad_hex"].as_str().unwrap(), "aad: {op}");
    }
}

#[test]
fn aead_roundtrip_matches_ts() {
    let v = vectors();
    for item in v["aead"].as_array().unwrap() {
        let label = item["label"].as_str().unwrap_or("");
        let key = unhex(item["key_hex"].as_str().unwrap());
        let nonce = unhex(item["nonce_hex"].as_str().unwrap());
        let aad = unhex(item["aad_hex"].as_str().unwrap());
        let ct_with_tag = unhex(item["ciphertext_with_tag_hex"].as_str().unwrap());
        let plaintext = unhex(item["plaintext_hex"].as_str().unwrap());
        let body = unhex(item["body_hex"].as_str().unwrap());

        // 1. decrypt(ciphertext||tag) == plaintext
        let dec = decrypt(&key, &nonce, &ct_with_tag, &aad).expect("decrypt");
        assert_eq!(dec, plaintext, "decrypt mismatch: {label}");

        // 2. encrypt(plaintext) == ciphertext||tag (deterministic nonce)
        let enc = encrypt(&key, &nonce, &plaintext, &aad).expect("encrypt");
        assert_eq!(
            hex::encode(&enc),
            item["ciphertext_with_tag_hex"].as_str().unwrap(),
            "encrypt mismatch: {label}"
        );

        // 3. full PNG-camouflaged body parses + decrypts
        let frame = parse_upload_body(&body).expect("frame");
        assert_eq!(frame.nonce, &nonce[..], "nonce framing: {label}");
        assert_eq!(frame.ct_with_tag, &ct_with_tag[..], "ct framing: {label}");
        let dec2 = decrypt(&key, frame.nonce, frame.ct_with_tag, &aad).expect("decrypt frame");
        assert_eq!(dec2, plaintext, "body decrypt mismatch: {label}");
    }
}

#[test]
fn path_safety_matches_ts() {
    let v = vectors();
    for item in v["paths"].as_array().unwrap() {
        let input = item["input"].as_str().unwrap();
        let result = &item["result"];
        match check_relative_path(input) {
            PathCheck::Ok(norm) => {
                assert_eq!(result["ok"].as_bool(), Some(true), "expected ok: {input:?}");
                assert_eq!(result["normalized"].as_str(), Some(norm.as_str()), "normalized: {input:?}");
            }
            PathCheck::Err(reason) => {
                assert_eq!(result["ok"].as_bool(), Some(false), "expected err: {input:?}");
                assert_eq!(result["reason"].as_str(), Some(reason.as_str()), "reason: {input:?}");
            }
        }
    }
}

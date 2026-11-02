//! Verify a Cloudflare Access JWT (the `Cf-Access-Jwt-Assertion` header Access sets at the edge) so
//! the ingress gate can accept an edge-authenticated session as well as its own cookie — no
//! double login behind Access. Pure: given the team JWKS, the app AUD and the issuer, and `now`,
//! it checks the RS256 signature and the claims. The JWKS is fetched and cached by the
//! sync-service; nothing here does I/O.
//!
//! Opt-in: the sync-service only consults this when ACCESS_TEAM_DOMAIN and ACCESS_AUD are set, so a
//! deployment NOT behind Access never trusts the header (a forged assertion to a non-Access origin
//! cannot pass — the signature must verify against the real team keys).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use serde::Deserialize;
use sha2::Sha256;

/// One RSA signing key from the team's `/cdn-cgi/access/certs` JWKS (base64url `n`, `e`).
#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    pub kid: String,
    pub n: String,
    pub e: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Jwks {
    #[serde(default)]
    pub keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessError {
    Malformed,
    UnsupportedAlg,
    UnknownKid,
    BadSignature,
    WrongAudience,
    WrongIssuer,
    Expired,
    NotYetValid,
}

#[derive(Debug, Deserialize)]
struct Header {
    alg: String,
    kid: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AudField {
    One(String),
    Many(Vec<String>),
}

impl AudField {
    fn contains(opt: &Option<AudField>, want: &str) -> bool {
        match opt {
            Some(AudField::One(s)) => s == want,
            Some(AudField::Many(v)) => v.iter().any(|s| s == want),
            None => false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Claims {
    #[serde(default)]
    aud: Option<AudField>,
    iss: String,
    exp: u64,
    #[serde(default)]
    nbf: Option<u64>,
}

/// Verify `token` against the JWKS, the expected app AUD and team issuer at `now` (epoch seconds).
/// The signature is checked BEFORE any claim is trusted. `Ok(())` means the assertion is a genuine,
/// current Access session for this application.
pub fn verify_access_jwt(
    token: &str,
    jwks: &Jwks,
    expected_aud: &str,
    expected_iss: &str,
    now: u64,
) -> Result<(), AccessError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AccessError::Malformed);
    }
    let header: Header = decode_json(parts[0]).ok_or(AccessError::Malformed)?;
    if header.alg != "RS256" {
        return Err(AccessError::UnsupportedAlg);
    }
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kid == header.kid)
        .ok_or(AccessError::UnknownKid)?;
    let key = rsa_key(jwk).ok_or(AccessError::BadSignature)?;
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| AccessError::Malformed)?;
    let sig = Signature::try_from(sig_bytes.as_slice()).map_err(|_| AccessError::BadSignature)?;
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    VerifyingKey::<Sha256>::new(key)
        .verify(signing_input.as_bytes(), &sig)
        .map_err(|_| AccessError::BadSignature)?;

    // Signature is good; now the claims can be trusted.
    let claims: Claims = decode_json(parts[1]).ok_or(AccessError::Malformed)?;
    if !AudField::contains(&claims.aud, expected_aud) {
        return Err(AccessError::WrongAudience);
    }
    if claims.iss != expected_iss {
        return Err(AccessError::WrongIssuer);
    }
    if now >= claims.exp {
        return Err(AccessError::Expired);
    }
    if let Some(nbf) = claims.nbf {
        if now < nbf {
            return Err(AccessError::NotYetValid);
        }
    }
    Ok(())
}

fn decode_json<T: for<'de> Deserialize<'de>>(b64url: &str) -> Option<T> {
    let bytes = URL_SAFE_NO_PAD.decode(b64url).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn rsa_key(jwk: &Jwk) -> Option<RsaPublicKey> {
    let n = BigUint::from_bytes_be(&URL_SAFE_NO_PAD.decode(&jwk.n).ok()?);
    let e = BigUint::from_bytes_be(&URL_SAFE_NO_PAD.decode(&jwk.e).ok()?);
    RsaPublicKey::new(n, e).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;
    use std::sync::OnceLock;

    const KID: &str = "test-kid-1";
    const AUD: &str = "test-aud-tag";
    const ISS: &str = "https://team.cloudflareaccess.com";

    // One throwaway 2048-bit key for the whole test binary (keygen is the slow part).
    fn key() -> &'static RsaPrivateKey {
        static K: OnceLock<RsaPrivateKey> = OnceLock::new();
        K.get_or_init(|| RsaPrivateKey::new(&mut rand_core::OsRng, 2048).unwrap())
    }

    fn jwks() -> Jwks {
        let pk = key().to_public_key();
        Jwks {
            keys: vec![Jwk {
                kid: KID.to_string(),
                n: URL_SAFE_NO_PAD.encode(pk.n().to_bytes_be()),
                e: URL_SAFE_NO_PAD.encode(pk.e().to_bytes_be()),
            }],
        }
    }

    /// Build a signed JWT from a header and claims JSON body.
    fn make_jwt(header: &str, claims: &str) -> String {
        let h = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let c = URL_SAFE_NO_PAD.encode(claims.as_bytes());
        let signing_input = format!("{h}.{c}");
        let sig = SigningKey::<Sha256>::new(key().clone()).sign(signing_input.as_bytes());
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(sig.to_bytes()))
    }

    fn good_claims(exp: u64) -> String {
        format!(r#"{{"aud":["{AUD}"],"iss":"{ISS}","exp":{exp},"iat":1000}}"#)
    }
    const H: &str = r#"{"alg":"RS256","kid":"test-kid-1","typ":"JWT"}"#;

    #[test]
    fn accepts_a_genuine_current_token() {
        let t = make_jwt(H, &good_claims(5000));
        assert_eq!(verify_access_jwt(&t, &jwks(), AUD, ISS, 1500), Ok(()));
        // aud as a bare string is also accepted.
        let t2 = make_jwt(H, &format!(r#"{{"aud":"{AUD}","iss":"{ISS}","exp":5000}}"#));
        assert_eq!(verify_access_jwt(&t2, &jwks(), AUD, ISS, 1500), Ok(()));
    }

    #[test]
    fn rejects_wrong_audience_and_issuer() {
        let t = make_jwt(H, &good_claims(5000));
        assert_eq!(
            verify_access_jwt(&t, &jwks(), "other-aud", ISS, 1500),
            Err(AccessError::WrongAudience)
        );
        assert_eq!(
            verify_access_jwt(&t, &jwks(), AUD, "https://evil.example", 1500),
            Err(AccessError::WrongIssuer)
        );
    }

    #[test]
    fn rejects_expired_and_not_yet_valid() {
        let t = make_jwt(H, &good_claims(5000));
        assert_eq!(
            verify_access_jwt(&t, &jwks(), AUD, ISS, 5000),
            Err(AccessError::Expired)
        );
        let future = make_jwt(
            H,
            &format!(r#"{{"aud":["{AUD}"],"iss":"{ISS}","exp":9000,"nbf":8000}}"#),
        );
        assert_eq!(
            verify_access_jwt(&future, &jwks(), AUD, ISS, 1500),
            Err(AccessError::NotYetValid)
        );
    }

    #[test]
    fn rejects_bad_signature_and_tampering() {
        // A claims body swapped after signing: signature no longer matches.
        let t = make_jwt(H, &good_claims(5000));
        let mut p: Vec<&str> = t.split('.').collect();
        let forged_claims = URL_SAFE_NO_PAD.encode(good_claims(9999).as_bytes());
        let forged = format!("{}.{}.{}", p[0], forged_claims, p[2]);
        assert_eq!(
            verify_access_jwt(&forged, &jwks(), AUD, ISS, 1500),
            Err(AccessError::BadSignature)
        );
        // Flip the signature.
        p[2] = "AAAA";
        let bad = p.join(".");
        assert!(matches!(
            verify_access_jwt(&bad, &jwks(), AUD, ISS, 1500),
            Err(AccessError::BadSignature | AccessError::Malformed)
        ));
    }

    #[test]
    fn rejects_unknown_kid_and_alg_and_shape() {
        let other_kid = make_jwt(
            r#"{"alg":"RS256","kid":"nope","typ":"JWT"}"#,
            &good_claims(5000),
        );
        assert_eq!(
            verify_access_jwt(&other_kid, &jwks(), AUD, ISS, 1500),
            Err(AccessError::UnknownKid)
        );
        let hs = make_jwt(r#"{"alg":"HS256","kid":"test-kid-1"}"#, &good_claims(5000));
        assert_eq!(
            verify_access_jwt(&hs, &jwks(), AUD, ISS, 1500),
            Err(AccessError::UnsupportedAlg)
        );
        for bad in ["", "only.two", "a.b.c.d"] {
            assert_eq!(
                verify_access_jwt(bad, &jwks(), AUD, ISS, 1500),
                Err(AccessError::Malformed)
            );
        }
    }
}

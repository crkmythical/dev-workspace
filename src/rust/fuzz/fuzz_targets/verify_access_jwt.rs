#![no_main]
//! Coverage-guided fuzzing of the Cloudflare Access assertion verifier. The `Cf-Access-Jwt-Assertion`
//! header reaches it from whoever can reach the origin; it must never panic on any bytes. The key
//! set holds one (valid-shaped, unrelated) RSA key under the id "k", so inputs that name that id
//! get past the lookup and into the key and signature handling.
use libfuzzer_sys::fuzz_target;
use sdw_core::auth::access_jwt::{verify_access_jwt_identity, Jwk, Jwks};

fuzz_target!(|data: &[u8]| {
    let Ok(token) = std::str::from_utf8(data) else {
        return;
    };
    let jwks = Jwks {
        keys: vec![Jwk {
            kid: "k".to_string(),
            // A 2048-bit modulus is not needed to reach the code paths; any well-formed pair does.
            n: "sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw".to_string(),
            e: "AQAB".to_string(),
        }],
    };
    let _ = verify_access_jwt_identity(token, &jwks, "aud", "https://team.example", 1_700_000_000);
});

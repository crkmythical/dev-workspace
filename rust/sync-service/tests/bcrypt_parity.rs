//! Pins Bun.password (the TS side's bcrypt) against the `bcrypt` crate the Rust
//! destruct route verifies with. The remote self-destruct turns on a `$2b$` hash
//! Bun wrote at unlock time; if the crate ever stopped reading Bun's hashes, the
//! endpoint would silently reject the real passphrase. These hashes were produced
//! by Bun v1.3.14: Bun.password.hash("correct horse battery staple",
//! { algorithm: "bcrypt", cost }). The passphrase is a public test value.

const PASSPHRASE: &str = "correct horse battery staple";

const BUN_HASHES: &[&str] = &[
    "$2b$08$efMja3NPRA/pb4MDgnmf.eCXOE56IzoWLpNS7VUjMCRFKM.GdaB3G",
    "$2b$10$tNKadSi.GVRpzgmLBHNFze2GtRI8KQq3u9rBb7.RJAdyiEQWO2XB6",
    "$2b$12$gVtQuf87OfLD4UAN8QJeUObEQzrP00NLgjQtRWOdMW6TK/l7GR.l6",
];

#[test]
fn bcrypt_crate_verifies_bun_hashes() {
    for h in BUN_HASHES {
        assert!(
            bcrypt::verify(PASSPHRASE, h).unwrap(),
            "accept the right passphrase: {h}"
        );
        assert!(
            !bcrypt::verify("wrong passphrase", h).unwrap(),
            "reject a wrong one: {h}"
        );
    }
}

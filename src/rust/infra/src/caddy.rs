//! Password hashing for the ingress auth gate (the sync-service verifies logins against it).

/// bcrypt cost of the master-password hash. Verified on every login attempt, so it is also what
/// bounds online guessing together with the login rate limit; 12 is about a quarter of a second.
const BCRYPT_COST: u32 = 12;

/// bcrypt hash of the master password, computed in this process. It used to be `caddy
/// hash-password --plaintext PASSWORD`, which put the master password (also the vault passphrase)
/// on a command line any process in the container could read from /proc while it ran. Empty when
/// the password cannot be hashed; the caller decides what that means.
pub fn caddy_password_hash(password: &str) -> String {
    bcrypt::hash(password, BCRYPT_COST).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_verifies_the_password_and_nothing_else() {
        let h = caddy_password_hash("p@ss w0rd \"quoted\" 'x'");
        assert!(h.starts_with("$2"), "a bcrypt hash: {h}");
        assert!(bcrypt::verify("p@ss w0rd \"quoted\" 'x'", &h).unwrap());
        assert!(!bcrypt::verify("p@ss w0rd", &h).unwrap());
    }
}

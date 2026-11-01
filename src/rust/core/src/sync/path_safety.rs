//! Relative-path validation — must match packages/core/src/path-safety.ts,
//! including the exact rejection-reason strings and check ordering.

#[derive(Debug, PartialEq, Eq)]
pub enum PathRejection {
    Empty,
    Absolute,
    NulByte,
    Traversal,
    Backslash,
}

impl PathRejection {
    pub fn as_str(&self) -> &'static str {
        match self {
            PathRejection::Empty => "empty",
            PathRejection::Absolute => "absolute",
            PathRejection::NulByte => "nul-byte",
            PathRejection::Traversal => "traversal",
            PathRejection::Backslash => "backslash",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PathCheck {
    Ok(String),
    Err(PathRejection),
}

pub fn check_relative_path(input: &str) -> PathCheck {
    if input.is_empty() {
        return PathCheck::Err(PathRejection::Empty);
    }
    if input.contains('\0') {
        return PathCheck::Err(PathRejection::NulByte);
    }
    if input.contains('\\') {
        return PathCheck::Err(PathRejection::Backslash);
    }
    if input.starts_with('/') {
        return PathCheck::Err(PathRejection::Absolute);
    }
    let mut out: Vec<&str> = Vec::new();
    for seg in input.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return PathCheck::Err(PathRejection::Traversal);
        }
        out.push(seg);
    }
    if out.is_empty() {
        return PathCheck::Err(PathRejection::Empty);
    }
    PathCheck::Ok(out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Strings built from an alphabet that heavily exercises the dangerous tokens (`..`, slashes,
    /// backslash, NUL, a non-ASCII char), so the fuzzer spends its budget on traversal attempts.
    fn hostile_path() -> impl Strategy<Value = String> {
        let token = prop_oneof![
            Just("a".to_string()),
            Just("..".to_string()),
            Just(".".to_string()),
            Just("/".to_string()),
            Just("\\".to_string()),
            Just("\0".to_string()),
            Just("日".to_string()),
            "[a-z]{1,3}",
        ];
        proptest::collection::vec(token, 0..14).prop_map(|v| v.concat())
    }

    proptest! {
        // The security invariant: whatever check_relative_path ACCEPTS must be a safe, normalized
        // relative path — never absolute, never a backslash/NUL, and with no empty / "." / ".."
        // segment. A traversal that slipped through would fail here. (This path is the only guard
        // on where an authenticated upload lands in the vault.)
        #[test]
        fn never_accepts_an_unsafe_path(s in hostile_path()) {
            if let PathCheck::Ok(norm) = check_relative_path(&s) {
                prop_assert!(!norm.is_empty());
                prop_assert!(!norm.starts_with('/'), "absolute slipped through: {:?}", norm);
                prop_assert!(!norm.contains('\0'));
                prop_assert!(!norm.contains('\\'));
                for seg in norm.split('/') {
                    prop_assert!(!seg.is_empty(), "empty segment in {:?}", norm);
                    prop_assert_ne!(seg, ".");
                    prop_assert_ne!(seg, "..", "traversal slipped through: {:?}", norm);
                }
            }
        }

        // Fully arbitrary input (unicode, control chars) must never panic.
        #[test]
        fn never_panics_on_arbitrary_input(s in ".*") {
            let _ = check_relative_path(&s);
        }
    }
}

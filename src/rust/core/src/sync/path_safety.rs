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

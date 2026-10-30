//! supervisord program files: switching on-demand programs to start at boot — a port of
//! packages/infra/src/supervisor.ts. Replaces `sed -i` one-liners with something testable.

use std::fs;

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\r'
}

/// Rewrite one line matching `^\s*autostart\s*=\s*false\b`, keeping its prefix and swapping only
/// `false` for `true`; any other line is returned unchanged.
fn rewrite_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_ws(bytes[i]) {
        i += 1;
    }
    if !line[i..].starts_with("autostart") {
        return line.to_string();
    }
    i += "autostart".len();
    while i < bytes.len() && is_ws(bytes[i]) {
        i += 1;
    }
    if bytes.get(i) != Some(&b'=') {
        return line.to_string();
    }
    i += 1;
    while i < bytes.len() && is_ws(bytes[i]) {
        i += 1;
    }
    if !line[i..].starts_with("false") {
        return line.to_string();
    }
    let after = i + "false".len();
    // \b: `false` must not be the start of a longer word (falsey).
    let boundary = line[after..]
        .chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    if !boundary {
        return line.to_string();
    }
    format!("{}true{}", &line[..i], &line[after..])
}

/// `autostart=false` -> `autostart=true`, on directive lines only (not in comments). Pure.
pub fn with_autostart(conf: &str) -> String {
    // split/join on '\n' preserves the exact structure, trailing newline included.
    conf.split('\n')
        .map(rewrite_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Make supervisord start the programs in `file` by itself. A missing file is skipped: not every
/// image contains every desktop stack. Returns whether the file changed.
pub fn enable_autostart(file: &str) -> bool {
    let before = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let after = with_autostart(&before);
    if after == before {
        return false;
    }
    fs::write(file, after).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flips_false_to_true_keeping_spacing() {
        assert_eq!(with_autostart("autostart=false"), "autostart=true");
        assert_eq!(with_autostart("  autostart = false"), "  autostart = true");
        assert_eq!(
            with_autostart("\tautostart\t=\tfalse"),
            "\tautostart\t=\ttrue"
        );
    }

    #[test]
    fn leaves_other_lines_untouched() {
        assert_eq!(with_autostart("autostart=true"), "autostart=true");
        assert_eq!(with_autostart(";autostart=false"), ";autostart=false");
        assert_eq!(with_autostart("# autostart=false"), "# autostart=false");
        assert_eq!(with_autostart("autostartx=false"), "autostartx=false");
        assert_eq!(with_autostart("autostart=falsey"), "autostart=falsey");
    }

    #[test]
    fn rewrites_only_the_matching_lines_of_a_block_and_keeps_structure() {
        let input = "[program:x]\ncommand=foo\nautostart=false\nautorestart=true\n";
        let want = "[program:x]\ncommand=foo\nautostart=true\nautorestart=true\n";
        assert_eq!(with_autostart(input), want);
    }

    #[test]
    fn keeps_a_trailing_comment_after_false() {
        assert_eq!(
            with_autostart("autostart=false ; was off"),
            "autostart=true ; was off"
        );
    }
}

//! Passphrase input is a CLI concern (TTY / stdin), so it stays out of the vault use cases, which
//! receive a lazy reader instead — a port of packages/cli/src/lib/passphrase.ts.

use std::io::{BufRead, IsTerminal, Read, Write};

/// Use `preset` when given (e.g. the PASSWORD variable the caller already read), else prompt on a
/// TTY, else read piped stdin. The prompt echoes, matching the TS `prompt()`.
pub fn read_passphrase(preset: Option<&str>, prompt_msg: &str) -> String {
    if let Some(p) = preset {
        if !p.is_empty() {
            return p.to_string();
        }
    }
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        print!("{prompt_msg}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            return String::new();
        }
        return line.trim_end_matches(['\n', '\r']).to_string();
    }
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return String::new();
    }
    buf.trim().to_string()
}

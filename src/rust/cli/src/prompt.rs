//! Interactive one-line prompt (echoed), for the confirmations the destructive commands make. A
//! port of Bun's `prompt()`: returns the line without its newline, or None on EOF.

use std::io::{BufRead, Write};

pub fn prompt_line(msg: &str) -> Option<String> {
    print!("{msg}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) => None, // EOF: no input at all
        Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
        Err(_) => None,
    }
}

//! `sdw env-example [--check]` — (re)generate `.env.example` from the env schema (core + plugins).
//! The dev tool behind `make env-example`; writes `.env.example` in the current directory, or with
//! `--check` exits 1 when the file is out of date. The drift test
//! (sdw_core::plugins::tests::env_example_is_up_to_date) fails CI if the committed file disagrees.

use sdw_core::plugins::env_groups;
use sdw_core::workspace::env_schema::render_env_example;

const TARGET: &str = ".env.example";

pub fn run(args: &[String]) -> i32 {
    let text = render_env_example(&env_groups());
    if args.iter().any(|a| a == "--check") {
        let current = std::fs::read_to_string(TARGET).unwrap_or_default();
        if current == text {
            println!("{TARGET} is up to date.");
            0
        } else {
            eprintln!("{TARGET} is out of date. Run: make env-example");
            1
        }
    } else {
        match std::fs::write(TARGET, &text) {
            Ok(()) => {
                println!("Wrote {TARGET}");
                0
            }
            Err(e) => {
                eprintln!("env-example: cannot write {TARGET}: {e}");
                1
            }
        }
    }
}

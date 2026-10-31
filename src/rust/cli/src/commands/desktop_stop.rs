//! desktop-stop — stop the remote desktop session(s). A port of packages/cli/src/desktop-stop.ts.
//!
//!   desktop-stop [selkies|vnc]   (no arg = every installed stack)

use sdw_infra::desktop::{parse_target, running_desktop_programs, stop_desktop};

pub fn run(args: &[String]) -> i32 {
    let target = match args.first().map(String::as_str) {
        None => None,
        Some(a) => match parse_target(a) {
            Some(t) => Some(t),
            None => {
                eprintln!("ERROR: unknown target '{a}'. Use 'selkies' or 'vnc'.");
                return 1;
            }
        },
    };

    stop_desktop(target);

    let still_running = running_desktop_programs();
    if still_running.is_empty() {
        println!("Desktop stopped.");
    } else {
        eprintln!("⚠ Some desktop processes still running:");
        for p in still_running {
            eprintln!("  {p}");
        }
    }
    0
}

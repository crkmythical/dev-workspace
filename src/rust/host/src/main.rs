mod cloudflare;
mod destroy;
mod docker_context;
mod env_file;
mod process;
mod secrets;
mod services;
mod watcher;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("destroy") => destroy::run(&args[2..]),
        Some("setup-tunnel") => cloudflare::setup_tunnel(&args[2..]),
        Some("provision") => cloudflare::provision(&args[2..]),
        Some("teardown") => cloudflare::teardown(&args[2..]),
        Some("up") => cloudflare::up(&args[2..]),
        Some("down") => cloudflare::down(&args[2..]),
        Some("run-tunnel") => cloudflare::run_tunnel(&args[2..]),
        Some("secrets-init") => secrets::run(&args[2..]),
        Some("services") => services::run(&args[2..]),
        Some("watch") => watcher::run(&args[2..]),
        Some("-h" | "--help") | None => {
            eprintln!(
                "usage: sdw-host <destroy|watch|services|secrets-init|setup-tunnel|provision|teardown|up|down|run-tunnel> [options]"
            );
            0
        }
        Some(command) => {
            eprintln!("sdw-host: unknown command '{command}'");
            2
        }
    };
    std::process::exit(code);
}

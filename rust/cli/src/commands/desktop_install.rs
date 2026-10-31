//! desktop-install — install GUI applications into the remote desktop. A port of
//! packages/cli/src/desktop-install.ts.
//!
//!   desktop-install <firefox|chromium|wireshark|idea|burpsuite>

use sdw_core::desktop::layout::{
    DESKTOP_APPLICATIONS_DIR, DESKTOP_APPS_DIR, DESKTOP_APPS_REGISTRY,
};
use sdw_infra::cmd;
use std::fs;

enum AppKind {
    Apt {
        package: &'static str,
    },
    Download {
        url: &'static str,
        extract_to: String,
        tarball: bool,
        filename: &'static str,
    },
}

struct AppDef {
    name: &'static str,
    kind: AppKind,
    binary: String,
    desktop_entry: String,
}

const AVAILABLE: &str = "firefox, chromium, wireshark, idea, burpsuite";

fn app_def(name: &str) -> Option<AppDef> {
    match name {
        "firefox" => Some(AppDef {
            name: "Firefox",
            kind: AppKind::Apt { package: "firefox-esr" },
            binary: "firefox-esr".to_string(),
            desktop_entry: "[Desktop Entry]\nName=Firefox\nExec=firefox-esr %u\nIcon=firefox-esr\nType=Application\nCategories=Network;WebBrowser;".to_string(),
        }),
        "chromium" => Some(AppDef {
            name: "Chromium",
            kind: AppKind::Apt { package: "chromium" },
            binary: "chromium".to_string(),
            desktop_entry: "[Desktop Entry]\nName=Chromium\nExec=chromium --no-sandbox %u\nIcon=chromium\nType=Application\nCategories=Network;WebBrowser;".to_string(),
        }),
        "wireshark" => Some(AppDef {
            name: "Wireshark",
            kind: AppKind::Apt { package: "wireshark" },
            binary: "wireshark".to_string(),
            desktop_entry: "[Desktop Entry]\nName=Wireshark\nExec=wireshark\nIcon=wireshark\nType=Application\nCategories=Network;".to_string(),
        }),
        "idea" => Some(AppDef {
            name: "IntelliJ IDEA",
            kind: AppKind::Download {
                url: "https://download.jetbrains.com/idea/ideaIC-2024.1.tar.gz",
                extract_to: format!("{DESKTOP_APPS_DIR}/idea"),
                tarball: true,
                filename: "idea",
            },
            binary: format!("{DESKTOP_APPS_DIR}/idea/bin/idea.sh"),
            desktop_entry: format!("[Desktop Entry]\nName=IntelliJ IDEA\nExec={DESKTOP_APPS_DIR}/idea/bin/idea.sh\nIcon={DESKTOP_APPS_DIR}/idea/bin/idea.svg\nType=Application\nCategories=Development;IDE;"),
        }),
        "burpsuite" => Some(AppDef {
            name: "Burp Suite",
            kind: AppKind::Download {
                url: "https://portswigger-cdn.net/burp/releases/download?product=community&type=Jar",
                extract_to: format!("{DESKTOP_APPS_DIR}/burpsuite"),
                tarball: false,
                filename: "burpsuite.jar",
            },
            binary: format!("java -jar {DESKTOP_APPS_DIR}/burpsuite/burpsuite.jar"),
            desktop_entry: format!("[Desktop Entry]\nName=Burp Suite\nExec=java -jar {DESKTOP_APPS_DIR}/burpsuite/burpsuite.jar\nIcon=burp\nType=Application\nCategories=Network;Security;"),
        }),
        _ => None,
    }
}

pub fn run(args: &[String]) -> i32 {
    let key = match args.first() {
        Some(a) => a.to_lowercase(),
        None => {
            println!("Usage: desktop-install <app>");
            println!("Available: {AVAILABLE}");
            return 1;
        }
    };
    let app = match app_def(&key) {
        Some(a) => a,
        None => {
            println!("Usage: desktop-install <app>");
            println!("Available: {AVAILABLE}");
            return 1;
        }
    };

    if !cmd::ok("nc", &["-z", "127.0.0.1", "7890"]) {
        eprintln!("⚠ Clash proxy not available. Downloads may fail.");
    }

    println!("Installing {}...", app.name);

    match &app.kind {
        AppKind::Apt { package } => {
            let script =
                format!("apt-get update && apt-get install -y --no-install-recommends {package}");
            if !cmd::ok("sh", &["-c", &script]) {
                eprintln!("ERROR: Failed to install {package}");
                return 1;
            }
        }
        AppKind::Download {
            url,
            extract_to,
            tarball,
            filename,
        } => {
            let _ = fs::create_dir_all(extract_to);
            if *tarball {
                let script =
                    format!("curl -fsSL '{url}' | tar xz -C '{extract_to}' --strip-components=1");
                if !cmd::ok("sh", &["-c", &script]) {
                    eprintln!("ERROR: Failed to download/extract {}", app.name);
                    return 1;
                }
            } else {
                let out = format!("{extract_to}/{filename}");
                if !cmd::ok("curl", &["-fsSL", url, "-o", &out]) {
                    eprintln!("ERROR: Failed to download {}", app.name);
                    return 1;
                }
            }
        }
    }

    // The .desktop file goes in the SYSTEM applications dir so BOTH desktop sessions see it (XFCE
    // reads /usr/share/applications regardless of per-stack HOME).
    let _ = fs::create_dir_all(DESKTOP_APPLICATIONS_DIR);
    let _ = fs::write(
        format!("{DESKTOP_APPLICATIONS_DIR}/{key}.desktop"),
        &app.desktop_entry,
    );

    // Update the registry (a JSON array of installed app keys).
    let _ = fs::create_dir_all(DESKTOP_APPS_DIR);
    let mut registry: Vec<String> = fs::read_to_string(DESKTOP_APPS_REGISTRY)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if !registry.contains(&key) {
        registry.push(key.clone());
        if let Ok(json) = serde_json::to_string_pretty(&registry) {
            let _ = fs::write(DESKTOP_APPS_REGISTRY, json);
        }
    }

    println!(
        "✓ Installed {}. Launch from desktop menu or: {}",
        app.name, app.binary
    );
    0
}

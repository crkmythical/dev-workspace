use std::cell::RefCell;
use std::collections::HashMap;

use gloo_timers::future::TimeoutFuture;
use sha2::{Digest, Sha256};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::Event;
use zeroize::Zeroize;

use crate::browser::{
    button, choose_directory, directory_name, directory_picker_available, element, input, log,
    open_diagnostics, poll_interval_ms, read_file_slice, request_status, scan_directory, set_class,
    set_hidden, set_status, set_text, window, FileEntry,
};
use crate::protocol::{
    chunk_request, derive_sync_key, finalize_request, list_headers, random_file_id, SyncKey,
    API_DESTRUCT, API_LIST, API_UPLOAD, CHUNK_SIZE, SIZE_LIMIT,
};

#[derive(Clone, Copy)]
struct BaselineEntry {
    mtime_ms: u64,
    size: u64,
}

#[derive(Default)]
struct AppState {
    key: Option<SyncKey>,
    directory: Option<wasm_bindgen::JsValue>,
    baseline: HashMap<String, BaselineEntry>,
    generation: u64,
    poll_in_flight: bool,
}

impl AppState {
    fn clear_key(&mut self) {
        if let Some(mut key) = self.key.take() {
            key.zeroize();
        }
    }

    fn configured(&self) -> bool {
        self.key.is_some() && self.directory.is_some()
    }
}

thread_local! {
    static STATE: RefCell<AppState> = RefCell::new(AppState::default());
}

pub fn start() -> Result<(), String> {
    attach_main_controls()?;
    attach_emergency_controls()?;
    attach_hash_listener()?;
    show_panel_for_hash();

    if directory_picker_available() {
        set_status("Set passphrase and choose a folder");
    } else {
        set_status("error — folder access needs Chrome or Edge");
        button("choose-folder")?.set_disabled(true);
    }
    Ok(())
}

fn attach_main_controls() -> Result<(), String> {
    on("set-passphrase", "click", || {
        spawn_local(async { set_passphrase().await });
    })?;
    on("choose-folder", "click", || {
        spawn_local(async { select_folder().await });
    })?;
    on("clear-key", "click", clear_key)?;
    on("diagnostics", "click", open_diagnostics)?;
    Ok(())
}

fn attach_emergency_controls() -> Result<(), String> {
    on("ep-pass", "input", update_emergency_button)?;
    on("ep-confirm", "input", update_emergency_button)?;
    on("ep-btn", "click", || {
        spawn_local(async { trigger_destruct().await });
    })?;
    Ok(())
}

fn attach_hash_listener() -> Result<(), String> {
    let callback = Closure::<dyn FnMut(Event)>::new(|_| show_panel_for_hash());
    window()?
        .add_event_listener_with_callback("hashchange", callback.as_ref().unchecked_ref())
        .map_err(js_message)?;
    callback.forget();
    Ok(())
}

fn on(id: &str, event: &str, mut action: impl FnMut() + 'static) -> Result<(), String> {
    let callback = Closure::<dyn FnMut(Event)>::new(move |_| action());
    element(id)?
        .add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())
        .map_err(js_message)?;
    callback.forget();
    Ok(())
}

fn js_message(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser event error: {value:?}"))
}

fn show_panel_for_hash() {
    let emergency = window()
        .and_then(|w| w.location().hash().map_err(js_message))
        .map(|hash| hash == "#emergency")
        .unwrap_or(false);
    set_hidden("sync-panel", emergency);
    set_hidden("emergency-panel", !emergency);
}

async fn set_passphrase() {
    let Ok(field) = input("passphrase") else {
        return;
    };
    let mut passphrase = field.value();
    if passphrase.is_empty() {
        set_status("error — passphrase is empty");
        return;
    }

    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.clear_key();
        state.baseline.clear();
        state.generation = state.generation.wrapping_add(1);
    });
    set_status("deriving key");
    TimeoutFuture::new(1).await;
    let key = derive_sync_key(&passphrase);
    passphrase.zeroize();
    field.set_value("");

    set_status("checking key");
    let headers = match list_headers(&key) {
        Ok(headers) => headers,
        Err(error) => {
            reject_key(key, &error);
            return;
        }
    };
    match request_status("GET", API_LIST, &headers, None).await {
        Ok(200) => {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                state.clear_key();
                state.key = Some(key);
                state.baseline.clear();
                state.generation = state.generation.wrapping_add(1);
            });
            log("Key verified and set for this tab");
            set_status("idle");
            restart_polling();
        }
        Ok(401) => reject_key(key, "passphrase does not match the unlocked vault"),
        Ok(503) => reject_key(key, "vault is locked"),
        Ok(status) => reject_key(key, &format!("key check failed: HTTP {status}")),
        Err(error) => reject_key(key, &format!("key check failed: {error}")),
    }
}

fn reject_key(mut key: SyncKey, reason: &str) {
    key.zeroize();
    set_status("error — key not accepted");
    log(&format!("✗ {reason}"));
}

async fn select_folder() {
    match choose_directory().await {
        Ok(directory) => {
            let name = directory_name(&directory);
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                state.directory = Some(directory);
                state.baseline.clear();
                state.generation = state.generation.wrapping_add(1);
            });
            set_text("folder-label", &format!("Folder: {name}"));
            log("Folder selected");
            set_status("idle");
            restart_polling();
        }
        Err(error) => {
            // A picker cancellation is not exceptional, but browsers expose it as a rejected
            // promise with implementation-specific text. Keep the UI usable and record it briefly.
            log(&format!("Folder selection cancelled: {error}"));
        }
    }
}

fn clear_key() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.clear_key();
        state.baseline.clear();
        state.generation = state.generation.wrapping_add(1);
    });
    if let Ok(field) = input("passphrase") {
        field.set_value("");
    }
    set_status("Key cleared — set passphrase to resume");
    log("Key cleared from browser memory");
}

fn restart_polling() {
    let generation = STATE.with(|state| {
        let state = state.borrow();
        state.configured().then_some(state.generation)
    });
    if let Some(generation) = generation {
        spawn_local(poll_loop(generation));
    }
}

async fn poll_loop(generation: u64) {
    loop {
        let (active, acquired) = STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.generation != generation || !state.configured() {
                return (false, false);
            }
            if state.poll_in_flight {
                (true, false)
            } else {
                state.poll_in_flight = true;
                (true, true)
            }
        });
        if !active {
            return;
        }

        if acquired {
            poll_once(generation).await;
            STATE.with(|state| state.borrow_mut().poll_in_flight = false);
        }

        if !STATE.with(|state| {
            let state = state.borrow();
            state.generation == generation && state.configured()
        }) {
            return;
        }
        TimeoutFuture::new(poll_interval_ms()).await;
    }
}

async fn poll_once(generation: u64) {
    let Some((key, directory, baseline)) = STATE.with(|state| {
        let state = state.borrow();
        Some((state.key?, state.directory.clone()?, state.baseline.clone()))
    }) else {
        return;
    };

    let current = match scan_directory(&directory).await {
        Ok(files) => files,
        Err(error) => {
            status_if_current(generation, "error — folder scan failed");
            log(&format!("✗ folder scan failed: {error}"));
            return;
        }
    };

    let changes: Vec<(String, FileEntry)> = current
        .iter()
        .filter(|(path, entry)| {
            baseline
                .get(*path)
                .is_none_or(|old| old.mtime_ms != entry.mtime_ms || old.size != entry.size)
        })
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect();
    if changes.is_empty() {
        status_if_current(generation, "idle");
        return;
    }

    status_if_current(generation, "syncing");
    let mut failed = 0usize;
    for (path, entry) in changes {
        if !is_current(generation) {
            return;
        }
        match upload_file(&key, &path, &entry).await {
            Ok(()) => {
                STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    if state.generation == generation {
                        state.baseline.insert(
                            path.clone(),
                            BaselineEntry {
                                mtime_ms: entry.mtime_ms,
                                size: entry.size,
                            },
                        );
                    }
                });
            }
            Err(error) => {
                failed += 1;
                log(&format!("✗ {path}: {error}"));
            }
        }
    }

    if failed == 0 {
        status_if_current(generation, "idle");
    } else {
        status_if_current(generation, &format!("error — {failed} file(s) not synced"));
    }
}

fn is_current(generation: u64) -> bool {
    STATE.with(|state| state.borrow().generation == generation)
}

fn status_if_current(generation: u64, status: &str) {
    if is_current(generation) {
        set_status(status);
    }
}

async fn upload_file(key: &SyncKey, path: &str, entry: &FileEntry) -> Result<(), String> {
    if entry.size > SIZE_LIMIT {
        // Treat this as an explicit, stable skip so it is not retried every two seconds.
        log(&format!("Skip (>500 MiB): {path}"));
        return Ok(());
    }

    let file_id = random_file_id()?;
    let total_chunks = entry.size.div_ceil(CHUNK_SIZE).max(1);
    let mut hasher = Sha256::new();

    for index in 0..total_chunks {
        let start = index * CHUNK_SIZE;
        let end = ((index + 1) * CHUNK_SIZE).min(entry.size);
        let chunk = read_file_slice(&entry.file, start, end).await?;
        hasher.update(&chunk);
        let request = chunk_request(key, &file_id, index, &chunk)?;
        let status =
            request_status("POST", API_UPLOAD, &request.headers, Some(&request.body)).await?;
        if status != 200 {
            return Err(format!(
                "chunk {index}/{total_chunks} returned HTTP {status}"
            ));
        }
        if total_chunks > 1 {
            set_status(&format!("syncing {path}: {}/{total_chunks}", index + 1));
        }
    }

    let digest = format!("{:x}", hasher.finalize());
    let request = finalize_request(key, &file_id, path, &digest, total_chunks)?;
    let status = request_status("POST", API_UPLOAD, &request.headers, Some(&request.body)).await?;
    if status != 200 {
        return Err(format!("finalize returned HTTP {status}"));
    }
    log(&format!("↑ {path}"));
    Ok(())
}

fn update_emergency_button() {
    let ready = input("ep-pass")
        .ok()
        .zip(input("ep-confirm").ok())
        .is_some_and(|(pass, confirm)| !pass.value().is_empty() && confirm.value() == "DESTROY");
    if let Ok(button) = button("ep-btn") {
        button.set_disabled(!ready);
    }
}

async fn trigger_destruct() {
    let Ok(pass) = input("ep-pass") else {
        return;
    };
    let Ok(confirm) = input("ep-confirm") else {
        return;
    };
    let Ok(execute) = button("ep-btn") else {
        return;
    };
    if pass.value().is_empty() || confirm.value() != "DESTROY" {
        return;
    }

    pass.set_disabled(true);
    confirm.set_disabled(true);
    execute.set_disabled(true);
    set_text("ep-status", "Scheduling destruction…");
    set_class("ep-status", "warning");

    let mut destruct_passphrase = pass.value();
    let body =
        serde_json::to_vec(&serde_json::json!({ "passphrase": destruct_passphrase.as_str() }))
            .unwrap_or_default();
    destruct_passphrase.zeroize();
    pass.set_value("");
    let headers = vec![("content-type".to_string(), "application/json".to_string())];
    match request_status("POST", API_DESTRUCT, &headers, Some(&body)).await {
        Ok(202) => {
            set_text("ep-status", "✓ Destruction scheduled");
            set_class("ep-status", "success");
        }
        Ok(429) => {
            set_text("ep-status", "✗ Rate limited. Wait 60 seconds.");
            set_class("ep-status", "warning");
            pass.set_disabled(false);
            confirm.set_disabled(false);
            update_emergency_button();
        }
        Ok(404) => {
            set_text("ep-status", "✗ Incorrect passphrase or not configured");
            set_class("ep-status", "warning");
            pass.set_disabled(false);
            confirm.set_disabled(false);
            update_emergency_button();
        }
        Ok(status) => {
            set_text(
                "ep-status",
                &format!("✗ Destruction could not start (HTTP {status})"),
            );
            set_class("ep-status", "error");
            pass.set_disabled(false);
            confirm.set_disabled(false);
            update_emergency_button();
        }
        Err(error) => {
            set_text("ep-status", &format!("✗ Connection failed: {error}"));
            set_class("ep-status", "error");
            pass.set_disabled(false);
            confirm.set_disabled(false);
            update_emergency_button();
        }
    }
}

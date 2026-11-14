use std::collections::HashMap;

use gloo_timers::callback::Timeout;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, Document, Element, File, HtmlButtonElement, HtmlInputElement, Request,
    RequestInit, Response, Window,
};

const REQUEST_TIMEOUT_MS: u32 = 30_000;

#[derive(Clone)]
pub struct FileEntry {
    pub mtime_ms: u64,
    pub size: u64,
    pub file: File,
}

pub fn window() -> Result<Window, String> {
    web_sys::window().ok_or_else(|| "browser window is unavailable".to_string())
}

pub fn document() -> Result<Document, String> {
    window()?
        .document()
        .ok_or_else(|| "browser document is unavailable".to_string())
}

pub fn element(id: &str) -> Result<Element, String> {
    document()?
        .get_element_by_id(id)
        .ok_or_else(|| format!("missing page element #{id}"))
}

pub fn input(id: &str) -> Result<HtmlInputElement, String> {
    element(id)?
        .dyn_into::<HtmlInputElement>()
        .map_err(|_| format!("#{id} is not an input"))
}

pub fn button(id: &str) -> Result<HtmlButtonElement, String> {
    element(id)?
        .dyn_into::<HtmlButtonElement>()
        .map_err(|_| format!("#{id} is not a button"))
}

pub fn set_text(id: &str, text: &str) {
    if let Ok(el) = element(id) {
        el.set_text_content(Some(text));
    }
}

pub fn set_class(id: &str, class_name: &str) {
    if let Ok(el) = element(id) {
        el.set_class_name(class_name);
    }
}

pub fn set_hidden(id: &str, hidden: bool) {
    if let Ok(el) = element(id) {
        let _ = if hidden {
            el.set_attribute("hidden", "")
        } else {
            el.remove_attribute("hidden")
        };
    }
}

pub fn set_status(status: &str) {
    let class = match status {
        "idle" => "dot ok",
        "syncing" | "deriving key" | "checking key" => "dot sync",
        s if s.starts_with("error") => "dot err",
        _ => "dot warn",
    };
    set_class("status-dot", class);
    set_text("status-text", status);
}

pub fn log(message: &str) {
    let Ok(container) = element("activity") else {
        return;
    };
    let Ok(entry) = document().and_then(|d| d.create_element("div").map_err(js_error)) else {
        return;
    };
    entry.set_class_name("log");
    let stamp = js_sys::Date::new_0()
        .to_locale_time_string("en-GB")
        .as_string()
        .unwrap_or_default();
    entry.set_text_content(Some(&format!("{stamp} {message}")));
    let _ = container.prepend_with_node_1(&entry);
    let children = container.children();
    while children.length() > 50 {
        if let Some(last) = children.item(children.length() - 1) {
            let _ = container.remove_child(&last);
        } else {
            break;
        }
    }
}

pub fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

pub fn poll_interval_ms() -> u32 {
    if document().map(|d| d.hidden()).unwrap_or(false) {
        crate::protocol::POLL_BACKGROUND_MS
    } else {
        crate::protocol::POLL_FOCUSED_MS
    }
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    window()?
        .crypto()
        .map_err(js_error)?
        .get_random_values_with_u8_array(&mut bytes)
        .map_err(js_error)?;
    Ok(bytes)
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| format!("browser API error: {value:?}"))
}

fn method(target: &JsValue, name: &str) -> Result<Function, String> {
    Reflect::get(target, &JsValue::from_str(name))
        .map_err(js_error)?
        .dyn_into::<Function>()
        .map_err(|_| format!("browser object has no {name}() method"))
}

fn call0(target: &JsValue, name: &str) -> Result<JsValue, String> {
    method(target, name)?.call0(target).map_err(js_error)
}

async fn await_value(value: JsValue) -> Result<JsValue, String> {
    JsFuture::from(Promise::resolve(&value))
        .await
        .map_err(js_error)
}

pub fn directory_picker_available() -> bool {
    window()
        .ok()
        .and_then(|w| Reflect::get(w.as_ref(), &JsValue::from_str("showDirectoryPicker")).ok())
        .is_some_and(|v| v.is_function())
}

pub async fn choose_directory() -> Result<JsValue, String> {
    let window = window()?;
    let picker = method(window.as_ref(), "showDirectoryPicker")?;
    let options = Object::new();
    Reflect::set(
        &options,
        &JsValue::from_str("mode"),
        &JsValue::from_str("read"),
    )
    .map_err(js_error)?;
    let pending = picker
        .call1(window.as_ref(), &options.into())
        .map_err(js_error)?;
    await_value(pending).await
}

pub fn directory_name(handle: &JsValue) -> String {
    Reflect::get(handle, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "selected folder".to_string())
}

pub async fn scan_directory(root: &JsValue) -> Result<HashMap<String, FileEntry>, String> {
    let mut files = HashMap::new();
    let mut dirs = vec![(root.clone(), String::new())];
    while let Some((dir, prefix)) = dirs.pop() {
        let iterator = call0(&dir, "entries")?;
        loop {
            let step = await_value(call0(&iterator, "next")?).await?;
            if Reflect::get(&step, &JsValue::from_str("done"))
                .map_err(js_error)?
                .as_bool()
                .unwrap_or(false)
            {
                break;
            }
            let value = Reflect::get(&step, &JsValue::from_str("value")).map_err(js_error)?;
            let pair = Array::from(&value);
            let Some(name) = pair.get(0).as_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let handle = pair.get(1);
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = Reflect::get(&handle, &JsValue::from_str("kind"))
                .map_err(js_error)?
                .as_string()
                .unwrap_or_default();
            match kind.as_str() {
                "file" => {
                    let file = await_value(call0(&handle, "getFile")?)
                        .await?
                        .dyn_into::<File>()
                        .map_err(|_| format!("{path}: getFile() did not return a File"))?;
                    files.insert(
                        path,
                        FileEntry {
                            mtime_ms: file.last_modified() as u64,
                            size: file.size() as u64,
                            file,
                        },
                    );
                }
                "directory" => dirs.push((handle, path)),
                _ => {}
            }
        }
    }
    Ok(files)
}

pub async fn read_file_slice(file: &File, start: u64, end: u64) -> Result<Vec<u8>, String> {
    let blob = file
        .slice_with_f64_and_f64(start as f64, end as f64)
        .map_err(js_error)?;
    let buffer = JsFuture::from(blob.array_buffer())
        .await
        .map_err(js_error)?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

pub async fn request_status(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<u16, String> {
    let controller = AbortController::new().map_err(js_error)?;
    let init = RequestInit::new();
    init.set_method(method);
    init.set_signal(Some(&controller.signal()));
    let body_array = body.map(Uint8Array::from);
    if let Some(bytes) = &body_array {
        init.set_body(&bytes.clone().into());
    }
    let request = Request::new_with_str_and_init(url, &init).map_err(js_error)?;
    for (name, value) in headers {
        request.headers().set(name, value).map_err(js_error)?;
    }

    let abort = controller.clone();
    let timeout = Timeout::new(REQUEST_TIMEOUT_MS, move || abort.abort());
    let response = JsFuture::from(window()?.fetch_with_request(&request))
        .await
        .map_err(js_error)?
        .dyn_into::<Response>()
        .map_err(|_| "fetch did not return a Response".to_string());
    drop(timeout);
    Ok(response?.status())
}

pub fn open_diagnostics() {
    if let Ok(window) = window() {
        let _ = window.open_with_url_and_target(crate::protocol::API_DOCTOR, "_blank");
    }
}

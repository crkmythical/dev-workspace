//! Browser UI compiled to WebAssembly.
//!
//! All application behavior lives in Rust. `wasm-bindgen` generates the small JavaScript loader
//! required by browsers; there is no TypeScript application or JavaScript business logic.

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod protocol;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    app::start().map_err(|e| JsValue::from_str(&e))
}

// Keep the crate buildable on the host as part of `cargo test --workspace`; the browser
// implementation and its dependencies are target-specific and compile only for wasm32.
#[cfg(not(target_arch = "wasm32"))]
pub fn wasm_only() {}

//! solana-program Javascript interface
#![cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

pub mod instructions;
#[deprecated(since = "2.2.0", note = "Use `solana_system_interface` crate instead")]
pub use solana_system_interface::wasm as system_instruction;

/// Initialize Javascript logging and panic handler
#[wasm_bindgen]
pub fn solana_program_init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).unwrap();
    });
}

pub fn display_to_jsvalue<T: std::fmt::Display>(display: T) -> JsValue {
    display.to_string().into()
}

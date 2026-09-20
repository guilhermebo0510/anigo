//! Stub implementation for non-Windows platforms.
//! All functions return Err("unsupported platform") to allow `cargo build` on Linux/macOS.

use anyhow::{bail, Result};
use image::{ImageBuffer, Rgba};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub struct Win32Harness;

impl Win32Harness {
    pub fn attach_interactive_desktop() {}

    pub fn find_anigo_window() -> Result<(*mut std::ffi::c_void, String, RECT, bool)> {
        bail!("Win32 window automation is not supported on this platform (requires Windows)")
    }

    pub fn find_anigo_window_with_hint(
        _hint: Option<*mut std::ffi::c_void>,
    ) -> Result<(*mut std::ffi::c_void, String, RECT, bool)> {
        bail!("Win32 window automation is not supported on this platform (requires Windows)")
    }

    pub fn ensure_window_active(_hwnd: *mut std::ffi::c_void) -> Result<RECT> {
        bail!("Win32 window automation is not supported on this platform")
    }

    pub fn maximize_window(_hwnd: *mut std::ffi::c_void) -> Result<RECT> {
        bail!("Win32 window automation is not supported on this platform")
    }

    pub fn restore_window(_hwnd: *mut std::ffi::c_void) -> Result<RECT> {
        bail!("Win32 window automation is not supported on this platform")
    }

    pub fn capture_window(
        _hwnd: *mut std::ffi::c_void,
        _rect: &RECT,
    ) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
        bail!("Win32 screenshot is not supported on this platform")
    }

    pub fn mouse_click(_screen_x: i32, _screen_y: i32, _button: &str) {
        // no-op on non-Windows; caller will have already errored via find_anigo_window
    }

    pub fn mouse_drag(
        _start_x: i32,
        _start_y: i32,
        _end_x: i32,
        _end_y: i32,
        _button: &str,
        _steps: u32,
    ) {
    }

    pub fn mouse_wheel(_delta: i32) {}

    pub fn send_key(_vk: u8) {}

    pub fn collect_logs_and_diagnostics() -> Value {
        serde_json::json!({
            "launch_log": "Win32 diagnostics not supported on this platform",
            "app_crash_log": "Win32 diagnostics not supported on this platform",
            "panic_log": "Win32 diagnostics not supported on this platform",
            "bridge_port_39090_active": false,
            "platform": std::env::consts::OS,
            "note": "Run on Windows for full diagnostics"
        })
    }
}

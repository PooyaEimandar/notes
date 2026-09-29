//! What the scene tells the page. Outside the browser these do nothing.
//!
//! Events, dispatched on `window` with a number as their detail:
//!
//! - `notes:ready`   the renderer has started
//! - `notes:scene`   the scene file was loaded; the detail is the note count
//! - `notes:select`  a note was selected; -1 when the selection was cleared
//! - `notes:hover`   the pointer is over a note; -1 when it is over none

use sib::render::RenderContext;

#[cfg(target_arch = "wasm32")]
pub fn emit(name: &str, detail: f64) {
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&wasm_bindgen::JsValue::from_f64(detail));
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(name, &init)
        && let Some(window) = web_sys::window()
    {
        let _ = window.dispatch_event(&event);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn emit(_name: &str, _detail: f64) {}

#[cfg(target_arch = "wasm32")]
pub fn report(message: &str) {
    web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(message));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn report(message: &str) {
    eprintln!("{message}");
}

/// Moves the canvas that winit created into the page's scene element.
#[cfg(target_arch = "wasm32")]
pub fn mount_canvas(context: &RenderContext) {
    use sib::render::winit::platform::web::WindowExtWebSys;

    let Some(canvas) = context.window.canvas() else {
        return;
    };
    canvas.set_id("webgpu-canvas");
    let _ = canvas.set_attribute("aria-hidden", "true");
    let _ = canvas.set_attribute("tabindex", "-1");
    if let Some(mount) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("scene"))
    {
        let _ = mount.insert_before(&canvas, mount.first_child().as_ref());
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mount_canvas(_context: &RenderContext) {}

#[cfg(target_arch = "wasm32")]
pub fn set_cursor(context: &RenderContext, cursor: &str) {
    use sib::render::winit::platform::web::WindowExtWebSys;

    if let Some(canvas) = context.window.canvas() {
        let _ = canvas.style().set_property("cursor", cursor);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set_cursor(_context: &RenderContext, _cursor: &str) {}

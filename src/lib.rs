//! The WebGPU scene of the notes, rendered with Sib.
//!
//! `web/app.ts` loads this module, hands it `data/scene.bin`, and drives it
//! through the functions exported below.

pub mod app;
pub mod bridge;
pub mod data;
pub mod gpu;
pub mod orbit;

#[cfg(target_arch = "wasm32")]
mod exports {
    use wasm_bindgen::prelude::wasm_bindgen;

    use crate::app::{self, Command, Notes};

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), wasm_bindgen::JsValue> {
        sib::render::run(Notes::default())
            .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
    }

    /// Loads `data/scene.bin`.
    #[wasm_bindgen]
    pub fn load_scene(bytes: &[u8]) {
        app::send(Command::LoadScene(bytes.to_vec()));
    }

    /// Lights only these notes and dims the rest.
    #[wasm_bindgen]
    pub fn set_matches(ids: &[u32]) {
        app::send(Command::Matches(Some(ids.to_vec())));
    }

    /// Lights every note.
    #[wasm_bindgen]
    pub fn clear_matches() {
        app::send(Command::Matches(None));
    }

    /// Selects a note and flies to it. A negative number clears the selection.
    #[wasm_bindgen]
    pub fn select(id: i32) {
        app::send(Command::Select(u32::try_from(id).ok()));
    }

    #[wasm_bindgen]
    pub fn set_reduced_motion(reduced: bool) {
        app::send(Command::ReducedMotion(reduced));
    }

    /// Stops drawing while the list is shown.
    #[wasm_bindgen]
    pub fn set_paused(paused: bool) {
        app::send(Command::Paused(paused));
    }

    /// Tells the scene how much of the canvas the interface covers, in CSS
    /// pixels, so it can centre itself in what is left.
    #[wasm_bindgen]
    pub fn set_insets(top: f32, right: f32, bottom: f32, left: f32) {
        app::send(Command::Insets([top, right, bottom, left]));
    }

    /// Five numbers per label: note, x, y, radius, flags. Positions are in CSS
    /// pixels from the top left of the canvas. Flags: 1 hovered, 2 selected.
    #[wasm_bindgen]
    pub fn labels() -> Vec<f32> {
        app::labels()
    }
}

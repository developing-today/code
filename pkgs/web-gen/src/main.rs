use color_eyre::eyre::Result;
use js_sys::try_iter;
use tracing::info;

pub use web_gen_ui::{App, DetailedFormExample, ProgressBar, RandomMultiplierForm, TestApp};

// wasm/leptos doesn't run these things right.. need multiple builds?
// need to get back to buck2 build setups i think, but cargo might support
// build and serve wasm from one and build and serve apiserver from another?
// cargo add axum tokio poem-openapi
fn main() {
    // color_eyre::install().unwrap();
    // info!("Starting app");
    leptos::mount::mount_to_body(App)
}

//! Facade contract test — crate split step 3 (#8969).
//!
//! The `start-axum` facade keeps the pre-split public paths working while the
//! implementation lives in `start-axum-ui` / `start-axum-error`. This test
//! references each pre-split path through the facade, so a dropped or renamed
//! re-export fails the suite at compile time.
//!
//! Gated on `ssr`, matching the step gates: the `fileserv` handler only exists
//! under `ssr`, and the hydrate build is covered separately by
//! `cargo check --locked --features hydrate`.
#![cfg(feature = "ssr")]

use start_axum::error_template::AppError;

#[test]
fn facade_paths_resolve() {
    // Value-position references to the component/handler items re-exported
    // through the facade.
    let _app = start_axum::app::App;
    let _counter = start_axum::app::SimpleCounter;
    let _toggle = start_axum::dark_mode::DarkModeToggle;
    let _template = start_axum::error_template::ErrorTemplate;
    let _handler = start_axum::fileserv::file_and_error_handler;

    // Type-position reference to the error type re-exported through the facade.
    let _err: Option<AppError> = None;
}
